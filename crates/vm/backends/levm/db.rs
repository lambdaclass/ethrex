use ethrex_common::U256 as CoreU256;
use ethrex_common::constants::EMPTY_KECCACK_HASH;
use ethrex_common::types::{AccountInfo, BlockHash};
use ethrex_common::{Address as CoreAddress, H256 as CoreH256};
use ethrex_levm::db::Database as LevmDatabase;

use crate::VmDatabase;
use crate::db::DynVmDatabase;
use ethrex_levm::errors::DatabaseError;
use std::collections::HashMap;
use std::result::Result;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct DatabaseLogger {
    pub block_hashes_accessed: Arc<Mutex<HashMap<u64, BlockHash>>>,
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
    fn get_account_info(&self, address: CoreAddress) -> Result<AccountInfo, DatabaseError> {
        self.state_accessed
            .lock()
            .map_err(|_| DatabaseError::Custom("Could not lock mutex".to_string()))?
            .entry(address)
            .or_default();
        let info = self
            .store
            .lock()
            .map_err(|_| DatabaseError::Custom("Could not lock mutex".to_string()))?
            .get_account_info(address)?;
        Ok(info)
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

    fn get_block_hash(&self, block_number: u64) -> Result<BlockHash, DatabaseError> {
        let block_hash = self
            .store
            .lock()
            .map_err(|_| DatabaseError::Custom("Could not lock mutex".to_string()))?
            .get_block_hash(block_number)?;
        self.block_hashes_accessed
            .lock()
            .map_err(|_| DatabaseError::Custom("Could not lock mutex".to_string()))?
            .insert(block_number, block_hash);
        Ok(block_hash)
    }

    fn get_chain_config(&self) -> Result<ethrex_common::types::ChainConfig, DatabaseError> {
        self.store
            .lock()
            .map_err(|_| {
                DatabaseError::Custom("Could not lock mutex and get chain config".to_string())
            })?
            .get_chain_config()
    }

    fn get_account_code(&self, code_hash: CoreH256) -> Result<bytes::Bytes, DatabaseError> {
        if code_hash != *EMPTY_KECCACK_HASH {
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

impl LevmDatabase for DynVmDatabase {
    fn get_account_info(&self, address: CoreAddress) -> Result<AccountInfo, DatabaseError> {
        let acc_info = <dyn VmDatabase>::get_account_info(self.as_ref(), address)
            .map_err(|e| DatabaseError::Custom(e.to_string()))?
            .unwrap_or_default();

        Ok(acc_info)
    }

    fn get_storage_value(
        &self,
        address: CoreAddress,
        key: CoreH256,
    ) -> Result<ethrex_common::U256, DatabaseError> {
        Ok(
            <dyn VmDatabase>::get_storage_slot(self.as_ref(), address, key)
                .map_err(|e| DatabaseError::Custom(e.to_string()))?
                .unwrap_or_default(),
        )
    }

    fn get_block_hash(&self, block_number: u64) -> Result<BlockHash, DatabaseError> {
        <dyn VmDatabase>::get_block_hash(self.as_ref(), block_number)
            .map_err(|e| DatabaseError::Custom(e.to_string()))
    }

    fn get_chain_config(&self) -> Result<ethrex_common::types::ChainConfig, DatabaseError> {
        <dyn VmDatabase>::get_chain_config(self.as_ref())
            .map_err(|e| DatabaseError::Custom(e.to_string()))
    }

    fn get_account_code(&self, code_hash: CoreH256) -> Result<bytes::Bytes, DatabaseError> {
        <dyn VmDatabase>::get_account_code(self.as_ref(), code_hash)
            .map_err(|e| DatabaseError::Custom(e.to_string()))
    }
}
