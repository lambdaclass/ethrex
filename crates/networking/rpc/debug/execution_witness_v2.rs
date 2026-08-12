//! `debug_executionWitnessV2` and `debug_executionWitnessV2ByBlockHash`.
//!
//! The EIP-8297 counterparts of the V1 pair: same parameters, same
//! range semantics, a witness over binary-trie node encodings instead of MPT
//! ones. See [`ethrex_common::types::binary_execution_witness`] for the wire
//! shape and why it carries a format discriminator and an explicit
//! `preStateRoot` where V1 carries neither.
//!
//! Both refuse a header that is *not* binary-committed and point back at V1 —
//! per header, never per chain, so a pre-activation block on a scheduled chain
//! is answered by V1 and only by V1. See [`super::witness_guard`].
//!
//! There is no witness cache here. V1's cache is populated by the block
//! executor for MPT witnesses; nothing populates a binary one, and reading V1's
//! cache would hand back an MPT witness under a V2 label — the exact confusion
//! the format discriminator exists to prevent.

use ethrex_common::types::BlockHash;
use ethrex_common::types::binary_execution_witness::RpcBinaryExecutionWitness;
use serde_json::Value;
use tracing::debug;

use crate::{
    RpcApiContext, RpcErr, RpcHandler, debug::witness_guard::require_binary_committed,
    types::block_identifier::BlockIdentifier,
};

/// Build the witness for `blocks` and serialize it.
async fn witness_for(
    context: &RpcApiContext,
    blocks: &[ethrex_common::types::Block],
) -> Result<Value, RpcErr> {
    let witness = context
        .blockchain
        .generate_binary_witness_for_blocks(blocks)
        .await
        .map_err(|e| RpcErr::Internal(format!("Failed to build binary execution witness {e}")))?;
    serde_json::to_value(RpcBinaryExecutionWitness::from(witness))
        .map_err(|error| RpcErr::Internal(error.to_string()))
}

pub struct ExecutionWitnessV2Request {
    pub from: BlockIdentifier,
    pub to: Option<BlockIdentifier>,
}

impl RpcHandler for ExecutionWitnessV2Request {
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

        Ok(ExecutionWitnessV2Request { from, to })
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

        debug!(
            "Requested binary execution witness from block: {from_block_number} to \
             {to_block_number}"
        );

        let mut blocks = Vec::new();
        for block_number in from_block_number..=to_block_number {
            let header = context
                .storage
                .get_block_header(block_number)?
                .ok_or(RpcErr::Internal("Could not get block header".to_string()))?;
            // Per header, never per chain: a range that reaches back before the
            // activation is refused naming the first block V2 cannot answer for.
            require_binary_committed(&context.storage, &header)?;
            let block = context
                .storage
                .get_block_by_hash(header.hash())
                .await?
                .ok_or(RpcErr::Internal("Could not get block body".to_string()))?;
            blocks.push(block);
        }

        witness_for(&context, &blocks).await
    }
}

pub struct ExecutionWitnessV2ByBlockHashRequest {
    pub block_hash: BlockHash,
}

impl RpcHandler for ExecutionWitnessV2ByBlockHashRequest {
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

        Ok(ExecutionWitnessV2ByBlockHashRequest { block_hash })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        debug!(
            "Requested binary execution witness for block hash: {:?}",
            self.block_hash
        );

        let block = context
            .storage
            .get_block_by_hash(self.block_hash)
            .await?
            .ok_or(RpcErr::Internal("Block not found".to_string()))?;

        require_binary_committed(&context.storage, &block.header)?;

        witness_for(&context, std::slice::from_ref(&block)).await
    }
}
