use std::{collections::HashMap, sync::Arc};

use ethrex_common::{Address as CoreAddress, H256 as CoreH256};

use crate::{backends::revm::execution_db::ToExecDB, errors::ExecutionDBError, EvmError};
use bytes::Bytes;
use ethrex_common::{
    types::{AccountInfo, Block, BlockHash, ChainConfig, Fork},
    Address, H160, H256, U256,
};
use ethrex_storage::{hash_address, hash_key, AccountUpdate, Store};
use ethrex_trie::{Node, NodeRLP, PathRLP, Trie, TrieError};
use serde::{Deserialize, Serialize};

use crate::backends::BlockExecutionResult;

#[cfg(not(feature = "levm-l2"))]
use crate::backends::revm::db::evm_state;
#[cfg(feature = "levm-l2")]
use std::sync::Arc;

#[derive(Clone)]
pub struct StoreWrapper {
    pub store: Store,
    pub block_hash: BlockHash,
}

/// In-memory EVM database for single execution data.
///
/// This is mainly used to store the relevant state data for executing a single block and then
/// feeding the DB into a zkVM program to prove the execution.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExecutionDB {
    /// indexed by account address
    pub accounts: HashMap<Address, AccountInfo>,
    /// indexed by code hash
    pub code: HashMap<H256, Bytes>,
    /// indexed by account address and storage key
    pub storage: HashMap<Address, HashMap<H256, U256>>,
    /// indexed by block number
    pub block_hashes: HashMap<u64, H256>,
    /// stored chain config
    pub chain_config: ChainConfig,
    /// Encoded nodes to reconstruct a state trie, but only including relevant data ("pruned trie").
    ///
    /// Root node is stored separately from the rest as the first tuple member.
    pub state_proofs: (Option<NodeRLP>, Vec<NodeRLP>),
    /// Encoded nodes to reconstruct every storage trie, but only including relevant data ("pruned
    /// trie").
    ///
    /// Root node is stored separately from the rest as the first tuple member.
    pub storage_proofs: HashMap<Address, (Option<NodeRLP>, Vec<NodeRLP>)>,
}

impl ExecutionDB {
    /// Gets the Vec<[AccountUpdate]>/StateTransitions obtained after executing a block.
    pub fn get_account_updates(
        block: &Block,
        store: &Store,
    ) -> Result<Vec<AccountUpdate>, ExecutionDBError> {
        // TODO: perform validation to exit early

        #[cfg(feature = "levm-l2")]
        {
            let store_wrapper = StoreWrapper {
                store: store.clone(),
                block_hash: block.header.parent_hash,
            };
            let result = crate::backends::levm::LEVM::execute_block(block, Arc::new(store_wrapper))
                .map_err(Box::new)?;
            Ok(result.account_updates)
        }
        #[cfg(not(feature = "levm-l2"))]
        {
            let mut state = evm_state(store.clone(), block.header.parent_hash);

            let result =
                crate::backends::revm::REVM::execute_block(block, &mut state).map_err(Box::new)?;
            Ok(result.account_updates)
        }
    }

    pub fn get_chain_config(&self) -> ChainConfig {
        self.chain_config
    }

    /// Recreates the state trie and storage tries from the encoded nodes.
    pub fn get_tries(&self) -> Result<(Trie, HashMap<H160, Trie>), ExecutionDBError> {
        let (state_trie_root, state_trie_nodes) = &self.state_proofs;
        let state_trie = Trie::from_nodes(state_trie_root.as_ref(), state_trie_nodes)?;

        let storage_trie = self
            .storage_proofs
            .iter()
            .map(|(address, nodes)| {
                let (storage_trie_root, storage_trie_nodes) = nodes;
                let trie = Trie::from_nodes(storage_trie_root.as_ref(), storage_trie_nodes)?;
                Ok((*address, trie))
            })
            .collect::<Result<_, TrieError>>()?;

        Ok((state_trie, storage_trie))
    }

    /// Execute a block and cache all state changes, returns the cache
    pub fn pre_execute_new(
        block: &Block,
        store_wrapper: &StoreWrapper,
    ) -> Result<(BlockExecutionResult, StoreWrapper), ExecutionDBError> {
        // this code was copied from the L1
        // TODO: if we change EvmState so that it accepts a CacheDB<RpcDB> then we can
        // simply call execute_block().

        let db = store_wrapper.clone();
        let mut result = BlockExecutionResult {
            account_updates: vec![],
            receipts: vec![],
            requests: vec![],
        };
        // beacon root call
        // #[cfg(not(feature = "l2"))]
        {
            let mut cache = HashMap::new();
            crate::backends::levm::LEVM::beacon_root_contract_call(
                &block.header,
                Arc::new(db.clone()),
                &mut cache,
            )
            .map_err(|e| ExecutionDBError::Evm(Box::new(e)))?;
            let account_updates = crate::backends::levm::LEVM::get_state_transitions(
                None,
                Arc::new(db.clone()),
                &block.header,
                &cache,
            )
            .map_err(|e| ExecutionDBError::Evm(Box::new(e)))?;

            db.store
                .apply_account_updates(block.hash(), &account_updates)
                .map_err(|e| ExecutionDBError::Store(e))?;

            result.account_updates = account_updates;
        }

        // execute block
        let report = crate::backends::levm::LEVM::execute_block(block, Arc::new(db.clone()))
            .map_err(Box::new)?;
        result.receipts = report.receipts;
        result.requests = report.requests;
        result.account_updates.extend(report.account_updates); // check if this is correct

        Ok((result, db))
    }
}

impl ToExecDB for StoreWrapper {
    fn to_exec_db(&self, block: &Block) -> Result<ExecutionDB, ExecutionDBError> {
        // TODO: Simplify this function and potentially merge with the implementation for
        // RpcDB.

        let parent_hash = block.header.parent_hash;
        let chain_config = self.store.get_chain_config()?;

        // pre-execute and get all state changes
        let (execution_result, store_wrapper) = ExecutionDB::pre_execute_new(block, self)
            .map_err(|err| Box::new(EvmError::from(err)))?; // TODO: ugly error handling

        // index read and touched account addresses and storage keys
        let index = execution_result.account_updates.iter().map(|update| {
            // CHECK if we only need the touched storage keys
            let address = update.address;
            let storage_keys: Vec<_> = update
                .added_storage
                .keys()
                .map(|key| CoreH256::from_slice(&key.to_fixed_bytes()))
                .collect();
            (address, storage_keys)
        });

        // fetch all read/written values from store
        let cache_accounts = cache.accounts.iter().filter_map(|(address, account)| {
            let address = CoreAddress::from_slice(address.0.as_ref());
            // filter new accounts (accounts that didn't exist before) assuming our store is
            // correct (based on the success of the pre-execution).
            if store_wrapper
                .store
                .get_account_info_by_hash(parent_hash, address)
                .is_ok_and(|account| account.is_some())
            {
                Some((address, account))
            } else {
                None
            }
        });
        let accounts = cache_accounts
            .clone()
            .map(|(address, _)| {
                // return error if account is missing
                let account = match store_wrapper
                    .store
                    .get_account_info_by_hash(parent_hash, address)
                {
                    Ok(Some(some)) => Ok(some),
                    Err(err) => Err(ExecutionDBError::Store(err)),
                    Ok(None) => unreachable!(), // we are filtering out accounts that are not present
                                                // in the store
                };
                Ok((address, account?))
            })
            .collect::<Result<HashMap<_, _>, ExecutionDBError>>()?;
        let code = cache_accounts
            .clone()
            .map(|(_, account)| {
                // return error if code is missing
                let hash = account.info.bytecode_hash();
                Ok((
                    hash,
                    store_wrapper
                        .store
                        .get_account_code(hash)?
                        .ok_or(ExecutionDBError::NewMissingCode(hash))?,
                ))
            })
            .collect::<Result<_, ExecutionDBError>>()?;
        let storage = cache_accounts
            .map(|(address, account)| {
                // return error if storage is missing
                Ok((
                    address,
                    account
                        .storage
                        .keys()
                        .map(|key| {
                            let key = CoreH256::from(key.to_fixed_bytes());
                            let value = store_wrapper
                                .store
                                .get_storage_at_hash(parent_hash, address, key)
                                .map_err(ExecutionDBError::Store)?
                                .ok_or(ExecutionDBError::NewMissingStorage(address, key))?;
                            Ok((key, value))
                        })
                        .collect::<Result<HashMap<_, _>, ExecutionDBError>>()?,
                ))
            })
            .collect::<Result<HashMap<_, _>, ExecutionDBError>>()?;
        let block_hashes = cache
            .block_hashes
            .into_iter()
            .map(|(num, hash)| (num, CoreH256::from(hash.0)))
            .collect();

        // get account proofs
        let state_trie = self
            .store
            .state_trie(block.hash())?
            .ok_or(ExecutionDBError::NewMissingStateTrie(parent_hash))?;
        let parent_state_trie = self
            .store
            .state_trie(parent_hash)?
            .ok_or(ExecutionDBError::NewMissingStateTrie(parent_hash))?;
        let hashed_addresses: Vec<_> = index
            .clone()
            .map(|(address, _)| hash_address(&address))
            .collect();
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
        for (address, storage_keys) in index {
            let Some(parent_storage_trie) = self.store.storage_trie(parent_hash, address)? else {
                // the storage of this account was empty or the account is newly created, either
                // way the storage trie was initially empty so there aren't any proofs to add.
                continue;
            };
            let storage_trie = self.store.storage_trie(block.hash(), address)?.ok_or(
                ExecutionDBError::NewMissingStorageTrie(block.hash(), address),
            )?;
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

        Ok(ExecutionDB {
            accounts,
            code,
            storage,
            block_hashes,
            chain_config,
            state_proofs,
            storage_proofs,
        })
    }
}

/// Get all potential child nodes of a node whose value was deleted.
///
/// After deleting a value from a (partial) trie it's possible that the node containing the value gets
/// replaced by its child, whose prefix is possibly modified by appending some nibbles to it.
/// If we don't have this child node (because we're modifying a partial trie), then we can't
/// perform the deletion. If we have the final proof of exclusion of the deleted value, we can
/// calculate all posible child nodes.
fn get_potential_child_nodes(proof: &[NodeRLP], key: &PathRLP) -> Option<Vec<Node>> {
    // TODO: Perhaps it's possible to calculate the child nodes instead of storing all possible ones.
    let trie = Trie::from_nodes(
        proof.first(),
        &proof.iter().skip(1).cloned().collect::<Vec<_>>(),
    )
    .unwrap();

    // return some only if this is a proof of exclusion
    if trie.get(key).unwrap().is_none() {
        let final_node = Node::decode_raw(proof.last().unwrap()).unwrap();
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
                    node.partial.next().is_some()
                } {}
                Some(variants)
            }
            _ => None,
        }
    } else {
        None
    }
}
