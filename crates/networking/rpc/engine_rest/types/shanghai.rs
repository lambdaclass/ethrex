//! Shanghai-shape SSZ types — Paris plus withdrawals.

use libssz_derive::{HashTreeRoot, SszDecode, SszEncode};
use libssz_types::SszList;

use super::common::{
    Bytes20, LogsBloom, MAX_BYTES_PER_TRANSACTION, MAX_EXTRA_DATA_BYTES,
    MAX_TRANSACTIONS_PER_PAYLOAD, MAX_WITHDRAWALS_PER_PAYLOAD,
};

/// SSZ `Withdrawal` container (CL withdrawal queue entry).
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct Withdrawal {
    pub index: u64,
    pub validator_index: u64,
    pub address: Bytes20,
    pub amount: u64,
}

/// Shanghai `ExecutionPayload`: Paris fields + `withdrawals`.
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
    pub withdrawals: SszList<Withdrawal, MAX_WITHDRAWALS_PER_PAYLOAD>,
}

/// Shanghai envelope: just the payload (no beacon block root, no requests).
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct ExecutionPayloadEnvelope {
    pub execution_payload: ExecutionPayload,
}

/// Shanghai payload attributes: Paris fields + `withdrawals`.
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct PayloadAttributes {
    pub timestamp: u64,
    pub prev_randao: [u8; 32],
    pub suggested_fee_recipient: Bytes20,
    pub withdrawals: SszList<Withdrawal, MAX_WITHDRAWALS_PER_PAYLOAD>,
}
