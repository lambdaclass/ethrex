use std::collections::HashMap;

use bytes::Bytes;
use ethrex_common::{
    Address, Bloom, H256, H64, U256,
    types::{
        Block, BlockBody, BlockHeader, Genesis, Withdrawal,
        block_access_list::BlockAccessList, compute_transactions_root,
        compute_withdrawals_root, requests::EncodedRequests,
    },
};
use ethrex_crypto::NativeCrypto;
use serde::Deserialize;

use ef_tests_blockchain::fork::Fork;
use ef_tests_blockchain::types::{Account, BlobSchedule, Info};

// ---- Top-level fixture map ----

/// A single JSON file maps test names to `EngineTestUnit`.
pub type EngineTestFile = HashMap<String, EngineTestUnit>;

// ---- Engine test fixture ----

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineTestUnit {
    #[serde(default, rename = "_info")]
    pub info: Info,
    pub network: Fork,
    pub genesis_block_header: Header,
    pub pre: HashMap<Address, Account>,
    #[serde(default)]
    pub post_state: Option<HashMap<Address, Account>>,
    pub lastblockhash: H256,
    #[serde(rename = "engineNewPayloads")]
    pub engine_new_payloads: Vec<EngineNewPayload>,
    pub config: Option<FixtureConfig>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FixtureConfig {
    pub blob_schedule: Option<BlobSchedule>,
}

impl EngineTestUnit {
    pub fn get_genesis(&self) -> Genesis {
        let mut config = *self.network.chain_config();
        if let Some(test_config) = &self.config
            && let Some(ref schedule) = test_config.blob_schedule
        {
            config.blob_schedule = schedule.clone().into();
        }
        Genesis {
            config,
            alloc: self
                .pre
                .clone()
                .into_iter()
                .map(|(key, val)| (key, val.into()))
                .collect(),
            coinbase: self.genesis_block_header.coinbase,
            difficulty: self.genesis_block_header.difficulty,
            extra_data: self.genesis_block_header.extra_data.clone(),
            gas_limit: self.genesis_block_header.gas_limit.as_u64(),
            nonce: self.genesis_block_header.nonce.to_low_u64_be(),
            mix_hash: self.genesis_block_header.mix_hash,
            timestamp: self.genesis_block_header.timestamp.as_u64(),
            base_fee_per_gas: self
                .genesis_block_header
                .base_fee_per_gas
                .map(|v| v.as_u64()),
            blob_gas_used: self
                .genesis_block_header
                .blob_gas_used
                .map(|v| v.as_u64()),
            excess_blob_gas: self
                .genesis_block_header
                .excess_blob_gas
                .map(|v| v.as_u64()),
            requests_hash: self.genesis_block_header.requests_hash,
            block_access_list_hash: self
                .genesis_block_header
                .block_access_list_hash,
            slot_number: self
                .genesis_block_header
                .slot_number
                .map(|v| v.as_u64()),
        }
    }
}

// ---- Genesis block header (reuse from blockchain tests) ----

#[derive(Debug, PartialEq, Eq, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Header {
    pub bloom: Bloom,
    pub coinbase: Address,
    pub difficulty: U256,
    #[serde(with = "ethrex_common::serde_utils::bytes")]
    pub extra_data: Bytes,
    pub gas_limit: U256,
    pub gas_used: U256,
    pub hash: H256,
    pub mix_hash: H256,
    pub nonce: H64,
    pub number: U256,
    pub parent_hash: H256,
    pub receipt_trie: H256,
    pub state_root: H256,
    pub timestamp: U256,
    pub transactions_trie: H256,
    pub uncle_hash: H256,
    pub base_fee_per_gas: Option<U256>,
    pub withdrawals_root: Option<H256>,
    pub blob_gas_used: Option<U256>,
    pub excess_blob_gas: Option<U256>,
    pub parent_beacon_block_root: Option<H256>,
    pub requests_hash: Option<H256>,
    pub block_access_list_hash: Option<H256>,
    pub slot_number: Option<U256>,
}

// ---- Engine new payload entry ----

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineNewPayload {
    /// params[0] = ExecutionPayload
    /// params[1] = versionedHashes (V3+)
    /// params[2] = parentBeaconBlockRoot (V3+)
    /// params[3] = executionRequests (V4+)
    pub params: Vec<serde_json::Value>,

    /// "1", "2", "3", "4", or "5"
    #[serde(deserialize_with = "deserialize_version_string")]
    pub new_payload_version: u8,

    /// "1", "2", "3", or "4"
    #[serde(deserialize_with = "deserialize_version_string")]
    pub forkchoice_updated_version: u8,

    /// Empty string means no error expected (VALID).
    /// Non-empty means INVALID with this error description.
    #[serde(default)]
    pub validation_error: Option<String>,

    /// JSON-RPC error code (e.g. -32602). Null/absent means no RPC
    /// error. May appear as an integer or a string in fixtures.
    #[serde(
        default,
        deserialize_with = "deserialize_error_code"
    )]
    pub error_code: Option<i64>,
}

fn deserialize_error_code<'de, D>(
    deserializer: D,
) -> Result<Option<i64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let val: Option<serde_json::Value> =
        Option::deserialize(deserializer)?;
    match val {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::Number(n)) => {
            Ok(Some(n.as_i64().ok_or_else(|| {
                serde::de::Error::custom(format!(
                    "errorCode number out of i64 range: {n}"
                ))
            })?))
        }
        Some(serde_json::Value::String(s)) => {
            let parsed = s.parse::<i64>().map_err(|e| {
                serde::de::Error::custom(format!(
                    "errorCode string parse error: {e}"
                ))
            })?;
            Ok(Some(parsed))
        }
        Some(other) => Err(serde::de::Error::custom(format!(
            "unexpected errorCode type: {other}"
        ))),
    }
}

fn deserialize_version_string<'de, D>(deserializer: D) -> Result<u8, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    s.parse::<u8>().map_err(serde::de::Error::custom)
}

// ---- Execution payload from fixture params[0] ----

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FixtureExecutionPayload {
    pub parent_hash: H256,
    pub fee_recipient: Address,
    pub state_root: H256,
    pub receipts_root: H256,
    pub logs_bloom: Bloom,
    #[serde(with = "ethrex_common::serde_utils::u64::hex_str")]
    pub block_number: u64,
    #[serde(with = "ethrex_common::serde_utils::u64::hex_str")]
    pub gas_limit: u64,
    #[serde(with = "ethrex_common::serde_utils::u64::hex_str")]
    pub gas_used: u64,
    #[serde(with = "ethrex_common::serde_utils::u64::hex_str")]
    pub timestamp: u64,
    #[serde(with = "ethrex_common::serde_utils::bytes")]
    pub extra_data: Bytes,
    pub prev_randao: H256,
    #[serde(with = "ethrex_common::serde_utils::u64::hex_str")]
    pub base_fee_per_gas: u64,
    pub block_hash: H256,
    pub transactions: Vec<serde_json::Value>,
    #[serde(default)]
    pub withdrawals: Option<Vec<Withdrawal>>,
    #[serde(
        default,
        with = "ethrex_common::serde_utils::u64::hex_str_opt"
    )]
    pub blob_gas_used: Option<u64>,
    #[serde(
        default,
        with = "ethrex_common::serde_utils::u64::hex_str_opt"
    )]
    pub excess_blob_gas: Option<u64>,
    // V5 field: block access list
    #[serde(
        default,
        with = "ethrex_common::serde_utils::block_access_list::rlp_str_opt"
    )]
    pub block_access_list: Option<BlockAccessList>,
    // V5 field: slot number
    #[serde(
        default,
        with = "ethrex_common::serde_utils::u64::hex_str_opt"
    )]
    pub slot_number: Option<u64>,
}

impl FixtureExecutionPayload {
    /// Convert the fixture execution payload into an ethrex `Block`.
    ///
    /// This mirrors the logic in
    /// `ethrex_rpc::types::payload::ExecutionPayload::into_block`.
    pub fn into_block(
        self,
        parent_beacon_block_root: Option<H256>,
        requests_hash: Option<H256>,
        block_access_list_hash: Option<H256>,
    ) -> Result<Block, String> {
        // Decode transactions from hex-encoded RLP
        let transactions = self
            .transactions
            .iter()
            .enumerate()
            .map(|(i, tx_val)| {
                let hex_str = tx_val
                    .as_str()
                    .ok_or_else(|| format!("tx[{i}] is not a string"))?;
                let bytes = hex::decode(hex_str.trim_start_matches("0x"))
                    .map_err(|e| format!("tx[{i}] hex decode: {e}"))?;
                ethrex_common::types::Transaction::decode_canonical(&bytes)
                    .map_err(|e| format!("tx[{i}] RLP decode: {e}"))
            })
            .collect::<Result<Vec<_>, _>>()?;

        let body = BlockBody {
            transactions: transactions.clone(),
            ommers: vec![],
            withdrawals: self.withdrawals,
        };

        let header = BlockHeader {
            parent_hash: self.parent_hash,
            ommers_hash: *ethrex_common::constants::DEFAULT_OMMERS_HASH,
            coinbase: self.fee_recipient,
            state_root: self.state_root,
            transactions_root: compute_transactions_root(
                &body.transactions,
                &NativeCrypto,
            ),
            receipts_root: self.receipts_root,
            logs_bloom: self.logs_bloom,
            difficulty: 0.into(),
            number: self.block_number,
            gas_limit: self.gas_limit,
            gas_used: self.gas_used,
            timestamp: self.timestamp,
            extra_data: self.extra_data,
            prev_randao: self.prev_randao,
            nonce: 0,
            base_fee_per_gas: Some(self.base_fee_per_gas),
            withdrawals_root: body
                .withdrawals
                .as_ref()
                .map(|w| compute_withdrawals_root(w, &NativeCrypto)),
            blob_gas_used: self.blob_gas_used,
            excess_blob_gas: self.excess_blob_gas,
            parent_beacon_block_root,
            requests_hash,
            slot_number: self.slot_number,
            block_access_list_hash,
            ..Default::default()
        };

        Ok(Block::new(header, body))
    }
}

/// Parse versioned hashes from params[1].
pub fn parse_versioned_hashes(
    val: &serde_json::Value,
) -> Result<Vec<H256>, String> {
    serde_json::from_value(val.clone())
        .map_err(|e| format!("Failed to parse versioned hashes: {e}"))
}

/// Parse parent beacon block root from params[2].
pub fn parse_beacon_root(val: &serde_json::Value) -> Result<H256, String> {
    serde_json::from_value(val.clone())
        .map_err(|e| format!("Failed to parse beacon root: {e}"))
}

/// Parse execution requests from params[3].
pub fn parse_execution_requests(
    val: &serde_json::Value,
) -> Result<Vec<EncodedRequests>, String> {
    serde_json::from_value(val.clone())
        .map_err(|e| format!("Failed to parse execution requests: {e}"))
}

/// Compute the BAL hash from the raw JSON payload, hashing the
/// original RLP bytes before deserialization reorders them.
pub fn compute_raw_bal_hash(
    payload_json: &serde_json::Value,
) -> Option<H256> {
    payload_json.get("blockAccessList").and_then(|v| {
        let hex_str = v.as_str()?;
        let bytes =
            hex::decode(hex_str.trim_start_matches("0x")).ok()?;
        Some(ethrex_common::utils::keccak(bytes))
    })
}
