use bytes::Bytes;
use ethrex_common::{
    Address, H256, U256,
    types::{
        AccountState, AccountUpdate, Block, BlockHeader, ChainConfig,
        block_execution_witness::{GuestProgramState, GuestProgramStateError},
    },
};
use ethrex_levm::db::Database as LevmDatabase;
use ethrex_levm::errors::DatabaseError;
use std::sync::{Arc, Mutex, MutexGuard};

#[derive(Clone)]
pub struct GuestProgramStateWrapper {
    inner: Arc<Mutex<GuestProgramState>>,
}

impl GuestProgramStateWrapper {
    pub fn new(db: GuestProgramState) -> Self {
        Self {
            inner: Arc::new(Mutex::new(db)),
        }
    }

    pub fn lock_mutex(&self) -> Result<MutexGuard<'_, GuestProgramState>, GuestProgramStateError> {
        self.inner
            .lock()
            .map_err(|_| GuestProgramStateError::Database("Failed to lock DB".to_string()))
    }

    pub fn apply_account_updates(
        &mut self,
        account_updates: &[AccountUpdate],
    ) -> Result<(), GuestProgramStateError> {
        self.lock_mutex()?.apply_account_updates(account_updates)
    }

    pub fn state_trie_root(&self) -> Result<H256, GuestProgramStateError> {
        self.lock_mutex()?.state_trie_root()
    }

    pub fn get_first_invalid_block_hash(&self) -> Result<Option<u64>, GuestProgramStateError> {
        self.lock_mutex()?.get_first_invalid_block_hash()
    }

    pub fn get_block_parent_header(
        &self,
        block_number: u64,
    ) -> Result<BlockHeader, GuestProgramStateError> {
        self.lock_mutex()?
            .get_block_parent_header(block_number)
            .cloned()
    }

    pub fn initialize_block_header_hashes(
        &self,
        blocks: &[Block],
    ) -> Result<(), GuestProgramStateError> {
        self.lock_mutex()?.initialize_block_header_hashes(blocks)
    }

    pub fn get_chain_config(&self) -> Result<ChainConfig, GuestProgramStateError> {
        self.lock_mutex()?.get_chain_config()
    }
}

impl LevmDatabase for GuestProgramStateWrapper {
    fn get_account_state(&self, address: Address) -> Result<AccountState, DatabaseError> {
        self.lock_mutex()
            .map_err(|_| DatabaseError::Custom("Failed to lock db".to_string()))?
            .get_account_state(address)
            .map(|opt| opt.unwrap_or_default())
            .map_err(|_| DatabaseError::Custom("Failed to get account info".to_string()))
    }

    fn get_storage_value(&self, address: Address, key: H256) -> Result<U256, DatabaseError> {
        self.lock_mutex()
            .map_err(|_| DatabaseError::Custom("Failed to lock db".to_string()))?
            .get_storage_slot(address, key)
            .map(|opt| opt.unwrap_or_default())
            .map_err(|_| DatabaseError::Custom("Failed get storage slot".to_string()))
    }

    fn get_block_hash(&self, block_number: u64) -> Result<H256, DatabaseError> {
        self.lock_mutex()
            .map_err(|_| DatabaseError::Custom("Failed to lock db".to_string()))?
            .get_block_hash(block_number)
            .map_err(|_| DatabaseError::Custom("Failed get block hash".to_string()))
    }

    fn get_chain_config(&self) -> Result<ethrex_common::types::ChainConfig, DatabaseError> {
        self.lock_mutex()
            .map_err(|_| DatabaseError::Custom("Failed to lock db".to_string()))?
            .get_chain_config()
            .map_err(|_| DatabaseError::Custom("Failed get chain config".to_string()))
    }

    fn get_account_code(&self, code_hash: H256) -> Result<Bytes, DatabaseError> {
        self.lock_mutex()
            .map_err(|_| DatabaseError::Custom("Failed to lock db".to_string()))?
            .get_account_code(code_hash)
            .map_err(|_| DatabaseError::Custom("Failed to get account code".to_string()))
    }
}
