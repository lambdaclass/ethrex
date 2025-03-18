use std::collections::HashMap;

use ethrex_common::{
    types::{Block, BlockHash},
    Address as CoreAddress, H256 as CoreH256,
};
use ethrex_storage::{error::StoreError, hash_address, hash_key, Store};
use ethrex_trie::{Node, NodeRLP, PathRLP, Trie, TrieError};
use revm::primitives::{
    AccountInfo as RevmAccountInfo, Address as RevmAddress, Bytecode as RevmBytecode,
    Bytes as RevmBytes, B256 as RevmB256, U256 as RevmU256,
};

use crate::{
    db::{ExecutionDB, StoreWrapper},
    errors::ExecutionDBError,
    spec_id, ChainConfig, EvmError,
};

/// State used when running the EVM. The state can be represented with a [StoreWrapper] database, or
/// with a [ExecutionDB] in case we only want to store the necessary data for some particular
/// execution, for example when proving in L2 mode.
///
/// Encapsulates state behaviour to be agnostic to the evm implementation for crate users.
pub enum EvmState {
    Store(revm::db::State<StoreWrapper>),
    Execution(Box<revm::db::CacheDB<ExecutionDB>>),
}

impl EvmState {
    /// Get a reference to inner `Store` database
    pub fn database(&self) -> Option<&Store> {
        if let EvmState::Store(db) = self {
            Some(&db.database.store)
        } else {
            None
        }
    }

    /// Gets the stored chain config
    pub fn chain_config(&self) -> Result<ChainConfig, EvmError> {
        match self {
            EvmState::Store(db) => db.database.store.get_chain_config().map_err(EvmError::from),
            EvmState::Execution(db) => Ok(db.db.get_chain_config()),
        }
    }
}

/// Builds EvmState from a Store
pub fn evm_state(store: Store, block_hash: BlockHash) -> EvmState {
    EvmState::Store(
        revm::db::State::builder()
            .with_database(StoreWrapper { store, block_hash })
            .with_bundle_update()
            .without_state_clear()
            .build(),
    )
}

impl From<ExecutionDB> for EvmState {
    fn from(value: ExecutionDB) -> Self {
        EvmState::Execution(Box::new(revm::db::CacheDB::new(value)))
    }
}

use super::execution_db::ToExecDB;

impl revm::Database for StoreWrapper {
    type Error = StoreError;

    fn basic(&mut self, address: RevmAddress) -> Result<Option<RevmAccountInfo>, Self::Error> {
        let acc_info = match self
            .store
            .get_account_info_by_hash(self.block_hash, CoreAddress::from(address.0.as_ref()))?
        {
            None => return Ok(None),
            Some(acc_info) => acc_info,
        };
        let code = self
            .store
            .get_account_code(acc_info.code_hash)?
            .map(|b| RevmBytecode::new_raw(RevmBytes(b)));

        Ok(Some(RevmAccountInfo {
            balance: RevmU256::from_limbs(acc_info.balance.0),
            nonce: acc_info.nonce,
            code_hash: RevmB256::from(acc_info.code_hash.0),
            code,
        }))
    }

    fn code_by_hash(&mut self, code_hash: RevmB256) -> Result<RevmBytecode, Self::Error> {
        self.store
            .get_account_code(CoreH256::from(code_hash.as_ref()))?
            .map(|b| RevmBytecode::new_raw(RevmBytes(b)))
            .ok_or_else(|| StoreError::Custom(format!("No code for hash {code_hash}")))
    }

    fn storage(&mut self, address: RevmAddress, index: RevmU256) -> Result<RevmU256, Self::Error> {
        Ok(self
            .store
            .get_storage_at_hash(
                self.block_hash,
                CoreAddress::from(address.0.as_ref()),
                CoreH256::from(index.to_be_bytes()),
            )?
            .map(|value| RevmU256::from_limbs(value.0))
            .unwrap_or_else(|| RevmU256::ZERO))
    }

    fn block_hash(&mut self, number: u64) -> Result<RevmB256, Self::Error> {
        self.store
            .get_block_header(number)?
            .map(|header| RevmB256::from_slice(&header.compute_block_hash().0))
            .ok_or_else(|| StoreError::Custom(format!("Block {number} not found")))
    }
}

impl revm::DatabaseRef for StoreWrapper {
    type Error = StoreError;

    fn basic_ref(&self, address: RevmAddress) -> Result<Option<RevmAccountInfo>, Self::Error> {
        let acc_info = match self
            .store
            .get_account_info_by_hash(self.block_hash, CoreAddress::from(address.0.as_ref()))?
        {
            None => return Ok(None),
            Some(acc_info) => acc_info,
        };
        let code = self
            .store
            .get_account_code(acc_info.code_hash)?
            .map(|b| RevmBytecode::new_raw(RevmBytes(b)));

        Ok(Some(RevmAccountInfo {
            balance: RevmU256::from_limbs(acc_info.balance.0),
            nonce: acc_info.nonce,
            code_hash: RevmB256::from(acc_info.code_hash.0),
            code,
        }))
    }

    fn code_by_hash_ref(&self, code_hash: RevmB256) -> Result<RevmBytecode, Self::Error> {
        self.store
            .get_account_code(CoreH256::from(code_hash.as_ref()))?
            .map(|b| RevmBytecode::new_raw(RevmBytes(b)))
            .ok_or_else(|| StoreError::Custom(format!("No code for hash {code_hash}")))
    }

    fn storage_ref(&self, address: RevmAddress, index: RevmU256) -> Result<RevmU256, Self::Error> {
        Ok(self
            .store
            .get_storage_at_hash(
                self.block_hash,
                CoreAddress::from(address.0.as_ref()),
                CoreH256::from(index.to_be_bytes()),
            )?
            .map(|value| RevmU256::from_limbs(value.0))
            .unwrap_or_else(|| RevmU256::ZERO))
    }

    fn block_hash_ref(&self, number: u64) -> Result<RevmB256, Self::Error> {
        self.store
            .get_block_header(number)?
            .map(|header| RevmB256::from_slice(&header.compute_block_hash().0))
            .ok_or_else(|| StoreError::Custom(format!("Block {number} not found")))
    }
}

impl ToExecDB for StoreWrapper {
    fn to_exec_db(&self, block: &Block) -> Result<ExecutionDB, ExecutionDBError> {
        // TODO: Simplify this function and potentially merge with the implementation for
        // RpcDB.

        let parent_hash = block.header.parent_hash;
        let chain_config = self.store.get_chain_config()?;

        // pre-execute and get all state changes
        let cache = ExecutionDB::pre_execute(
            block,
            chain_config.chain_id,
            spec_id(&chain_config, block.header.timestamp),
            self,
        )
        .map_err(|err| Box::new(EvmError::from(err)))?; // TODO: ugly error handling
        let store_wrapper = cache.db;

        // index read and touched account addresses and storage keys
        let index = cache.accounts.iter().map(|(address, account)| {
            let address = CoreAddress::from(address.0.as_ref());
            let storage_keys: Vec<_> = account
                .storage
                .keys()
                .map(|key| CoreH256::from_slice(&key.to_be_bytes_vec()))
                .collect();
            (address, storage_keys)
        });

        // fetch all read/written values from store
        let cache_accounts = cache.accounts.iter().filter_map(|(address, account)| {
            let address = CoreAddress::from(address.0.as_ref());
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
                let hash = CoreH256::from(account.info.code_hash.0);
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
                            let key = CoreH256::from(key.to_be_bytes());
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
            .map(|(num, hash)| (num.try_into().unwrap(), CoreH256::from(hash.0)))
            .collect();
        // WARN: unwrapping because revm wraps a u64 as a U256

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
