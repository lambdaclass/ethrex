use crate::modules::types::{
    AccessListItem, AuthorizationListTuple, RawPostValue, TransactionExpectedException,
};

use ethrex_common::{U256, types::Fork};
use serde::{Deserialize, Deserializer};
use std::collections::HashMap;

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
                    "TransactionException.INTRINSIC_GAS_BELOW_FLOOR_GAS_COST" => {
                        TransactionExpectedException::IntrinsicGasBelowFloorGasCost
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
                    "TransactionException.RLP_INVALID_VALUE" => {
                        TransactionExpectedException::RlpInvalidValue
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
