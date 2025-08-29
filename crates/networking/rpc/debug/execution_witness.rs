use std::collections::HashMap;

use bytes::Bytes;
use ethrex_common::{
    Address, H256, serde_utils,
    types::{
        BlockHeader, ChainConfig,
        block_execution_witness::{ExecutionWitnessError, ExecutionWitnessResult},
    },
};
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};
use keccak_hash::keccak;
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

impl From<ExecutionWitnessResult> for RpcExecutionWitness {
    fn from(value: ExecutionWitnessResult) -> Self {
        let mut keys = Vec::new();

        let touched_account_storage_slots = value.touched_account_storage_slots;

        for (address, touched_storage_slots) in touched_account_storage_slots {
            keys.push(Bytes::from(address.as_bytes().to_vec()));
            for slot in touched_storage_slots.iter() {
                keys.push(Bytes::from(slot.as_bytes().to_vec()));
            }
        }

        Self {
            state: value
                .state_nodes
                .values()
                .map(|v| Bytes::from(v.as_ref().clone()))
                .collect(),
            keys,
            codes: value.codes.values().cloned().collect(),
            headers: value
                .block_headers
                .values()
                .map(BlockHeader::encode_to_vec)
                .map(Into::into)
                .collect(),
        }
    }
}

// TODO: Ideally this would be a try_from but crate dependencies complicate this matter
pub fn execution_witness_from_rpc_chain_config(
    rpc_witness: RpcExecutionWitness,
    chain_config: ChainConfig,
    first_block_number: u64,
) -> Result<ExecutionWitnessResult, ExecutionWitnessError> {
    let codes = rpc_witness
        .codes
        .iter()
        .map(|code| (keccak_hash::keccak(code), code.clone()))
        .collect::<HashMap<_, _>>();

    let block_headers = rpc_witness
        .headers
        .iter()
        .map(Bytes::as_ref)
        .map(BlockHeader::decode)
        .collect::<Result<Vec<_>, _>>()
        .expect("Failed to decode block headers from RpcExecutionWitness")
        .iter()
        .map(|header| (header.number, header.clone()))
        .collect::<HashMap<_, _>>();

    let parent_number = first_block_number
        .checked_sub(1)
        .ok_or(ExecutionWitnessError::Custom(
            "First block number cannot be zero".to_string(),
        ))?;

    let parent_header = block_headers.get(&parent_number).cloned().ok_or(
        ExecutionWitnessError::MissingParentHeaderOf(first_block_number),
    )?;

    let mut state_nodes = HashMap::new();
    for node in rpc_witness.state.into_iter() {
        state_nodes.insert(keccak(&node), node.to_vec().into());
    }

    let mut touched_account_storage_slots = HashMap::new();
    let mut address = Address::default();
    for bytes in rpc_witness.keys {
        if bytes.len() == Address::len_bytes() {
            address = Address::from_slice(&bytes);
        } else {
            let slot = H256::from_slice(&bytes);
            // Insert in the vec of the address value
            touched_account_storage_slots
                .entry(address)
                .or_insert_with(Vec::new)
                .push(slot);
        }
    }

    let mut witness = ExecutionWitnessResult {
        codes,
        state_trie: None, // `None` because we'll rebuild the tries afterwards
        storage_tries: HashMap::new(), // empty map because we'll rebuild the tries afterwards
        block_headers,
        chain_config,
        parent_block_header: parent_header,
        state_nodes,
        touched_account_storage_slots,
    };

    witness.rebuild_state_trie()?;

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
