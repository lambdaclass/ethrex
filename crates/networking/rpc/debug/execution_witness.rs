use bytes::Bytes;
use ethrex_common::{
    serde_utils,
    types::{
        ChainConfig,
        block_execution_witness::{ExecutionWitness, GuestProgramStateError},
    },
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::debug;

use crate::{RpcApiContext, RpcErr, RpcHandler, types::block_identifier::BlockIdentifier};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcExecutionWitness {
    #[serde(
        serialize_with = "serde_utils::bytes::vec::serialize",
        deserialize_with = "serde_utils::bytes::vec::deserialize"
    )]
    pub state: Vec<Bytes>,
    #[serde(
        serialize_with = "serde_utils::bytes::vec::serialize",
        deserialize_with = "serde_utils::bytes::vec::deserialize"
    )]
    pub keys: Vec<Bytes>,
    #[serde(
        serialize_with = "serde_utils::bytes::vec::serialize",
        deserialize_with = "serde_utils::bytes::vec::deserialize"
    )]
    pub codes: Vec<Bytes>,
    #[serde(
        serialize_with = "serde_utils::bytes::vec::serialize",
        deserialize_with = "serde_utils::bytes::vec::deserialize"
    )]
    pub headers: Vec<Bytes>,
}

impl From<ExecutionWitness> for RpcExecutionWitness {
    fn from(value: ExecutionWitness) -> Self {
        Self {
            state: value.nodes.into_iter().map(Bytes::from).collect(),
            keys: value.keys.into_iter().map(Bytes::from).collect(),
            codes: value.codes.into_iter().map(Bytes::from).collect(),
            headers: value
                .block_headers_bytes
                .into_iter()
                .map(Bytes::from)
                .collect(),
        }
    }
}

// TODO: Ideally this would be a try_from but crate dependencies complicate this matter
pub fn execution_witness_from_rpc_chain_config(
    rpc_witness: RpcExecutionWitness,
    chain_config: ChainConfig,
    first_block_number: u64,
) -> Result<ExecutionWitness, GuestProgramStateError> {
    let witness = ExecutionWitness {
        codes: rpc_witness.codes.into_iter().map(|b| b.to_vec()).collect(),
        chain_config,
        first_block_number,
        block_headers_bytes: rpc_witness
            .headers
            .into_iter()
            .map(|b| b.to_vec())
            .collect(),
        nodes: rpc_witness.state.into_iter().map(|b| b.to_vec()).collect(),
        keys: rpc_witness.keys.into_iter().map(|b| b.to_vec()).collect(),
    };

    Ok(witness)
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
            debug!(
                "Requested execution witness from block: {from_block_number} to {to_block_number}",
            );
        } else {
            debug!("Requested execution witness for block: {from_block_number}",);
        }

        let mut blocks = Vec::new();
        let mut block_headers = Vec::new();
        for block_number in from_block_number..=to_block_number {
            let header = context
                .storage
                .get_block_header(block_number)?
                .ok_or(RpcErr::Internal("Could not get block header".to_string()))?;
            let parent_header = context
                .storage
                .get_block_header_by_hash(header.parent_hash)?
                .ok_or(RpcErr::Internal(
                    "Could not get parent block header".to_string(),
                ))?;
            block_headers.push(parent_header);
            let block = context
                .storage
                .get_block_by_hash(header.hash())
                .await?
                .ok_or(RpcErr::Internal("Could not get block body".to_string()))?;
            blocks.push(block);
        }

        let execution_witness = context
            .blockchain
            .generate_witness_for_blocks(&blocks)
            .await
            .map_err(|e| RpcErr::Internal(format!("Failed to build execution witness {e}")))?;

        let rpc_execution_witness = RpcExecutionWitness::from(execution_witness);

        serde_json::to_value(rpc_execution_witness)
            .map_err(|error| RpcErr::Internal(error.to_string()))
    }
}
