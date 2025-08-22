use ethrex_common::types::{BlockHash, batch::Batch};
use ethrex_storage::Store;
use serde::Serialize;
use serde_json::Value;
use tracing::debug;

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
    pub batch_number: u64,
    pub block_hashes: bool,
}

impl RpcHandler for GetBatchByBatchNumberRequest {
    fn parse(params: &Option<Vec<Value>>) -> Result<GetBatchByBatchNumberRequest, RpcErr> {
        let params = params.as_ref().ok_or(ethrex_rpc::RpcErr::BadParams(
            "No params provided".to_owned(),
        ))?;
        if params.len() != 2 {
            return Err(ethrex_rpc::RpcErr::BadParams(
                "Expected 2 params".to_owned(),
            ))?;
        };
        // Parse BatchNumber
        let hex_str = serde_json::from_value::<String>(params[0].clone())
            .map_err(|e| ethrex_rpc::RpcErr::BadParams(e.to_string()))?;

        // Check that the BatchNumber is 0x prefixed
        let hex_str = hex_str
            .strip_prefix("0x")
            .ok_or(ethrex_rpc::RpcErr::BadHexFormat(0))?;

        // Parse hex string
        let batch_number =
            u64::from_str_radix(hex_str, 16).map_err(|_| ethrex_rpc::RpcErr::BadHexFormat(0))?;

        let block_hashes = serde_json::from_value(params[1].clone())?;

        Ok(GetBatchByBatchNumberRequest {
            batch_number,
            block_hashes,
        })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        debug!("Requested batch with number: {}", self.batch_number);
        let Some(batch) = context.rollup_store.get_batch(self.batch_number).await? else {
            return Ok(Value::Null);
        };
        let rpc_batch = RpcBatch::build(batch, self.block_hashes, &context.l1_ctx.storage).await?;

        serde_json::to_value(&rpc_batch).map_err(|error| RpcErr::Internal(error.to_string()))
    }
}
