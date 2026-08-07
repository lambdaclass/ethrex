//! Cancun-shape SSZ types — Shanghai plus blob fields (payload) and beacon block root (envelope, attrs).

use libssz_derive::{HashTreeRoot, SszDecode, SszEncode};
use libssz_types::SszList;

use super::common::{
    Bytes20, LogsBloom, MAX_BYTES_PER_TRANSACTION, MAX_EXTRA_DATA_BYTES,
    MAX_TRANSACTIONS_PER_PAYLOAD, MAX_WITHDRAWALS_PER_PAYLOAD,
};
use super::shanghai::Withdrawal;

/// Cancun `ExecutionPayload`: Shanghai fields + `blob_gas_used` + `excess_blob_gas`.
/// Note: `parent_beacon_block_root` lives on the **envelope**, not the payload.
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
    pub base_fee_per_gas: [u8; 32],
    pub block_hash: [u8; 32],
    pub transactions: SszList<SszList<u8, MAX_BYTES_PER_TRANSACTION>, MAX_TRANSACTIONS_PER_PAYLOAD>,
    pub withdrawals: SszList<Withdrawal, MAX_WITHDRAWALS_PER_PAYLOAD>,
    pub blob_gas_used: u64,
    pub excess_blob_gas: u64,
}

/// Cancun envelope: payload + parent_beacon_block_root.
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct ExecutionPayloadEnvelope {
    pub execution_payload: ExecutionPayload,
    pub parent_beacon_block_root: [u8; 32],
}

/// Cancun payload attributes: Shanghai fields + `parent_beacon_block_root`.
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct PayloadAttributes {
    pub timestamp: u64,
    pub prev_randao: [u8; 32],
    pub suggested_fee_recipient: Bytes20,
    pub withdrawals: SszList<Withdrawal, MAX_WITHDRAWALS_PER_PAYLOAD>,
    pub parent_beacon_block_root: [u8; 32],
}
