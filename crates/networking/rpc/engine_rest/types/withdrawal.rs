//! Shanghai withdrawal container.

use libssz_derive::{SszDecode, SszEncode};

use crate::engine_rest::types::common::Bytes20;

#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode)]
pub struct WithdrawalV1 {
    pub index: u64,
    pub validator_index: u64,
    pub address: Bytes20,
    pub amount: u64,
}
