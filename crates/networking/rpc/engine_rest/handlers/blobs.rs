//! POST /engine/v{1,2,3}/blobs.
//!
//! V1 returns `BlobAndProofV1` (single proof, no nullability). V2 returns
//! `BlobAndProofV2` (cell proofs); all-or-nothing — empty list when any blob
//! is missing. V3 returns per-element nullable `BlobAndProofV2`.

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use ethrex_common::H256;
use libssz_types::SszList;

use crate::engine_rest::error::{EngineError, EngineRestError};
use crate::engine_rest::extractors::Ssz;
use crate::engine_rest::responses::SszBody;
use crate::engine_rest::types::blobs::{
    BlobAndProofV1, BlobAndProofV2, GetBlobsV1Request, GetBlobsV1Response, GetBlobsV2Request,
    GetBlobsV2Response, GetBlobsV3Request, GetBlobsV3Response,
};
use crate::engine_rest::types::common::{
    BLOB_SIZE, Blob, MAX_BLOB_HASHES_REQUEST, ssz_none, ssz_some,
};
use crate::rpc::RpcApiContext;

fn check_count(n: usize) -> Result<(), EngineRestError> {
    if n > MAX_BLOB_HASHES_REQUEST {
        return Err(EngineRestError::payload_too_large(format!(
            "request exceeds MAX_BLOB_HASHES_REQUEST ({MAX_BLOB_HASHES_REQUEST})"
        )));
    }
    Ok(())
}

fn copy_blob(b: &[u8]) -> Blob {
    let mut out = [0u8; BLOB_SIZE];
    out.copy_from_slice(b);
    out
}

pub async fn blobs_v1(
    State(ctx): State<RpcApiContext>,
    Ssz(req): Ssz<GetBlobsV1Request>,
) -> Response {
    if let Err(e) = check_count(req.blob_versioned_hashes.len()) {
        return e.into();
    }
    let hashes: Vec<H256> = req
        .blob_versioned_hashes
        .iter()
        .map(|h| H256::from(*h))
        .collect();
    let tuples = match ctx
        .blockchain
        .mempool
        .get_blobs_data_by_versioned_hashes(&hashes)
    {
        Ok(t) => t,
        Err(e) => return EngineError::internal(&format!("mempool: {e}")),
    };

    let mut items: Vec<BlobAndProofV1> = Vec::new();
    for t in tuples.into_iter().flatten() {
        let (blob, _commitment, proofs) = t;
        let Some(proof) = proofs.first() else {
            continue;
        };
        items.push(BlobAndProofV1 {
            blob: copy_blob(blob.as_ref()),
            proof: *proof,
        });
    }
    let blobs_and_proofs = match items.try_into() {
        Ok(s) => s,
        Err(_) => return EngineError::internal("blobs_and_proofs overflow"),
    };
    SszBody(GetBlobsV1Response { blobs_and_proofs }).into_response()
}

async fn require_osaka_tip(ctx: &RpcApiContext, version: u8) -> Result<(), EngineRestError> {
    let latest = ctx
        .storage
        .get_latest_block_number()
        .await
        .map_err(|e| EngineRestError::internal(format!("storage: {e}")))?;
    let header = ctx
        .storage
        .get_block_header(latest)
        .map_err(|e| EngineRestError::internal(format!("storage: {e}")))?
        .ok_or_else(|| {
            EngineRestError::internal(format!("missing header for latest block {latest}"))
        })?;
    if !ctx
        .storage
        .get_chain_config()
        .is_osaka_activated(header.timestamp)
    {
        return Err(EngineRestError::unprocessable(format!(
            "getBlobsV{version} engine only supported for Osaka"
        )));
    }
    Ok(())
}

pub async fn blobs_v2(
    State(ctx): State<RpcApiContext>,
    Ssz(req): Ssz<GetBlobsV2Request>,
) -> Response {
    if let Err(e) = check_count(req.blob_versioned_hashes.len()) {
        return e.into();
    }
    if let Err(e) = require_osaka_tip(&ctx, 2).await {
        return e.into();
    }
    let hashes: Vec<H256> = req
        .blob_versioned_hashes
        .iter()
        .map(|h| H256::from(*h))
        .collect();
    let tuples = match ctx
        .blockchain
        .mempool
        .get_blobs_data_by_versioned_hashes(&hashes)
    {
        Ok(t) => t,
        Err(e) => return EngineError::internal(&format!("mempool: {e}")),
    };
    // Spec allows 204 on missing blobs but Prysm's SSZ-REST decoder rejects
    // 0-byte bodies; emit a 200 with an empty list instead (also spec-valid).
    if tuples.iter().any(|t| match t {
        None => true,
        Some((_, _, proofs)) => proofs.is_empty(),
    }) {
        let blobs_and_proofs = match Vec::<BlobAndProofV2>::new().try_into() {
            Ok(s) => s,
            Err(_) => return EngineError::internal("empty V2 list overflow"),
        };
        return SszBody(GetBlobsV2Response { blobs_and_proofs }).into_response();
    }
    let mut items: Vec<BlobAndProofV2> = Vec::with_capacity(tuples.len());
    for t in tuples.into_iter().flatten() {
        let (blob, _commitment, proofs) = t;
        let proofs_list: SszList<
            [u8; 48],
            { crate::engine_rest::types::common::CELLS_PER_EXT_BLOB },
        > = match proofs.into_iter().collect::<Vec<_>>().try_into() {
            Ok(p) => p,
            Err(_) => return EngineError::internal("proofs overflow CELLS_PER_EXT_BLOB"),
        };
        items.push(BlobAndProofV2 {
            blob: copy_blob(blob.as_ref()),
            proofs: proofs_list,
        });
    }
    let blobs_and_proofs = match items.try_into() {
        Ok(s) => s,
        Err(_) => return EngineError::internal("blobs_and_proofs overflow"),
    };
    SszBody(GetBlobsV2Response { blobs_and_proofs }).into_response()
}

pub async fn blobs_v3(
    State(ctx): State<RpcApiContext>,
    Ssz(req): Ssz<GetBlobsV3Request>,
) -> Response {
    if let Err(e) = check_count(req.blob_versioned_hashes.len()) {
        return e.into();
    }
    if let Err(e) = require_osaka_tip(&ctx, 3).await {
        return e.into();
    }
    let hashes: Vec<H256> = req
        .blob_versioned_hashes
        .iter()
        .map(|h| H256::from(*h))
        .collect();
    let tuples = match ctx
        .blockchain
        .mempool
        .get_blobs_data_by_versioned_hashes(&hashes)
    {
        Ok(t) => t,
        Err(e) => return EngineError::internal(&format!("mempool: {e}")),
    };
    let mut slots: Vec<SszList<BlobAndProofV2, 1>> = Vec::with_capacity(tuples.len());
    for t in tuples.into_iter() {
        let slot = match t {
            Some((blob, _commitment, proofs)) if !proofs.is_empty() => {
                let proofs_list: SszList<
                    [u8; 48],
                    { crate::engine_rest::types::common::CELLS_PER_EXT_BLOB },
                > = match proofs.into_iter().collect::<Vec<_>>().try_into() {
                    Ok(p) => p,
                    Err(_) => return EngineError::internal("proofs overflow CELLS_PER_EXT_BLOB"),
                };
                ssz_some(BlobAndProofV2 {
                    blob: copy_blob(blob.as_ref()),
                    proofs: proofs_list,
                })
            }
            _ => ssz_none(),
        };
        slots.push(slot);
    }
    let blobs_and_proofs = match slots.try_into() {
        Ok(s) => s,
        Err(_) => return EngineError::internal("blobs_and_proofs overflow"),
    };
    SszBody(GetBlobsV3Response { blobs_and_proofs }).into_response()
}
