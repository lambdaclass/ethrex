//! /blobs/v{1..4} — blob retrieval from mempool.

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use ethrex_common::H256;

use crate::engine_rest::error::ProblemJson;
use crate::engine_rest::extractors::Ssz;
use crate::engine_rest::handlers::capabilities::BLOBS_MAX_COUNT;
use crate::engine_rest::responses::SszBody;
use crate::engine_rest::types::blobs::{
    BYTES_PER_PROOF, BlobAndCellsV4, BlobAndProofV1, BlobAndProofV2, BlobsRequest, BlobsRequestV4,
    BlobsResponseV1, BlobsResponseV2, BlobsResponseV4, NUM_CELLS_PER_BLOB, OptBlobAndCellsV4,
    OptBlobAndProofV1, OptBlobAndProofV2,
};
use crate::rpc::RpcApiContext;

pub async fn blobs_v1(State(ctx): State<RpcApiContext>, Ssz(req): Ssz<BlobsRequest>) -> Response {
    if req.versioned_hashes.len() > BLOBS_MAX_COUNT as usize {
        return ProblemJson::payload_too_large(&format!(
            "request exceeds BLOBS_MAX_COUNT ({BLOBS_MAX_COUNT})"
        ))
        .into_response();
    }

    let hashes: Vec<H256> = req
        .versioned_hashes
        .iter()
        .map(|h| H256::from(*h))
        .collect();

    let tuples = match ctx
        .blockchain
        .mempool
        .get_blobs_data_by_versioned_hashes(&hashes)
    {
        Ok(t) => t,
        Err(e) => return ProblemJson::internal(&format!("mempool: {e}")).into_response(),
    };

    // Proof = Bytes48 = [u8; 48], so proofs[0] is already [u8; BYTES_PER_PROOF].
    let response_items: Vec<OptBlobAndProofV1> = tuples
        .into_iter()
        .map(|maybe_tuple| match maybe_tuple {
            None => OptBlobAndProofV1::none(),
            Some((blob, _commitment, proofs)) => {
                if proofs.is_empty() {
                    return OptBlobAndProofV1::none();
                }
                let proof: [u8; BYTES_PER_PROOF] = proofs[0];
                let blob_bytes: Vec<u8> = blob.as_ref().to_vec();
                match blob_bytes.try_into() {
                    Ok(blob_ssz) => OptBlobAndProofV1::some(BlobAndProofV1 {
                        blob: blob_ssz,
                        proof,
                    }),
                    Err(_) => OptBlobAndProofV1::none(),
                }
            }
        })
        .collect();

    SszBody(BlobsResponseV1 {
        items: response_items,
    })
    .into_response()
}

pub async fn blobs_v2(State(ctx): State<RpcApiContext>, Ssz(req): Ssz<BlobsRequest>) -> Response {
    blobs_v2_v3_inner(ctx, req).await
}

pub async fn blobs_v3(State(ctx): State<RpcApiContext>, Ssz(req): Ssz<BlobsRequest>) -> Response {
    blobs_v2_v3_inner(ctx, req).await
}

// V2 (Cancun) and V3 (Osaka) differ in the spec: V2 is all-or-nothing across the
// requested set, V3 allows partial returns of per-blob cell proofs. The current
// mempool stores complete proof sets atomically per blob, so partial state
// doesn't occur and both endpoints behave identically. Reintroduce a flag here
// when the mempool gains Osaka-aware partial storage.
async fn blobs_v2_v3_inner(ctx: RpcApiContext, req: BlobsRequest) -> Response {
    if req.versioned_hashes.len() > BLOBS_MAX_COUNT as usize {
        return ProblemJson::payload_too_large(&format!(
            "request exceeds BLOBS_MAX_COUNT ({BLOBS_MAX_COUNT})"
        ))
        .into_response();
    }

    let hashes: Vec<H256> = req
        .versioned_hashes
        .iter()
        .map(|h| H256::from(*h))
        .collect();

    let tuples = match ctx
        .blockchain
        .mempool
        .get_blobs_data_by_versioned_hashes(&hashes)
    {
        Ok(t) => t,
        Err(e) => return ProblemJson::internal(&format!("mempool: {e}")).into_response(),
    };

    // Both V2 and V3 require non-empty proofs to return Some — the mempool
    // doesn't expose partial proof state, so the two endpoints currently share
    // behavior. Distinguishing Cancun (1 proof) vs Osaka cell proofs (128) is a
    // follow-up; the mempool stores whatever was inserted at transaction time.
    let response_items: Vec<OptBlobAndProofV2> = tuples
        .into_iter()
        .map(|maybe_tuple| match maybe_tuple {
            None => OptBlobAndProofV2::none(),
            Some((blob, _commitment, proofs)) => {
                if proofs.is_empty() {
                    return OptBlobAndProofV2::none();
                }
                let blob_bytes: Vec<u8> = blob.as_ref().to_vec();
                let blob_ssz = match blob_bytes.try_into() {
                    Ok(b) => b,
                    Err(_) => return OptBlobAndProofV2::none(),
                };
                let proofs_arr: Vec<[u8; BYTES_PER_PROOF]> = proofs.into_iter().collect();
                let proofs_ssz = match proofs_arr.try_into() {
                    Ok(p) => p,
                    Err(_) => return OptBlobAndProofV2::none(),
                };
                OptBlobAndProofV2::some(BlobAndProofV2 {
                    blob: blob_ssz,
                    proofs: proofs_ssz,
                })
            }
        })
        .collect();

    SszBody(BlobsResponseV2 {
        items: response_items,
    })
    .into_response()
}

pub async fn blobs_v4(State(ctx): State<RpcApiContext>, Ssz(req): Ssz<BlobsRequestV4>) -> Response {
    if req.versioned_hashes.len() > BLOBS_MAX_COUNT as usize {
        return ProblemJson::payload_too_large(&format!(
            "request exceeds BLOBS_MAX_COUNT ({BLOBS_MAX_COUNT})"
        ))
        .into_response();
    }

    // Validate cell_indices: in-range, no duplicates.
    let indices: Vec<u8> = req.cell_indices.iter().copied().collect();
    let mut seen = [false; NUM_CELLS_PER_BLOB];
    for &i in &indices {
        let idx = i as usize;
        if idx >= NUM_CELLS_PER_BLOB {
            return ProblemJson::bad_request(&format!(
                "cell_index {i} out of range (max {})",
                NUM_CELLS_PER_BLOB - 1
            ))
            .into_response();
        }
        if seen[idx] {
            return ProblemJson::bad_request(&format!("duplicate cell_index {i}")).into_response();
        }
        seen[idx] = true;
    }

    let hashes: Vec<H256> = req
        .versioned_hashes
        .iter()
        .map(|h| H256::from(*h))
        .collect();

    let tuples = match ctx
        .blockchain
        .mempool
        .get_blobs_data_by_versioned_hashes(&hashes)
    {
        Ok(t) => t,
        Err(e) => return ProblemJson::internal(&format!("mempool: {e}")).into_response(),
    };

    let response_items: Vec<OptBlobAndCellsV4> = tuples
        .into_iter()
        .map(|maybe_tuple| match maybe_tuple {
            None => OptBlobAndCellsV4::none(),
            Some((blob, _commitment, proofs)) => {
                let blob_bytes: Vec<u8> = blob.as_ref().to_vec();
                let blob_ssz = match blob_bytes.try_into() {
                    Ok(b) => b,
                    Err(_) => return OptBlobAndCellsV4::none(),
                };

                // Cell filtering: mempool currently doesn't store per-cell data.
                // For each requested cell_index, we return an empty cell + the
                // proof if the index is within `proofs.len()`. Sub-project 4 /
                // future work will wire up cell-level storage.
                let mut filtered_indices: Vec<u8> = Vec::new();
                let mut filtered_cells: Vec<Vec<u8>> = Vec::new();
                let mut filtered_proofs: Vec<[u8; BYTES_PER_PROOF]> = Vec::new();
                for &i in &indices {
                    if let Some(proof) = proofs.get(i as usize) {
                        filtered_indices.push(i);
                        filtered_cells.push(Vec::new()); // empty cell — limitation
                        filtered_proofs.push(*proof);
                    }
                }

                let cell_indices_ssz = match filtered_indices.try_into() {
                    Ok(s) => s,
                    Err(_) => return OptBlobAndCellsV4::none(),
                };

                let cells_ssz_items: Vec<_> = filtered_cells
                    .into_iter()
                    .filter_map(|c| c.try_into().ok())
                    .collect();
                let cells_ssz = match cells_ssz_items.try_into() {
                    Ok(s) => s,
                    Err(_) => return OptBlobAndCellsV4::none(),
                };

                let proofs_ssz = match filtered_proofs.try_into() {
                    Ok(s) => s,
                    Err(_) => return OptBlobAndCellsV4::none(),
                };

                OptBlobAndCellsV4::some(BlobAndCellsV4 {
                    blob: blob_ssz,
                    cell_indices: cell_indices_ssz,
                    cells: cells_ssz,
                    proofs: proofs_ssz,
                })
            }
        })
        .collect();

    SszBody(BlobsResponseV4 {
        items: response_items,
    })
    .into_response()
}
