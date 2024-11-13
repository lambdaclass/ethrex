use keccak_hash::H256;
use secp256k1::SecretKey;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub mod config;
pub mod eth_client;
pub mod merkle_tree;
pub mod test_data_io;

pub fn secret_key_deserializer<'de, D>(deserializer: D) -> Result<SecretKey, D::Error>
where
    D: Deserializer<'de>,
{
    let hex = H256::deserialize(deserializer)?;
    SecretKey::from_slice(hex.as_bytes()).map_err(serde::de::Error::custom)
}

pub fn secret_key_serializer<S>(secret_key: &SecretKey, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let hex = H256::from_slice(&secret_key.secret_bytes());
    hex.serialize(serializer)
}
