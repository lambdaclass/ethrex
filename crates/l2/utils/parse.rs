use std::str::FromStr;

use ethereum_types::{Address, H256};
use reqwest::Url;

pub fn hash_to_address(hash: H256) -> Address {
    Address::from_slice(&hash.as_fixed_bytes()[12..])
}

pub fn deserialize_url<'de, D>(deserializer: D) -> Result<Url, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let url_str: String = serde::Deserialize::deserialize(deserializer)?;
    Url::from_str(&url_str).map_err(|e| serde::de::Error::custom(e))
}

pub fn deserialize_optional_url<'de, D>(deserializer: D) -> Result<Option<Url>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(deserialize_url(deserializer).ok())
}
