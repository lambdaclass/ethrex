use std::sync::Mutex;

use ethrex_common::{
    Address, H256, U256,
    types::{
        AccountState, ChainConfig, Code, CodeMetadata, block_execution_witness::GuestProgramState,
    },
};

use super::Database;
use crate::errors::DatabaseError;

/// Adapter that implements LEVM's [`Database`] trait backed by a [`GuestProgramState`].
///
/// Uses a `Mutex` for interior mutability because `GuestProgramState` methods
/// require `&mut self` (they lazily populate caches like `account_hashes_by_address`).
pub struct GuestProgramStateDb {
    pub state: Mutex<GuestProgramState>,
}

impl GuestProgramStateDb {
    pub fn new(state: GuestProgramState) -> Self {
        Self {
            state: Mutex::new(state),
        }
    }
}

impl Database for GuestProgramStateDb {
    fn get_account_state(&self, address: Address) -> Result<AccountState, DatabaseError> {
        Ok(self
            .state
            .lock()
            .map_err(|e| DatabaseError::Custom(format!("Lock poisoned: {e}")))?
            .get_account_state(address)
            .map_err(|e| DatabaseError::Custom(e.to_string()))?
            .unwrap_or_default())
    }

    fn get_storage_value(&self, address: Address, key: H256) -> Result<U256, DatabaseError> {
        Ok(self
            .state
            .lock()
            .map_err(|e| DatabaseError::Custom(format!("Lock poisoned: {e}")))?
            .get_storage_slot(address, key)
            .map_err(|e| DatabaseError::Custom(e.to_string()))?
            .unwrap_or_default())
    }

    fn get_block_hash(&self, block_number: u64) -> Result<H256, DatabaseError> {
        self.state
            .lock()
            .map_err(|e| DatabaseError::Custom(format!("Lock poisoned: {e}")))?
            .get_block_hash(block_number)
            .map_err(|e| DatabaseError::Custom(e.to_string()))
    }

    fn get_chain_config(&self) -> Result<ChainConfig, DatabaseError> {
        self.state
            .lock()
            .map_err(|e| DatabaseError::Custom(format!("Lock poisoned: {e}")))?
            .get_chain_config()
            .map_err(|e| DatabaseError::Custom(e.to_string()))
    }

    fn get_account_code(&self, code_hash: H256) -> Result<Code, DatabaseError> {
        self.state
            .lock()
            .map_err(|e| DatabaseError::Custom(format!("Lock poisoned: {e}")))?
            .get_account_code(code_hash)
            .map_err(|e| DatabaseError::Custom(e.to_string()))
    }

    fn get_code_metadata(&self, code_hash: H256) -> Result<CodeMetadata, DatabaseError> {
        self.state
            .lock()
            .map_err(|e| DatabaseError::Custom(format!("Lock poisoned: {e}")))?
            .get_code_metadata(code_hash)
            .map_err(|e| DatabaseError::Custom(e.to_string()))
    }
}
