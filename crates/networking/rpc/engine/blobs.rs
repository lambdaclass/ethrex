use std::time::Duration;

use bytes::Bytes;
use ethrex_common::{
    H256,
    serde_utils::{self},
    types::{BYTES_PER_CELL, Blob, CELLS_PER_EXT_BLOB, Proof},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::debug;

use crate::{
    rpc::{RpcApiContext, RpcHandler},
    utils::RpcErr,
};

// -> https://github.com/ethereum/execution-apis/blob/d41fdf10fabbb73c4d126fb41809785d830acace/src/engine/cancun.md?plain=1#L186
pub(crate) const GET_BLOBS_V1_REQUEST_MAX_SIZE: usize = 128;

#[derive(Debug, Serialize, Deserialize)]
pub struct BlobsV1Request {
    blob_versioned_hashes: Vec<H256>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BlobsV2Request {
    blob_versioned_hashes: Vec<H256>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BlobsV3Request {
    blob_versioned_hashes: Vec<H256>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlobAndProofV1 {
    #[serde(with = "serde_utils::blob")]
    pub blob: Blob,
    #[serde(with = "serde_utils::bytes48")]
    pub proof: Proof,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlobAndProofV2 {
    #[serde(with = "serde_utils::blob")]
    pub blob: Blob,
    #[serde(with = "serde_utils::bytes48::vec")]
    pub proofs: Vec<Proof>,
}

impl RpcHandler for BlobsV1Request {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() != 1 {
            return Err(RpcErr::BadParams("Expected 1 param".to_owned()));
        };
        Ok(BlobsV1Request {
            blob_versioned_hashes: serde_json::from_value(params[0].clone())?,
        })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        debug!("Received new engine request: Requested Blobs");

        // Intentional fall-through: before a canonical tip exists, there is no
        // block timestamp to compare against Osaka, so the node is treated as pre-Osaka.
        if let Some(current_block_header) = context
            .storage
            .get_block_header(context.storage.get_latest_block_number().await?)?
            && context
                .storage
                .get_chain_config()
                .is_osaka_activated(current_block_header.timestamp)
        {
            return Err(RpcErr::UnsupportedFork(
                "getBlobsV1 engine only supported before Osaka".to_string(),
            ));
        }

        if self.blob_versioned_hashes.len() > GET_BLOBS_V1_REQUEST_MAX_SIZE {
            return Err(RpcErr::TooLargeRequest);
        }

        let blob_tuples = context
            .blockchain
            .mempool
            .get_blobs_data_by_versioned_hashes(&self.blob_versioned_hashes)?;

        debug_assert_eq!(self.blob_versioned_hashes.len(), blob_tuples.len());

        let res: Vec<Option<BlobAndProofV1>> = blob_tuples
            .into_iter()
            .map(|b| {
                b.and_then(|(blob, _, proofs)| {
                    // getBlobsV1 serves the single EIP-4844 blob proof. A v0 bundle yields
                    // exactly one proof here (see `get_blob_tuple_by_index`); a v1 (EIP-7594)
                    // sidecar yields 128 cell proofs per blob and can now reach a pre-Osaka
                    // mempool. Cell proofs can't be represented as a single blob proof, so
                    // report the blob as unavailable (the CL re-fetches it from gossip)
                    // rather than returning a cell proof in the blob-proof field.
                    (proofs.len() == 1).then(|| BlobAndProofV1 {
                        blob: *blob,
                        proof: proofs[0],
                    })
                })
            })
            .collect();

        serde_json::to_value(res).map_err(|error| RpcErr::Internal(error.to_string()))
    }
}

impl RpcHandler for BlobsV2Request {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() != 1 {
            return Err(RpcErr::BadParams("Expected 1 param".to_owned()));
        };
        Ok(BlobsV2Request {
            blob_versioned_hashes: serde_json::from_value(params[0].clone())?,
        })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        debug!("Received new engine request: Requested Blobs V2");
        let res = get_blobs_and_proof(&self.blob_versioned_hashes, context).await?;
        if res.iter().any(|blob| blob.is_none()) {
            return Ok(Value::Null);
        }
        serde_json::to_value(res).map_err(|error| RpcErr::Internal(error.to_string()))
    }
}

impl RpcHandler for BlobsV3Request {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() != 1 {
            return Err(RpcErr::BadParams("Expected 1 param".to_owned()));
        };
        Ok(BlobsV3Request {
            blob_versioned_hashes: serde_json::from_value(params[0].clone())?,
        })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        debug!("Received new engine request: Requested Blobs V3");
        let res = get_blobs_and_proof(&self.blob_versioned_hashes, context).await?;
        serde_json::to_value(res).map_err(|error| RpcErr::Internal(error.to_string()))
    }
}

/// Get blob data and proofs for a given list of blob versioned hashes.
async fn get_blobs_and_proof(
    blob_versioned_hashes: &[H256],
    context: RpcApiContext,
) -> Result<Vec<Option<BlobAndProofV2>>, RpcErr> {
    if blob_versioned_hashes.len() > GET_BLOBS_V1_REQUEST_MAX_SIZE {
        return Err(RpcErr::TooLargeRequest);
    }

    // getBlobsV2/V3 (EIP-7594) serve cell proofs, which only exist once the chain is at
    // Osaka. The engine spec does NOT define a pre-fork `-38005` for these methods (that
    // code is for the opposite direction, e.g. getBlobsV1 *after* Osaka); their contract is
    // simply to return `null` for any blob we don't have. So before our canonical tip is at
    // Osaka, return `null` for every requested hash rather than a bespoke error. This also
    // covers the syncing case, where the local head is still pre-Osaka while we catch up
    // (the spec likewise prescribes `null` while syncing).
    let head_is_osaka = match context
        .storage
        .get_block_header(context.storage.get_latest_block_number().await?)?
    {
        Some(current_block_header) => context
            .storage
            .get_chain_config()
            .is_osaka_activated(current_block_header.timestamp),
        // No canonical tip yet: treat as pre-Osaka.
        None => false,
    };
    if !head_is_osaka {
        return Ok(vec![None; blob_versioned_hashes.len()]);
    }

    let blob_tuples = context
        .blockchain
        .mempool
        .get_blobs_data_by_versioned_hashes(blob_versioned_hashes)?;

    debug_assert_eq!(blob_versioned_hashes.len(), blob_tuples.len());

    let res = blob_tuples
        .into_iter()
        .map(|b| {
            b.map(|(blob, _, proofs)| BlobAndProofV2 {
                blob: *blob,
                proofs,
            })
        })
        .collect();

    Ok(res)
}

// ── engine_getBlobsV4 ────────────────────────────────────────────────────────

/// Per-blob response for `engine_getBlobsV4`.
///
/// Both `blob_cells` and `proofs` are **sparse, fixed-length** matrices of
/// length `CELLS_PER_EXT_BLOB` (128), indexed by absolute column index. The
/// entry at index `i` is non-null only when bit `i` of `indices_bitarray` is
/// set **and** the cell is available locally; every other index is `null`.
/// Cell and proof are always both-null or both-present at a given index, so a
/// caller can verify each cell against its positionally-aligned proof.
///
/// This matches the EIP-8070 "partial matrix ... with nil entries for missing
/// cells" wording and the execution-spec-tests `getBlobsV4` validator
/// (execution-specs PR #2948), which requires length-128 matrices with `null`
/// at every non-requested index.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlobCellsAndProofsV1 {
    /// Sparse length-128 column matrix: hex-encoded cell (2048 bytes) at
    /// requested+held indices, `null` everywhere else.
    #[serde(with = "opt_bytes_vec")]
    pub blob_cells: Vec<Option<Bytes>>,
    /// Sparse length-128 KZG cell proofs (48 bytes each), positionally aligned
    /// with `blob_cells`: `null` at every index where `blob_cells` is `null`.
    #[serde(with = "opt_proofs_vec")]
    pub proofs: Vec<Option<Proof>>,
}

/// Serde helper: serialize `Vec<Option<Bytes>>` as an array of hex strings or null.
mod opt_bytes_vec {
    use bytes::Bytes;
    use serde::Serializer;

    pub fn serialize<S>(value: &Vec<Option<Bytes>>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use serde::ser::SerializeSeq;
        let mut seq = serializer.serialize_seq(Some(value.len()))?;
        for item in value {
            match item {
                Some(b) => seq.serialize_element(&format!("0x{b:x}"))?,
                None => seq.serialize_element(&serde_json::Value::Null)?,
            }
        }
        seq.end()
    }
}

/// Serde helper: serialize `Vec<Option<Proof>>` as an array of hex strings or null.
mod opt_proofs_vec {
    use ethrex_common::types::Proof;
    use serde::Serializer;

    pub fn serialize<S>(value: &Vec<Option<Proof>>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use serde::ser::SerializeSeq;
        let mut seq = serializer.serialize_seq(Some(value.len()))?;
        for item in value {
            match item {
                Some(p) => seq.serialize_element(&format!("0x{}", hex::encode(p)))?,
                None => seq.serialize_element(&serde_json::Value::Null)?,
            }
        }
        seq.end()
    }
}

/// Request body for `engine_getBlobsV4`.
pub struct BlobsV4Request {
    /// Versioned blob hashes to look up.
    pub(crate) versioned_blob_hashes: Vec<H256>,
    /// Bitmask of column indices to return (bit i set ⇒ return column i).
    /// Encoded as a 16-byte big-endian hex string on the wire.
    pub(crate) indices_bitarray: u128,
}

impl RpcHandler for BlobsV4Request {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() != 2 {
            return Err(RpcErr::BadParams("Expected 2 params".to_owned()));
        }
        let versioned_blob_hashes: Vec<H256> = serde_json::from_value(params[0].clone())?;
        let indices_bitarray = parse_indices_bitarray(&params[1])?;
        Ok(BlobsV4Request {
            versioned_blob_hashes,
            indices_bitarray,
        })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        debug!("Received new engine request: getBlobsV4");

        // Spec (execution-apis amsterdam.md, getBlobsV4 §5): clients MUST support at
        // least 128 hashes, so exactly MAX must be accepted; reject only above it.
        if self.versioned_blob_hashes.len() > GET_BLOBS_V1_REQUEST_MAX_SIZE {
            return Err(RpcErr::TooLargeRequest);
        }

        // getBlobsV4 (EIP-8070) is an Amsterdam Engine API method. Per amsterdam.md
        // getBlobsV4 §6, its contract is to return `null` (not a bespoke -38005)
        // while syncing or otherwise unable to serve, mirroring getBlobsV3. Before
        // our canonical tip is at Amsterdam — including while the local head is still
        // pre-fork during sync — return `null` for every requested hash.
        let head_ts = context
            .storage
            .get_block_header(context.storage.get_latest_block_number().await?)?
            .map(|h| h.timestamp)
            .unwrap_or(0);
        if !context
            .storage
            .get_chain_config()
            .is_amsterdam_activated(head_ts)
        {
            let nulls: Vec<Option<BlobCellsAndProofsV1>> = (0..self.versioned_blob_hashes.len())
                .map(|_| None)
                .collect();
            return serde_json::to_value(nulls).map_err(|e| RpcErr::Internal(e.to_string()));
        }

        let mask = self.indices_bitarray;
        let hashes = self.versioned_blob_hashes.clone();
        let mempool = &context.blockchain.mempool;

        // Wrap the per-hash resolution in a 500 ms timeout; return whatever
        // is ready on elapse rather than erroring.
        let result = tokio::time::timeout(Duration::from_millis(500), async {
            let mut responses: Vec<Option<BlobCellsAndProofsV1>> = Vec::with_capacity(hashes.len());

            for versioned_hash in &hashes {
                // Resolve: versioned_hash → (tx_hash, blob_index).
                let lookup = mempool
                    .get_tx_and_blob_idx_by_versioned_hash(*versioned_hash)
                    .map_err(|e| RpcErr::Internal(e.to_string()))?;

                let Some((tx_hash, blob_idx)) = lookup else {
                    responses.push(None);
                    continue;
                };

                let bundle = mempool
                    .get_blobs_bundle(tx_hash)
                    .map_err(|e| RpcErr::Internal(e.to_string()))?;

                let Some(bundle) = bundle else {
                    responses.push(None);
                    continue;
                };

                // Cell proofs only exist in the cell-proof wrapper (version != 0,
                // Osaka). A version-0 bundle cannot supply per-cell proofs, so
                // return null rather than fabricating a zero proof.
                if bundle.version == 0 {
                    responses.push(None);
                    continue;
                }

                // Sparse length-128 matrices indexed by absolute column index:
                // null at every non-requested or unheld column. Cell and proof
                // are kept in lock-step (both Some or both None) so the caller can
                // verify each cell against its positionally-aligned proof.
                let mut cells: Vec<Option<Bytes>> = Vec::with_capacity(CELLS_PER_EXT_BLOB);
                let mut proofs: Vec<Option<Proof>> = Vec::with_capacity(CELLS_PER_EXT_BLOB);

                // Optionally compute all cells from the bundle blob (when the blob is present).
                #[cfg(feature = "c-kzg")]
                let computed_cells: Option<Vec<[u8; BYTES_PER_CELL]>> = bundle
                    .blobs
                    .get(blob_idx)
                    .and_then(|b| ethrex_common::crypto::kzg::compute_cells(b).ok());
                #[cfg(not(feature = "c-kzg"))]
                let computed_cells: Option<Vec<[u8; BYTES_PER_CELL]>> = None;

                for col in 0..CELLS_PER_EXT_BLOB {
                    // Non-requested column: null cell + null proof.
                    if (mask >> col) & 1 == 0 {
                        cells.push(None);
                        proofs.push(None);
                        continue;
                    }
                    // Cell: prefer stored (verified), then computed from full blob, else null.
                    let cell_opt = if let Some(cell) = mempool.get_cell(tx_hash, blob_idx, col) {
                        Some(Bytes::copy_from_slice(cell.as_ref()))
                    } else if let Some(ref all) = computed_cells {
                        all.get(col).map(|c| Bytes::copy_from_slice(c.as_ref()))
                    } else {
                        None
                    };
                    // Only emit a cell when we also have its sidecar proof; a cell
                    // without a verifiable proof is useless to the caller, so emit
                    // null for both rather than pairing a cell with no proof (or a
                    // proof with no cell).
                    let proof_idx = blob_idx * CELLS_PER_EXT_BLOB + col;
                    match (cell_opt, bundle.proofs.get(proof_idx)) {
                        (Some(cell), Some(&proof)) => {
                            cells.push(Some(cell));
                            proofs.push(Some(proof));
                        }
                        _ => {
                            cells.push(None);
                            proofs.push(None);
                        }
                    }
                }

                responses.push(Some(BlobCellsAndProofsV1 {
                    blob_cells: cells,
                    proofs,
                }));
            }

            Ok::<_, RpcErr>(responses)
        })
        .await;

        let responses = match result {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => return Err(e),
            // Timeout: return nulls for all hashes rather than erroring.
            Err(_elapsed) => (0..hashes.len()).map(|_| None).collect(),
        };

        serde_json::to_value(responses).map_err(|e| RpcErr::Internal(e.to_string()))
    }
}

/// Parse the 16-byte big-endian hex `indices_bitarray` param.
pub(crate) fn parse_indices_bitarray(value: &Value) -> Result<u128, RpcErr> {
    let hex_str = value
        .as_str()
        .ok_or_else(|| RpcErr::BadParams("indices_bitarray must be a hex string".into()))?;
    let stripped = hex_str.strip_prefix("0x").unwrap_or(hex_str);
    let bytes = hex::decode(stripped)
        .map_err(|_| RpcErr::BadParams("indices_bitarray: invalid hex".into()))?;
    if bytes.len() != 16 {
        return Err(RpcErr::BadParams(format!(
            "indices_bitarray must be 16 bytes, got {}",
            bytes.len()
        )));
    }
    let mut arr = [0u8; 16];
    arr.copy_from_slice(&bytes);
    Ok(u128::from_be_bytes(arr))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::default_context_with_storage;
    use ethrex_common::{
        Address, H256,
        types::{
            BYTES_PER_BLOB, BlobsBundle, CELLS_PER_EXT_BLOB, ChainConfig, Commitment, Proof,
            kzg_commitment_to_versioned_hash,
        },
    };
    use ethrex_storage::{EngineType, Store};

    fn sample_bundle(count: usize) -> (BlobsBundle, Vec<H256>) {
        let blobs = vec![[1u8; BYTES_PER_BLOB]; count];
        let commitments: Vec<Commitment> = (0..count).map(|i| [i as u8; 48]).collect();
        let proofs: Vec<Proof> = vec![[2u8; 48]; count * CELLS_PER_EXT_BLOB];

        let hashes = commitments
            .iter()
            .map(kzg_commitment_to_versioned_hash)
            .collect();

        let bundle = BlobsBundle {
            blobs,
            commitments,
            proofs,
            version: 1,
        };
        (bundle, hashes)
    }

    fn sample_v0_bundle(count: usize) -> (BlobsBundle, Vec<H256>) {
        let blobs = vec![[1u8; BYTES_PER_BLOB]; count];
        let commitments: Vec<Commitment> = (0..count).map(|i| [i as u8; 48]).collect();
        // v0 (EIP-4844): exactly one blob proof per blob.
        let proofs: Vec<Proof> = vec![[2u8; 48]; count];

        let hashes = commitments
            .iter()
            .map(kzg_commitment_to_versioned_hash)
            .collect();

        let bundle = BlobsBundle {
            blobs,
            commitments,
            proofs,
            version: 0,
        };
        (bundle, hashes)
    }

    fn blob_and_proof(bundle: &BlobsBundle, index: usize) -> BlobAndProofV2 {
        let start = index * CELLS_PER_EXT_BLOB;
        let end = start + CELLS_PER_EXT_BLOB;
        BlobAndProofV2 {
            blob: bundle.blobs[index],
            proofs: bundle.proofs[start..end].to_vec(),
        }
    }

    // `active` gates the blob-serving forks used by these tests: Osaka (getBlobsV3)
    // and Amsterdam (getBlobsV4). Both share the same activation here so the V3 and
    // V4 positive/negative paths can be driven by a single flag.
    fn chain_config(active: bool) -> ChainConfig {
        ChainConfig {
            chain_id: 1,
            shanghai_time: Some(0),
            cancun_time: Some(0),
            prague_time: Some(0),
            osaka_time: active.then_some(0),
            amsterdam_time: active.then_some(0),
            deposit_contract_address: Address::zero(),
            ..Default::default()
        }
    }

    async fn context_with_chain_config(osaka_active: bool) -> RpcApiContext {
        let mut storage =
            Store::new("test-blobs", EngineType::InMemory).expect("Failed to create test store");
        storage
            .set_chain_config(&chain_config(osaka_active))
            .await
            .expect("Failed to set chain config");
        default_context_with_storage(storage).await
    }

    #[tokio::test]
    async fn blobs_v2_returns_null_when_missing_one() {
        let context = context_with_chain_config(true).await;
        let (bundle, hashes) = sample_bundle(2);
        context
            .blockchain
            .mempool
            .add_blobs_bundle(H256::from_low_u64_be(1), bundle)
            .unwrap();

        let request = BlobsV2Request {
            blob_versioned_hashes: vec![hashes[0], H256::from_low_u64_be(999)],
        };

        let result = request.handle(context).await.unwrap();
        assert_eq!(result, serde_json::Value::Null);
    }

    #[tokio::test]
    async fn blobs_v2_returns_full_when_all_present() {
        let context = context_with_chain_config(true).await;
        let (bundle, hashes) = sample_bundle(2);
        context
            .blockchain
            .mempool
            .add_blobs_bundle(H256::from_low_u64_be(1), bundle.clone())
            .unwrap();

        let request = BlobsV2Request {
            blob_versioned_hashes: hashes.clone(),
        };

        let result = request.handle(context).await.unwrap();
        let expected = serde_json::to_value(vec![
            Some(blob_and_proof(&bundle, 0)),
            Some(blob_and_proof(&bundle, 1)),
        ])
        .unwrap();
        assert_eq!(result, expected);
    }

    #[tokio::test]
    async fn blobs_v3_returns_partial_results() {
        let context = context_with_chain_config(true).await;
        let (bundle, hashes) = sample_bundle(2);
        context
            .blockchain
            .mempool
            .add_blobs_bundle(H256::from_low_u64_be(1), bundle.clone())
            .unwrap();

        let request = BlobsV3Request {
            blob_versioned_hashes: vec![hashes[0], H256::from_low_u64_be(999)],
        };

        let result = request.handle(context).await.unwrap();
        let expected = serde_json::to_value(vec![Some(blob_and_proof(&bundle, 0)), None]).unwrap();
        assert_eq!(result, expected);
    }

    #[tokio::test]
    async fn blobs_v1_returns_v0_proof_before_osaka() {
        let context = context_with_chain_config(false).await;
        let (bundle, hashes) = sample_v0_bundle(1);
        context
            .blockchain
            .mempool
            .add_blobs_bundle(H256::from_low_u64_be(1), bundle.clone())
            .unwrap();

        let request = BlobsV1Request {
            blob_versioned_hashes: hashes,
        };

        let result = request.handle(context).await.unwrap();
        let expected = serde_json::to_value(vec![Some(BlobAndProofV1 {
            blob: bundle.blobs[0],
            proof: bundle.proofs[0],
        })])
        .unwrap();
        assert_eq!(result, expected);
    }

    #[tokio::test]
    async fn blobs_v1_returns_null_for_v1_sidecar_before_osaka() {
        // A v1 (cell-proof) sidecar can reach a pre-Osaka mempool, but getBlobsV1 can only
        // serve a single EIP-4844 blob proof, so it must report the blob as unavailable
        // rather than returning a cell proof in the blob-proof field.
        let context = context_with_chain_config(false).await;
        let (bundle, hashes) = sample_bundle(1);
        context
            .blockchain
            .mempool
            .add_blobs_bundle(H256::from_low_u64_be(1), bundle)
            .unwrap();

        let request = BlobsV1Request {
            blob_versioned_hashes: hashes,
        };

        let result = request.handle(context).await.unwrap();
        let expected = serde_json::to_value(vec![None::<BlobAndProofV1>]).unwrap();
        assert_eq!(result, expected);
    }

    #[tokio::test]
    async fn blobs_v1_rejects_after_osaka() {
        let context = context_with_chain_config(true).await;
        let request = BlobsV1Request {
            blob_versioned_hashes: vec![H256::from_low_u64_be(1)],
        };

        let err = request.handle(context).await.unwrap_err();
        assert!(matches!(err, RpcErr::UnsupportedFork(_)));
    }

    #[tokio::test]
    async fn blobs_v3_returns_null_before_osaka() {
        // Pre-Osaka, getBlobsV3 must not error: the spec contract is to return `null`
        // for blobs we don't have (which, pre-Osaka, is all of them). Returning a bespoke
        // -38005 here is a spec misread and spams the CL while the node is still syncing.
        let context = context_with_chain_config(false).await;
        let request = BlobsV3Request {
            blob_versioned_hashes: vec![H256::from_low_u64_be(1), H256::from_low_u64_be(2)],
        };

        let result = request.handle(context).await.unwrap();
        let expected = serde_json::to_value(vec![None::<BlobAndProofV2>, None]).unwrap();
        assert_eq!(result, expected);
    }

    #[tokio::test]
    async fn blobs_v3_rejects_too_many_hashes() {
        let context = context_with_chain_config(true).await;
        let request = BlobsV3Request {
            blob_versioned_hashes: vec![H256::zero(); GET_BLOBS_V1_REQUEST_MAX_SIZE + 1],
        };

        let err = request.handle(context).await.unwrap_err();
        assert!(matches!(err, RpcErr::TooLargeRequest));
    }

    #[tokio::test]
    async fn blobs_v3_accepts_exactly_max_size() {
        // Spec: clients MUST support at least MAX hashes, so exactly MAX must not be rejected.
        let context = context_with_chain_config(true).await;
        let request = BlobsV3Request {
            blob_versioned_hashes: vec![H256::zero(); GET_BLOBS_V1_REQUEST_MAX_SIZE],
        };
        let result = request.handle(context).await;
        assert!(!matches!(result, Err(RpcErr::TooLargeRequest)));
    }

    #[tokio::test]
    async fn blobs_v1_accepts_exactly_max_size_before_osaka() {
        let context = context_with_chain_config(false).await;
        let request = BlobsV1Request {
            blob_versioned_hashes: vec![H256::zero(); GET_BLOBS_V1_REQUEST_MAX_SIZE],
        };
        let result = request.handle(context).await;
        assert!(!matches!(result, Err(RpcErr::TooLargeRequest)));
    }

    // NOTE: the BlobsV4 (eth/72 / EIP-8070) unit tests were moved to
    // test/tests/rpc/eth72_engine_tests.rs.
}
