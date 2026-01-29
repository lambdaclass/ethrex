use std::collections::BTreeMap;

use bytes::Bytes;
use ethrex_common::{
    Address, H256,
    types::{
        AccountState, BlockHeader, ChainConfig,
        block_execution_witness::{ExecutionWitness, GuestProgramStateError, RpcExecutionWitness},
    },
    utils::keccak,
};
use ethrex_rlp::{decode::RLPDecode, error::RLPDecodeError};
use ethrex_storage::hash_address;
use ethrex_trie::{EMPTY_TRIE_HASH, Node, NodeRef, Trie};
use serde_json::Value;
use tracing::debug;

use crate::{RpcApiContext, RpcErr, RpcHandler, types::block_identifier::BlockIdentifier};

// TODO: Ideally this would be a try_from but crate dependencies complicate this matter
// This function is used by ethrex-replay
pub fn execution_witness_from_rpc_chain_config(
    rpc_witness: RpcExecutionWitness,
    chain_config: ChainConfig,
    first_block_number: u64,
) -> Result<ExecutionWitness, GuestProgramStateError> {
    let mut initial_state_root = None;

    for h in &rpc_witness.headers {
        let header = BlockHeader::decode(h)?;
        if header.number == first_block_number - 1 {
            initial_state_root = Some(header.state_root);
            break;
        }
    }

    let initial_state_root = initial_state_root.ok_or_else(|| {
        GuestProgramStateError::Custom(format!(
            "header for block {} not found",
            first_block_number - 1
        ))
    })?;

    let nodes: BTreeMap<H256, Node> = rpc_witness
        .state
        .into_iter()
        .filter_map(|b| {
            if b == Bytes::from_static(&[0x80]) {
                // other implementations of debug_executionWitness allow for a `Null` node,
                // which would fail to decode in ours
                return None;
            }
            let hash = keccak(&b);
            Some(Node::decode(&b).map(|node| (hash, node)))
        })
        .collect::<Result<_, RLPDecodeError>>()?;

    // get state trie root and embed the rest of the trie into it
    let state_trie_root = if let NodeRef::Node(state_trie_root, _) =
        Trie::get_embedded_root(&nodes, initial_state_root)?
    {
        Some((*state_trie_root).clone())
    } else {
        None
    };

    // get all storage trie roots and embed the rest of the trie into it
    let state_trie = if let Some(state_trie_root) = &state_trie_root {
        Trie::new_temp_with_root(state_trie_root.clone().into())
    } else {
        Trie::new_temp()
    };
    let mut storage_trie_roots = BTreeMap::new();
    for key in &rpc_witness.keys {
        if key.len() != 20 {
            continue; // not an address
        }
        let address = Address::from_slice(key);
        let hashed_address = hash_address(&address);
        let Some(encoded_account) = state_trie.get(&hashed_address)? else {
            continue; // empty account, doesn't have a storage trie
        };
        let storage_root_hash = AccountState::decode(&encoded_account)?.storage_root;
        if storage_root_hash == *EMPTY_TRIE_HASH {
            continue; // empty storage trie
        }
        if !nodes.contains_key(&storage_root_hash) {
            continue; // storage trie isn't relevant to this execution
        }
        let node = Trie::get_embedded_root(&nodes, storage_root_hash)?;
        let NodeRef::Node(node, _) = node else {
            return Err(GuestProgramStateError::Custom(
                "execution witness does not contain non-empty storage trie".to_string(),
            ));
        };
        storage_trie_roots.insert(address, (*node).clone());
    }

    let witness = ExecutionWitness {
        codes: rpc_witness.codes.into_iter().map(|b| b.to_vec()).collect(),
        chain_config,
        first_block_number,
        block_headers_bytes: rpc_witness
            .headers
            .into_iter()
            .map(|b| b.to_vec())
            .collect(),
        state_trie_root,
        storage_trie_roots,
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
