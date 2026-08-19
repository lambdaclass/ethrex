//! Conversion of geth t8n transaction JSON objects into ethrex
//! [`Transaction`]s.
//!
//! Conversion is per-transaction and fallible: a transaction that cannot be
//! represented (unknown type, field overflow, creation on a tx type that
//! forbids it) is reported in the t8n `rejected` output rather than failing
//! the whole run.

use bytes::Bytes;
use ethrex_common::types::tx_fields::{AccessList, AuthorizationList, AuthorizationTuple};
use ethrex_common::types::{
    EIP1559Transaction, EIP2930Transaction, EIP4844Transaction, EIP7702Transaction,
    LegacyTransaction, Transaction, TxKind,
};
use ethrex_common::{Address, H256, U256};
use serde::Deserialize;

/// A t8n input transaction as serialized by test fillers: camelCase field
/// names, `0x`-prefixed hex values, an explicit `"to": null` for contract
/// creation, and the signature in `v`/`r`/`s`. Wide integer types are used
/// so range checks happen in [`to_ethrex_transaction`], where they can
/// produce per-transaction rejections.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TxJson {
    #[serde(rename = "type")]
    pub tx_type: Option<U256>,
    pub chain_id: Option<U256>,
    pub nonce: U256,
    pub gas_price: Option<U256>,
    pub max_priority_fee_per_gas: Option<U256>,
    pub max_fee_per_gas: Option<U256>,
    pub gas: U256,
    pub to: Option<Address>,
    pub value: U256,
    pub input: Option<String>,
    pub access_list: Option<Vec<AccessListItemJson>>,
    pub max_fee_per_blob_gas: Option<U256>,
    pub blob_versioned_hashes: Option<Vec<H256>>,
    pub authorization_list: Option<Vec<AuthorizationTupleJson>>,
    pub v: U256,
    pub r: U256,
    pub s: U256,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AccessListItemJson {
    pub address: Address,
    pub storage_keys: Vec<H256>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AuthorizationTupleJson {
    pub chain_id: U256,
    pub address: Address,
    pub nonce: U256,
    /// Fillers may emit the parity as `v`, `yParity`, or both.
    pub v: Option<U256>,
    pub y_parity: Option<U256>,
    pub r: U256,
    pub s: U256,
}

impl AuthorizationTupleJson {
    fn parity(&self) -> U256 {
        self.v.or(self.y_parity).unwrap_or_default()
    }
}

fn to_u64(value: U256, field: &str) -> Result<u64, String> {
    u64::try_from(value).map_err(|_| format!("transaction {field} exceeds 2^64-1"))
}

fn decode_hex(value: &Option<String>) -> Result<Bytes, String> {
    let Some(raw) = value else {
        return Ok(Bytes::new());
    };
    let raw = raw.strip_prefix("0x").unwrap_or(raw);
    hex::decode(raw)
        .map(Bytes::from)
        .map_err(|e| format!("invalid transaction input hex: {e}"))
}

fn to_tx_kind(to: &Option<Address>) -> TxKind {
    match to {
        Some(address) => TxKind::Call(*address),
        None => TxKind::Create,
    }
}

fn to_call_address(to: &Option<Address>, tx_type: u64) -> Result<Address, String> {
    to.ok_or_else(|| format!("Contract creation in type {tx_type} transaction"))
}

fn y_parity(v: U256) -> Result<bool, String> {
    if v == U256::zero() {
        Ok(false)
    } else if v == U256::one() {
        Ok(true)
    } else {
        Err(format!("invalid signature y parity value: {v}"))
    }
}

fn access_list(items: &Option<Vec<AccessListItemJson>>) -> AccessList {
    items
        .as_deref()
        .unwrap_or_default()
        .iter()
        .map(|item| (item.address, item.storage_keys.clone()))
        .collect()
}

fn authorization_list(
    entries: &Option<Vec<AuthorizationTupleJson>>,
) -> Result<AuthorizationList, String> {
    entries
        .as_deref()
        .unwrap_or_default()
        .iter()
        .map(|entry| {
            Ok(AuthorizationTuple {
                chain_id: entry.chain_id,
                address: entry.address,
                nonce: to_u64(entry.nonce, "authorization nonce")?,
                y_parity: entry.parity(),
                r_signature: entry.r,
                s_signature: entry.s,
            })
        })
        .collect()
}

/// Convert a parsed t8n transaction into an ethrex [`Transaction`].
pub fn to_ethrex_transaction(tx: &TxJson) -> Result<Transaction, String> {
    let tx_type = match tx.tx_type {
        Some(value) => to_u64(value, "type")?,
        None => 0,
    };
    let nonce = to_u64(tx.nonce, "nonce")?;
    let gas = to_u64(tx.gas, "gas limit")?;
    let data = decode_hex(&tx.input)?;
    let chain_id = || -> Result<u64, String> {
        tx.chain_id
            .ok_or_else(|| "transaction is missing chainId".to_string())
            .and_then(|value| {
                u64::try_from(value).map_err(|_| "Transaction has invalid chain id".to_string())
            })
    };
    let gas_price = || -> Result<U256, String> {
        tx.gas_price
            .ok_or_else(|| "transaction is missing gasPrice".to_string())
    };
    let max_fees = || -> Result<(u64, u64), String> {
        let max_priority = tx
            .max_priority_fee_per_gas
            .ok_or_else(|| "transaction is missing maxPriorityFeePerGas".to_string())?;
        let max_fee = tx
            .max_fee_per_gas
            .ok_or_else(|| "transaction is missing maxFeePerGas".to_string())?;
        Ok((
            to_u64(max_priority, "max priority fee per gas")?,
            to_u64(max_fee, "max fee per gas")?,
        ))
    };

    match tx_type {
        0 => Ok(Transaction::LegacyTransaction(LegacyTransaction {
            nonce,
            gas_price: gas_price()?,
            gas,
            to: to_tx_kind(&tx.to),
            value: tx.value,
            data,
            v: tx.v,
            r: tx.r,
            s: tx.s,
            ..Default::default()
        })),
        1 => Ok(Transaction::EIP2930Transaction(EIP2930Transaction {
            chain_id: chain_id()?,
            nonce,
            gas_price: gas_price()?,
            gas_limit: gas,
            to: to_tx_kind(&tx.to),
            value: tx.value,
            data,
            access_list: access_list(&tx.access_list),
            signature_y_parity: y_parity(tx.v)?,
            signature_r: tx.r,
            signature_s: tx.s,
            ..Default::default()
        })),
        2 => {
            let (max_priority_fee_per_gas, max_fee_per_gas) = max_fees()?;
            Ok(Transaction::EIP1559Transaction(EIP1559Transaction {
                chain_id: chain_id()?,
                nonce,
                max_priority_fee_per_gas,
                max_fee_per_gas,
                gas_limit: gas,
                to: to_tx_kind(&tx.to),
                value: tx.value,
                data,
                access_list: access_list(&tx.access_list),
                signature_y_parity: y_parity(tx.v)?,
                signature_r: tx.r,
                signature_s: tx.s,
                ..Default::default()
            }))
        }
        3 => {
            let (max_priority_fee_per_gas, max_fee_per_gas) = max_fees()?;
            Ok(Transaction::EIP4844Transaction(EIP4844Transaction {
                chain_id: chain_id()?,
                nonce,
                max_priority_fee_per_gas,
                max_fee_per_gas,
                gas,
                to: to_call_address(&tx.to, 3)?,
                value: tx.value,
                data,
                access_list: access_list(&tx.access_list),
                max_fee_per_blob_gas: tx
                    .max_fee_per_blob_gas
                    .ok_or_else(|| "transaction is missing maxFeePerBlobGas".to_string())?,
                blob_versioned_hashes: tx.blob_versioned_hashes.clone().unwrap_or_default(),
                signature_y_parity: y_parity(tx.v)?,
                signature_r: tx.r,
                signature_s: tx.s,
                ..Default::default()
            }))
        }
        4 => {
            let (max_priority_fee_per_gas, max_fee_per_gas) = max_fees()?;
            Ok(Transaction::EIP7702Transaction(EIP7702Transaction {
                chain_id: chain_id()?,
                nonce,
                max_priority_fee_per_gas,
                max_fee_per_gas,
                gas_limit: gas,
                to: to_call_address(&tx.to, 4)?,
                value: tx.value,
                data,
                access_list: access_list(&tx.access_list),
                authorization_list: authorization_list(&tx.authorization_list)?,
                signature_y_parity: y_parity(tx.v)?,
                signature_r: tx.r,
                signature_s: tx.s,
                ..Default::default()
            }))
        }
        other => Err(format!("unsupported transaction type {other}")),
    }
}
