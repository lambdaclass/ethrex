use ethrex_common::{
    H256,
    serde_utils::{self},
    types::{Blob, Fork, Proof, blobs_bundle::kzg_commitment_to_versioned_hash},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::info;

use crate::{
    rpc::{RpcApiContext, RpcHandler},
    utils::RpcErr,
};

// -> https://github.com/ethereum/execution-apis/blob/d41fdf10fabbb73c4d126fb41809785d830acace/src/engine/cancun.md?plain=1#L186
const GET_BLOBS_REQUEST_MAX_SIZE: usize = 128;

#[derive(Debug, Serialize, Deserialize)]
pub struct BlobsV1Request {
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
        info!("Received new engine request: Requested Blobs");
        if self.blob_versioned_hashes.len() >= GET_BLOBS_REQUEST_MAX_SIZE {
            return Err(RpcErr::TooLargeRequest);
        }
        if context.blockchain.current_fork().await? >= Fork::Osaka {
            // validation requested in https://github.com/ethereum/execution-apis/blob/a1d95fb555cd91efb3e0d6555e4ab556d9f5dd06/src/engine/osaka.md?plain=1#L130
            return Err(RpcErr::UnsuportedFork(
                "getBlobsV1 engine request not supported for Osaka".to_string(),
            ));
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

#[derive(Debug, Serialize, Deserialize)]
pub struct BlobsV2Request {
    blob_versioned_hashes: Vec<H256>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlobAndProofV2 {
    #[serde(with = "serde_utils::blob")]
    pub blob: Blob,
    #[serde(with = "serde_utils::bytes48::vec")]
    pub proofs: Vec<Proof>,
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
        info!("Received new engine request: Requested Blobs");
        if self.blob_versioned_hashes.len() >= GET_BLOBS_REQUEST_MAX_SIZE {
            return Err(RpcErr::TooLargeRequest);
        }

        let mut res: Vec<Option<BlobAndProofV2>> = vec![None; self.blob_versioned_hashes.len()];

        for blobs_bundle in context.blockchain.mempool.get_blobs_bundle_pool()? {
            // Go over all blobs bundles from the blobs bundle pool.
            let blobs_in_bundle = blobs_bundle.blobs;
            let commitments_in_bundle = blobs_bundle.commitments;
            let proofs_in_bundle = blobs_bundle.proofs;

            let cells_per_ext_blob = proofs_in_bundle.len() / blobs_in_bundle.len();

            // Go over all the commitments in each blobs bundle to calculate the blobs versioned hash.
            for (i, (commitment, blob)) in commitments_in_bundle
                .iter()
                .zip(blobs_in_bundle.iter())
                .enumerate()
            {
                let current_versioned_hash = kzg_commitment_to_versioned_hash(commitment);
                if let Some(index) = self
                    .blob_versioned_hashes
                    .iter()
                    .position(|&hash| hash == current_versioned_hash)
                {
                    // If the versioned hash is one of the requested we save its corresponding blob and its proofs in the returned vector. We store them in the same position as the versioned hash was received.
                    res[index] = Some(BlobAndProofV2 {
                        blob: *blob,
                        proofs: proofs_in_bundle
                            [i * cells_per_ext_blob..(i + 1) * cells_per_ext_blob]
                            .to_vec(),
                    });
                }
            }
        }

        serde_json::to_value(res).map_err(|error| RpcErr::Internal(error.to_string()))
    }
}
