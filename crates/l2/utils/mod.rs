use crate::utils::eth_client::errors::EthClientError;
use ethrex_core::Address;
use keccak_hash::{keccak, H256};
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

pub fn get_address_from_secret_key(secret_key: &SecretKey) -> Result<Address, EthClientError> {
    let public_key = secret_key
        .public_key(secp256k1::SECP256K1)
        .serialize_uncompressed();
    let hash = keccak(&public_key[1..]);

    // Get the last 20 bytes of the hash
    let address_bytes: [u8; 20] = hash
        .as_ref()
        .get(12..32)
        .ok_or(EthClientError::Custom(
            "Failed to get_address_from_secret_key: error slicing address_bytes".to_owned(),
        ))?
        .try_into()
        .map_err(|err| {
            EthClientError::Custom(format!("Failed to get_address_from_secret_key: {err}"))
        })?;

    Ok(Address::from(address_bytes))
}
