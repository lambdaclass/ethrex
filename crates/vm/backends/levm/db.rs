use ethrex_common::U256 as CoreU256;
use ethrex_common::constants::EMPTY_KECCACK_HASH;
use ethrex_common::types::{AccountState, Code};
use ethrex_common::{Address as CoreAddress, H256 as CoreH256};
use ethrex_levm::db::Database as LevmDatabase;

use crate::VmDatabase;
use crate::db::DynVmDatabase;
use ethrex_levm::errors::DatabaseError;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::result::Result;
use std::sync::Arc;

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
    fn get_account_state(&self, address: CoreAddress) -> Result<AccountState, DatabaseError> {
        // parking_lot::Mutex::lock() returns guard directly (no Result)
        self.state_accessed.lock().entry(address).or_default();
        let state = self.store.lock().get_account_state(address)?;
        Ok(state)
    }

    fn get_storage_value(
        &self,
        address: CoreAddress,
        key: CoreH256,
    ) -> Result<CoreU256, DatabaseError> {
        self.state_accessed
            .lock()
            .entry(address)
            .and_modify(|keys| keys.push(key))
            .or_insert(vec![key]);
        self.store.lock().get_storage_value(address, key)
    }

    fn get_block_hash(&self, block_number: u64) -> Result<CoreH256, DatabaseError> {
        let block_hash = self.store.lock().get_block_hash(block_number)?;
        self.block_hashes_accessed
            .lock()
            .insert(block_number, block_hash);
        Ok(block_hash)
    }

    fn get_chain_config(&self) -> Result<ethrex_common::types::ChainConfig, DatabaseError> {
        self.store.lock().get_chain_config()
    }

    fn get_account_code(&self, code_hash: CoreH256) -> Result<Code, DatabaseError> {
        if code_hash != *EMPTY_KECCACK_HASH {
            self.code_accessed.lock().push(code_hash);
        }
        self.store.lock().get_account_code(code_hash)
    }
}

impl LevmDatabase for DynVmDatabase {
    fn get_account_state(&self, address: CoreAddress) -> Result<AccountState, DatabaseError> {
        let acc_state = <dyn VmDatabase>::get_account_state(self.as_ref(), address)
            .map_err(|e| DatabaseError::Custom(e.to_string()))?
            .unwrap_or_default();

        Ok(acc_state)
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

    fn get_block_hash(&self, block_number: u64) -> Result<CoreH256, DatabaseError> {
        <dyn VmDatabase>::get_block_hash(self.as_ref(), block_number)
            .map_err(|e| DatabaseError::Custom(e.to_string()))
    }

    fn get_chain_config(&self) -> Result<ethrex_common::types::ChainConfig, DatabaseError> {
        <dyn VmDatabase>::get_chain_config(self.as_ref())
            .map_err(|e| DatabaseError::Custom(e.to_string()))
    }

    fn get_account_code(&self, code_hash: CoreH256) -> Result<Code, DatabaseError> {
        <dyn VmDatabase>::get_account_code(self.as_ref(), code_hash)
            .map_err(|e| DatabaseError::Custom(e.to_string()))
    }
}
