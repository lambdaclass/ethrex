use crate::types::EFTest;
use bytes::Bytes;
use ethereum_rust_core::U256;
use serde::Deserialize;
use std::{collections::HashMap, str::FromStr};

pub fn deserialize_ef_post_value_indexes<'de, D>(
    deserializer: D,
) -> Result<HashMap<String, U256>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let aux: HashMap<String, u64> = HashMap::deserialize(deserializer)?;
    let indexes = aux
        .iter()
        .map(|(key, value)| (key.clone(), U256::from(*value)))
        .collect();
    Ok(indexes)
}

pub fn deserialize_hex_bytes<'de, D>(deserializer: D) -> Result<Bytes, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    Ok(Bytes::from(
        hex::decode(s.trim_start_matches("0x")).map_err(|err| {
            serde::de::Error::custom(format!(
                "error decoding hex data when deserializing bytes: {err}"
            ))
        })?,
    ))
}

pub fn deserialize_hex_bytes_vec<'de, D>(deserializer: D) -> Result<Vec<Bytes>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = Vec::<String>::deserialize(deserializer)?;
    let mut ret = Vec::new();
    for s in s {
        ret.push(Bytes::from(
            hex::decode(s.trim_start_matches("0x")).map_err(|err| {
                serde::de::Error::custom(format!(
                    "error decoding hex data when deserializing bytes vec: {err}"
                ))
            })?,
        ));
    }
    Ok(ret)
}

pub fn deserialize_u256_safe<'de, D>(deserializer: D) -> Result<U256, D::Error>
where
    D: serde::Deserializer<'de>,
{
    U256::from_str(String::deserialize(deserializer)?.trim_start_matches("0x:bigint ")).map_err(
        |err| {
            serde::de::Error::custom(format!(
                "error parsing U256 when deserializing U256 safely: {err}"
            ))
        },
    )
}

pub fn deserialize_u256_optional_safe<'de, D>(deserializer: D) -> Result<Option<U256>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = Option::<String>::deserialize(deserializer)?;
    match s {
        Some(s) => U256::from_str(s.trim_start_matches("0x:bigint "))
            .map_err(|err| {
                serde::de::Error::custom(format!(
                    "error parsing U256 when deserializing U256 safely: {err}"
                ))
            })
            .map(Some),
        None => Ok(None),
    }
}

pub fn deserialize_u256_vec_safe<'de, D>(deserializer: D) -> Result<Vec<U256>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Vec::<String>::deserialize(deserializer)?
        .iter()
        .map(|s| {
            U256::from_str(s.trim_start_matches("0x:bigint ")).map_err(|err| {
                serde::de::Error::custom(format!(
                    "error parsing U256 when deserializing U256 safely: {err}"
                ))
            })
        })
        .collect()
}

pub fn deserialize_u256_valued_hashmap_safe<'de, D>(
    deserializer: D,
) -> Result<HashMap<U256, U256>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    HashMap::<String, String>::deserialize(deserializer)?
        .iter()
        .map(|(key, value)| {
            let key = U256::from_str(key.trim_start_matches("0x:bigint ")).map_err(|err| {
                serde::de::Error::custom(format!(
                    "(key) error parsing U256 when deserializing U256 valued hashmap safely: {err}"
                ))
            })?;
            let value = U256::from_str(value.trim_start_matches("0x:bigint ")).map_err(|err| {
                serde::de::Error::custom(format!(
                    "(value) error parsing U256 when deserializing U256 valued hashmap safely: {err}"
                ))
            })?;
            Ok((key, value))
        })
        .collect()
}

impl<'de> Deserialize<'de> for EFTest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let aux: HashMap<String, HashMap<String, serde_json::Value>> =
            HashMap::deserialize(deserializer)?;
        let test_name = aux
            .keys()
            .next()
            .ok_or(serde::de::Error::missing_field("test name"))?;
        let test_data = aux
            .get(test_name)
            .ok_or(serde::de::Error::missing_field("test data value"))?;
        let ef_test = Self {
            name: test_name.to_owned().to_owned(),
            _info: serde_json::from_value(
                test_data
                    .get("_info")
                    .ok_or(serde::de::Error::missing_field("_info"))?
                    .clone(),
            )
            .map_err(|err| {
                serde::de::Error::custom(format!(
                    "error deserializing test \"{test_name}\", \"_info\" field: {err}"
                ))
            })?,
            env: serde_json::from_value(
                test_data
                    .get("env")
                    .ok_or(serde::de::Error::missing_field("env"))?
                    .clone(),
            )
            .map_err(|err| {
                serde::de::Error::custom(format!(
                    "error deserializing test \"{test_name}\", \"env\" field: {err}"
                ))
            })?,
            post: serde_json::from_value(
                test_data
                    .get("post")
                    .ok_or(serde::de::Error::missing_field("post"))?
                    .clone(),
            )
            .map_err(|err| {
                serde::de::Error::custom(format!(
                    "error deserializing test \"{test_name}\", \"post\" field: {err}"
                ))
            })?,
            pre: serde_json::from_value(
                test_data
                    .get("pre")
                    .ok_or(serde::de::Error::missing_field("pre"))?
                    .clone(),
            )
            .map_err(|err| {
                serde::de::Error::custom(format!(
                    "error deserializing test \"{test_name}\", \"pre\" field: {err}"
                ))
            })?,
            transaction: serde_json::from_value(
                test_data
                    .get("transaction")
                    .ok_or(serde::de::Error::missing_field("transaction"))?
                    .clone(),
            )
            .map_err(|err| {
                serde::de::Error::custom(format!(
                    "error deserializing test \"{test_name}\", \"transaction\" field: {err}"
                ))
            })?,
        };
        Ok(ef_test)
    }
}
