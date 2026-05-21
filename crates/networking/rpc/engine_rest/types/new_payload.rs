//! NewPayload V1..V5 request containers.

use libssz_derive::{SszDecode, SszEncode};
use libssz_types::SszList;

use crate::engine_rest::types::common::{
    Bytes32, MAX_BLOB_COMMITMENTS_PER_BLOCK, MAX_BYTES_PER_TRANSACTION, MAX_EXECUTION_REQUESTS,
};
use crate::engine_rest::types::execution_payload::{
    ExecutionPayloadV1, ExecutionPayloadV2, ExecutionPayloadV3, ExecutionPayloadV4,
};

pub type BlobVersionedHashes = SszList<Bytes32, MAX_BLOB_COMMITMENTS_PER_BLOCK>;
pub type ExecutionRequests =
    SszList<SszList<u8, MAX_BYTES_PER_TRANSACTION>, MAX_EXECUTION_REQUESTS>;

#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode)]
pub struct NewPayloadV1Request {
    pub execution_payload: ExecutionPayloadV1,
}

#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode)]
pub struct NewPayloadV2Request {
    pub execution_payload: ExecutionPayloadV2,
}

#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode)]
pub struct NewPayloadV3Request {
    pub execution_payload: ExecutionPayloadV3,
    pub expected_blob_versioned_hashes: BlobVersionedHashes,
    pub parent_beacon_block_root: Bytes32,
}

#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode)]
pub struct NewPayloadV4Request {
    pub execution_payload: ExecutionPayloadV3,
    pub expected_blob_versioned_hashes: BlobVersionedHashes,
    pub parent_beacon_block_root: Bytes32,
    pub execution_requests: ExecutionRequests,
}

#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode)]
pub struct NewPayloadV5Request {
    pub execution_payload: ExecutionPayloadV4,
    pub expected_blob_versioned_hashes: BlobVersionedHashes,
    pub parent_beacon_block_root: Bytes32,
    pub execution_requests: ExecutionRequests,
}
