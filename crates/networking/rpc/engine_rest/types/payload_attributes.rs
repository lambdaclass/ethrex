//! PayloadAttributes V1..V4.

use libssz_derive::{SszDecode, SszEncode};

use crate::engine_rest::types::common::{Bytes20, Bytes32};
use crate::engine_rest::types::execution_payload::Withdrawals;

#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode)]
pub struct PayloadAttributesV1 {
    pub timestamp: u64,
    pub prev_randao: Bytes32,
    pub suggested_fee_recipient: Bytes20,
}

#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode)]
pub struct PayloadAttributesV2 {
    pub timestamp: u64,
    pub prev_randao: Bytes32,
    pub suggested_fee_recipient: Bytes20,
    pub withdrawals: Withdrawals,
}

#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode)]
pub struct PayloadAttributesV3 {
    pub timestamp: u64,
    pub prev_randao: Bytes32,
    pub suggested_fee_recipient: Bytes20,
    pub withdrawals: Withdrawals,
    pub parent_beacon_block_root: Bytes32,
}

#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode)]
pub struct PayloadAttributesV4 {
    pub timestamp: u64,
    pub prev_randao: Bytes32,
    pub suggested_fee_recipient: Bytes20,
    pub withdrawals: Withdrawals,
    pub parent_beacon_block_root: Bytes32,
    pub slot_number: u64,
    pub target_gas_limit: u64,
}
