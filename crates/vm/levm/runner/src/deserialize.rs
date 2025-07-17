use std::collections::HashMap;

use bytes::Bytes;
use ethrex_common::U256;
use serde::{Deserialize, Deserializer, de};

pub fn deserialize_u64_str<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: Deserializer<'de>,
{
    let s: String = Deserialize::deserialize(deserializer)?;

    if let Some(hex) = s.strip_prefix("0x") {
        u64::from_str_radix(hex, 16).map_err(de::Error::custom)
    } else {
        s.parse::<u64>().map_err(de::Error::custom)
    }
}

pub fn deserialize_u256_str<'de, D>(deserializer: D) -> Result<U256, D::Error>
where
    D: Deserializer<'de>,
{
    let s: Option<String> = Option::deserialize(deserializer)?;

    match s {
        Some(s) => {
            if let Some(hex) = s.strip_prefix("0x") {
                U256::from_str_radix(hex, 16).map_err(de::Error::custom)
            } else {
                U256::from_dec_str(&s).map_err(de::Error::custom)
            }
        }
        None => Ok(U256::default()), // <- returns zero here!
    }
}

fn parse_u256_str(s: &str) -> Result<U256, String> {
    if let Some(hex) = s.strip_prefix("0x") {
        U256::from_str_radix(hex, 16).map_err(|e| e.to_string())
    } else {
        U256::from_dec_str(s).map_err(|e| e.to_string())
    }
}

pub fn deserialize_u256_vec<'de, D>(deserializer: D) -> Result<Vec<U256>, D::Error>
where
    D: Deserializer<'de>,
{
    let vec = Vec::<String>::deserialize(deserializer)?;
    vec.into_iter()
        .map(|s| {
            parse_u256_str(&s)
                .map_err(|err| de::Error::custom(format!("error parsing U256 in vec: {err}")))
        })
        .collect()
}

pub fn deserialize_u64_vec<'de, D>(deserializer: D) -> Result<Vec<u64>, D::Error>
where
    D: Deserializer<'de>,
{
    let vec = Vec::<String>::deserialize(deserializer)?;
    vec.into_iter()
        .map(|s| {
            if let Some(hex) = s.strip_prefix("0x") {
                u64::from_str_radix(hex, 16)
            } else {
                s.parse::<u64>()
            }
            .map_err(|err| de::Error::custom(format!("error parsing u64 in vec: {err}")))
        })
        .collect()
}

pub fn deserialize_u256_valued_hashmap<'de, D>(
    deserializer: D,
) -> Result<HashMap<U256, U256>, D::Error>
where
    D: Deserializer<'de>,
{
    let map = HashMap::<String, String>::deserialize(deserializer)?;
    map.into_iter()
        .map(|(k, v)| {
            let key = parse_u256_str(&k)
                .map_err(|err| de::Error::custom(format!("(key) error parsing U256: {err}")))?;
            let value = parse_u256_str(&v)
                .map_err(|err| de::Error::custom(format!("(value) error parsing U256: {err}")))?;
            Ok((key, value))
        })
        .collect()
}
