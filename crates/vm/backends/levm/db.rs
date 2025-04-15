use ethrex_common::types::AccountInfo;
use ethrex_common::U256 as CoreU256;
use ethrex_common::{Address as CoreAddress, H256 as CoreH256};
use ethrex_levm::constants::EMPTY_CODE_HASH;
use ethrex_levm::db::Database as LevmDatabase;

use crate::db::{ExecutionDB, StoreWrapper};
use ethrex_levm::db::error::DatabaseError;
use std::collections::HashMap;
use std::result::Result;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct DatabaseLogger {
    pub block_hashes_accessed: Arc<Mutex<HashMap<u64, CoreH256>>>,
    pub accounts_accessed: Arc<Mutex<Vec<CoreAddress>>>,
    pub storage_accessed: Arc<Mutex<HashMap<(CoreAddress, CoreH256), CoreU256>>>,
    pub code_accessed: Arc<Mutex<Vec<CoreH256>>>,
    pub store: Arc<dyn LevmDatabase>,
}

impl DatabaseLogger {
    pub fn new(store: Arc<dyn LevmDatabase>) -> Self {
        Self {
            block_hashes_accessed: Arc::new(Mutex::new(HashMap::new())),
            accounts_accessed: Arc::new(Mutex::new(Vec::new())),
            storage_accessed: Arc::new(Mutex::new(HashMap::new())),
            code_accessed: Arc::new(Mutex::new(vec![])),
            store,
        }
    }
}

impl LevmDatabase for DatabaseLogger {
    fn get_account_info(
        &self,
        address: CoreAddress,
    ) -> Result<ethrex_levm::AccountInfo, DatabaseError> {
        let acc_info = self.store.get_account_info(address)?;
        self.accounts_accessed
            .lock()
            .map_err(|_| DatabaseError::Custom("Could not lock mutex".to_string()))?
            .push(address);
        Ok(acc_info)
    }

    fn account_exists(&self, address: CoreAddress) -> bool {
        self.store.account_exists(address)
    }

    fn get_storage_slot(
        &self,
        address: CoreAddress,
        key: CoreH256,
    ) -> Result<CoreU256, DatabaseError> {
        let slot = self.store.get_storage_slot(address, key)?;
        self.storage_accessed
            .lock()
            .map_err(|_| DatabaseError::Custom("Could not lock mutex".to_string()))?
            .insert((address, key), slot);
        Ok(slot)
    }

    fn get_block_hash(&self, block_number: u64) -> Result<Option<CoreH256>, DatabaseError> {
        let block_hash = self.store.get_block_hash(block_number)?;
        if let Some(hash) = block_hash {
            self.block_hashes_accessed
                .lock()
                .map_err(|_| DatabaseError::Custom("Could not lock mutex".to_string()))?
                .insert(block_number, hash);
        }
        Ok(block_hash)
    }

    fn get_chain_config(&self) -> ethrex_common::types::ChainConfig {
        self.store.get_chain_config()
    }

    fn get_account_info_by_hash(
        &self,
        block_hash: ethrex_common::types::BlockHash,
        address: CoreAddress,
    ) -> Result<Option<AccountInfo>, DatabaseError> {
        let account = self.store.get_account_info_by_hash(block_hash, address)?;
        {
            if let Some(acc) = account.clone() {
                let mut code_accessed = self
                    .code_accessed
                    .lock()
                    .map_err(|_| DatabaseError::Custom("Could not lock mutex".to_string()))?;
                code_accessed.push(acc.code_hash);
            }
        }

        Ok(account)
    }

    fn get_account_code(&self, code_hash: CoreH256) -> Result<Option<bytes::Bytes>, DatabaseError> {
        {
            let mut code_accessed = self
                .code_accessed
                .lock()
                .map_err(|_| DatabaseError::Custom("Could not lock mutex".to_string()))?;
            code_accessed.push(code_hash);
        }
        self.store.get_account_code(code_hash)
    }
}

impl LevmDatabase for StoreWrapper {
    fn get_account_info(
        &self,
        address: CoreAddress,
    ) -> Result<ethrex_levm::AccountInfo, DatabaseError> {
        let acc_info = self
            .store
            .get_account_info_by_hash(self.block_hash, address)
            .unwrap_or(None)
            .unwrap_or_default();

        let acc_code = self
            .store
            .get_account_code(acc_info.code_hash)
            .map_err(|e| DatabaseError::Custom(e.to_string()))?
            .unwrap_or_default();

        Ok(ethrex_levm::account::AccountInfo {
            balance: acc_info.balance,
            nonce: acc_info.nonce,
            bytecode: acc_code,
        })
    }

    fn account_exists(&self, address: CoreAddress) -> bool {
        let acc_info = self
            .store
            .get_account_info_by_hash(self.block_hash, address)
            .unwrap();

        acc_info.is_some()
    }

    fn get_storage_slot(
        &self,
        address: CoreAddress,
        key: CoreH256,
    ) -> Result<ethrex_common::U256, DatabaseError> {
        Ok(self
            .store
            .get_storage_at_hash(self.block_hash, address, key)
            .map_err(|e| DatabaseError::Custom(e.to_string()))?
            .unwrap_or_default())
    }

    fn get_block_hash(&self, block_number: u64) -> Result<Option<CoreH256>, DatabaseError> {
        Ok(self
            .store
            .get_block_header(block_number)
            .map_err(|e| DatabaseError::Custom(e.to_string()))?
            .map(|header| CoreH256::from(header.compute_block_hash().0)))
    }

    fn get_chain_config(&self) -> ethrex_common::types::ChainConfig {
        self.store.get_chain_config().unwrap()
    }

    fn get_account_info_by_hash(
        &self,
        block_hash: ethrex_common::types::BlockHash,
        address: CoreAddress,
    ) -> Result<Option<AccountInfo>, DatabaseError> {
        self.store
            .get_account_info_by_hash(block_hash, address)
            .map_err(|e| DatabaseError::Custom(e.to_string()))
    }

    fn get_account_code(&self, code_hash: CoreH256) -> Result<Option<bytes::Bytes>, DatabaseError> {
        self.store
            .get_account_code(code_hash)
            .map_err(|e| DatabaseError::Custom(e.to_string()))
    }
}

impl LevmDatabase for ExecutionDB {
    fn get_account_info(
        &self,
        address: CoreAddress,
    ) -> Result<ethrex_levm::AccountInfo, DatabaseError> {
        let Some(acc_info) = self.accounts.get(&address) else {
            return Ok(ethrex_levm::AccountInfo::default());
        };

        let acc_code = if acc_info.code_hash != EMPTY_CODE_HASH {
            self.code
                .get(&acc_info.code_hash)
                .ok_or(DatabaseError::Custom(format!(
                    "Could not find account's code hash {}",
                    &acc_info.code_hash
                )))?
        } else {
            &bytes::Bytes::new()
        };

        Ok(ethrex_levm::AccountInfo {
            balance: acc_info.balance,
            bytecode: acc_code.clone(),
            nonce: acc_info.nonce,
        })
    }

    fn account_exists(&self, address: CoreAddress) -> bool {
        self.accounts.contains_key(&address)
    }

    fn get_block_hash(&self, block_number: u64) -> Result<Option<CoreH256>, DatabaseError> {
        Ok(self.block_hashes.get(&block_number).cloned())
    }

    fn get_storage_slot(
        &self,
        address: CoreAddress,
        key: CoreH256,
    ) -> Result<CoreU256, DatabaseError> {
        let Some(storage) = self.storage.get(&address) else {
            return Ok(CoreU256::default());
        };
        Ok(*storage.get(&key).unwrap_or(&CoreU256::default()))
    }

    fn get_chain_config(&self) -> ethrex_common::types::ChainConfig {
        self.get_chain_config()
    }

    fn get_account_info_by_hash(
        &self,
        _block_hash: ethrex_common::types::BlockHash,
        address: CoreAddress,
    ) -> Result<Option<ethrex_common::types::AccountInfo>, DatabaseError> {
        Ok(self.accounts.get(&address).cloned())
    }

    fn get_account_code(&self, code_hash: CoreH256) -> Result<Option<bytes::Bytes>, DatabaseError> {
        Ok(self.code.get(&code_hash).cloned())
    }
}
