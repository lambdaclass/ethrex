use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use ethrex_blockchain::vm::StoreVmDatabase;
use ethrex_common::types::{AccountUpdate, Block};
use ethrex_common::{Address, H256};
use ethrex_storage::{hash_address, hash_key, Store};
use ethrex_trie::{Node, PathRLP};
use ethrex_trie::{NodeRLP, Trie, TrieError};
use ethrex_vm::backends::levm::db::DatabaseLogger;
use ethrex_vm::{DynVmDatabase, Evm, ProverDB, ProverDBError};

pub async fn to_prover_db(store: &Store, blocks: &[Block]) -> Result<ProverDB, ProverDBError> {
    let chain_config = store
        .get_chain_config()
        .map_err(|e| ProverDBError::Store(e.to_string()))?;
    let Some(first_block_parent_hash) = blocks.first().map(|e| e.header.parent_hash) else {
        return Err(ProverDBError::Custom("Unable to get first block".into()));
    };
    let Some(last_block) = blocks.last() else {
        return Err(ProverDBError::Custom("Unable to get last block".into()));
    };

    let vm_db: DynVmDatabase =
        Box::new(StoreVmDatabase::new(store.clone(), first_block_parent_hash));

    let logger = Arc::new(DatabaseLogger::new(Arc::new(Mutex::new(Box::new(vm_db)))));

    let mut execution_updates: HashMap<Address, AccountUpdate> = HashMap::new();
    for block in blocks {
        let mut vm = Evm::new_from_db(logger.clone());
        // pre-execute and get all state changes
        let _ = vm.execute_block(block);
        let account_updates = vm.get_state_transitions().map_err(Box::new)?;
        for update in account_updates {
            execution_updates
                .entry(update.address)
                .and_modify(|existing| existing.merge(update.clone()))
                .or_insert(update);
        }

        // Update de block_hash for the next execution.
        let new_store: DynVmDatabase = Box::new(StoreVmDatabase::new(store.clone(), block.hash()));

        // Replace the store
        *logger.store.lock().map_err(|err| {
            ProverDBError::Database(format!("Failed to lock 'store' with error: {err}"))
        })? = Box::new(new_store);
    }

    // index accessed account addresses and storage keys
    let state_accessed = logger
        .state_accessed
        .lock()
        .map_err(|_| ProverDBError::Store("Could not lock mutex".to_string()))?
        .clone();

    // fetch all read/written accounts from store
    let accounts = state_accessed
        .keys()
        .chain(execution_updates.keys())
        .filter_map(|address| {
            store
                .get_account_info_by_hash(first_block_parent_hash, *address)
                .transpose()
                .map(|account| {
                    account
                        .map(|a| (*address, a))
                        .map_err(|e| ProverDBError::Store(e.to_string()))
                })
        })
        .collect::<Result<HashMap<_, _>, ProverDBError>>()?;

    // fetch all read/written code from store
    let code_accessed = logger
        .code_accessed
        .lock()
        .map_err(|_| ProverDBError::Store("Could not lock mutex".to_string()))?
        .clone();
    let code = accounts
        .values()
        .map(|account| account.code_hash)
        .chain(code_accessed.into_iter())
        .filter_map(|hash| {
            store.get_account_code(hash).transpose().map(|account| {
                account
                    .map(|a| (hash, a))
                    .map_err(|e| ProverDBError::Store(e.to_string()))
            })
        })
        .collect::<Result<HashMap<_, _>, ProverDBError>>()?;

    // fetch all read/written storage from store
    let added_storage = execution_updates.iter().filter_map(|(address, update)| {
        if !update.added_storage.is_empty() {
            let keys = update.added_storage.keys().cloned().collect::<Vec<_>>();
            Some((*address, keys))
        } else {
            None
        }
    });
    let storage = state_accessed
        .clone()
        .into_iter()
        .chain(added_storage)
        .map(|(address, keys)| {
            let keys: Result<HashMap<_, _>, ProverDBError> = keys
                .iter()
                .filter_map(|key| {
                    store
                        .get_storage_at_hash(first_block_parent_hash, address, *key)
                        .transpose()
                        .map(|value| {
                            value
                                .map(|v| (*key, v))
                                .map_err(|e| ProverDBError::Store(e.to_string()))
                        })
                })
                .collect();
            Ok((address, keys?))
        })
        .collect::<Result<HashMap<_, _>, ProverDBError>>()?;

    let block_hashes = logger
        .block_hashes_accessed
        .lock()
        .map_err(|_| ProverDBError::Store("Could not lock mutex".to_string()))?
        .clone()
        .into_iter()
        .map(|(num, hash)| (num, H256::from(hash.0)))
        .collect();

    // get account proofs
    let state_trie = store
        .state_trie(last_block.hash())
        .map_err(|e| ProverDBError::Store(e.to_string()))?
        .ok_or(ProverDBError::NewMissingStateTrie(last_block.hash()))?;
    let parent_state_trie = store
        .state_trie(first_block_parent_hash)
        .map_err(|e| ProverDBError::Store(e.to_string()))?
        .ok_or(ProverDBError::NewMissingStateTrie(first_block_parent_hash))?;
    let hashed_addresses: Vec<_> = state_accessed.keys().map(hash_address).collect();
    let initial_state_proofs = parent_state_trie.get_proofs(&hashed_addresses)?;
    let final_state_proofs: Vec<_> = hashed_addresses
        .iter()
        .map(|hashed_address| Ok((hashed_address, state_trie.get_proof(hashed_address)?)))
        .collect::<Result<_, TrieError>>()?;
    let potential_account_child_nodes = final_state_proofs
        .iter()
        .filter_map(|(hashed_address, proof)| get_potential_child_nodes(proof, hashed_address))
        .flat_map(|nodes| nodes.into_iter().map(|node| node.encode_raw()))
        .collect();
    let state_proofs = (
        initial_state_proofs.0,
        [initial_state_proofs.1, potential_account_child_nodes].concat(),
    );

    // get storage proofs
    let mut storage_proofs = HashMap::new();
    let mut final_storage_proofs = HashMap::new();
    for (address, storage_keys) in state_accessed {
        let Some(parent_storage_trie) = store
            .storage_trie(first_block_parent_hash, address)
            .map_err(|e| ProverDBError::Store(e.to_string()))?
        else {
            // the storage of this account was empty or the account is newly created, either
            // way the storage trie was initially empty so there aren't any proofs to add.
            continue;
        };
        let storage_trie = store
            .storage_trie(last_block.hash(), address)
            .map_err(|e| ProverDBError::Store(e.to_string()))?
            .ok_or(ProverDBError::NewMissingStorageTrie(
                last_block.hash(),
                address,
            ))?;
        let paths = storage_keys.iter().map(hash_key).collect::<Vec<_>>();

        let initial_proofs = parent_storage_trie.get_proofs(&paths)?;
        let final_proofs: Vec<(_, Vec<_>)> = storage_keys
            .iter()
            .map(|key| {
                let hashed_key = hash_key(key);
                let proof = storage_trie.get_proof(&hashed_key)?;
                Ok((hashed_key, proof))
            })
            .collect::<Result<_, TrieError>>()?;

        let potential_child_nodes: Vec<NodeRLP> = final_proofs
            .iter()
            .filter_map(|(hashed_key, proof)| get_potential_child_nodes(proof, hashed_key))
            .flat_map(|nodes| nodes.into_iter().map(|node| node.encode_raw()))
            .collect();
        let proofs = (
            initial_proofs.0,
            [initial_proofs.1, potential_child_nodes].concat(),
        );

        storage_proofs.insert(address, proofs);
        final_storage_proofs.insert(address, final_proofs);
    }

    Ok(ProverDB {
        accounts,
        code,
        storage,
        block_hashes,
        chain_config,
        state_proofs,
        storage_proofs,
    })
}

/// Get all potential child nodes of a node whose value was deleted.
///
/// After deleting a value from a (partial) trie it's possible that the node containing the value gets
/// replaced by its child, whose prefix is possibly modified by appending some nibbles to it.
/// If we don't have this child node (because we're modifying a partial trie), then we can't
/// perform the deletion. If we have the final proof of exclusion of the deleted value, we can
/// calculate all posible child nodes.
pub fn get_potential_child_nodes(proof: &[NodeRLP], key: &PathRLP) -> Option<Vec<Node>> {
    // TODO: Perhaps it's possible to calculate the child nodes instead of storing all possible ones?.
    // TODO: https://github.com/lambdaclass/ethrex/issues/2938
    let trie = Trie::from_nodes(
        proof.first(),
        &proof.iter().skip(1).cloned().collect::<Vec<_>>(),
    )
    .ok()?;

    // return some only if this is a proof of exclusion
    if trie.get(key).ok()?.is_none() {
        let final_node = Node::decode_raw(proof.last()?).ok()?;
        match final_node {
            Node::Extension(mut node) => {
                let mut variants = Vec::with_capacity(node.prefix.len());
                while {
                    variants.push(Node::from(node.clone()));
                    node.prefix.next().is_some()
                } {}
                Some(variants)
            }
            Node::Leaf(mut node) => {
                let mut variants = Vec::with_capacity(node.partial.len());
                while {
                    variants.push(Node::from(node.clone()));
                    node.partial.next();
                    !node.partial.is_empty() // skip the last nibble, which is the leaf flag.
                                             // if we encode a leaf with its flag missing, it’s going to be encoded as an
                                             // extension.
                } {}
                Some(variants)
            }
            _ => None,
        }
    } else {
        None
    }
}
