//! SSZ `BuiltPayload` containers for `GET /{fork}/payloads/{id}` (replaces
//! `engine_getPayload`). Per-fork shapes mirror `engine_getPayloadV1..V6` and the
//! consensoor CL. Field order per refactor.md: `payload`, `block_value`,
//! `blobs_bundle`, `execution_requests`, `should_override_builder`.

use libssz_derive::{HashTreeRoot, SszDecode, SszEncode};
use libssz_types::{SszList, SszVector};

use super::blobs::{BYTES_PER_BLOB, BYTES_PER_PROOF, CELLS_PER_EXT_BLOB};
use super::common::{MAX_EXECUTION_REQUESTS_PER_PAYLOAD, MAX_REQUEST_BYTES};
use super::{amsterdam, cancun, paris, prague, shanghai};

/// EIP-4844 SSZ list bound on blob commitments per block.
pub const MAX_BLOB_COMMITMENTS_PER_BLOCK: usize = 4096;
/// Cell-proof bound for `BlobsBundleV2`: one proof per cell per blob.
pub const MAX_CELL_PROOFS: usize = MAX_BLOB_COMMITMENTS_PER_BLOCK * CELLS_PER_EXT_BLOB;

/// `BlobsBundleV1` (Cancun/Prague) — one whole-blob proof per blob.
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct BlobsBundleV1 {
    pub commitments: SszList<[u8; BYTES_PER_PROOF], MAX_BLOB_COMMITMENTS_PER_BLOCK>,
    pub proofs: SszList<[u8; BYTES_PER_PROOF], MAX_BLOB_COMMITMENTS_PER_BLOCK>,
    pub blobs: SszList<SszVector<u8, BYTES_PER_BLOB>, MAX_BLOB_COMMITMENTS_PER_BLOCK>,
}

/// `BlobsBundleV2` (Osaka/Amsterdam) — cell proofs (`CELLS_PER_EXT_BLOB` per blob).
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct BlobsBundleV2 {
    pub commitments: SszList<[u8; BYTES_PER_PROOF], MAX_BLOB_COMMITMENTS_PER_BLOCK>,
    pub proofs: SszList<[u8; BYTES_PER_PROOF], MAX_CELL_PROOFS>,
    pub blobs: SszList<SszVector<u8, BYTES_PER_BLOB>, MAX_BLOB_COMMITMENTS_PER_BLOCK>,
}

/// Execution requests list (EIP-7685), shared with the newPayload envelopes.
pub type ExecutionRequestsList =
    SszList<SszList<u8, MAX_REQUEST_BYTES>, MAX_EXECUTION_REQUESTS_PER_PAYLOAD>;

/// Paris built payload (replaces `engine_getPayloadV1`).
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct BuiltPayloadParis {
    pub payload: paris::ExecutionPayload,
    pub block_value: [u8; 32],
}

/// Shanghai built payload (replaces `engine_getPayloadV2`).
///
/// Omits `should_override_builder` by design: that flag was introduced in
/// `engine_getPayloadV3`/Cancun (alongside `blobs_bundle`), not Shanghai. The
/// execution-apis #793 spec confirms this — `BuiltPayloadShanghai` is
/// `{payload, block_value}` and the flag first appears on `BuiltPayloadCancun`
/// (resolved upstream 2026-06-17, commit "advertise all forks"; matches the
/// legacy `engine_getPayloadV2` shape).
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct BuiltPayloadShanghai {
    pub payload: shanghai::ExecutionPayload,
    pub block_value: [u8; 32],
}

/// Cancun built payload (replaces `engine_getPayloadV3`).
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct BuiltPayloadCancun {
    pub payload: cancun::ExecutionPayload,
    pub block_value: [u8; 32],
    pub blobs_bundle: BlobsBundleV1,
    pub should_override_builder: bool,
}

/// Prague built payload (replaces `engine_getPayloadV4`).
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct BuiltPayloadPrague {
    pub payload: prague::ExecutionPayload,
    pub block_value: [u8; 32],
    pub blobs_bundle: BlobsBundleV1,
    pub execution_requests: ExecutionRequestsList,
    pub should_override_builder: bool,
}

/// Osaka built payload (replaces `engine_getPayloadV5`) — cell-proof blobs bundle.
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct BuiltPayloadOsaka {
    pub payload: prague::ExecutionPayload,
    pub block_value: [u8; 32],
    pub blobs_bundle: BlobsBundleV2,
    pub execution_requests: ExecutionRequestsList,
    pub should_override_builder: bool,
}

/// Amsterdam built payload (replaces `engine_getPayloadV6`).
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct BuiltPayloadAmsterdam {
    pub payload: amsterdam::ExecutionPayload,
    pub block_value: [u8; 32],
    pub blobs_bundle: BlobsBundleV2,
    pub execution_requests: ExecutionRequestsList,
    pub should_override_builder: bool,
}
