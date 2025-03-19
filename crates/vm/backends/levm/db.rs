use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use ethrex_common::types::Block;
use ethrex_storage::{hash_address, hash_key, AccountUpdate};
use ethrex_trie::{NodeRLP, TrieError};
use lazy_static::lazy_static;

use ethrex_common::U256 as CoreU256;
use ethrex_common::{Address as CoreAddress, H256 as CoreH256};
use ethrex_levm::db::Database as LevmDatabase;

use crate::db::{get_potential_child_nodes, ExecutionDB, StoreWrapper};
use crate::errors::ExecutionDBError;
use crate::EvmError;

lazy_static! {
    pub static ref BLOCKS_ACCESSED: Mutex<HashMap<u64, CoreH256>> = Mutex::new(HashMap::new());
}

impl LevmDatabase for StoreWrapper {
    fn get_account_info(&self, address: CoreAddress) -> ethrex_levm::account::AccountInfo {
        let acc_info = self
            .store
            .get_account_info_by_hash(self.block_hash, address)
            .unwrap_or(None)
            .unwrap_or_default();

        let acc_code = self
            .store
            .get_account_code(acc_info.code_hash)
            .unwrap()
            .unwrap_or_default();

        ethrex_levm::account::AccountInfo {
            balance: acc_info.balance,
            nonce: acc_info.nonce,
            bytecode: acc_code,
        }
    }

    fn account_exists(&self, address: CoreAddress) -> bool {
        let acc_info = self
            .store
            .get_account_info_by_hash(self.block_hash, address)
            .unwrap();

        acc_info.is_some()
    }

    fn get_storage_slot(&self, address: CoreAddress, key: CoreH256) -> CoreU256 {
        self.store
            .get_storage_at_hash(self.block_hash, address, key)
            .unwrap()
            .unwrap_or_default()
    }

    fn get_block_hash(&self, block_number: u64) -> Option<CoreH256> {
        let block_header = self.store.get_block_header(block_number).unwrap();

        let block_hash = block_header.map(|header| CoreH256::from(header.compute_block_hash().0));

        BLOCKS_ACCESSED
            .lock()
            .unwrap()
            .insert(block_number, block_hash.unwrap());

        block_hash
    }

    fn get_chain_config(&self) -> ethrex_common::types::ChainConfig {
        self.store.get_chain_config().unwrap()
    }
}

impl LevmDatabase for crate::db::ExecutionDB {
    fn get_account_info(&self, address: CoreAddress) -> ethrex_levm::AccountInfo {
        let Some(acc_info) = self.accounts.get(&address) else {
            return ethrex_levm::AccountInfo::default();
        };
        let acc_code = self.code.get(&acc_info.code_hash).unwrap();
        ethrex_levm::AccountInfo {
            balance: acc_info.balance,
            bytecode: acc_code.clone(),
            nonce: acc_info.nonce,
        }
    }

    fn account_exists(&self, address: CoreAddress) -> bool {
        self.accounts.contains_key(&address)
    }

    fn get_block_hash(&self, block_number: u64) -> Option<CoreH256> {
        self.block_hashes.get(&block_number).cloned()
    }

    fn get_storage_slot(&self, address: CoreAddress, key: CoreH256) -> CoreU256 {
        let Some(storage) = self.storage.get(&address) else {
            return CoreU256::default();
        };
        *storage.get(&key).unwrap_or(&CoreU256::default())
    }

    fn get_chain_config(&self) -> ethrex_common::types::ChainConfig {
        self.chain_config
    }
}

impl ExecutionDB {
    pub fn pre_execute_levm(
        block: &Block,
        store_wrapper: &StoreWrapper,
    ) -> Result<(Vec<AccountUpdate>, StoreWrapper), ExecutionDBError> {
        // this code was copied from the L1
        // TODO: if we change EvmState so that it accepts a CacheDB<RpcDB> then we can
        // simply call execute_block().

        let db = store_wrapper.clone();
        BLOCKS_ACCESSED.lock().unwrap().clear();
        let mut account_updates = vec![];
        // beacon root call
        #[cfg(not(feature = "l2"))]
        {
            let mut cache = HashMap::new();
            crate::backends::levm::LEVM::beacon_root_contract_call(
                &block.header,
                Arc::new(db.clone()),
                &mut cache,
            )
            .map_err(|e| ExecutionDBError::Evm(Box::new(e)))?;
            let account_updates_beacon = crate::backends::levm::LEVM::get_state_transitions(
                None,
                Arc::new(db.clone()),
                &block.header,
                &cache,
            )
            .map_err(|e| ExecutionDBError::Evm(Box::new(e)))?;

            db.store
                .apply_account_updates(block.hash(), &account_updates_beacon)
                .map_err(ExecutionDBError::Store)?;

            account_updates.extend(account_updates_beacon);
        }

        // execute block
        let report = crate::backends::levm::LEVM::execute_block(block, Arc::new(db.clone()))
            .map_err(Box::new)?;
        account_updates.extend(report.account_updates);

        Ok((account_updates, db))
    }
}

impl StoreWrapper {
    pub fn to_exec_db_levm(&self, block: &Block) -> Result<ExecutionDB, ExecutionDBError> {
        // TODO: Simplify this function and potentially merge with the implementation for
        // RpcDB.

        let parent_hash = block.header.parent_hash;
        let chain_config = self.store.get_chain_config()?;

        // pre-execute and get all state changes
        let (execution_updates, store_wrapper) = ExecutionDB::pre_execute_levm(block, self)
            .map_err(|err| Box::new(EvmError::from(err)))?; // TODO: ugly error handling

        // index read and touched account addresses and storage keys
        let index = execution_updates.iter().map(|update| {
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
        let cache_accounts = execution_updates.iter().filter_map(|update| {
            let address = update.address;
            // filter new accounts (accounts that didn't exist before) assuming our store is
            // correct (based on the success of the pre-execution).
            if store_wrapper
                .store
                .get_account_info_by_hash(parent_hash, address)
                .is_ok_and(|account| account.is_some())
            {
                Some((address, update.info.clone()))
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
        let code = execution_updates
            .clone()
            .iter()
            .map(|update| {
                // return error if code is missing
                let hash = update.info.clone().unwrap_or_default().code_hash;
                Ok((
                    hash,
                    store_wrapper
                        .store
                        .get_account_code(hash)?
                        .ok_or(ExecutionDBError::NewMissingCode(hash))?,
                ))
            })
            .collect::<Result<_, ExecutionDBError>>()?;
        let storage = execution_updates
            .iter()
            .map(|update| {
                // return error if storage is missing
                Ok((
                    update.address,
                    update
                        .added_storage
                        .keys()
                        .map(|key| {
                            let key = CoreH256::from(key.to_fixed_bytes());
                            let value = store_wrapper
                                .store
                                .get_storage_at_hash(parent_hash, update.address, key)
                                .map_err(ExecutionDBError::Store)?
                                .ok_or(ExecutionDBError::NewMissingStorage(update.address, key))?;
                            Ok((key, value))
                        })
                        .collect::<Result<HashMap<_, _>, ExecutionDBError>>()?,
                ))
            })
            .collect::<Result<HashMap<_, _>, ExecutionDBError>>()?;
        let block_hashes = BLOCKS_ACCESSED
            .lock()
            .unwrap()
            .clone()
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
        if let Some(a) = &initial_state_proofs.0 {
            println!("a: {}", a.len());
        }
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
