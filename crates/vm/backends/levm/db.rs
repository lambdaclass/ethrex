use ethrex_common::U256 as CoreU256;
use ethrex_common::constants::EMPTY_KECCAK_HASH;
use ethrex_common::types::{AccountState, Code, CodeMetadata};
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
    pub block_hashes_accessed: Arc<Mutex<HashMap<u64, CoreH256>>>,
    pub state_accessed: Arc<Mutex<HashMap<CoreAddress, Vec<CoreH256>>>>,
    pub code_accessed: Arc<Mutex<Vec<CoreH256>>>,
    pub store: Arc<dyn LevmDatabase>,
}

impl DatabaseLogger {
    pub fn new(store: Arc<dyn LevmDatabase>) -> Self {
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
        self.state_accessed
            .lock()
            .map_err(|_| DatabaseError::Custom("Could not lock mutex".to_string()))?
            .entry(address)
            .or_default();
        let state = self.store.as_ref().get_account_state(address)?;
        Ok(state)
    }

    /// Forwarded, and recorded as an account access.
    ///
    /// Recording matters even though execution always pairs this with a
    /// `get_account_state` on the same address today: `state_accessed` is what
    /// the execution witness is built from, and an address the block only ever
    /// asked the storage question about would otherwise be missing from the
    /// witness, making the proof unverifiable. Cheap insurance against that
    /// pairing being loosened later.
    fn has_storage(&self, address: CoreAddress) -> Result<bool, DatabaseError> {
        self.state_accessed
            .lock()
            .map_err(|_| DatabaseError::Custom("Could not lock mutex".to_string()))?
            .entry(address)
            .or_default();
        self.store.as_ref().has_storage(address)
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
        self.store.as_ref().get_storage_value(address, key)
    }

    fn get_block_hash(&self, block_number: u64) -> Result<CoreH256, DatabaseError> {
        let block_hash = self.store.as_ref().get_block_hash(block_number)?;
        self.block_hashes_accessed
            .lock()
            .map_err(|_| DatabaseError::Custom("Could not lock mutex".to_string()))?
            .insert(block_number, block_hash);
        Ok(block_hash)
    }

    fn get_chain_config(&self) -> Result<ethrex_common::types::ChainConfig, DatabaseError> {
        self.store.as_ref().get_chain_config()
    }

    fn get_account_code(&self, code_hash: CoreH256) -> Result<Code, DatabaseError> {
        if code_hash != *EMPTY_KECCAK_HASH {
            let mut code_accessed = self
                .code_accessed
                .lock()
                .map_err(|_| DatabaseError::Custom("Could not lock mutex".to_string()))?;
            code_accessed.push(code_hash);
        }
        self.store.as_ref().get_account_code(code_hash)
    }

    fn get_code_metadata(&self, code_hash: CoreH256) -> Result<CodeMetadata, DatabaseError> {
        self.store.get_code_metadata(code_hash)
    }
}

impl LevmDatabase for DynVmDatabase {
    fn get_account_state(&self, address: CoreAddress) -> Result<AccountState, DatabaseError> {
        let acc_state = <dyn VmDatabase>::get_account_state(self.as_ref(), address)
            .map_err(|e| DatabaseError::Custom(e.to_string()))?
            .unwrap_or_default();

        Ok(acc_state)
    }

    fn get_account_states_batch(
        &self,
        addresses: &[CoreAddress],
    ) -> Result<Vec<AccountState>, DatabaseError> {
        let states = <dyn VmDatabase>::get_account_states_batch(self.as_ref(), addresses)
            .map_err(|e| DatabaseError::Custom(e.to_string()))?;
        Ok(states
            .into_iter()
            .map(|opt| opt.unwrap_or_default())
            .collect())
    }

    /// The seam where the levm-side question reaches the trie that can answer
    /// it. Deriving it from `get_account_state` here instead — the trait's
    /// default — would drop the binary-trie answer on the floor, since
    /// `AccountState` has nowhere to carry it.
    fn has_storage(&self, address: CoreAddress) -> Result<bool, DatabaseError> {
        <dyn VmDatabase>::has_storage(self.as_ref(), address)
            .map_err(|e| DatabaseError::Custom(e.to_string()))
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

    fn get_storage_values_batch(
        &self,
        keys: &[(CoreAddress, CoreH256)],
    ) -> Result<Vec<CoreU256>, DatabaseError> {
        let values = <dyn VmDatabase>::get_storage_slots_batch(self.as_ref(), keys)
            .map_err(|e| DatabaseError::Custom(e.to_string()))?;
        Ok(values
            .into_iter()
            .map(|opt| opt.unwrap_or_default())
            .collect())
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

    fn get_code_metadata(&self, code_hash: CoreH256) -> Result<CodeMetadata, DatabaseError> {
        <dyn VmDatabase>::get_code_metadata(self.as_ref(), code_hash)
            .map_err(|e| DatabaseError::Custom(e.to_string()))
    }
}
