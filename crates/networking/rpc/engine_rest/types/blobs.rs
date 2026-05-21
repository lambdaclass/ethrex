//! Blob request/response + bundle containers.

use libssz_derive::{SszDecode, SszEncode};
use libssz_types::SszList;

use crate::engine_rest::types::common::{
    Blob, Bytes32, Bytes48, CELLS_PER_EXT_BLOB, MAX_BLOB_COMMITMENTS_PER_BLOCK,
    MAX_BLOB_HASHES_REQUEST, MAX_BLOB_PROOFS_PER_BUNDLE,
};

#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode)]
pub struct GetBlobsV1Request {
    pub blob_versioned_hashes: SszList<Bytes32, MAX_BLOB_HASHES_REQUEST>,
}

#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode)]
pub struct GetBlobsV2Request {
    pub blob_versioned_hashes: SszList<Bytes32, MAX_BLOB_HASHES_REQUEST>,
}

#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode)]
pub struct GetBlobsV3Request {
    pub blob_versioned_hashes: SszList<Bytes32, MAX_BLOB_HASHES_REQUEST>,
}

#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode)]
pub struct BlobsBundleV1 {
    pub commitments: SszList<Bytes48, MAX_BLOB_COMMITMENTS_PER_BLOCK>,
    pub proofs: SszList<Bytes48, MAX_BLOB_COMMITMENTS_PER_BLOCK>,
    pub blobs: SszList<Blob, MAX_BLOB_COMMITMENTS_PER_BLOCK>,
}

#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode)]
pub struct BlobsBundleV2 {
    pub commitments: SszList<Bytes48, MAX_BLOB_COMMITMENTS_PER_BLOCK>,
    pub proofs: SszList<Bytes48, MAX_BLOB_PROOFS_PER_BUNDLE>,
    pub blobs: SszList<Blob, MAX_BLOB_COMMITMENTS_PER_BLOCK>,
}

#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode)]
pub struct BlobAndProofV1 {
    pub blob: Blob,
    pub proof: Bytes48,
}

#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode)]
pub struct BlobAndProofV2 {
    pub blob: Blob,
    pub proofs: SszList<Bytes48, CELLS_PER_EXT_BLOB>,
}

#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode)]
pub struct GetBlobsV1Response {
    pub blobs_and_proofs: SszList<BlobAndProofV1, MAX_BLOB_HASHES_REQUEST>,
}

#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode)]
pub struct GetBlobsV2Response {
    pub blobs_and_proofs: SszList<BlobAndProofV2, MAX_BLOB_HASHES_REQUEST>,
}

/// V3 uses per-element nullability: each inner list has 0 or 1 element.
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode)]
pub struct GetBlobsV3Response {
    pub blobs_and_proofs: SszList<SszList<BlobAndProofV2, 1>, MAX_BLOB_HASHES_REQUEST>,
}
