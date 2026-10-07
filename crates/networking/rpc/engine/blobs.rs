use std::time::Duration;

use bytes::Bytes;
use ethrex_common::{
    H256,
    serde_utils::{self},
    types::{BYTES_PER_CELL, Blob, BlobsBundle, CELLS_PER_EXT_BLOB, Proof},
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
            .get_block_header(context.storage.get_latest_block_number()?)?
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
        let res = get_blobs_and_proof(&self.blob_versioned_hashes, context)?;
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
        let res = get_blobs_and_proof(&self.blob_versioned_hashes, context)?;
        serde_json::to_value(res).map_err(|error| RpcErr::Internal(error.to_string()))
    }
}

/// Get blob data and proofs for a given list of blob versioned hashes.
fn get_blobs_and_proof(
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
        .get_block_header(context.storage.get_latest_block_number()?)?
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
/// Both `blob_cells` and `proofs` are **compact** matrices carrying only the
/// columns selected by the request's `indices_bitarray`, in ascending column
/// order, so `blob_cells[k]` is the k-th requested column (execution-apis
/// amsterdam.md, `engine_getBlobsV4` §1). An entry is `null` only when that
/// requested cell is unavailable locally (§4). Cell and proof are always
/// both-null or both-present at a position, so a caller can verify each cell
/// against its positionally-aligned proof.
///
/// The field descriptions in EIP-8070 and execution-apis ("partial matrix ...
/// with `null` entries for missing cells") read as though this were a
/// fixed length-128 matrix indexed by absolute column, `null` at unrequested
/// indices. It is not: §1 is decisive, and a length-128 response fails every
/// partial-mask conformance test.
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

/// Derive all `CELLS_PER_EXT_BLOB` cells of `bundle.blobs[blob_idx]` from the full
/// blob. Returns `None` when the blob is elided from the bundle, or when KZG cell
/// support is not compiled in.
///
/// One call extends the entire blob (~3.7 ms in release) however few columns the
/// request selects, so callers derive at most once per blob and only when a
/// requested cell is not already held.
#[cfg(feature = "c-kzg")]
fn derive_blob_cells(bundle: &BlobsBundle, blob_idx: usize) -> Option<Vec<[u8; BYTES_PER_CELL]>> {
    ethrex_crypto::kzg::compute_cells(bundle.blobs.get(blob_idx)?).ok()
}

#[cfg(not(feature = "c-kzg"))]
fn derive_blob_cells(_bundle: &BlobsBundle, _blob_idx: usize) -> Option<Vec<[u8; BYTES_PER_CELL]>> {
    None
}

/// Request body for `engine_getBlobsV4`.
pub struct BlobsV4Request {
    /// Versioned blob hashes to look up.
    pub(crate) versioned_blob_hashes: Vec<H256>,
    /// Bitmask of column indices to return (bit i set ⇒ return column i).
    /// Encoded as a 16-byte little-endian hex string on the wire (column i →
    /// byte i/8, bit i%8; same CustodyBitmap layout as `custodyColumns`).
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
            .get_block_header(context.storage.get_latest_block_number()?)?
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

                // Compact matrices: only the mask-selected columns, ascending, so
                // `blob_cells[k]` is the k-th requested column (amsterdam.md
                // getBlobsV4 §1). Cell and proof stay in lock-step so the caller can
                // verify each cell against its positionally-aligned proof.
                let requested: Vec<usize> = (0..CELLS_PER_EXT_BLOB)
                    .filter(|col| (mask >> col) & 1 == 1)
                    .collect();
                let mut cells: Vec<Option<Bytes>> = Vec::with_capacity(requested.len());
                let mut proofs: Vec<Option<Proof>> = Vec::with_capacity(requested.len());

                // Cells sampled from peers are served as-is. Whatever is missing is
                // derived from the full blob; that extends all 128 columns at once, so
                // it runs at most once per blob and only on the first miss.
                let mut derived_cells: Option<Vec<[u8; BYTES_PER_CELL]>> = None;
                let mut derivation_attempted = false;

                for col in requested {
                    // Cell: prefer stored (verified), then derived from the full blob.
                    let cell_opt = if let Some(cell) = mempool.get_cell(tx_hash, blob_idx, col) {
                        Some(Bytes::copy_from_slice(cell.as_ref()))
                    } else {
                        if !derivation_attempted {
                            derivation_attempted = true;
                            derived_cells = derive_blob_cells(&bundle, blob_idx);
                        }
                        derived_cells
                            .as_ref()
                            .and_then(|all| all.get(col))
                            .map(|cell| Bytes::copy_from_slice(cell.as_ref()))
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

/// Parse the 16-byte little-endian hex `indices_bitarray` param (column `i` →
/// byte `i/8`, bit `i%8`; same CustodyBitmap layout as `custodyColumns`).
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
    Ok(u128::from_le_bytes(arr))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::{TestContext, default_context_with_storage};
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

    async fn context_with_chain_config(osaka_active: bool) -> TestContext {
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

        let result = request.handle(context.clone()).await.unwrap();
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

        let result = request.handle(context.clone()).await.unwrap();
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

        let result = request.handle(context.clone()).await.unwrap();
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

        let result = request.handle(context.clone()).await.unwrap();
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

        let result = request.handle(context.clone()).await.unwrap();
        let expected = serde_json::to_value(vec![None::<BlobAndProofV1>]).unwrap();
        assert_eq!(result, expected);
    }

    #[tokio::test]
    async fn blobs_v1_rejects_after_osaka() {
        let context = context_with_chain_config(true).await;
        let request = BlobsV1Request {
            blob_versioned_hashes: vec![H256::from_low_u64_be(1)],
        };

        let err = request.handle(context.clone()).await.unwrap_err();
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

        let result = request.handle(context.clone()).await.unwrap();
        let expected = serde_json::to_value(vec![None::<BlobAndProofV2>, None]).unwrap();
        assert_eq!(result, expected);
    }

    #[tokio::test]
    async fn blobs_v3_rejects_too_many_hashes() {
        let context = context_with_chain_config(true).await;
        let request = BlobsV3Request {
            blob_versioned_hashes: vec![H256::zero(); GET_BLOBS_V1_REQUEST_MAX_SIZE + 1],
        };

        let err = request.handle(context.clone()).await.unwrap_err();
        assert!(matches!(err, RpcErr::TooLargeRequest));
    }

    #[tokio::test]
    async fn blobs_v3_accepts_exactly_max_size() {
        // Spec: clients MUST support at least MAX hashes, so exactly MAX must not be rejected.
        let context = context_with_chain_config(true).await;
        let request = BlobsV3Request {
            blob_versioned_hashes: vec![H256::zero(); GET_BLOBS_V1_REQUEST_MAX_SIZE],
        };
        let result = request.handle(context.clone()).await;
        assert!(!matches!(result, Err(RpcErr::TooLargeRequest)));
    }

    #[tokio::test]
    async fn blobs_v1_accepts_exactly_max_size_before_osaka() {
        let context = context_with_chain_config(false).await;
        let request = BlobsV1Request {
            blob_versioned_hashes: vec![H256::zero(); GET_BLOBS_V1_REQUEST_MAX_SIZE],
        };
        let result = request.handle(context.clone()).await;
        assert!(!matches!(result, Err(RpcErr::TooLargeRequest)));
    }

    // NOTE: the BlobsV4 (eth/72 / EIP-8070) unit tests were moved to
    // test/tests/rpc/eth72_engine_tests.rs.
    /// Bundle whose proof at absolute index `blob * 128 + col` encodes `(blob, col)`,
    /// so a compact response's proof-to-column mapping can be checked positionally.
    fn sample_bundle_tagged(count: usize) -> (BlobsBundle, Vec<H256>) {
        let blobs = vec![[1u8; BYTES_PER_BLOB]; count];
        let commitments: Vec<Commitment> = (0..count).map(|i| [i as u8; 48]).collect();
        let mut proofs: Vec<Proof> = vec![[0u8; 48]; count * CELLS_PER_EXT_BLOB];
        for blob in 0..count {
            for col in 0..CELLS_PER_EXT_BLOB {
                let proof = &mut proofs[blob * CELLS_PER_EXT_BLOB + col];
                proof[0] = blob as u8;
                proof[1] = col as u8;
            }
        }
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

    /// Bundle as a sparse-blobpool node holds it after eth/72 elision: commitments
    /// and cell proofs are kept, the blobs themselves are gone, so cells can only
    /// come from what was sampled from peers.
    fn sample_bundle_elided(count: usize) -> (BlobsBundle, Vec<H256>) {
        let (mut bundle, hashes) = sample_bundle_tagged(count);
        bundle.blobs.clear();
        (bundle, hashes)
    }

    /// Store sampled cells whose bytes encode `(blob_index, column)`. Seeding the
    /// mempool keeps these tests independent of the `c-kzg` feature: the handler
    /// serves stored cells before it derives any.
    fn seed_cells(context: &TestContext, tx_hash: H256, blob_count: usize, columns: &[usize]) {
        let cells = (0..blob_count)
            .flat_map(|blob| {
                columns.iter().map(move |&col| {
                    let mut cell = Box::new([0u8; BYTES_PER_CELL]);
                    cell[0] = blob as u8;
                    cell[1] = col as u8;
                    (blob, col, cell)
                })
            })
            .collect();
        context
            .blockchain
            .mempool
            .store_cells(tx_hash, blob_count, cells)
            .unwrap();
    }

    fn hex_opt_array(value: &Value) -> Vec<Option<Vec<u8>>> {
        value
            .as_array()
            .expect("expected a JSON array")
            .iter()
            .map(|entry| {
                entry
                    .as_str()
                    .map(|s| hex::decode(s.trim_start_matches("0x")).expect("hex-encoded entry"))
            })
            .collect()
    }

    /// Decode the `blob_cells` / `proofs` arrays of one `getBlobsV4` entry, or
    /// `None` when the client reported that blob as unavailable.
    #[allow(clippy::type_complexity)]
    fn v4_entry(
        result: &Value,
        index: usize,
    ) -> Option<(Vec<Option<Vec<u8>>>, Vec<Option<Vec<u8>>>)> {
        let entry = result.as_array().expect("expected a JSON array")[index].as_object()?;
        Some((
            hex_opt_array(&entry["blobCells"]),
            hex_opt_array(&entry["proofs"]),
        ))
    }

    fn mask_of(columns: &[usize]) -> u128 {
        columns.iter().fold(0u128, |mask, col| mask | 1u128 << col)
    }

    /// The response carries only the mask-selected columns, ascending, so position
    /// `k` holds the k-th requested column — not column `k` of a length-128 matrix
    /// (execution-apis amsterdam.md, `engine_getBlobsV4` §1).
    #[tokio::test]
    async fn blobs_v4_compacts_requested_columns_in_ascending_order() {
        let context = context_with_chain_config(true).await;
        let (bundle, hashes) = sample_bundle_tagged(1);
        let tx_hash = H256::from_low_u64_be(1);
        context
            .blockchain
            .mempool
            .add_blobs_bundle(tx_hash, bundle)
            .unwrap();
        let columns = [3usize, 7, 100];
        seed_cells(&context, tx_hash, 1, &columns);

        let result = BlobsV4Request {
            versioned_blob_hashes: vec![hashes[0]],
            indices_bitarray: mask_of(&columns),
        }
        .handle(context.clone())
        .await
        .unwrap();

        let (cells, proofs) = v4_entry(&result, 0).expect("blob is available");
        assert_eq!(cells.len(), columns.len());
        assert_eq!(proofs.len(), columns.len());
        for (position, &col) in columns.iter().enumerate() {
            let cell = cells[position].as_ref().expect("requested cell is held");
            assert_eq!((cell[0], cell[1]), (0, col as u8), "cell at {position}");
            let proof = proofs[position]
                .as_ref()
                .expect("proof accompanies its cell");
            assert_eq!((proof[0], proof[1]), (0, col as u8), "proof at {position}");
        }
    }

    /// An empty mask selects nothing, so both matrices are empty rather than 128
    /// nulls.
    #[tokio::test]
    async fn blobs_v4_returns_empty_matrices_for_empty_mask() {
        let context = context_with_chain_config(true).await;
        let (bundle, hashes) = sample_bundle_tagged(1);
        context
            .blockchain
            .mempool
            .add_blobs_bundle(H256::from_low_u64_be(1), bundle)
            .unwrap();

        let result = BlobsV4Request {
            versioned_blob_hashes: vec![hashes[0]],
            indices_bitarray: 0,
        }
        .handle(context.clone())
        .await
        .unwrap();

        let (cells, proofs) = v4_entry(&result, 0).expect("blob is available");
        assert!(cells.is_empty());
        assert!(proofs.is_empty());
    }

    #[tokio::test]
    async fn blobs_v4_full_mask_returns_every_column_in_order() {
        let context = context_with_chain_config(true).await;
        let (bundle, hashes) = sample_bundle_tagged(1);
        let tx_hash = H256::from_low_u64_be(1);
        context
            .blockchain
            .mempool
            .add_blobs_bundle(tx_hash, bundle)
            .unwrap();
        let all: Vec<usize> = (0..CELLS_PER_EXT_BLOB).collect();
        seed_cells(&context, tx_hash, 1, &all);

        let result = BlobsV4Request {
            versioned_blob_hashes: vec![hashes[0]],
            indices_bitarray: u128::MAX,
        }
        .handle(context.clone())
        .await
        .unwrap();

        let (cells, proofs) = v4_entry(&result, 0).expect("blob is available");
        assert_eq!(cells.len(), CELLS_PER_EXT_BLOB);
        assert_eq!(proofs.len(), CELLS_PER_EXT_BLOB);
        for col in all {
            let cell = cells[col].as_ref().expect("cell is held");
            assert_eq!(cell[1], col as u8);
        }
    }

    /// A requested column we do not hold is `null` at its own position, and takes
    /// its proof down with it, leaving the surrounding positions intact
    /// (amsterdam.md `engine_getBlobsV4` §4). Uses an elided bundle, because with
    /// the full blob present every column is derivable and nothing is ever unheld.
    #[tokio::test]
    async fn blobs_v4_nulls_only_the_unheld_requested_columns() {
        let context = context_with_chain_config(true).await;
        let (bundle, hashes) = sample_bundle_elided(1);
        let tx_hash = H256::from_low_u64_be(1);
        context
            .blockchain
            .mempool
            .add_blobs_bundle(tx_hash, bundle)
            .unwrap();
        // Request three columns but only hold the outer two.
        seed_cells(&context, tx_hash, 1, &[3, 100]);

        let result = BlobsV4Request {
            versioned_blob_hashes: vec![hashes[0]],
            indices_bitarray: mask_of(&[3, 7, 100]),
        }
        .handle(context.clone())
        .await
        .unwrap();

        let (cells, proofs) = v4_entry(&result, 0).expect("blob is available");
        assert_eq!(cells.len(), 3);
        assert_eq!(cells[0].as_ref().expect("column 3 is held")[1], 3);
        assert!(cells[1].is_none(), "column 7 is not held");
        assert!(proofs[1].is_none(), "proof follows its cell");
        assert_eq!(cells[2].as_ref().expect("column 100 is held")[1], 100);
    }

    /// Proofs for the second blob of a bundle live at absolute index
    /// `blob_idx * 128 + col`, so a compact response must not read them from the
    /// first blob's range.
    #[tokio::test]
    async fn blobs_v4_maps_columns_of_a_later_blob() {
        let context = context_with_chain_config(true).await;
        let (bundle, hashes) = sample_bundle_tagged(2);
        let tx_hash = H256::from_low_u64_be(1);
        context
            .blockchain
            .mempool
            .add_blobs_bundle(tx_hash, bundle)
            .unwrap();
        let columns = [0usize, 42];
        seed_cells(&context, tx_hash, 2, &columns);

        let result = BlobsV4Request {
            versioned_blob_hashes: vec![hashes[1]],
            indices_bitarray: mask_of(&columns),
        }
        .handle(context.clone())
        .await
        .unwrap();

        let (cells, proofs) = v4_entry(&result, 0).expect("second blob is available");
        assert_eq!(cells.len(), columns.len());
        for (position, &col) in columns.iter().enumerate() {
            let cell = cells[position].as_ref().expect("cell is held");
            assert_eq!((cell[0], cell[1]), (1, col as u8), "cell at {position}");
            let proof = proofs[position].as_ref().expect("proof is present");
            assert_eq!((proof[0], proof[1]), (1, col as u8), "proof at {position}");
        }
    }

    /// Unavailable blobs are `null` at their own position; the array keeps the
    /// length and order of the request (amsterdam.md `engine_getBlobsV4` §3).
    #[tokio::test]
    async fn blobs_v4_nulls_unknown_hashes_and_keeps_order() {
        let context = context_with_chain_config(true).await;
        let (bundle, hashes) = sample_bundle_tagged(1);
        let tx_hash = H256::from_low_u64_be(1);
        context
            .blockchain
            .mempool
            .add_blobs_bundle(tx_hash, bundle)
            .unwrap();
        seed_cells(&context, tx_hash, 1, &[5]);

        let result = BlobsV4Request {
            versioned_blob_hashes: vec![H256::from_low_u64_be(999), hashes[0]],
            indices_bitarray: mask_of(&[5]),
        }
        .handle(context.clone())
        .await
        .unwrap();

        assert_eq!(result.as_array().expect("array").len(), 2);
        assert!(v4_entry(&result, 0).is_none(), "unknown hash is null");
        let (cells, _) = v4_entry(&result, 1).expect("known hash is served");
        assert_eq!(cells[0].as_ref().expect("cell is held")[1], 5);
    }

    #[tokio::test]
    async fn blobs_v4_returns_null_before_amsterdam() {
        let context = context_with_chain_config(false).await;
        let (bundle, hashes) = sample_bundle_tagged(1);
        let tx_hash = H256::from_low_u64_be(1);
        context
            .blockchain
            .mempool
            .add_blobs_bundle(tx_hash, bundle)
            .unwrap();
        seed_cells(&context, tx_hash, 1, &[0]);

        let result = BlobsV4Request {
            versioned_blob_hashes: vec![hashes[0]],
            indices_bitarray: u128::MAX,
        }
        .handle(context.clone())
        .await
        .unwrap();

        assert_eq!(result, serde_json::to_value(vec![None::<u8>]).unwrap());
    }

    #[tokio::test]
    async fn blobs_v4_rejects_too_many_hashes_but_accepts_the_maximum() {
        let context = context_with_chain_config(true).await;
        let at_max = BlobsV4Request {
            versioned_blob_hashes: vec![H256::zero(); GET_BLOBS_V1_REQUEST_MAX_SIZE],
            indices_bitarray: 1,
        }
        .handle(context.clone())
        .await;
        assert!(!matches!(at_max, Err(RpcErr::TooLargeRequest)));

        let over_max = BlobsV4Request {
            versioned_blob_hashes: vec![H256::zero(); GET_BLOBS_V1_REQUEST_MAX_SIZE + 1],
            indices_bitarray: 1,
        }
        .handle(context.clone())
        .await;
        assert!(matches!(over_max, Err(RpcErr::TooLargeRequest)));
    }

    /// With no sampled cells the handler falls back to deriving them from the full
    /// blob, which is the path hive's `execute` mode exercises: blob txs arrive over
    /// `eth_sendRawTransaction`, so nothing is ever sampled from peers.
    #[cfg(feature = "c-kzg")]
    #[tokio::test]
    async fn blobs_v4_derives_cells_when_none_were_sampled() {
        let context = context_with_chain_config(true).await;
        let (bundle, hashes) = sample_bundle_tagged(1);
        context
            .blockchain
            .mempool
            .add_blobs_bundle(H256::from_low_u64_be(1), bundle.clone())
            .unwrap();
        let columns = [0usize, 64, 127];

        let result = BlobsV4Request {
            versioned_blob_hashes: vec![hashes[0]],
            indices_bitarray: mask_of(&columns),
        }
        .handle(context.clone())
        .await
        .unwrap();

        let expected = ethrex_crypto::kzg::compute_cells(&bundle.blobs[0]).unwrap();
        let (cells, proofs) = v4_entry(&result, 0).expect("blob is available");
        assert_eq!(cells.len(), columns.len());
        for (position, &col) in columns.iter().enumerate() {
            let cell = cells[position].as_ref().expect("cell was derived");
            assert_eq!(
                cell.as_slice(),
                expected[col].as_slice(),
                "cell at {position}"
            );
            assert!(proofs[position].is_some(), "proof at {position}");
        }
    }
}
