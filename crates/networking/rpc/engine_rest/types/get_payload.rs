//! GetPayload response containers. V1 is a bare `ExecutionPayloadV1`; V2..V6
//! are envelopes with block value, blobs bundle, etc.

use libssz_derive::{SszDecode, SszEncode};

use crate::engine_rest::types::blobs::{BlobsBundleV1, BlobsBundleV2};
use crate::engine_rest::types::common::Uint256;
use crate::engine_rest::types::execution_payload::{
    ExecutionPayloadV2, ExecutionPayloadV3, ExecutionPayloadV4,
};
use crate::engine_rest::types::new_payload::ExecutionRequests;

#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode)]
pub struct GetPayloadResponseV2 {
    pub execution_payload: ExecutionPayloadV2,
    pub block_value: Uint256,
}

#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode)]
pub struct GetPayloadResponseV3 {
    pub execution_payload: ExecutionPayloadV3,
    pub block_value: Uint256,
    pub blobs_bundle: BlobsBundleV1,
    pub should_override_builder: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode)]
pub struct GetPayloadResponseV4 {
    pub execution_payload: ExecutionPayloadV3,
    pub block_value: Uint256,
    pub blobs_bundle: BlobsBundleV1,
    pub should_override_builder: bool,
    pub execution_requests: ExecutionRequests,
}

#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode)]
pub struct GetPayloadResponseV5 {
    pub execution_payload: ExecutionPayloadV3,
    pub block_value: Uint256,
    pub blobs_bundle: BlobsBundleV2,
    pub should_override_builder: bool,
    pub execution_requests: ExecutionRequests,
}

#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode)]
pub struct GetPayloadResponseV6 {
    pub execution_payload: ExecutionPayloadV4,
    pub block_value: Uint256,
    pub blobs_bundle: BlobsBundleV2,
    pub should_override_builder: bool,
    pub execution_requests: ExecutionRequests,
}
