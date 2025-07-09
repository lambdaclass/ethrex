use ethrex_common::types::{BlockHash, batch::Batch};
use ethrex_rpc::types::block_identifier::{BlockIdentifier, BlockTag};
use ethrex_storage::Store;
use serde::Serialize;
use serde_json::Value;
use tracing::info;

use crate::{
    rpc::{RpcApiContext, RpcHandler},
    utils::RpcErr,
};

#[derive(Serialize)]
pub struct RpcBatch {
    #[serde(flatten)]
    pub batch: Batch,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_hashes: Option<Vec<BlockHash>>,
}

impl RpcBatch {
    pub async fn build(batch: Batch, block_hashes: bool, store: &Store) -> Result<Self, RpcErr> {
        let block_hashes = if block_hashes {
            Some(get_block_hashes(
                batch.first_block,
                batch.last_block,
                store,
            )?)
        } else {
            None
        };

        Ok(RpcBatch {
            batch,
            block_hashes,
        })
    }
}

fn get_block_hashes(
    first_block: u64,
    last_block: u64,
    store: &Store,
) -> Result<Vec<BlockHash>, RpcErr> {
    let mut block_hashes = Vec::new();
    for block_number in first_block..=last_block {
        let header = store
            .get_block_header(block_number)?
            .ok_or(RpcErr::Internal(format!(
                "Failed to retrieve block header for block number {block_number}"
            )))?;
        let hash = header.hash();
        block_hashes.push(hash);
    }
    Ok(block_hashes)
}

pub struct GetBatchByBatchNumberRequest {
    pub batch_id: BlockIdentifier,
    pub block_hashes: bool,
}

impl RpcHandler for GetBatchByBatchNumberRequest {
    fn parse(params: &Option<Vec<Value>>) -> Result<GetBatchByBatchNumberRequest, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::L1RpcErr(ethrex_rpc::RpcErr::BadParams(
                "No params provided".to_owned(),
            )))?;
        if params.len() != 2 {
            return Err(RpcErr::L1RpcErr(ethrex_rpc::RpcErr::BadParams(
                "Expected exactly 2 params".to_owned(),
            )));
        }

        let batch_id = BlockIdentifier::parse(params[0].clone(), 0).map_err(RpcErr::L1RpcErr)?;
        let block_hashes = serde_json::from_value(params[1].clone())
            .map_err(|e| RpcErr::Internal(e.to_string()))?;

        Ok(GetBatchByBatchNumberRequest {
            batch_id,
            block_hashes,
        })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        info!("Requested batch with id: {}", self.batch_id);

        let batch_number = match &self.batch_id {
            BlockIdentifier::Number(n) => *n,
            // both latest and pending now return the “in-flight” batch
            BlockIdentifier::Tag(BlockTag::Latest) | BlockIdentifier::Tag(BlockTag::Pending) => {
                context.rollup_store.get_latest_batch().await?
            }
            other => {
                return Err(RpcErr::Internal(format!(
                    "unsupported batch tag: {:?}",
                    other
                )));
            }
        };

        let batch = match context.rollup_store.get_batch(batch_number).await? {
            Some(b) => b,
            None => return Ok(Value::Null),
        };

        let rpc_batch = RpcBatch::build(batch, self.block_hashes, &context.l1_ctx.storage).await?;
        serde_json::to_value(rpc_batch).map_err(|e| RpcErr::Internal(e.to_string()))
    }
}
