use ethrex_common::types::BlockHash;
use ethrex_trie::execution_witness_to_rpc;
use serde_json::Value;
use tracing::debug;

use crate::{RpcApiContext, RpcErr, RpcHandler};

pub struct ExecutionWitnessByBlockHashRequest {
    pub block_hash: BlockHash,
}

impl RpcHandler for ExecutionWitnessByBlockHashRequest {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() != 1 {
            return Err(RpcErr::BadParams(format!(
                "Expected one param and {} were provided",
                params.len()
            )));
        }

        let block_hash: BlockHash = serde_json::from_value(params[0].clone())
            .map_err(|e| RpcErr::BadParams(format!("Invalid block hash: {e}")))?;

        Ok(ExecutionWitnessByBlockHashRequest { block_hash })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        debug!(
            "Requested execution witness for block hash: {:?}",
            self.block_hash
        );

        let block = context
            .storage
            .get_block_by_hash(self.block_hash)
            .await?
            .ok_or(RpcErr::Internal("Block not found".to_string()))?;

        // Check if we have a cached witness for this block
        if let Some(json_bytes) = context
            .storage
            .get_witness_json_bytes(block.header.number, block.hash())?
        {
            return serde_json::from_slice(&json_bytes)
                .map_err(|e| RpcErr::Internal(format!("Failed to parse cached witness: {e}")));
        }

        let execution_witness = context
            .blockchain
            .generate_witness_for_blocks(&[block])
            .await
            .map_err(|e| RpcErr::Internal(format!("Failed to build execution witness {e}")))?;

        let rpc_execution_witness = execution_witness_to_rpc(execution_witness);

        serde_json::to_value(rpc_execution_witness)
            .map_err(|error| RpcErr::Internal(error.to_string()))
    }
}
