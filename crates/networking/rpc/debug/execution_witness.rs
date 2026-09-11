use ethrex_common::types::block_execution_witness::RpcExecutionWitness;
use serde_json::Value;
use tracing::debug;

use crate::{RpcApiContext, RpcErr, RpcHandler, types::block_identifier::BlockIdentifier};

pub struct ExecutionWitnessRequest {
    pub from: BlockIdentifier,
    pub to: Option<BlockIdentifier>,
}

impl RpcHandler for ExecutionWitnessRequest {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        // The lower bound matters as much as the upper one: `params[0]` below
        // panics on an empty array, which drops the connection with no
        // JSON-RPC response at all instead of reporting a bad request.
        if params.is_empty() || params.len() > 2 {
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
            debug!(
                "Requested execution witness from block: {from_block_number} to {to_block_number}",
            );
        } else {
            debug!("Requested execution witness for block: {from_block_number}",);
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

        let execution_witness = context
            .blockchain
            .generate_witness_for_blocks(&blocks)
            .await
            .map_err(|e| RpcErr::Internal(format!("Failed to build execution witness {e}")))?;

        let rpc_execution_witness = RpcExecutionWitness::try_from(execution_witness)
            .map_err(|e| RpcErr::Internal(format!("Failed to create rpc execution witness {e}")))?;

        serde_json::to_value(rpc_execution_witness)
            .map_err(|error| RpcErr::Internal(error.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// `debug_executionWitness` with `params: []` used to index `params[0]` past
    /// the end of an empty vector. The panic unwound through the connection task,
    /// so the caller got a dropped connection and no JSON-RPC response at all.
    #[test]
    fn empty_params_are_rejected_instead_of_panicking() {
        let err = ExecutionWitnessRequest::parse(&Some(vec![]))
            .err()
            .expect("empty params must not parse");
        assert!(
            matches!(err, RpcErr::BadParams(_)),
            "expected BadParams, got {err:?}"
        );
    }

    #[test]
    fn missing_params_are_rejected() {
        let err = ExecutionWitnessRequest::parse(&None)
            .err()
            .expect("absent params must not parse");
        assert!(
            matches!(err, RpcErr::BadParams(_)),
            "expected BadParams, got {err:?}"
        );
    }

    #[test]
    fn one_or_two_block_identifiers_parse() {
        let single = ExecutionWitnessRequest::parse(&Some(vec![json!("latest")]))
            .expect("a single block identifier is a valid request");
        assert!(single.to.is_none());

        let range = ExecutionWitnessRequest::parse(&Some(vec![json!("0x1"), json!("0x2")]))
            .expect("a block range is a valid request");
        assert!(range.to.is_some());
    }

    #[test]
    fn more_than_two_params_are_rejected() {
        let err =
            ExecutionWitnessRequest::parse(&Some(vec![json!("0x1"), json!("0x2"), json!("0x3")]))
                .err()
                .expect("three params must not parse");
        assert!(
            matches!(err, RpcErr::BadParams(_)),
            "expected BadParams, got {err:?}"
        );
    }
}
