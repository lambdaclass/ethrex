use crate::{EvmError, VmDatabase};
use bytes::Bytes;
use ethrex_common::{
    Address, H256, U256,
    types::{
        AccountInfo, AccountUpdate, Block, BlockHeader, ChainConfig,
        block_execution_witness::{ExecutionWitnessError, ExecutionWitnessResult},
    },
};
use std::sync::{Arc, Mutex, MutexGuard};

#[derive(Clone)]
pub struct ExecutionWitnessWrapper {
    inner: Arc<Mutex<ExecutionWitnessResult>>,
}

impl ExecutionWitnessWrapper {
    pub fn new(db: ExecutionWitnessResult) -> Self {
        Self {
            inner: Arc::new(Mutex::new(db)),
        }
    }

    fn lock_mutex(&self) -> Result<MutexGuard<ExecutionWitnessResult>, ExecutionWitnessError> {
        self.inner
            .lock()
            .map_err(|_| ExecutionWitnessError::Database("Failed to lock DB".to_string()))
    }

    pub fn apply_account_updates(
        &mut self,
        account_updates: &[AccountUpdate],
    ) -> Result<(), ExecutionWitnessError> {
        self.lock_mutex()?.apply_account_updates(account_updates)
    }

    pub fn state_trie_root(&self) -> Result<H256, ExecutionWitnessError> {
        self.lock_mutex()?.state_trie_root()
    }

    pub fn get_first_invalid_block_hash(&self) -> Result<Option<u64>, ExecutionWitnessError> {
        self.lock_mutex()?.get_first_invalid_block_hash()
    }

    pub fn get_block_parent_header(
        &self,
        block_number: u64,
    ) -> Result<BlockHeader, ExecutionWitnessError> {
        self.lock_mutex()?
            .get_block_parent_header(block_number)
            .cloned()
    }

    pub fn initialize_block_header_hashes(
        &self,
        blocks: &[Block],
    ) -> Result<(), ExecutionWitnessError> {
        self.lock_mutex()?.initialize_block_header_hashes(blocks)
    }
}

impl VmDatabase for ExecutionWitnessWrapper {
    fn get_account_code(&self, code_hash: H256) -> Result<Bytes, EvmError> {
        self.lock_mutex()
            .map_err(|_| EvmError::DB("Failed to lock db".to_string()))?
            .get_account_code(code_hash)
            .map_err(|_| EvmError::DB("Failed to get account code".to_string()))
    }

    fn get_account_info(&self, address: Address) -> Result<Option<AccountInfo>, EvmError> {
        self.lock_mutex()
            .map_err(|_| EvmError::DB("Failed to lock db".to_string()))?
            .get_account_info(address)
            .map_err(|_| EvmError::DB("Failed to get account info".to_string()))
    }

    fn get_block_hash(&self, block_number: u64) -> Result<H256, EvmError> {
        self.lock_mutex()
            .map_err(|_| EvmError::DB("Failed to lock db".to_string()))?
            .get_block_hash(block_number)
            .map_err(|_| EvmError::DB("Failed get block hash".to_string()))
    }

    fn get_chain_config(&self) -> Result<ChainConfig, EvmError> {
        self.lock_mutex()
            .map_err(|_| EvmError::DB("Failed to lock db".to_string()))?
            .get_chain_config()
            .map_err(|_| EvmError::DB("Failed get chain config".to_string()))
    }

    fn get_storage_slot(&self, address: Address, key: H256) -> Result<Option<U256>, EvmError> {
        self.lock_mutex()
            .map_err(|_| EvmError::DB("Failed to lock db".to_string()))?
            .get_storage_slot(address, key)
            .map_err(|_| EvmError::DB("Failed get storage slot".to_string()))
    }

    fn get_account_info_batch(
        &self,
        addresses: &[Address],
    ) -> Result<std::collections::BTreeMap<Address, AccountInfo>, EvmError> {
        self.lock_mutex()
            .map_err(|_| EvmError::DB("Failed to lock db".to_string()))?
            .get_account_info_by_hash_batch(addresses)
            .map_err(|_| EvmError::DB("Failed to get account infos".to_string()))
    }
}
