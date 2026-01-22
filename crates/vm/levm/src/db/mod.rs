use crate::errors::DatabaseError;
use ethrex_common::{
    Address, H256, U256,
    types::{AccountState, ChainConfig, Code},
};
use rustc_hash::FxHashMap;
use std::sync::{Arc, RwLock};

pub mod gen_db;

pub trait Database: Send + Sync {
    fn get_account_state(&self, address: Address) -> Result<AccountState, DatabaseError>;
    fn get_storage_value(&self, address: Address, key: H256) -> Result<U256, DatabaseError>;
    fn get_block_hash(&self, block_number: u64) -> Result<H256, DatabaseError>;
    fn get_chain_config(&self) -> Result<ChainConfig, DatabaseError>;
    fn get_account_code(&self, code_hash: H256) -> Result<Code, DatabaseError>;
}

/// Shared cache for parallel pre-warming during block execution.
///
/// This cache is shared across all parallel warming workers and can also be
/// reused by the sequential execution phase. It reduces redundant database/trie
/// lookups when multiple transactions touch the same accounts.
///
/// Thread-safe via RwLock - optimized for read-heavy concurrent access.
#[derive(Debug, Default, Clone)]
pub struct WarmingCache {
    /// Cached account states (balance, nonce, code_hash, storage_root)
    accounts: Arc<RwLock<FxHashMap<Address, AccountState>>>,
    /// Cached storage values
    storage: Arc<RwLock<FxHashMap<(Address, H256), U256>>>,
    /// Cached contract code
    code: Arc<RwLock<FxHashMap<H256, Code>>>,
}

impl WarmingCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Get cached account state
    pub fn get_account(&self, address: &Address) -> Option<AccountState> {
        self.accounts
            .read()
            .expect("poisoned rwlock")
            .get(address)
            .cloned()
    }

    /// Insert account state into cache
    pub fn insert_account(&self, address: Address, state: AccountState) {
        self.accounts
            .write()
            .expect("poisoned rwlock")
            .insert(address, state);
    }

    /// Get cached storage value
    pub fn get_storage(&self, address: &Address, key: &H256) -> Option<U256> {
        self.storage
            .read()
            .expect("poisoned rwlock")
            .get(&(*address, *key))
            .copied()
    }

    /// Insert storage value into cache
    pub fn insert_storage(&self, address: Address, key: H256, value: U256) {
        self.storage
            .write()
            .expect("poisoned rwlock")
            .insert((address, key), value);
    }

    /// Get cached code
    pub fn get_code(&self, code_hash: &H256) -> Option<Code> {
        self.code
            .read()
            .expect("poisoned rwlock")
            .get(code_hash)
            .cloned()
    }

    /// Insert code into cache
    pub fn insert_code(&self, code_hash: H256, code: Code) {
        self.code
            .write()
            .expect("poisoned rwlock")
            .insert(code_hash, code);
    }
}

/// A database wrapper that checks a shared warming cache before falling back
/// to the underlying database. Populates the cache on miss.
///
/// This enables parallel warming workers to share cached data, and allows
/// the sequential execution phase to reuse warmed state.
pub struct CachingDatabase {
    inner: Arc<dyn Database>,
    cache: WarmingCache,
}

impl CachingDatabase {
    pub fn new(inner: Arc<dyn Database>, cache: WarmingCache) -> Self {
        Self { inner, cache }
    }

    /// Get the underlying warming cache (useful for passing to other components)
    pub fn cache(&self) -> &WarmingCache {
        &self.cache
    }
}

impl Database for CachingDatabase {
    fn get_account_state(&self, address: Address) -> Result<AccountState, DatabaseError> {
        // Check cache first
        if let Some(state) = self.cache.get_account(&address) {
            return Ok(state);
        }

        // Cache miss: query underlying database
        let state = self.inner.get_account_state(address)?;

        // Populate cache
        self.cache.insert_account(address, state.clone());

        Ok(state)
    }

    fn get_storage_value(&self, address: Address, key: H256) -> Result<U256, DatabaseError> {
        // Check cache first
        if let Some(value) = self.cache.get_storage(&address, &key) {
            return Ok(value);
        }

        // Cache miss: query underlying database
        let value = self.inner.get_storage_value(address, key)?;

        // Populate cache
        self.cache.insert_storage(address, key, value);

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
        // Check cache first
        if let Some(code) = self.cache.get_code(&code_hash) {
            return Ok(code);
        }

        // Cache miss: query underlying database
        let code = self.inner.get_account_code(code_hash)?;

        // Populate cache
        self.cache.insert_code(code_hash, code.clone());

        Ok(code)
    }
}
