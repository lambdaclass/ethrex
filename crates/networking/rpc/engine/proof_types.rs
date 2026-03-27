//! EIP-8025 Engine API types for proof endpoints.
//!
//! These are JSON-RPC request/response types specific to the Engine API proof
//! methods. Shared types used by both the coordinator and RPC handlers
//! (e.g. `ExecutionProofV1`, `ProofGenId`) live in
//! `ethrex_blockchain::proof_coordinator::types`.

use bytes::Bytes;
use ethrex_common::H256;
use serde::{Deserialize, Serialize};

/// Minimum required execution proofs per payload.
pub const MIN_REQUIRED_EXECUTION_PROOFS: usize = 1;

/// Proof types a prover is willing to generate.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProofAttributesV1 {
    /// Requested proof type identifiers.
    pub proof_types: Vec<u64>,
}

/// Status of a proof verification or generation request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofStatusV1 {
    pub status: ProofValidationStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Proof validation status values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ProofValidationStatus {
    Valid,
    Invalid,
    Syncing,
    NotSupported,
}

/// Headerized execution payload for JSON-RPC transport (20 fields matching
/// CL `ExecutionPayloadHeader`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionPayloadHeaderV1 {
    pub parent_hash: H256,
    pub fee_recipient: ethrex_common::Address,
    pub state_root: H256,
    pub receipts_root: H256,
    #[serde(with = "ethrex_common::serde_utils::bytes")]
    pub logs_bloom: Bytes,
    pub prev_randao: H256,
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
    /// Accepts both u64 QUANTITY hex (e.g. `"0x342770c0"`) and full
    /// 32-byte big-endian hex (e.g. `"0x00...342770c0"`).
    #[serde(deserialize_with = "base_fee_h256_or_quantity")]
    pub base_fee_per_gas: H256,
    pub block_hash: H256,
    pub transactions_root: H256,
    pub withdrawals_root: H256,
    #[serde(with = "ethrex_common::serde_utils::u64::hex_str")]
    pub blob_gas_used: u64,
    #[serde(with = "ethrex_common::serde_utils::u64::hex_str")]
    pub excess_blob_gas: u64,
    pub deposit_requests_root: H256,
    pub withdrawal_requests_root: H256,
    pub consolidation_requests_root: H256,
}

/// Deserialize `baseFeePerGas` from either a short QUANTITY hex string
/// (e.g. `"0x342770c0"`) or a full 32-byte big-endian hex string.
/// This allows reusing the value directly from `ExecutionPayload.baseFeePerGas`
/// without manual conversion.
fn base_fee_h256_or_quantity<'de, D>(deserializer: D) -> Result<H256, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    let hex_str = s.trim_start_matches("0x");

    if hex_str.len() > 64 {
        return Err(serde::de::Error::custom(format!(
            "baseFeePerGas hex too long: {} chars (max 64)",
            hex_str.len()
        )));
    }

    // Zero-pad to 64 hex chars (32 bytes, big-endian).
    let padded = format!("{:0>64}", hex_str);
    let mut bytes = [0u8; 32];
    for (i, chunk) in padded.as_bytes().chunks(2).enumerate() {
        let pair = std::str::from_utf8(chunk).map_err(serde::de::Error::custom)?;
        bytes[i] = u8::from_str_radix(pair, 16).map_err(serde::de::Error::custom)?;
    }
    Ok(H256(bytes))
}

/// Headerized new-payload request for JSON-RPC transport.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewPayloadRequestHeaderV1 {
    pub execution_payload_header: ExecutionPayloadHeaderV1,
    pub versioned_hashes: Vec<H256>,
    pub parent_beacon_block_root: H256,
    #[serde(with = "ethrex_common::serde_utils::bytes::vec")]
    pub execution_requests: Vec<Bytes>,
}
