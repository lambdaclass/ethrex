use std::collections::{BTreeMap, HashMap, HashSet};

use bytes::Bytes;
use ethrex_common::{
    Address, H160, H256, serde_utils,
    types::{
        AccountState, AccountUpdate, Block, BlockHeader, ChainConfig,
        block_execution_witness::{ExecutionWitnessError, ExecutionWitnessResult},
    },
};
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};
use ethrex_storage::{hash_address, hash_key};
use ethrex_trie::{NodeHash, NodeRLP, Trie, TrieLogger, TrieWitness};
use ethrex_vm::{Evm, EvmEngine, ExecutionWitnessWrapper};
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
    blocks: &[&Block],
) -> Result<ExecutionWitnessResult, ExecutionWitnessError> {
    let codes = rpc_witness
        .codes
        .iter()
        .map(|code| (keccak_hash::keccak(code), code.clone()))
        .collect::<BTreeMap<_, _>>();

    let block_headers = rpc_witness
        .headers
        .iter()
        .map(Bytes::as_ref)
        .map(BlockHeader::decode)
        .collect::<Result<Vec<_>, _>>()
        .expect("Failed to decode block headers from RpcExecutionWitness")
        .iter()
        .map(|header| (header.number, header.clone()))
        .collect::<BTreeMap<_, _>>();

    let parent_number = first_block_number
        .checked_sub(1)
        .ok_or(ExecutionWitnessError::Custom(
            "First block number cannot be zero".to_string(),
        ))?;

    let parent_header = block_headers.get(&parent_number).cloned().ok_or(
        ExecutionWitnessError::MissingParentHeaderOf(first_block_number),
    )?;

    let mut state_nodes = BTreeMap::new();
    for node in rpc_witness.state.iter() {
        state_nodes.insert(keccak(node), node.to_vec());
    }

    let mut state_trie = Trie::from_nodes(NodeHash::Hashed(parent_header.state_root), &state_nodes)
        .map_err(|e| ExecutionWitnessError::RebuildTrie(format!("State trie: {e}")))?;

    let mut touched_account_storage_slots = BTreeMap::new();
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

    let mut storage_trie_nodes_by_address: HashMap<H160, HashSet<Vec<u8>>> = HashMap::new();
    let mut used_storage_tries = HashMap::new();

    for (address, slots) in &touched_account_storage_slots {
        let Some(account_rlp) = state_trie
            .get(&hash_address(address))
            .map_err(|e| ExecutionWitnessError::Custom(e.to_string()))?
        else {
            continue;
        };

        let AccountState { storage_root, .. } =
            AccountState::decode(&account_rlp).map_err(|e| {
                ExecutionWitnessError::Custom(format!(
                    "Failed to decode account state RLP for address {address:#x}: {e}"
                ))
            })?;

        let Ok(mut storage_trie) = Trie::from_nodes(NodeHash::Hashed(storage_root), &state_nodes)
            .map_err(|e| {
                ExecutionWitnessError::RebuildTrie(format!(
                    "Storage trie for address {address:#x}: {e}"
                ))
            })
        else {
            continue;
        };
        let hash = storage_trie.hash().map_err(|e| {
            ExecutionWitnessError::RebuildTrie(format!(
                "Storage trie for address {address:#x}: {e}"
            ))
        })?;

        let (storage_trie_witness, storage_trie_wrapped) =
            TrieLogger::open_trie(storage_trie, NodeHash::from(hash).into());
        for key in slots {
            storage_trie_wrapped.get(&hash_key(key)).map_err(|e| {
                ExecutionWitnessError::Custom(format!("Failed to get storage trie node: {e}"))
            })?;
        }

        used_storage_tries.insert(
            *address,
            (storage_trie_witness.clone(), storage_trie_wrapped),
        );

        let witness_nodes = {
            let mut w = storage_trie_witness.lock().map_err(|_| {
                ExecutionWitnessError::Custom("Failed to lock storage trie witness".to_string())
            })?;
            std::mem::take(&mut *w)
        };
        storage_trie_nodes_by_address
            .entry(*address)
            .or_default()
            .extend(witness_nodes);
    }
    let storage_trie_nodes: BTreeMap<H160, Vec<H256>> = storage_trie_nodes_by_address
        .clone()
        .into_iter()
        .map(|(addr, nodes_set)| (addr, nodes_set.into_iter().map(keccak).collect()))
        .collect();

    let mut witness = ExecutionWitnessResult {
        codes,
        state_trie: None, // `None` because we'll rebuild the tries afterwards
        storage_tries: BTreeMap::new(), // empty map because we'll rebuild the tries afterwards
        block_headers,
        chain_config,
        parent_block_header: parent_header,
        state_nodes: state_nodes.clone(),
        storage_trie_nodes,
        touched_account_storage_slots,
        account_hashes_by_address: BTreeMap::new(), // This must be filled during stateless execution
    };

    // block execution - this is for getting the account updates

    for block in blocks {
        let mut witness_clone = ExecutionWitnessResult {
            codes: witness.codes.clone(),
            state_trie: None,
            storage_tries: BTreeMap::new(),
            block_headers: witness.block_headers.clone(),
            parent_block_header: witness.parent_block_header.clone(),
            chain_config: witness.chain_config,
            state_nodes: witness.state_nodes.clone(),
            storage_trie_nodes: witness.storage_trie_nodes.clone(),
            touched_account_storage_slots: witness.touched_account_storage_slots.clone(),
            account_hashes_by_address: witness.account_hashes_by_address.clone(),
        };
        witness_clone.rebuild_state_trie()?;
        let wrapped_db = ExecutionWitnessWrapper::new(witness_clone);
        let mut vm = Evm::new_for_l1(EvmEngine::LEVM, wrapped_db.clone());
        let _ = vm
            .execute_block(block)
            .map_err(|e| ExecutionWitnessError::Custom(format!("Failed to execute block: {e}")))?;
        let account_updates: Vec<AccountUpdate> = vm.get_state_transitions().map_err(|e| {
            ExecutionWitnessError::Custom(format!("Failed to get state transitions: {e}"))
        })?;
        let (_, trie_loggers) = apply_account_updates_from_trie_with_witness(
            &mut state_trie,
            &account_updates,
            &mut used_storage_tries,
            &state_nodes,
        )?;
        for (address, (witness_ref, _)) in trie_loggers {
            let mut witness_lock = witness_ref.lock().map_err(|_| {
                ExecutionWitnessError::Custom("Failed to lock storage trie witness".to_string())
            })?;
            let nodes_set = storage_trie_nodes_by_address.entry(*address).or_default();
            nodes_set.extend(std::mem::take(&mut *witness_lock));
        }
    }

    witness.storage_trie_nodes = storage_trie_nodes_by_address
        .into_iter()
        .map(|(addr, nodes_set)| (addr, nodes_set.into_iter().map(keccak).collect()))
        .collect();
    Ok(witness)
}

/// Performs the same actions as apply_account_updates_from_trie
///  but also returns the used storage tries with witness recorded
#[allow(clippy::type_complexity)]
fn apply_account_updates_from_trie_with_witness<'a>(
    state_trie: &'a mut Trie,
    account_updates: &'a [AccountUpdate],
    storage_tries: &'a mut HashMap<Address, (TrieWitness, Trie)>,
    state_nodes: &'a BTreeMap<H256, NodeRLP>,
) -> Result<(&'a mut Trie, &'a mut HashMap<Address, (TrieWitness, Trie)>), ExecutionWitnessError> {
    for update in account_updates.iter() {
        let hashed_address = hash_address(&update.address);
        if update.removed {
            continue;
        } else {
            // Add or update AccountState in the trie
            // Fetch current state or create a new state to be inserted
            let mut account_state = match state_trie.get(&hashed_address).unwrap_or_default() {
                Some(encoded_state) => AccountState::decode(&encoded_state).map_err(|e| {
                    ExecutionWitnessError::Custom(format!(
                        "Failed to decode account state RLP for address {}: {e}",
                        update.address
                    ))
                })?,
                None => AccountState::default(),
            };
            if let Some(info) = &update.info {
                account_state.nonce = info.nonce;
                account_state.balance = info.balance;
                account_state.code_hash = info.code_hash;
            }
            // Store the added storage in the account's storage trie and compute its new root
            if !update.added_storage.is_empty() {
                let (_witness, storage_trie) = match storage_tries.entry(update.address) {
                    std::collections::hash_map::Entry::Occupied(value) => value.into_mut(),
                    std::collections::hash_map::Entry::Vacant(vacant) => {
                        let trie = Trie::from_nodes(account_state.storage_root.into(), state_nodes)
                            .map_err(|e| {
                                ExecutionWitnessError::Custom(format!(
                                    "Failed to build storage trie for account {}: {e}",
                                    update.address
                                ))
                            })?;
                        let root = trie.hash_no_commit();
                        vacant.insert(TrieLogger::open_trie(trie, NodeHash::from(root).into()))
                    }
                };

                for (storage_key, storage_value) in &update.added_storage {
                    let hashed_key = hash_key(storage_key);
                    if storage_value.is_zero() {
                        storage_trie.remove(&hashed_key).map_err(|e| {
                            ExecutionWitnessError::Custom(format!(
                                "Failed to remove storage key: {e}",
                            ))
                        })?;
                    } else {
                        storage_trie
                            .insert(hashed_key, storage_value.encode_to_vec())
                            .map_err(|e| {
                                ExecutionWitnessError::Custom(format!(
                                    "Failed to insert storage key: {e}",
                                ))
                            })?;
                    }
                }
                account_state.storage_root = storage_trie.hash_no_commit();
            }
            state_trie
                .insert(hashed_address, account_state.encode_to_vec())
                .map_err(|e| {
                    ExecutionWitnessError::Custom(format!(
                        "Failed to insert account state for address: {e}",
                    ))
                })?;
        }
    }

    Ok((state_trie, storage_tries))
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
