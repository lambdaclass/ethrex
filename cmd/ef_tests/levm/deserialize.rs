use crate::types::{EFTest, EFTestAccessListItem, EFTests, TransactionExpectedException};
use bytes::Bytes;
use ethrex_core::{H256, U256};
use serde::{Deserialize, Deserializer};
use std::{collections::HashMap, str::FromStr};

use crate::types::{EFTestRawTransaction, EFTestTransaction};

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
            .map(|s| match s.trim() {
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
                "TransactionException.SENDER_NOT_EOA" => TransactionExpectedException::SenderNotEoa,
                "TransactionException.PRIORITY_GREATER_THAN_MAX_FEE_PER_GAS" => {
                    TransactionExpectedException::PriorityGreaterThanMaxFeePerGas
                }
                "TransactionException.GAS_ALLOWANCE_EXCEEDED" => {
                    TransactionExpectedException::GasAllowanceExceeded
                }
                "TransactionException.INSUFFICIENT_MAX_FEE_PER_GAS" => {
                    TransactionExpectedException::InsufficientMaxFeePerGas
                }
                "TransactionException.RLP_INVALID_VALUE" => {
                    TransactionExpectedException::RlpInvalidValue
                }
                "TransactionException.GASLIMIT_PRICE_PRODUCT_OVERFLOW" => {
                    TransactionExpectedException::GasLimitPriceProductOverflow
                }
                "TransactionException.TYPE_3_TX_PRE_FORK" => {
                    TransactionExpectedException::Type3TxPreFork
                }
                "TransactionException.INSUFFICIENT_MAX_FEE_PER_BLOB_GAS" => {
                    TransactionExpectedException::InsufficientMaxFeePerBlobGas
                }
                other => panic!("Unexpected error type: {}", other), // Should not fail, TODO is to return an error
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
) -> Result<Option<Vec<Vec<EFTestAccessListItem>>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let access_lists: Option<Vec<Option<Vec<EFTestAccessListItem>>>> =
        Option::<Vec<Option<Vec<EFTestAccessListItem>>>>::deserialize(deserializer)?;

    let mut final_access_lists: Vec<Vec<EFTestAccessListItem>> = Vec::new();

    if let Some(access_lists) = access_lists {
        for access_list in access_lists {
            // Treat `null` as an empty vector
            final_access_lists.push(access_list.unwrap_or_default());
        }
    }

    Ok(Some(final_access_lists))
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

impl<'de> Deserialize<'de> for EFTests {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let mut ef_tests = Vec::new();
        let aux: HashMap<String, HashMap<String, serde_json::Value>> =
            HashMap::deserialize(deserializer)?;

        for test_name in aux.keys() {
            let test_data = aux
                .get(test_name)
                .ok_or(serde::de::Error::missing_field("test data value"))?;

            let raw_tx: EFTestRawTransaction = serde_json::from_value(
                test_data
                    .get("transaction")
                    .ok_or(serde::de::Error::missing_field("transaction"))?
                    .clone(),
            )
            .map_err(|err| {
                serde::de::Error::custom(format!(
                    "error deserializing test \"{test_name}\", \"transaction\" field: {err}"
                ))
            })?;

            let mut transactions = HashMap::new();

            // Note that in this order of iteration, in an example tx with 2 datas, 2 gasLimit and 2 values, order would be
            // 111, 112, 121, 122, 211, 212, 221, 222
            for (data_id, data) in raw_tx.data.iter().enumerate() {
                for (gas_limit_id, gas_limit) in raw_tx.gas_limit.iter().enumerate() {
                    for (value_id, value) in raw_tx.value.iter().enumerate() {
                        let tx = EFTestTransaction {
                            data: data.clone(),
                            gas_limit: *gas_limit,
                            gas_price: raw_tx.gas_price,
                            nonce: raw_tx.nonce,
                            secret_key: raw_tx.secret_key,
                            sender: raw_tx.sender,
                            to: raw_tx.to.clone(),
                            value: *value,
                            blob_versioned_hashes: raw_tx
                                .blob_versioned_hashes
                                .clone()
                                .unwrap_or_default(),
                            max_fee_per_blob_gas: raw_tx.max_fee_per_blob_gas,
                            max_priority_fee_per_gas: raw_tx.max_priority_fee_per_gas,
                            max_fee_per_gas: raw_tx.max_fee_per_gas,
                            access_list: raw_tx
                                .access_lists
                                .clone()
                                .unwrap_or_default()
                                .get(data_id)
                                .cloned()
                                .unwrap_or_default(),
                        };
                        transactions.insert((data_id, gas_limit_id, value_id), tx);
                    }
                }
            }

            let ef_test = EFTest {
                name: test_name.to_owned().to_owned(),
                dir: String::default(),
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
                transactions,
            };
            ef_tests.push(ef_test);
        }
        Ok(Self(ef_tests))
    }
}
