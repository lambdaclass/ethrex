use std::str::FromStr;

use ethereum_types::{Address, H256};
use reqwest::Url;
use serde::Deserializer;

pub fn hash_to_address(hash: H256) -> Address {
    Address::from_slice(&hash.as_fixed_bytes()[12..])
}

pub fn url_deserializer<'de, D>(deserializer: D) -> Result<Url, D::Error>
where
    D: Deserializer<'de>,
{
    let url_str = serde::Deserialize::deserialize(deserializer)?;
    Url::from_str(url_str).map_err(serde::de::Error::custom)
}
