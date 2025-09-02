use std::collections::{HashMap, HashSet};

use bytes::Bytes;
use ethrex_common::{
    Address, H256, serde_utils,
    types::{
        AccountState, BlockHeader, ChainConfig,
        block_execution_witness::{ExecutionWitnessError, ExecutionWitnessResult},
    },
};
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};
use ethrex_storage::{hash_address, hash_key};
use ethrex_trie::{NodeHash, Trie, TrieLogger};
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
            keys.push(Bytes::copy_from_slice(address.as_bytes()));
            for slot in touched_storage_slots.iter() {
                keys.push(Bytes::copy_from_slice(slot.as_bytes()));
            }
        }

        Self {
            state: value
                .state_nodes
                .values()
                .cloned()
                .map(Into::into)
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
    dbg!("a");

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
    dbg!("b");

    let parent_number = first_block_number
        .checked_sub(1)
        .ok_or(ExecutionWitnessError::Custom(
            "First block number cannot be zero".to_string(),
        ))?;

    let parent_header = block_headers.get(&parent_number).cloned().ok_or(
        ExecutionWitnessError::MissingParentHeaderOf(first_block_number),
    )?;

    let mut state_nodes = HashMap::new();
    for node in rpc_witness.state.iter() {
        state_nodes.insert(keccak(node), node.to_vec());
    }

    let state_trie = Trie::from_nodes(
        NodeHash::Hashed(parent_header.state_root),
        state_nodes
            .clone()
            .into_iter()
            .map(|(k, v)| (NodeHash::Hashed(k), v))
            .collect(),
    )
    .map_err(|e| ExecutionWitnessError::RebuildTrie(format!("State trie: {e}")))?;

    dbg!("c");

    // let mut touched_account_storage_slots = HashMap::new();
    // let mut address = Address::default();
    // for bytes in rpc_witness.keys {
    //     if bytes.len() == Address::len_bytes() {
    //         address = Address::from_slice(&bytes);
    //     } else {
    //         let slot = H256::from_slice(&bytes);
    //         // Insert in the vec of the address value
    //         touched_account_storage_slots
    //             .entry(address)
    //             .or_insert_with(Vec::new)
    //             .push(slot);
    //     }
    // }
    let mut addresses = HashSet::new();
    for bytes in rpc_witness.keys.iter() {
        if bytes.len() == Address::len_bytes() {
            let address = Address::from_slice(bytes);
            addresses.insert(address);
        }
    }
    let mut keys = HashSet::new();
    for bytes in rpc_witness.keys.iter() {
        if bytes.len() == H256::len_bytes() {
            let slot = H256::from_slice(bytes);
            keys.insert(slot);
        }
    }
    dbg!("d");

    let mut storage_trie_nodes_by_address = HashMap::new();
    let mut touched_account_storage_slots = HashMap::new();
    for address in addresses {
        // if storage_slots.is_empty() {
        //     continue;
        // }
        dbg!("1");

        if let Some(account_state_rlp) = state_trie
            .get(&hash_address(&address))
            .map_err(|e| ExecutionWitnessError::Custom(e.to_string()))?
        {
            dbg!("2");
            let AccountState { storage_root, .. } = AccountState::decode(&account_state_rlp)
                .map_err(|e| {
                    ExecutionWitnessError::Custom(format!(
                        "Failed to decode account state RLP for address {address:#x}: {e}"
                    ))
                })?;
            dbg!(storage_root);
            dbg!("3");

            // let storage_trie = Trie::open(&state_trie.db.clone, storage_root);
            let states_nodes_clone: HashMap<NodeHash, Vec<u8>> = state_nodes
                .clone()
                .into_iter()
                .map(|(k, v)| (NodeHash::Hashed(k), v))
                .collect();

            let Ok(storage_trie) =
                Trie::from_nodes(NodeHash::Hashed(storage_root), states_nodes_clone.clone())
                    .map_err(|e| {
                        ExecutionWitnessError::RebuildTrie(format!(
                            "Storage trie for address {address:#x}: {e}"
                        ))
                    })
            else {
                continue;
            };

            let mut found_hashed_keys = Vec::new();
            for key in keys.iter() {
                let hash = hash_key(key);
                if let Ok(Some(abcd)) = storage_trie.get(&hash) {
                    found_hashed_keys.push(hash);
                    dbg!(abcd);
                    dbg!(states_nodes_clone.get(&NodeHash::Hashed(*key)));
                    dbg!("found key");
                }
            }

            // dbg!("4");
            // dbg!(&state_nodes.len());
            dbg!(storage_root);

            let (storage_trie_witness, storage_trie_wrapped) =
                TrieLogger::open_trie(storage_trie, storage_root);

            dbg!(&storage_trie_wrapped.hash_no_commit());
            dbg!(&storage_trie_wrapped.root);

            for a in found_hashed_keys {
                if storage_trie_wrapped.get(&a).is_err() {
                    dbg!("INCONSISTENT");
                    panic!();
                }
            }

            // are the storage slots correct?
            for slot in keys.iter() {
                // dbg!("4.1");
                // dbg!(slot);
                // println!("{:?}", hex::encode(hash_key(slot)));
                if let Ok(storage_slot) = storage_trie_wrapped.get(&hash_key(slot)).map_err(|e| {
                    ExecutionWitnessError::Custom(format!(
                        "Failed to get storage slot {slot:#x} for address {address:#x}: {e}"
                    ))
                }) {
                    touched_account_storage_slots
                        .entry(address)
                        .or_insert_with(Vec::new)
                        .push(*slot);
                    // dbg!("4.2");
                    if storage_slot.is_some() {
                        dbg!("found slot");
                    }
                    // dbg!(storage_slot);
                }
            }
            dbg!("5");

            let witness = {
                let mut w = storage_trie_witness.lock().map_err(|_| {
                    ExecutionWitnessError::Custom("Failed to lock storage trie witness".to_string())
                })?;
                let w = std::mem::take(&mut *w);
                w.into_iter().collect::<Vec<_>>()
            };
            dbg!("6");

            storage_trie_nodes_by_address.insert(address, witness);
        }
    }
    dbg!("e");

    let mut witness = ExecutionWitnessResult {
        codes,
        state_trie: None, // `None` because we'll rebuild the tries afterwards
        storage_tries: HashMap::new(), // empty map because we'll rebuild the tries afterwards
        block_headers,
        chain_config,
        parent_block_header: parent_header,
        state_nodes,
        storage_trie_nodes: storage_trie_nodes_by_address,
        touched_account_storage_slots,
    };

    dbg!("enter here");
    witness.rebuild_state_trie()?;
    // for (account, keys) in touched_account_storage_slots {
    //     if let Some(storage_trie) = witness.rebuild_storage_trie(&account) {
    //         for key in keys {
    //             let _ = storage_trie.get(&hash_key(&key)).inspect_err(|err| {
    //                 dbg!(err);
    //             });
    //         }
    //     }
    // }

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
