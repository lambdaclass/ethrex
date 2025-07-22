use crate::runner_v2::types::{
    AccessListItem, AuthorizationListTuple, RawPostValue, TransactionExpectedException,
};
use bytes::Bytes;
use ethrex_common::{H256, U256, types::Fork};
use serde::{Deserialize, Deserializer};
use std::{collections::HashMap, str::FromStr};

pub fn deserialize_transaction_expected_exception<'de, D>(
    deserializer: D,
) -> Result<Option<Vec<TransactionExpectedException>>, D::Error>
where
    D: Deserializer<'de>,
{
    let option: Option<String> = Option::deserialize(deserializer)?;

    if let Some(value) = option {
        let exceptions = value
            .split('|')
            .map(|s| {
                match s.trim() {
                    "TransactionException.INITCODE_SIZE_EXCEEDED" => {
                        TransactionExpectedException::InitcodeSizeExceeded
                    }
                    "TransactionException.NONCE_IS_MAX" => TransactionExpectedException::NonceIsMax,
                    "TransactionException.TYPE_3_TX_BLOB_COUNT_EXCEEDED" => {
                        TransactionExpectedException::Type3TxBlobCountExceeded
                    }
                    "TransactionException.TYPE_3_TX_ZERO_BLOBS" => {
                        TransactionExpectedException::Type3TxZeroBlobs
                    }
                    "TransactionException.TYPE_3_TX_CONTRACT_CREATION" => {
                        TransactionExpectedException::Type3TxContractCreation
                    }
                    "TransactionException.TYPE_3_TX_INVALID_BLOB_VERSIONED_HASH" => {
                        TransactionExpectedException::Type3TxInvalidBlobVersionedHash
                    }
                    "TransactionException.INTRINSIC_GAS_TOO_LOW" => {
                        TransactionExpectedException::IntrinsicGasTooLow
                    }
                    "TransactionException.INSUFFICIENT_ACCOUNT_FUNDS" => {
                        TransactionExpectedException::InsufficientAccountFunds
                    }
                    "TransactionException.SENDER_NOT_EOA" => {
                        TransactionExpectedException::SenderNotEoa
                    }
                    "TransactionException.PRIORITY_GREATER_THAN_MAX_FEE_PER_GAS" => {
                        TransactionExpectedException::PriorityGreaterThanMaxFeePerGas
                    }
                    "TransactionException.GAS_ALLOWANCE_EXCEEDED" => {
                        TransactionExpectedException::GasAllowanceExceeded
                    }
                    "TransactionException.INSUFFICIENT_MAX_FEE_PER_GAS" => {
                        TransactionExpectedException::InsufficientMaxFeePerGas
                    }
                    "TransactionException.GASLIMIT_PRICE_PRODUCT_OVERFLOW" => {
                        TransactionExpectedException::GasLimitPriceProductOverflow
                    }
                    "TransactionException.TYPE_3_TX_PRE_FORK" => {
                        TransactionExpectedException::Type3TxPreFork
                    }
                    "TransactionException.TYPE_4_TX_CONTRACT_CREATION" => {
                        TransactionExpectedException::Type4TxContractCreation
                    }
                    "TransactionException.INSUFFICIENT_MAX_FEE_PER_BLOB_GAS" => {
                        TransactionExpectedException::InsufficientMaxFeePerBlobGas
                    }
                    _other => TransactionExpectedException::Other, //TODO: Support exceptions that enter here.
                }
            })
            .collect();

        Ok(Some(exceptions))
    } else {
        Ok(None)
    }
}

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

/// This serializes a hexadecimal string to u64
pub fn deserialize_u64_safe<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    u64::from_str_radix(
        String::deserialize(deserializer)?.trim_start_matches("0x"),
        16,
    )
    .map_err(|err| {
        serde::de::Error::custom(format!(
            "error parsing U64 when deserializing U64 safely: {err}"
        ))
    })
}

pub fn deserialize_h256_vec_optional_safe<'de, D>(
    deserializer: D,
) -> Result<Option<Vec<H256>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = Option::<Vec<String>>::deserialize(deserializer)?;
    match s {
        Some(s) => {
            let mut ret = Vec::new();
            for s in s {
                ret.push(H256::from_str(s.trim_start_matches("0x")).map_err(|err| {
                    serde::de::Error::custom(format!(
                        "error parsing H256 when deserializing H256 vec optional: {err}"
                    ))
                })?);
            }
            Ok(Some(ret))
        }
        None => Ok(None),
    }
}

pub fn deserialize_access_lists<'de, D>(
    deserializer: D,
) -> Result<Option<Vec<Vec<AccessListItem>>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let access_lists: Option<Vec<Option<Vec<AccessListItem>>>> =
        Option::<Vec<Option<Vec<AccessListItem>>>>::deserialize(deserializer)?;

    let mut final_access_lists: Vec<Vec<AccessListItem>> = Vec::new();

    if let Some(access_lists) = access_lists {
        for access_list in access_lists {
            // Treat `null` as an empty vector
            final_access_lists.push(access_list.unwrap_or_default());
        }
    }

    Ok(Some(final_access_lists))
}

pub fn deserialize_authorization_lists<'de, D>(
    deserializer: D,
) -> Result<Option<Vec<AuthorizationListTuple>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let authorization_list: Option<Vec<AuthorizationListTuple>> =
        Option::<Vec<AuthorizationListTuple>>::deserialize(deserializer)?;

    Ok(authorization_list)
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
                    "error parsing U256 when deserializing U256 vector safely: {err}"
                ))
            })
        })
        .collect()
}
pub fn deserialize_u64_vec_safe<'de, D>(deserializer: D) -> Result<Vec<u64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Vec::<String>::deserialize(deserializer)?
        .iter()
        .map(|s| {
            u64::from_str_radix(s.trim_start_matches("0x"), 16).map_err(|err| {
                serde::de::Error::custom(format!(
                    "error parsing u64 when deserializing u64 vector safely: {err}"
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

pub fn deserialize_post<'de, D>(
    deserializer: D,
) -> Result<HashMap<Fork, Vec<RawPostValue>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let post_deserialized = HashMap::<String, Vec<RawPostValue>>::deserialize(deserializer)?;
    let mut post_parsed = HashMap::new();
    for (fork_str, values) in post_deserialized {
        let fork = match fork_str.as_str() {
            "Frontier" => Fork::Frontier,
            "Homestead" => Fork::Homestead,
            "Constantinople" => Fork::Constantinople,
            "ConstantinopleFix" | "Petersburg" => Fork::Petersburg,
            "Istanbul" => Fork::Istanbul,
            "Berlin" => Fork::Berlin,
            "London" => Fork::London,
            "Paris" | "Merge" => Fork::Paris,
            "Shanghai" => Fork::Shanghai,
            "Cancun" => Fork::Cancun,
            "Prague" => Fork::Prague,
            "Byzantium" => Fork::Byzantium,
            "EIP158" => Fork::SpuriousDragon,
            "EIP150" => Fork::Tangerine,
            other => {
                return Err(serde::de::Error::custom(format!(
                    "Unknown fork name: {other}",
                )));
            }
        };
        post_parsed.insert(fork, values);
    }

    Ok(post_parsed)
}
