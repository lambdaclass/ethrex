//! Payload bodies containers. `payload_bodies` entries use `List[Body, 1]`
//! (0 = unknown block, 1 = known). `block_access_list` is nullable similarly.

use libssz_derive::{SszDecode, SszEncode};
use libssz_types::SszList;

use crate::engine_rest::types::common::{
    Bytes32, MAX_BYTES_PER_TRANSACTION, MAX_PAYLOAD_BODIES_REQUEST,
};
use crate::engine_rest::types::execution_payload::{Transactions, Withdrawals};

#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode)]
pub struct GetPayloadBodiesByHashV1Request {
    pub block_hashes: SszList<Bytes32, MAX_PAYLOAD_BODIES_REQUEST>,
}

#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode)]
pub struct GetPayloadBodiesByHashV2Request {
    pub block_hashes: SszList<Bytes32, MAX_PAYLOAD_BODIES_REQUEST>,
}

#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode)]
pub struct GetPayloadBodiesByRangeV1Request {
    pub start: u64,
    pub count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode)]
pub struct GetPayloadBodiesByRangeV2Request {
    pub start: u64,
    pub count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode)]
pub struct ExecutionPayloadBodyV1 {
    pub transactions: Transactions,
    pub withdrawals: Withdrawals,
}

#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode)]
pub struct ExecutionPayloadBodyV2 {
    pub transactions: Transactions,
    pub withdrawals: Withdrawals,
    pub block_access_list: SszList<SszList<u8, MAX_BYTES_PER_TRANSACTION>, 1>,
}

#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode)]
pub struct PayloadBodiesV1Response {
    pub payload_bodies: SszList<SszList<ExecutionPayloadBodyV1, 1>, MAX_PAYLOAD_BODIES_REQUEST>,
}

#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode)]
pub struct PayloadBodiesV2Response {
    pub payload_bodies: SszList<SszList<ExecutionPayloadBodyV2, 1>, MAX_PAYLOAD_BODIES_REQUEST>,
}
