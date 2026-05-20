//! Amsterdam-shape SSZ types — Prague plus block_access_list, slot_number (payload),
//! and custody_columns (payload_attributes, decode-only).

use libssz_derive::{HashTreeRoot, SszDecode, SszEncode};
use libssz_types::SszList;

use super::common::{
    Bytes20, LogsBloom, MAX_BLOCK_ACCESS_LIST_BYTES, MAX_BYTES_PER_TRANSACTION,
    MAX_CUSTODY_COLUMNS, MAX_EXECUTION_REQUESTS_PER_PAYLOAD, MAX_EXTRA_DATA_BYTES,
    MAX_REQUEST_BYTES, MAX_TRANSACTIONS_PER_PAYLOAD, MAX_WITHDRAWALS_PER_PAYLOAD,
};
use super::shanghai::Withdrawal;

/// Amsterdam `ExecutionPayload`: Prague fields + `block_access_list` + `slot_number`.
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
    pub block_access_list: SszList<u8, MAX_BLOCK_ACCESS_LIST_BYTES>,
    pub slot_number: u64,
}

/// Amsterdam envelope: Prague envelope shape with the new payload variant.
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct ExecutionPayloadEnvelope {
    pub execution_payload: ExecutionPayload,
    pub parent_beacon_block_root: [u8; 32],
    pub execution_requests:
        SszList<SszList<u8, MAX_REQUEST_BYTES>, MAX_EXECUTION_REQUESTS_PER_PAYLOAD>,
}

/// Amsterdam payload attributes: Prague fields + `custody_columns`.
/// `custody_columns` is decoded for spec compliance; the value is ignored by
/// ethrex's payload builder until PeerDAS execution lands.
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct PayloadAttributes {
    pub timestamp: u64,
    pub prev_randao: [u8; 32],
    pub suggested_fee_recipient: Bytes20,
    pub withdrawals: SszList<Withdrawal, MAX_WITHDRAWALS_PER_PAYLOAD>,
    pub parent_beacon_block_root: [u8; 32],
    pub custody_columns: SszList<u64, MAX_CUSTODY_COLUMNS>,
}
