use std::collections::HashMap;

use ethrex_common::{
    serde_utils, Address, Bytes, H256, U256,
    types::{BlockHeader, GenericTransaction, Withdrawal},
};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

// ── Request types ──────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimulatePayload {
    pub block_state_calls: Vec<BlockStateCall>,
    #[serde(default)]
    pub trace_transfers: bool,
    #[serde(default)]
    pub validation: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockStateCall {
    #[serde(default)]
    pub state_overrides: Option<HashMap<Address, AccountOverride>>,
    #[serde(default)]
    pub block_overrides: Option<BlockOverrides>,
    #[serde(default)]
    pub calls: Vec<GenericTransaction>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountOverride {
    pub balance: Option<U256>,
    #[serde(default, with = "serde_utils::u64::hex_str_opt")]
    pub nonce: Option<u64>,
    #[serde(default, deserialize_with = "deserialize_optional_bytes")]
    pub code: Option<Bytes>,
    /// Full storage replacement – mutually exclusive with `state_diff`.
    pub state: Option<HashMap<H256, H256>>,
    /// Partial storage diff – mutually exclusive with `state`.
    pub state_diff: Option<HashMap<H256, H256>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockOverrides {
    #[serde(default, with = "serde_utils::u64::hex_str_opt")]
    pub number: Option<u64>,
    #[serde(default, with = "serde_utils::u64::hex_str_opt")]
    pub time: Option<u64>,
    #[serde(default, with = "serde_utils::u64::hex_str_opt")]
    pub gas_limit: Option<u64>,
    pub fee_recipient: Option<Address>,
    pub prev_randao: Option<H256>,
    pub base_fee_per_gas: Option<U256>,
    pub blob_base_fee: Option<U256>,
    #[serde(default)]
    pub withdrawals: Option<Vec<Withdrawal>>,
}

// ── Response types ─────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SimulatedBlock {
    pub hash: H256,
    #[serde(with = "serde_utils::u64::hex_str")]
    pub size: u64,
    #[serde(flatten)]
    pub header: BlockHeader,
    pub calls: Vec<CallResult>,
    pub transactions: Vec<Value>,
    pub uncles: Vec<H256>,
    pub withdrawals: Vec<Withdrawal>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CallResult {
    #[serde(with = "serde_utils::u64::hex_str")]
    pub status: u64,
    #[serde(with = "serde_utils::bytes")]
    pub return_data: Bytes,
    #[serde(with = "serde_utils::u64::hex_str")]
    pub gas_used: u64,
    pub logs: Vec<SimulatedLog>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<CallError>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SimulatedLog {
    pub address: Address,
    pub topics: Vec<H256>,
    #[serde(with = "serde_utils::bytes")]
    pub data: Bytes,
    #[serde(with = "serde_utils::u64::hex_str")]
    pub log_index: u64,
    #[serde(with = "serde_utils::u64::hex_str")]
    pub block_number: u64,
    pub block_hash: H256,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CallError {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
}

fn deserialize_optional_bytes<'de, D>(d: D) -> Result<Option<Bytes>, D::Error>
where
    D: Deserializer<'de>,
{
    let Some(value) = Option::<String>::deserialize(d)? else {
        return Ok(None);
    };
    let bytes = hex::decode(value.trim_start_matches("0x"))
        .map_err(|e| serde::de::Error::custom(e.to_string()))?;
    Ok(Some(Bytes::from(bytes)))
}
