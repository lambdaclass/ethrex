use ethrex_common::types::{
    ChainConfig,
    block_execution_witness::{ExecutionWitness, GuestProgramStateError, RpcExecutionWitness},
};
use serde_json::Value;

use crate::{RpcApiContext, RpcErr, RpcHandler, types::block_identifier::BlockIdentifier};

// TODO: Ideally this would be a try_from but crate dependencies complicate this matter
// This function is used by ethrex-replay
pub fn execution_witness_from_rpc_chain_config(
    _rpc_witness: RpcExecutionWitness,
    _chain_config: ChainConfig,
    _first_block_number: u64,
) -> Result<ExecutionWitness, GuestProgramStateError> {
    // MPT-based ExecutionWitness construction removed; binary trie branch uses a different format
    todo!("execution_witness_from_rpc_chain_config not supported on binary trie branch")
}

pub struct ExecutionWitnessRequest {
    pub from: BlockIdentifier,
    pub to: Option<BlockIdentifier>,
}

impl RpcHandler for ExecutionWitnessRequest {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() > 2 {
            return Err(RpcErr::BadParams(format!(
                "Expected one or two params and {} were provided",
                params.len()
            )));
        }

        let from = BlockIdentifier::parse(params[0].clone(), 0)?;
        let to = if let Some(param) = params.get(1) {
            Some(BlockIdentifier::parse(param.clone(), 1)?)
        } else {
            None
        };

        Ok(ExecutionWitnessRequest { from, to })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let from_block_number = self
            .from
            .resolve_block_number(&context.storage)
            .await?
            .ok_or(RpcErr::Internal(
                "Failed to resolve block number".to_string(),
            ))?;
        let to_block_number = self
            .to
            .as_ref()
            .unwrap_or(&self.from)
            .resolve_block_number(&context.storage)
            .await?
            .ok_or(RpcErr::Internal(
                "Failed to resolve block number".to_string(),
            ))?;

        if from_block_number > to_block_number {
            return Err(RpcErr::BadParams(
                "From block number is greater than To block number".to_string(),
            ));
        }

        if self.to.is_some() {
            tracing::debug!(
                "Requested execution witness from block: {from_block_number} to {to_block_number}",
            );
        } else {
            tracing::debug!("Requested execution witness for block: {from_block_number}",);
        }

        let mut blocks = Vec::new();
        for block_number in from_block_number..=to_block_number {
            let header = context
                .storage
                .get_block_header(block_number)?
                .ok_or(RpcErr::Internal("Could not get block header".to_string()))?;
            let block = context
                .storage
                .get_block_by_hash(header.hash())
                .await?
                .ok_or(RpcErr::Internal("Could not get block body".to_string()))?;
            blocks.push(block);
        }

        if blocks.len() == 1 {
            // Check if we have a cached witness for this block
            // Use raw JSON bytes path to avoid deserialization + re-serialization
            let block = &blocks[0];
            if let Some(json_bytes) = context
                .storage
                .get_witness_json_bytes(block.header.number, block.hash())?
            {
                // Parse directly to Value - witness is already in RPC format
                return serde_json::from_slice(&json_bytes)
                    .map_err(|e| RpcErr::Internal(format!("Failed to parse cached witness: {e}")));
            }
        }

        let binary_trie_witness = context
            .blockchain
            .generate_witness_for_blocks(&blocks)
            .await
            .map_err(|e| RpcErr::Internal(format!("Failed to build execution witness {e}")))?;

        serde_json::to_value(binary_trie_witness)
            .map_err(|error| RpcErr::Internal(error.to_string()))
    }
}
