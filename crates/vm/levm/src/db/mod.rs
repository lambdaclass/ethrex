use crate::errors::DatabaseError;
use ethrex_common::{
    Address, H256, U256,
    types::{AccountState, ChainConfig, Code, CodeMetadata},
};
use rustc_hash::{FxHashMap, FxHashSet};
use std::sync::{Arc, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

pub mod gen_db;

// Type aliases for cache storage maps
type AccountCache = FxHashMap<Address, AccountState>;
type StorageCache = FxHashMap<(Address, H256), U256>;
type CodeCache = FxHashMap<H256, Code>;

pub trait Database: Send + Sync {
    fn get_account_state(&self, address: Address) -> Result<AccountState, DatabaseError>;
    fn get_storage_value(&self, address: Address, key: H256) -> Result<U256, DatabaseError>;
    fn get_block_hash(&self, block_number: u64) -> Result<H256, DatabaseError>;
    fn get_chain_config(&self) -> Result<ChainConfig, DatabaseError>;
    fn get_account_code(&self, code_hash: H256) -> Result<Code, DatabaseError>;
    fn get_code_metadata(&self, code_hash: H256) -> Result<CodeMetadata, DatabaseError>;
}

/// A database wrapper that caches state lookups across blocks and for parallel pre-warming.
///
/// This enables parallel warming workers to share cached data, and allows
/// the sequential execution phase to reuse warmed state. Reduces redundant
/// database/trie lookups when multiple transactions touch the same accounts.
///
/// The inner store can be swapped between blocks (via [`update_inner`](Self::update_inner))
/// while preserving cached entries for accounts that weren't modified. After each
/// block, call [`invalidate_modified`](Self::invalidate_modified) to remove stale
/// entries for accounts that changed during execution.
///
/// Thread-safe via RwLock - optimized for read-heavy concurrent access.
///
/// This caching database is inspired by reth's overlay/proof worker cache.
pub struct CachingDatabase {
    inner: RwLock<Arc<dyn Database>>,
    /// Cached account states (balance, nonce, code_hash, storage_root)
    accounts: RwLock<AccountCache>,
    /// Cached storage values
    storage: RwLock<StorageCache>,
    /// Cached contract code
    code: RwLock<CodeCache>,
}

impl CachingDatabase {
    pub fn new(inner: Arc<dyn Database>) -> Self {
        Self {
            inner: RwLock::new(inner),
            accounts: RwLock::new(FxHashMap::default()),
            storage: RwLock::new(FxHashMap::default()),
            code: RwLock::new(FxHashMap::default()),
        }
    }

    /// Update the underlying store while preserving cached data.
    ///
    /// Used for cross-block caching: the inner store changes per block
    /// (different state root) but cached entries for unmodified accounts
    /// remain valid.
    pub fn update_inner(&self, new_inner: Arc<dyn Database>) {
        *self
            .inner
            .write()
            .unwrap_or_else(|e| e.into_inner()) = new_inner;
    }

    /// Remove cached entries for accounts that were modified during block execution.
    /// Also removes all storage entries for those accounts.
    /// Unmodified entries and code (keyed by content hash) remain valid across blocks.
    pub fn invalidate_modified(&self, modified_accounts: &FxHashSet<Address>) {
        if modified_accounts.is_empty() {
            return;
        }
        let mut accounts = self
            .accounts
            .write()
            .unwrap_or_else(|e| e.into_inner());
        for addr in modified_accounts {
            accounts.remove(addr);
        }
        drop(accounts);
        let mut storage = self
            .storage
            .write()
            .unwrap_or_else(|e| e.into_inner());
        storage.retain(|&(addr, _), _| !modified_accounts.contains(&addr));
    }

    /// Clear all cached data. Used when cache correctness cannot be guaranteed,
    /// such as during chain reorganizations.
    pub fn clear(&self) {
        self.accounts
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        self.storage
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        self.code
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }

    fn read_inner(&self) -> Result<RwLockReadGuard<'_, Arc<dyn Database>>, DatabaseError> {
        self.inner.read().map_err(poison_error_to_db_error)
    }

    fn read_accounts(&self) -> Result<RwLockReadGuard<'_, AccountCache>, DatabaseError> {
        self.accounts.read().map_err(poison_error_to_db_error)
    }

    fn write_accounts(&self) -> Result<RwLockWriteGuard<'_, AccountCache>, DatabaseError> {
        self.accounts.write().map_err(poison_error_to_db_error)
    }

    fn read_storage(&self) -> Result<RwLockReadGuard<'_, StorageCache>, DatabaseError> {
        self.storage.read().map_err(poison_error_to_db_error)
    }

    fn write_storage(&self) -> Result<RwLockWriteGuard<'_, StorageCache>, DatabaseError> {
        self.storage.write().map_err(poison_error_to_db_error)
    }

    fn read_code(&self) -> Result<RwLockReadGuard<'_, CodeCache>, DatabaseError> {
        self.code.read().map_err(poison_error_to_db_error)
    }

    fn write_code(&self) -> Result<RwLockWriteGuard<'_, CodeCache>, DatabaseError> {
        self.code.write().map_err(poison_error_to_db_error)
    }
}

impl std::fmt::Debug for CachingDatabase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CachingDatabase")
            .field(
                "accounts_cached",
                &self.accounts.read().map(|a| a.len()).unwrap_or(0),
            )
            .field(
                "storage_cached",
                &self.storage.read().map(|s| s.len()).unwrap_or(0),
            )
            .field(
                "code_cached",
                &self.code.read().map(|c| c.len()).unwrap_or(0),
            )
            .finish()
    }
}

fn poison_error_to_db_error<T>(err: PoisonError<T>) -> DatabaseError {
    DatabaseError::Custom(format!("Cache lock poisoned: {err}"))
}

impl Database for CachingDatabase {
    fn get_account_state(&self, address: Address) -> Result<AccountState, DatabaseError> {
        // Check cache first
        if let Some(state) = self.read_accounts()?.get(&address).copied() {
            return Ok(state);
        }

        // Cache miss: query underlying database
        let inner = self.read_inner()?;
        let state = inner.get_account_state(address)?;
        drop(inner);

        // Populate cache (AccountState is Copy, no clone needed)
        self.write_accounts()?.insert(address, state);

        Ok(state)
    }

    fn get_storage_value(&self, address: Address, key: H256) -> Result<U256, DatabaseError> {
        // Check cache first
        if let Some(value) = self.read_storage()?.get(&(address, key)).copied() {
            return Ok(value);
        }

        // Cache miss: query underlying database
        let inner = self.read_inner()?;
        let value = inner.get_storage_value(address, key)?;
        drop(inner);

        // Populate cache (U256 is Copy, no clone needed)
        self.write_storage()?.insert((address, key), value);

        Ok(value)
    }

    fn get_block_hash(&self, block_number: u64) -> Result<H256, DatabaseError> {
        // Block hashes don't benefit much from caching here
        // (they're already cached in StoreVmDatabase)
        self.read_inner()?.get_block_hash(block_number)
    }

    fn get_chain_config(&self) -> Result<ChainConfig, DatabaseError> {
        // Chain config is constant, no need to cache
        self.read_inner()?.get_chain_config()
    }

    fn get_account_code(&self, code_hash: H256) -> Result<Code, DatabaseError> {
        // Check cache first
        if let Some(code) = self.read_code()?.get(&code_hash).cloned() {
            return Ok(code);
        }

        // Cache miss: query underlying database
        let inner = self.read_inner()?;
        let code = inner.get_account_code(code_hash)?;
        drop(inner);

        // Populate cache (Code contains Bytes which is ref-counted, clone is cheap)
        self.write_code()?.insert(code_hash, code.clone());

        Ok(code)
    }

    fn get_code_metadata(&self, code_hash: H256) -> Result<CodeMetadata, DatabaseError> {
        // Delegate directly to the underlying database.
        // The underlying Store already has its own code_metadata_cache,
        // so we don't need to duplicate caching here.
        self.read_inner()?.get_code_metadata(code_hash)
    }
}
