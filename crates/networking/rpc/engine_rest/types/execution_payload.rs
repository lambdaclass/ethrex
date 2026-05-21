//! ExecutionPayload containers V1..V4.
//!
//! V1 (Paris) base; V2 (Shanghai) adds `withdrawals`; V3 (Cancun) adds
//! `blob_gas_used`/`excess_blob_gas`; V4 (Amsterdam) adds `block_access_list`
//! and `slot_number`.

use libssz_derive::{SszDecode, SszEncode};
use libssz_types::SszList;

use crate::engine_rest::types::common::{
    Bytes20, Bytes32, LogsBloom, MAX_BYTES_PER_TRANSACTION, MAX_EXTRA_DATA_BYTES,
    MAX_TRANSACTIONS_PER_PAYLOAD, MAX_WITHDRAWALS_PER_PAYLOAD, Uint256,
};
use crate::engine_rest::types::withdrawal::WithdrawalV1;

pub type Transactions =
    SszList<SszList<u8, MAX_BYTES_PER_TRANSACTION>, MAX_TRANSACTIONS_PER_PAYLOAD>;

pub type Withdrawals = SszList<WithdrawalV1, MAX_WITHDRAWALS_PER_PAYLOAD>;

#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode)]
pub struct ExecutionPayloadV1 {
    pub parent_hash: Bytes32,
    pub fee_recipient: Bytes20,
    pub state_root: Bytes32,
    pub receipts_root: Bytes32,
    pub logs_bloom: LogsBloom,
    pub prev_randao: Bytes32,
    pub block_number: u64,
    pub gas_limit: u64,
    pub gas_used: u64,
    pub timestamp: u64,
    pub extra_data: SszList<u8, MAX_EXTRA_DATA_BYTES>,
    pub base_fee_per_gas: Uint256,
    pub block_hash: Bytes32,
    pub transactions: Transactions,
}

#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode)]
pub struct ExecutionPayloadV2 {
    pub parent_hash: Bytes32,
    pub fee_recipient: Bytes20,
    pub state_root: Bytes32,
    pub receipts_root: Bytes32,
    pub logs_bloom: LogsBloom,
    pub prev_randao: Bytes32,
    pub block_number: u64,
    pub gas_limit: u64,
    pub gas_used: u64,
    pub timestamp: u64,
    pub extra_data: SszList<u8, MAX_EXTRA_DATA_BYTES>,
    pub base_fee_per_gas: Uint256,
    pub block_hash: Bytes32,
    pub transactions: Transactions,
    pub withdrawals: Withdrawals,
}

#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode)]
pub struct ExecutionPayloadV3 {
    pub parent_hash: Bytes32,
    pub fee_recipient: Bytes20,
    pub state_root: Bytes32,
    pub receipts_root: Bytes32,
    pub logs_bloom: LogsBloom,
    pub prev_randao: Bytes32,
    pub block_number: u64,
    pub gas_limit: u64,
    pub gas_used: u64,
    pub timestamp: u64,
    pub extra_data: SszList<u8, MAX_EXTRA_DATA_BYTES>,
    pub base_fee_per_gas: Uint256,
    pub block_hash: Bytes32,
    pub transactions: Transactions,
    pub withdrawals: Withdrawals,
    pub blob_gas_used: u64,
    pub excess_blob_gas: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode)]
pub struct ExecutionPayloadV4 {
    pub parent_hash: Bytes32,
    pub fee_recipient: Bytes20,
    pub state_root: Bytes32,
    pub receipts_root: Bytes32,
    pub logs_bloom: LogsBloom,
    pub prev_randao: Bytes32,
    pub block_number: u64,
    pub gas_limit: u64,
    pub gas_used: u64,
    pub timestamp: u64,
    pub extra_data: SszList<u8, MAX_EXTRA_DATA_BYTES>,
    pub base_fee_per_gas: Uint256,
    pub block_hash: Bytes32,
    pub transactions: Transactions,
    pub withdrawals: Withdrawals,
    pub blob_gas_used: u64,
    pub excess_blob_gas: u64,
    pub block_access_list: SszList<u8, MAX_BYTES_PER_TRANSACTION>,
    pub slot_number: u64,
}
