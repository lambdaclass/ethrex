//! Paris-shape SSZ types for the engine REST API.
//!
//! Paris is the base shape: no withdrawals, no blob fields, no execution
//! requests, no block-access list.

use libssz_derive::{HashTreeRoot, SszDecode, SszEncode};
use libssz_types::SszList;

pub use super::common::{
    Bytes20, LogsBloom, MAX_BYTES_PER_TRANSACTION, MAX_EXTRA_DATA_BYTES,
    MAX_TRANSACTIONS_PER_PAYLOAD,
};

/// SSZ Paris `ExecutionPayload`.
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct ExecutionPayload {
    pub parent_hash: [u8; 32],
    pub fee_recipient: Bytes20,
    pub state_root: [u8; 32],
    pub receipts_root: [u8; 32],
    pub logs_bloom: LogsBloom,
    pub prev_randao: [u8; 32],
    pub block_number: u64,
    pub gas_limit: u64,
    pub gas_used: u64,
    pub timestamp: u64,
    pub extra_data: SszList<u8, MAX_EXTRA_DATA_BYTES>,
    /// `base_fee_per_gas` encoded as a 256-bit unsigned integer (little-endian).
    pub base_fee_per_gas: [u8; 32],
    pub block_hash: [u8; 32],
    pub transactions: SszList<SszList<u8, MAX_BYTES_PER_TRANSACTION>, MAX_TRANSACTIONS_PER_PAYLOAD>,
}

/// Paris envelope: just the payload (no beacon block root, no requests).
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct ExecutionPayloadEnvelope {
    pub execution_payload: ExecutionPayload,
}

/// Paris payload attributes (used by /forkchoice when triggering a build).
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct PayloadAttributes {
    pub timestamp: u64,
    pub prev_randao: [u8; 32],
    pub suggested_fee_recipient: Bytes20,
}
