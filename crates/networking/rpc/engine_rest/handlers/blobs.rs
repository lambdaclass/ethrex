//! /blobs/v{1..4} — blob retrieval from mempool, per execution-apis #793.
//!
//! Response shape is `List[BlobV*Entry]` where each entry carries an
//! `available` flag plus `contents` (zeroed when unavailable). `/blobs/v2` is
//! all-or-nothing: if any requested blob is missing the handler returns
//! `204 No Content` instead of emitting unavailable entries. `/blobs/v3`
//! surfaces missing blobs per entry. `/blobs/v4` requires per-cell data that
//! the mempool does not store, so it returns `204 No Content` ("EL cannot
//! serve this request at all").

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use ethrex_common::H256;

use crate::engine_rest::error::ProblemJson;
use crate::engine_rest::extractors::Ssz;
use crate::engine_rest::handlers::capabilities::BLOBS_MAX_COUNT;
use crate::engine_rest::responses::SszBody;
use crate::engine_rest::types::blobs::{
    BlobAndProofV1, BlobAndProofV2, BlobV1Entry, BlobV2Entry, BlobsRequest, BlobsRequestV4,
    BlobsV1Response, BlobsV2Response, BlobsV3Response,
};
use crate::rpc::RpcApiContext;

/// Map the requested versioned hashes to `H256`, rejecting over-cap requests.
fn request_hashes(versioned_hashes: &[[u8; 32]]) -> Result<Vec<H256>, ProblemJson> {
    if versioned_hashes.len() > BLOBS_MAX_COUNT as usize {
        return Err(ProblemJson::payload_too_large(&format!(
            "request exceeds BLOBS_MAX_COUNT ({BLOBS_MAX_COUNT})"
        )));
    }
    Ok(versioned_hashes.iter().map(|h| H256::from(*h)).collect())
}

pub async fn blobs_v1(State(ctx): State<RpcApiContext>, Ssz(req): Ssz<BlobsRequest>) -> Response {
    let hashes = match request_hashes(&req.versioned_hashes) {
        Ok(h) => h,
        Err(p) => return p.into_response(),
    };

    let tuples = match ctx
        .blockchain
        .mempool
        .get_blobs_data_by_versioned_hashes(&hashes)
    {
        Ok(t) => t,
        Err(e) => return ProblemJson::internal(&format!("mempool: {e}")).into_response(),
    };

    let mut entries: Vec<BlobV1Entry> = Vec::with_capacity(tuples.len());
    for maybe_tuple in tuples {
        let entry = match maybe_tuple {
            Some((blob, _commitment, proofs)) if !proofs.is_empty() => {
                // Cancun blobs carry a single whole-blob proof; proofs[0] is it.
                match blob.as_ref().to_vec().try_into() {
                    Ok(blob_ssz) => BlobV1Entry::available(BlobAndProofV1 {
                        blob: blob_ssz,
                        proof: proofs[0],
                    }),
                    Err(_) => BlobV1Entry::unavailable(),
                }
            }
            _ => BlobV1Entry::unavailable(),
        };
        entries.push(entry);
    }

    match entries.try_into() {
        Ok(entries) => SszBody(BlobsV1Response { entries }).into_response(),
        Err(_) => ProblemJson::internal("blobs response exceeds MAX_BLOBS_REQUEST").into_response(),
    }
}

/// `/blobs/v2` — all-or-nothing (Osaka). If any requested blob is missing the
/// EL MUST return `204 No Content` (spec §`POST /blobs/v2`), mirroring
/// `engine_getBlobsV2`'s `null` response.
pub async fn blobs_v2(State(ctx): State<RpcApiContext>, Ssz(req): Ssz<BlobsRequest>) -> Response {
    let hashes = match request_hashes(&req.versioned_hashes) {
        Ok(h) => h,
        Err(p) => return p.into_response(),
    };

    let tuples = match ctx
        .blockchain
        .mempool
        .get_blobs_data_by_versioned_hashes(&hashes)
    {
        Ok(t) => t,
        Err(e) => return ProblemJson::internal(&format!("mempool: {e}")).into_response(),
    };

    let mut entries: Vec<BlobV2Entry> = Vec::with_capacity(tuples.len());
    for maybe_tuple in tuples {
        match maybe_tuple {
            Some((blob, _commitment, proofs)) if !proofs.is_empty() => {
                let blob_ssz = match blob.as_ref().to_vec().try_into() {
                    Ok(b) => b,
                    // A malformed stored blob means we can't satisfy the set.
                    Err(_) => return StatusCode::NO_CONTENT.into_response(),
                };
                let proofs_ssz = match proofs.into_iter().collect::<Vec<_>>().try_into() {
                    Ok(p) => p,
                    Err(_) => return StatusCode::NO_CONTENT.into_response(),
                };
                entries.push(BlobV2Entry::available(BlobAndProofV2 {
                    blob: blob_ssz,
                    proofs: proofs_ssz,
                }));
            }
            // Any missing blob → all-or-nothing: serve nothing.
            _ => return StatusCode::NO_CONTENT.into_response(),
        }
    }

    match entries.try_into() {
        Ok(entries) => SszBody(BlobsV2Response { entries }).into_response(),
        Err(_) => ProblemJson::internal("blobs response exceeds MAX_BLOBS_REQUEST").into_response(),
    }
}

/// `/blobs/v3` — partial responses (Osaka). Missing blobs surface as
/// `available == false` entries; `204 No Content` is reserved for "EL cannot
/// serve the request at all".
pub async fn blobs_v3(State(ctx): State<RpcApiContext>, Ssz(req): Ssz<BlobsRequest>) -> Response {
    let hashes = match request_hashes(&req.versioned_hashes) {
        Ok(h) => h,
        Err(p) => return p.into_response(),
    };

    let tuples = match ctx
        .blockchain
        .mempool
        .get_blobs_data_by_versioned_hashes(&hashes)
    {
        Ok(t) => t,
        Err(e) => return ProblemJson::internal(&format!("mempool: {e}")).into_response(),
    };

    let mut entries: Vec<BlobV2Entry> = Vec::with_capacity(tuples.len());
    for maybe_tuple in tuples {
        let entry = match maybe_tuple {
            Some((blob, _commitment, proofs)) if !proofs.is_empty() => {
                let blob_ssz = blob.as_ref().to_vec().try_into();
                let proofs_ssz = proofs.into_iter().collect::<Vec<_>>().try_into();
                match (blob_ssz, proofs_ssz) {
                    (Ok(blob), Ok(proofs)) => {
                        BlobV2Entry::available(BlobAndProofV2 { blob, proofs })
                    }
                    _ => BlobV2Entry::unavailable(),
                }
            }
            _ => BlobV2Entry::unavailable(),
        };
        entries.push(entry);
    }

    match entries.try_into() {
        Ok(entries) => SszBody(BlobsV3Response { entries }).into_response(),
        Err(_) => ProblemJson::internal("blobs response exceeds MAX_BLOBS_REQUEST").into_response(),
    }
}

/// `/blobs/v4` — Amsterdam cell-range selection. The mempool stores blobs and
/// KZG proofs but not the per-cell data this endpoint must return, so the EL
/// cannot serve the request at all and responds `204 No Content` (spec
/// §`POST /blobs/v4`) rather than emitting empty/zeroed cells that would fail
/// KZG cell-proof verification at the CL. The request is still SSZ-decoded
/// (validating the `indices_bitarray` shape) before we respond.
pub async fn blobs_v4(
    State(_ctx): State<RpcApiContext>,
    Ssz(req): Ssz<BlobsRequestV4>,
) -> Response {
    if let Err(p) = request_hashes(&req.versioned_hashes) {
        return p.into_response();
    }
    StatusCode::NO_CONTENT.into_response()
}
