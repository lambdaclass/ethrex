use ethrex_common::types::Account;
use ethrex_common::U256 as CoreU256;
use ethrex_common::{Address as CoreAddress, H256 as CoreH256};
use ethrex_levm::constants::EMPTY_CODE_HASH;
use ethrex_levm::db::Database as LevmDatabase;

use crate::db::StoreWrapper;
use crate::ProverDB;
use ethrex_levm::db::error::DatabaseError;
use std::collections::HashMap;
use std::result::Result;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct DatabaseLogger {
    pub block_hashes_accessed: Arc<Mutex<HashMap<u64, CoreH256>>>,
    pub state_accessed: Arc<Mutex<HashMap<CoreAddress, Vec<CoreH256>>>>,
    pub code_accessed: Arc<Mutex<Vec<CoreH256>>>,
    // TODO: Refactor this
    pub store: Arc<Mutex<Box<dyn LevmDatabase>>>,
}

impl DatabaseLogger {
    pub fn new(store: Arc<Mutex<Box<dyn LevmDatabase>>>) -> Self {
        Self {
            block_hashes_accessed: Arc::new(Mutex::new(HashMap::new())),
            state_accessed: Arc::new(Mutex::new(HashMap::new())),
            code_accessed: Arc::new(Mutex::new(vec![])),
            store,
        }
    }
}

impl LevmDatabase for DatabaseLogger {
    fn get_account(&self, address: CoreAddress) -> Result<Account, DatabaseError> {
        self.state_accessed
            .lock()
            .map_err(|_| DatabaseError::Custom("Could not lock mutex".to_string()))?
            .entry(address)
            .or_default();
        self.store
            .lock()
            .map_err(|_| DatabaseError::Custom("Could not lock mutex".to_string()))?
            .get_account(address)
    }

    fn account_exists(&self, address: CoreAddress) -> bool {
        self.store.lock().unwrap().account_exists(address)
    }

    fn get_storage_value(
        &self,
        address: CoreAddress,
        key: CoreH256,
    ) -> Result<CoreU256, DatabaseError> {
        self.state_accessed
            .lock()
            .map_err(|_| DatabaseError::Custom("Could not lock mutex".to_string()))?
            .entry(address)
            .and_modify(|keys| keys.push(key))
            .or_insert(vec![key]);
        self.store
            .lock()
            .map_err(|_| DatabaseError::Custom("Could not lock mutex".to_string()))?
            .get_storage_value(address, key)
    }

    fn get_block_hash(&self, block_number: u64) -> Result<Option<CoreH256>, DatabaseError> {
        let block_hash = self
            .store
            .lock()
            .map_err(|_| DatabaseError::Custom("Could not lock mutex".to_string()))?
            .get_block_hash(block_number)?;
        if let Some(hash) = block_hash {
            self.block_hashes_accessed
                .lock()
                .map_err(|_| DatabaseError::Custom("Could not lock mutex".to_string()))?
                .insert(block_number, hash);
        }
        Ok(block_hash)
    }

    fn get_chain_config(&self) -> ethrex_common::types::ChainConfig {
        self.store.lock().unwrap().get_chain_config()
    }

    fn get_account_code(&self, code_hash: CoreH256) -> Result<Option<bytes::Bytes>, DatabaseError> {
        {
            let mut code_accessed = self
                .code_accessed
                .lock()
                .map_err(|_| DatabaseError::Custom("Could not lock mutex".to_string()))?;
            code_accessed.push(code_hash);
        }
        self.store
            .lock()
            .map_err(|_| DatabaseError::Custom("Could not lock mutex".to_string()))?
            .get_account_code(code_hash)
    }
}

impl LevmDatabase for StoreWrapper {
    fn get_account(&self, address: CoreAddress) -> Result<Account, DatabaseError> {
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

        Ok(Account::new(
            acc_info.balance,
            acc_code,
            acc_info.nonce,
            HashMap::new(),
        ))
    }

    fn account_exists(&self, address: CoreAddress) -> bool {
        let acc_info = self
            .store
            .get_account_info_by_hash(self.block_hash, address)
            .unwrap();

        acc_info.is_some()
    }

    fn get_storage_value(
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

    fn get_account_code(&self, code_hash: CoreH256) -> Result<Option<bytes::Bytes>, DatabaseError> {
        self.store
            .get_account_code(code_hash)
            .map_err(|e| DatabaseError::Custom(e.to_string()))
    }
}

impl LevmDatabase for ProverDB {
    fn get_account(&self, address: CoreAddress) -> Result<Account, DatabaseError> {
        let Some(acc_info) = self.accounts.get(&address) else {
            return Ok(Account::default());
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

        Ok(Account::new(
            acc_info.balance,
            acc_code.clone(),
            acc_info.nonce,
            HashMap::new(),
        ))
    }

    fn account_exists(&self, address: CoreAddress) -> bool {
        self.accounts.contains_key(&address)
    }

    fn get_block_hash(&self, block_number: u64) -> Result<Option<CoreH256>, DatabaseError> {
        Ok(self.block_hashes.get(&block_number).cloned())
    }

    fn get_storage_value(
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

    fn get_account_code(&self, code_hash: CoreH256) -> Result<Option<bytes::Bytes>, DatabaseError> {
        Ok(self.code.get(&code_hash).cloned())
    }
}
