use ethrex_common::{
    H256, serde_utils,
    types::{Blob, Proof},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::info;

use crate::{
    rpc::{RpcApiContext, RpcHandler},
    utils::RpcErr,
};

// -> https://github.com/ethereum/execution-apis/blob/main/src/engine/cancun.md#specification-3
const GET_BLOBS_V1_REQUEST_MAX_SIZE: usize = 128;

#[derive(Debug, Serialize, Deserialize)]
pub struct BlobsV1Request {
    blob_versioned_hashes: Vec<H256>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlobsV1Response {
    #[serde(with = "serde_utils::blob::vec")]
    pub blobs: Vec<Blob>,
    #[serde(with = "serde_utils::bytes48::vec")]
    pub proofs: Vec<Proof>,
}

impl std::fmt::Display for BlobsV1Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Blob versioned hashes: {:#?}",
            self.blob_versioned_hashes
        )
    }
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
        info!("Received new engine request: {self}");
        if self.blob_versioned_hashes.len() >= GET_BLOBS_V1_REQUEST_MAX_SIZE {
            return Err(RpcErr::TooLargeRequest);
        }

        let mut blobs_bundles = Vec::new();
        for hash in self.blob_versioned_hashes.iter() {
            let blob_bundle = context.blockchain.mempool.get_blobs_bundle(*hash)?;
            match blob_bundle {
                None => blobs_bundles.push(
                    serde_json::to_value(blob_bundle)
                        .map_err(|error| RpcErr::Internal(error.to_string()))?,
                ),
                Some(blob_bundle) => blobs_bundles.push(
                    serde_json::to_value(BlobsV1Response {
                        blobs: blob_bundle.blobs,
                        proofs: blob_bundle.proofs,
                    })
                    .map_err(|error| RpcErr::Internal(error.to_string()))?,
                ),
            }
        }

        serde_json::to_value(blobs_bundles).map_err(|error| RpcErr::Internal(error.to_string()))
    }
}
