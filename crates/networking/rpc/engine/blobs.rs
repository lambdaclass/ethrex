use ethrex_common::{
    H256,
    serde_utils::{self},
    types::{Blob, Proof},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::debug;

use crate::{
    rpc::{RpcApiContext, RpcHandler},
    utils::RpcErr,
};

// -> https://github.com/ethereum/execution-apis/blob/d41fdf10fabbb73c4d126fb41809785d830acace/src/engine/cancun.md?plain=1#L186
const GET_BLOBS_V1_REQUEST_MAX_SIZE: usize = 128;

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
                    // getBlobsV1 serves the single EIP-4844 blob proof. After the
                    // v1-only sidecar cleanup (go-ethereum#35191 / #6927), the
                    // mempool only admits v1 (128 cell proofs per blob), so this
                    // length check never succeeds for pool-admitted bundles and
                    // always reports null — matching geth's decline-don't-derive
                    // behavior.
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

    fn chain_config(osaka_active: bool) -> ChainConfig {
        ChainConfig {
            chain_id: 1,
            shanghai_time: Some(0),
            cancun_time: Some(0),
            prague_time: Some(0),
            osaka_time: osaka_active.then_some(0),
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
    async fn blobs_v1_pool_rejects_v0_injection() {
        // After #6927 the mempool insert path requires the v1 cell-proof layout.
        // getBlobsV1 therefore never sees a single-proof sidecar from the pool and
        // returns null for admitted (v1) bundles — see the next test.
        let context = context_with_chain_config(false).await;
        let (bundle, _) = sample_v0_bundle(1);
        let err = context
            .blockchain
            .mempool
            .add_blobs_bundle(H256::from_low_u64_be(1), bundle)
            .expect_err("v0 bundles must not enter the pool");
        assert!(
            err.to_string().contains("invalid v1 blob bundle"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn blobs_v1_returns_null_for_v1_sidecar_before_osaka() {
        // Post go-ethereum#35191 / #6927 the mempool is v1-only. getBlobsV1 cannot
        // convert cell proofs to a single EIP-4844 blob proof, so it reports the
        // blob unavailable (null) rather than returning a cell proof in the
        // blob-proof field — matching geth's decline-don't-derive behavior.
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
}
