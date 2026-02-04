use crate::errors::DatabaseError;
use dashmap::DashMap;
use ethrex_common::{
    Address, H256, U256,
    types::{AccountState, ChainConfig, Code, CodeMetadata},
};
use std::sync::Arc;

pub mod gen_db;

pub trait Database: Send + Sync {
    fn get_account_state(&self, address: Address) -> Result<AccountState, DatabaseError>;
    fn get_storage_value(&self, address: Address, key: H256) -> Result<U256, DatabaseError>;
    fn get_block_hash(&self, block_number: u64) -> Result<H256, DatabaseError>;
    fn get_chain_config(&self) -> Result<ChainConfig, DatabaseError>;
    fn get_account_code(&self, code_hash: H256) -> Result<Code, DatabaseError>;
    fn get_code_metadata(&self, code_hash: H256) -> Result<CodeMetadata, DatabaseError>;
}

/// A database wrapper that caches state lookups for parallel pre-warming.
///
/// This enables parallel warming workers to share cached data, and allows
/// the sequential execution phase to reuse warmed state. Reduces redundant
/// database/trie lookups when multiple transactions touch the same accounts.
///
/// Thread-safe via DashMap - provides lock-free reads and fine-grained locking
/// for writes (per-shard instead of global). This eliminates the double-lock
/// pattern where RwLock required: read lock -> miss -> release -> write lock.
///
/// This caching database is inspired by reth's overlay/proof worker cache.
pub struct CachingDatabase {
    inner: Arc<dyn Database>,
    /// Cached account states (balance, nonce, code_hash, storage_root)
    accounts: DashMap<Address, AccountState>,
    /// Cached storage values
    storage: DashMap<(Address, H256), U256>,
    /// Cached contract code
    code: DashMap<H256, Code>,
}

impl CachingDatabase {
    pub fn new(inner: Arc<dyn Database>) -> Self {
        Self {
            inner,
            accounts: DashMap::default(),
            storage: DashMap::default(),
            code: DashMap::default(),
        }
    }
}

impl Database for CachingDatabase {
    fn get_account_state(&self, address: Address) -> Result<AccountState, DatabaseError> {
        // Check cache first (lock-free read via DashMap)
        if let Some(state) = self.accounts.get(&address) {
            return Ok(*state);
        }

        // Cache miss: query underlying database
        let state = self.inner.get_account_state(address)?;

        // Populate cache (fine-grained per-shard lock, no global write lock)
        self.accounts.insert(address, state);

        Ok(state)
    }

    fn get_storage_value(&self, address: Address, key: H256) -> Result<U256, DatabaseError> {
        // Check cache first (lock-free read via DashMap)
        if let Some(value) = self.storage.get(&(address, key)) {
            return Ok(*value);
        }

        // Cache miss: query underlying database
        let value = self.inner.get_storage_value(address, key)?;

        // Populate cache (fine-grained per-shard lock, no global write lock)
        self.storage.insert((address, key), value);

        Ok(value)
    }

    fn get_block_hash(&self, block_number: u64) -> Result<H256, DatabaseError> {
        // Block hashes don't benefit much from caching here
        // (they're already cached in StoreVmDatabase)
        self.inner.get_block_hash(block_number)
    }

    fn get_chain_config(&self) -> Result<ChainConfig, DatabaseError> {
        // Chain config is constant, no need to cache
        self.inner.get_chain_config()
    }

    fn get_account_code(&self, code_hash: H256) -> Result<Code, DatabaseError> {
        // Check cache first (lock-free read via DashMap)
        if let Some(code) = self.code.get(&code_hash) {
            return Ok(code.clone());
        }

        // Cache miss: query underlying database
        let code = self.inner.get_account_code(code_hash)?;

        // Populate cache (fine-grained per-shard lock, no global write lock)
        self.code.insert(code_hash, code.clone());

        Ok(code)
    }

    fn get_code_metadata(&self, code_hash: H256) -> Result<CodeMetadata, DatabaseError> {
        // Delegate directly to the underlying database.
        // The underlying Store already has its own code_metadata_cache,
        // so we don't need to duplicate caching here.
        self.inner.get_code_metadata(code_hash)
    }
}
