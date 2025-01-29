use std::collections::HashMap;

use ethrex_core::{
    types::{Block, BlockHash},
    Address as CoreAddress, H256 as CoreH256,
};
use ethrex_storage::{error::StoreError, hash_address, hash_key, Store};
use revm::primitives::{
    AccountInfo as RevmAccountInfo, Address as RevmAddress, Bytecode as RevmBytecode,
    Bytes as RevmBytes, B256 as RevmB256, U256 as RevmU256,
};

use crate::{
    errors::ExecutionDBError,
    execution_db::{ExecutionDB, ToExecDB},
    spec_id, EvmError,
};

pub struct StoreWrapper {
    pub store: Store,
    pub block_hash: BlockHash,
}

cfg_if::cfg_if! {
    if #[cfg(feature = "levm")] {
        use ethrex_core::{U256 as CoreU256};
        use ethrex_levm::db::Database as LevmDatabase;

        impl LevmDatabase for StoreWrapper {
            fn get_account_info(&self, address: CoreAddress) -> ethrex_levm::account::AccountInfo {
                let acc_info = self
                    .store
                    .get_account_info_by_hash(self.block_hash, address)
                    .unwrap()
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
                let a = self.store.get_block_header(block_number).unwrap();

                a.map(|a| CoreH256::from(a.compute_block_hash().0))
            }
        }
    }
}

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
        let parent_hash = block.header.parent_hash;
        let chain_config = self.store.get_chain_config()?;

        // pre-execute and get all state changes
        let cache = ExecutionDB::pre_execute(
            block,
            chain_config.chain_id,
            spec_id(&chain_config, block.header.timestamp),
            self,
        )
        .map_err(|err| Box::new(EvmError::from(err)))?; // TODO: must be a better way
        let store_wrapper = cache.db;

        // fetch all read/written values from store
        let already_existing_accounts = cache
            .accounts
            .iter()
            // filter out new accounts, we're only interested in already existing accounts.
            // new accounts are storage cleared, self-destructed accounts too but they're marked with "not
            // existing" status instead.
            .filter_map(|(address, account)| {
                if !account.account_state.is_storage_cleared() {
                    Some((CoreAddress::from(address.0.as_ref()), account))
                } else {
                    None
                }
            });
        let accounts = already_existing_accounts
            .clone()
            .map(|(address, _)| {
                // return error if account is missing
                let account = match store_wrapper
                    .store
                    .get_account_info_by_hash(parent_hash, address)
                {
                    Ok(None) => Err(ExecutionDBError::NewMissingAccountInfo(address)),
                    Ok(Some(some)) => Ok(some),
                    Err(err) => Err(ExecutionDBError::Store(err)),
                };
                Ok((address, account?))
            })
            .collect::<Result<HashMap<_, _>, ExecutionDBError>>()?;
        let code = already_existing_accounts
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
        let storage = already_existing_accounts
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

        // get proofs
        let state_trie = self
            .store
            .state_trie(parent_hash)?
            .ok_or(ExecutionDBError::NewMissingStateTrie(parent_hash))?;

        let state_proofs =
            state_trie.get_proofs(&accounts.keys().map(hash_address).collect::<Vec<_>>())?;

        let mut storage_proofs = HashMap::new();
        for (address, storages) in &storage {
            let storage_trie = self.store.storage_trie(parent_hash, *address)?.ok_or(
                ExecutionDBError::NewMissingStorageTrie(parent_hash, *address),
            )?;

            let paths = storages.keys().map(hash_key).collect::<Vec<_>>();
            storage_proofs.insert(*address, storage_trie.get_proofs(&paths)?);
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
