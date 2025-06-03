use ethrex_common::types::Account;
use ethrex_common::U256 as CoreU256;
use ethrex_common::{Address as CoreAddress, H256 as CoreH256};
use ethrex_levm::db::Database as LevmDatabase;

use crate::db::DynVmDatabase;
use crate::VmDatabase;
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
        let account = self
            .store
            .lock()
            .map_err(|_| DatabaseError::Custom("Could not lock mutex".to_string()))?
            .get_account(address)?;
        // We have to treat the code as accessed because Account has access to the code
        // And some parts of LEVM use the bytecode from the account instead of using get_account_code
        let mut code_accessed = self
            .code_accessed
            .lock()
            .map_err(|_| DatabaseError::Custom("Could not lock mutex".to_string()))?;
        code_accessed.push(account.info.code_hash);
        Ok(account)
    }

    fn account_exists(&self, address: CoreAddress) -> Result<bool, DatabaseError> {
        self.store
            .lock()
            .map_err(|_| DatabaseError::Custom("Could not lock mutex".to_string()))?
            .account_exists(address)
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

    fn get_block_hash(&self, block_number: u64) -> Result<CoreH256, DatabaseError> {
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

impl LevmDatabase for DynVmDatabase {
    fn get_account(&self, address: CoreAddress) -> Result<Account, DatabaseError> {
        let acc_info = <dyn VmDatabase>::get_account_info(self.as_ref(), address)
            .map_err(|e| DatabaseError::Custom(e.to_string()))?
            .unwrap_or_default();

        let acc_code = <dyn VmDatabase>::get_account_code(self.as_ref(), acc_info.code_hash)
            .map_err(|e| DatabaseError::Custom(e.to_string()))?;

        Ok(Account::new(
            acc_info.balance,
            acc_code,
            acc_info.nonce,
            HashMap::new(),
        ))
    }

    fn account_exists(&self, address: CoreAddress) -> Result<bool, DatabaseError> {
        let acc_info = <dyn VmDatabase>::get_account_info(self.as_ref(), address)
            .map_err(|e| DatabaseError::Custom(e.to_string()))?;
        Ok(acc_info.is_some())
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

    fn get_account_code(&self, code_hash: CoreH256) -> Result<bytes::Bytes, DatabaseError> {
        <dyn VmDatabase>::get_account_code(self.as_ref(), code_hash)
            .map_err(|e| DatabaseError::Custom(e.to_string()))
    }
}
