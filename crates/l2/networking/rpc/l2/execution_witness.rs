use ethrex_common::types::block_execution_witness::RpcExecutionWitness;
use ethrex_rpc::{RpcErr, debug::execution_witness::ExecutionWitnessRequest};
use serde_json::Value;
use tracing::debug;

use crate::rpc::RpcApiContext;

/// Copy of the L1 handler for execution witness, but
/// fetches fee configs from the rollup store, as they can vary from block to block.
pub async fn handle_execution_witness(
    request: &ExecutionWitnessRequest,
    context: RpcApiContext,
) -> Result<Value, RpcErr> {
    let from_block_number = request
        .from
        .resolve_block_number(&context.l1_ctx.storage)
        .await?
        .ok_or(RpcErr::Internal(
            "Failed to resolve block number".to_string(),
        ))?;
    let to_block_number = request
        .to
        .as_ref()
        .unwrap_or(&request.from)
        .resolve_block_number(&context.l1_ctx.storage)
        .await?
        .ok_or(RpcErr::Internal(
            "Failed to resolve block number".to_string(),
        ))?;

    if from_block_number > to_block_number {
        return Err(RpcErr::BadParams(
            "From block number is greater than To block number".to_string(),
        ));
    }

    if request.to.is_some() {
        debug!("Requested execution witness from block: {from_block_number} to {to_block_number}",);
    } else {
        debug!("Requested execution witness for block: {from_block_number}",);
    }

    let mut blocks = Vec::new();
    let mut fee_configs = Vec::new();
    for block_number in from_block_number..=to_block_number {
        let header = context
            .l1_ctx
            .storage
            .get_block_header(block_number)?
            .ok_or(RpcErr::Internal("Could not get block header".to_string()))?;
        let block = context
            .l1_ctx
            .storage
            .get_block_by_hash(header.hash())
            .await?
            .ok_or(RpcErr::Internal("Could not get block body".to_string()))?;
        let fee_config = context
            .rollup_store
            .get_fee_config_by_block(block_number)
            .await
            .map_err(|e| RpcErr::Internal(format!("Failed to get fee config {e}")))?
            .ok_or(RpcErr::Internal(format!(
                "Fee config not found for block {}",
                block_number
            )))?;

        blocks.push(block);
        fee_configs.push(fee_config);
    }

    let execution_witness = context
        .l1_ctx
        .blockchain
        .generate_witness_for_blocks_with_fee_configs(&blocks, Some(&fee_configs))
        .await
        .map_err(|e| RpcErr::Internal(format!("Failed to build execution witness {e}")))?;

    let rpc_execution_witness = RpcExecutionWitness::try_from(execution_witness)
        .map_err(|e| RpcErr::Internal(format!("Failed to build rpc execution witness {e}")))?;

    serde_json::to_value(rpc_execution_witness).map_err(|error| RpcErr::Internal(error.to_string()))
}
