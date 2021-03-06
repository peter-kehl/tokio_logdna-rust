use axum::http::StatusCode;
use axum::response::Response;
use axum::Json;
use csv::{ReaderBuilder, StringRecordsIntoIter};
use serde::{Deserialize, Serialize};
// use serde_json::Result as SerdeResult;
use logger::Logger;
use serde_json::json;
use std::collections::HashMap;

pub mod logger;

// @TODO discuss whether to use CamelCase and to_string() transformation.
// But if we use CSV crate, see also why CSV crate shortens "appt_suite_number"
// to "appt_suite_nu" -
// if that is not negotiable, we may want our own field mapping even more.
#[allow(non_camel_case_types)]
#[derive(Debug, Serialize, Deserialize)]
enum AddressType {
    appt,
    house,
    suite,
}

impl ToString for AddressType {
    fn to_string(&self) -> String {
        // @TODO discuss
        match self {
            Self::appt => "appt".to_owned(),
            Self::house => "house".to_owned(),
            Self::suite => "suite".to_owned(),
        }
    }
}

impl TryFrom<&str> for AddressType {
    type Error = StatusCode;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "appt" => Ok(Self::appt),
            "house" => Ok(Self::house),
            "suite" => Ok(Self::suite),
            _ => Err(StatusCode::NOT_ACCEPTABLE),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Address {
    reference: String,
    address_type: AddressType,
    suite_number: Option<String>,
    street_number: u32, //required; @TODO check address standards
    street: String,
    city: String,
    state: String,    // @TODO consider enum
    postcode: String, //to preserve any leading zeros (if allowed - TODO: check address standards if leadnig zeros are allowed). Consider "zipcode" for the US, "postcode" for overseas.
}

fn field_to_column_idx(
    csv_iter: &mut StringRecordsIntoIter<&[u8]>,
) -> Result<HashMap<String, usize>, StatusCode> {
    let mut result_map = HashMap::<String, usize>::new();
    if let Some(header) = csv_iter.next() {
        if header.is_err() {
            // @TODO find better error statuses and include a message if needed; the same below
            return Err(StatusCode::NOT_ACCEPTABLE); // "Missing CSV header."
        } else {
            let header = header.unwrap();
            let column_names_owned = (0..header.len())
                .map(|col_idx| header.get(col_idx))
                .collect::<Vec<_>>();

            if !(column_names_owned
                .iter()
                .all(|col_name_result| col_name_result.is_some()))
            {
                return Err(StatusCode::NOT_ACCEPTABLE);
            }
            column_names_owned
                .iter()
                .map(|col_name_result| col_name_result.unwrap().to_owned())
                .enumerate()
                .for_each(|(col_idx, col_name)| {
                    result_map.insert(col_name, col_idx);
                });
            // @TODO there must be a macro to get a number (and list of) fields of a struct
            if result_map.len() != 8 {
                return Err(StatusCode::NOT_ACCEPTABLE);
            }

            let mut expected_fields = vec![
                "reference",
                "address_type",
                "appt_suite_number",
                "street_number",
                "street",
                "city",
                "state",
                "postcode",
            ];
            expected_fields.sort();
            let mut actual_fields = result_map
                .keys()
                .map(|field_name| field_name)
                .collect::<Vec<_>>();
            actual_fields.sort();
            if expected_fields != actual_fields {
                return Err(StatusCode::NOT_ACCEPTABLE);
            }
        }
    } else {
        // require the header
        return Err(StatusCode::NOT_ACCEPTABLE);
    }
    Ok(result_map)
}

pub async fn addresses_to_result_csv_crate_serde_json(
    logger: Logger,
    bytes: &[u8],
) -> Result<String, StatusCode> {
    let mut addresses = vec![];
    {
        let mut csv_iter;
        {
            let mut reader_builder = ReaderBuilder::default();
            reader_builder.has_headers(false); // counterintuitive: false means "include headers"

            assert!(logger.info("Parsing CSV (with CSV crate).").await.is_ok()); // @TODO return an Err on error instead of panic
            let reader = reader_builder.from_reader(bytes);
            // On nightly we could use #![allow(iterator_try_collect)] and reader.records().try_collect::<Vec<_>>();
            csv_iter = reader.into_records();
        }
        assert!(logger.info("Processing the CSV.").await.is_ok());
        // We accept the CSV independent of its field order. Here we store a map of field names to their CSV field position (0-based).
        let field_to_column_idx = field_to_column_idx(&mut csv_iter)?;

        let col_name_to_idx = |col_name: &str| {
            let idx_option = field_to_column_idx.get(col_name).map(|&idx| idx);
            idx_option
        };

        for csv_record_result in csv_iter {
            if csv_record_result.is_err() {
                return Err(StatusCode::NOT_ACCEPTABLE);
            } else {
                let record = csv_record_result.unwrap();

                let col_name_to_value = |col_name: &str| match col_name_to_idx(col_name) {
                    None => Err(StatusCode::NOT_ACCEPTABLE),
                    Some(idx) => {
                        let value_opt = record.get(idx);
                        value_opt
                            .map(|value| Ok(value))
                            .unwrap_or(Err(StatusCode::NOT_ACCEPTABLE))
                    }
                };

                let address = Address {
                    reference: col_name_to_value("reference")?.to_owned(),
                    address_type: col_name_to_value("address_type")?.try_into()?,
                    suite_number: {
                        match col_name_to_value("appt_suite_number")?.trim() {
                            //@TODO shortened field name - discuss
                            "" => None,
                            suite_number => Some(suite_number.to_owned()),
                        }
                    },
                    street_number: col_name_to_value("street_number")?
                        .parse::<u32>()
                        .or(Err(StatusCode::NOT_ACCEPTABLE))?,
                    street: col_name_to_value("street")?.to_owned(),
                    city: col_name_to_value("city")?.to_owned(),
                    state: col_name_to_value("state")?.to_owned(),
                    postcode: col_name_to_value("postcode")?.to_owned(),
                };
                addresses.push(address);
            }
        }
    }
    assert!(logger.info("Generating JSON.").await.is_ok());
    let json = serde_json::to_string(&addresses);
    match json {
        Ok(json_string) => Ok(json_string),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

fn field_to_column_idx_map<'a>(
    column_names: impl Iterator<Item = &'a str>,
) -> HashMap<&'a str, usize> {
    let mut result = HashMap::new();
    column_names.enumerate().for_each(|(col_idx, col_name)| {
        result.insert(col_name, col_idx);
    });
    result
}

pub fn addresses_to_json(addresses: &Vec<Address>) -> String {
    let address_jsons = addresses
        .iter()
        .map(|address| {
            "{reference: ".to_owned()
                + address.reference.as_str()
                + ", address_type: "
                + address.address_type.to_string().as_str()
                + ", appt_suite_number: "
                + address
                    .suite_number
                    .clone()
                    .map_or("".to_owned(), |suite_number| suite_number.to_string())
                    .as_str()
                + ", street_number: "
                + address.street_number.to_string().as_str()
                + ", street: "
                + address.street.as_str()
                + ", city: "
                + address.city.as_str()
                + ", state: "
                + address.state.as_str()
                + ", postcode: "
                + address.postcode.as_str()
                + "}"
        })
        .collect::<Vec<_>>();

    let address_jsons_joined = address_jsons.join(",\n");
    "[".to_owned() + address_jsons_joined.as_str() + "]"
}

pub async fn addresses_to_result_own_csv_parser_own_json(
    logger: Logger,
    csv_content: String,
) -> Result<String, StatusCode> {
    assert!(logger.info("Parsing CSV (our own parser).").await.is_ok());
    let mut lines = csv_content.lines();
    let header = lines.next();
    if !header.is_some() {
        return Err(StatusCode::NOT_ACCEPTABLE);
    }
    let header = header.unwrap();

    let headings = header.split(',');
    //let column_names = headings.collect::<Vec<_>>();
    let field_to_column_idx = field_to_column_idx_map(headings);
    if field_to_column_idx.len() != 8 {
        //@TODO explore some macro for this
        return Err(StatusCode::NOT_ACCEPTABLE);
    }

    let mut expected_fields = vec![
        "reference",
        "address_type",
        "appt_suite_number",
        "street_number",
        "street",
        "city",
        "state",
        "postcode",
    ];
    expected_fields.sort();
    let mut actual_fields = field_to_column_idx
        .keys()
        .map(|&field_name| field_name)
        .collect::<Vec<_>>();
    actual_fields.sort();
    if expected_fields != actual_fields {
        return Err(StatusCode::NOT_ACCEPTABLE);
    }

    assert!(logger.info("Processing the CSV.").await.is_ok());
    let mut addresses = vec![];
    for line in lines {
        let values = line.split(',').collect::<Vec<_>>();

        let col_name_to_value = |col_name: &str| values[field_to_column_idx[col_name]];

        let address = Address {
            reference: col_name_to_value("reference").to_owned(),
            address_type: col_name_to_value("address_type").try_into()?,
            suite_number: {
                match col_name_to_value("appt_suite_number").trim() {
                    //@TODO shortened field name - discuss
                    "" => None,
                    suite_number => Some(suite_number.to_owned()),
                }
            },
            street_number: col_name_to_value("street_number")
                .parse::<u32>()
                .or(Err(StatusCode::NOT_ACCEPTABLE))?,
            street: col_name_to_value("street").to_owned(),
            city: col_name_to_value("city").to_owned(),
            state: col_name_to_value("state").to_owned(),
            postcode: col_name_to_value("postcode").to_owned(),
        };
        addresses.push(address);
    }

    // @TODO:::
    // serde_json was returning an error, life is too short
    /*let json = serde_json::to_string(&addresses);
    match json {
        Ok(json_string) => Ok(json_string),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }*/
    assert!(logger.info("Generating JSON.").await.is_ok());
    Ok(addresses_to_json(&addresses))
}

#[cfg(test)]
mod test;
