use ethrex_common::{
    H256,
    serde_utils::{self},
    types::{Blob, CELLS_PER_EXT_BLOB, Proof, blobs_bundle::kzg_commitment_to_versioned_hash},
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

        if self.blob_versioned_hashes.len() >= GET_BLOBS_V1_REQUEST_MAX_SIZE {
            return Err(RpcErr::TooLargeRequest);
        }

        let mut res: Vec<Option<BlobAndProofV1>> = vec![None; self.blob_versioned_hashes.len()];

        for blobs_bundle in context.blockchain.mempool.get_blobs_bundle_pool()? {
            // Go over all blobs bundles from the blobs bundle pool.
            let blobs_in_bundle = blobs_bundle.blobs;
            let commitments_in_bundle = blobs_bundle.commitments;
            let proofs_in_bundle = blobs_bundle.proofs;

            // Go over all the commitments in each blobs bundle to calculate the blobs versioned hash.
            for (commitment, (blob, proof)) in commitments_in_bundle
                .iter()
                .zip(blobs_in_bundle.iter().zip(proofs_in_bundle.iter()))
            {
                let current_versioned_hash = kzg_commitment_to_versioned_hash(commitment);
                if let Some(index) = self
                    .blob_versioned_hashes
                    .iter()
                    .position(|&hash| hash == current_versioned_hash)
                {
                    // If the versioned hash is one of the requested we save its corresponding blob and proof in the returned vector. We store them in the same position as the versioned hash was received.
                    res[index] = Some(BlobAndProofV1 {
                        blob: *blob,
                        proof: *proof,
                    });
                }
            }
        }

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
        let res = get_blobs_and_proof(&self.blob_versioned_hashes, context, 2).await?;
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
        let res = get_blobs_and_proof(&self.blob_versioned_hashes, context, 3).await?;
        serde_json::to_value(res).map_err(|error| RpcErr::Internal(error.to_string()))
    }
}

/// Get blob data and proofs for a given list of blob versioned hashes.
async fn get_blobs_and_proof(
    blob_versioned_hashes: &[H256],
    context: RpcApiContext,
    version: u64,
) -> Result<Vec<Option<BlobAndProofV2>>, RpcErr> {
    if blob_versioned_hashes.len() >= GET_BLOBS_V1_REQUEST_MAX_SIZE {
        return Err(RpcErr::TooLargeRequest);
    }

    if let Some(current_block_header) = context
        .storage
        .get_block_header(context.storage.get_latest_block_number().await?)?
        && !context
            .storage
            .get_chain_config()
            .is_osaka_activated(current_block_header.timestamp)
    {
        // validation requested in https://github.com/ethereum/execution-apis/blob/a1d95fb555cd91efb3e0d6555e4ab556d9f5dd06/src/engine/osaka.md?plain=1#L130
        return Err(RpcErr::UnsuportedFork(format!(
            "getBlobsV{} engine only supported for Osaka",
            version
        )));
    };

    let mut res: Vec<Option<BlobAndProofV2>> = vec![None; blob_versioned_hashes.len()];

    for blobs_bundle in context.blockchain.mempool.get_blobs_bundle_pool()? {
        // Go over all blobs bundles from the blobs bundle pool.
        let blobs_in_bundle = blobs_bundle.blobs;
        let commitments_in_bundle = blobs_bundle.commitments;
        let proofs_in_bundle = blobs_bundle.proofs;

        // Go over all the commitments in each blobs bundle to calculate the blobs versioned hash.
        for (commitment, (blob, proofs)) in commitments_in_bundle.iter().zip(
            blobs_in_bundle
                .iter()
                .zip(proofs_in_bundle.chunks(CELLS_PER_EXT_BLOB)),
        ) {
            let current_versioned_hash = kzg_commitment_to_versioned_hash(commitment);
            if let Some(index) = blob_versioned_hashes
                .iter()
                .position(|&hash| hash == current_versioned_hash)
            {
                // If the versioned hash is one of the requested we save its corresponding blob and proof in the returned vector. We store them in the same position as the versioned hash was received.
                res[index] = Some(BlobAndProofV2 {
                    blob: *blob,
                    proofs: proofs.to_vec(),
                });
            }
        }
    }
    Ok(res)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::default_context_with_storage;
    use ethrex_common::{
        Address, H256,
        types::{BYTES_PER_BLOB, BlobsBundle, ChainConfig, Commitment, Proof},
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
    async fn blobs_v3_requires_osaka() {
        let context = context_with_chain_config(false).await;
        let request = BlobsV3Request {
            blob_versioned_hashes: vec![H256::from_low_u64_be(1)],
        };

        let err = request.handle(context).await.unwrap_err();
        assert!(matches!(err, RpcErr::UnsuportedFork(_)));
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
}
