use crate::errors::DatabaseError;
use ethrex_common::{
    Address, H256, U256,
    types::{AccountState, ChainConfig, Code, CodeMetadata},
};
use gen_db::GeneralizedDatabase;
use rustc_hash::FxHashMap;
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

/// A database wrapper that caches state lookups for parallel pre-warming.
///
/// This enables parallel warming workers to share cached data, and allows
/// the sequential execution phase to reuse warmed state. Reduces redundant
/// database/trie lookups when multiple transactions touch the same accounts.
///
/// Thread-safe via RwLock - optimized for read-heavy concurrent access.
///
/// This caching database is inspired by reth's overlay/proof worker cache.
pub struct CachingDatabase {
    inner: Arc<dyn Database>,
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
            inner,
            accounts: RwLock::new(FxHashMap::default()),
            storage: RwLock::new(FxHashMap::default()),
            code: RwLock::new(FxHashMap::default()),
        }
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
        let state = self.inner.get_account_state(address)?;

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
        let value = self.inner.get_storage_value(address, key)?;

        // Populate cache (U256 is Copy, no clone needed)
        self.write_storage()?.insert((address, key), value);

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
        if let Some(code) = self.read_code()?.get(&code_hash).cloned() {
            return Ok(code);
        }

        // Cache miss: query underlying database
        let code = self.inner.get_account_code(code_hash)?;

        // Populate cache (Code contains Bytes which is ref-counted, clone is cheap)
        self.write_code()?.insert(code_hash, code.clone());

        Ok(code)
    }

    fn get_code_metadata(&self, code_hash: H256) -> Result<CodeMetadata, DatabaseError> {
        // Delegate directly to the underlying database.
        // The underlying Store already has its own code_metadata_cache,
        // so we don't need to duplicate caching here.
        self.inner.get_code_metadata(code_hash)
    }
}

/// A database layer that shares execution state across parallel prewarm workers.
///
/// Sits between per-thread `GeneralizedDatabase` instances and the `CachingDatabase`.
/// After each transaction, the executing thread merges its modified accounts/storage
/// into this shared layer. Other threads then see those changes on subsequent loads,
/// reducing reverts caused by cross-group state dependencies (e.g. multiple senders
/// interacting with the same contract).
///
/// Not fully correct (race conditions between concurrent reads and writes are possible),
/// but good enough to significantly reduce the revert rate during prewarming.
pub struct SharedPrewarmDB {
    inner: Arc<dyn Database>,
    /// Account state from prewarm execution, updated after each tx
    accounts: RwLock<AccountCache>,
    /// Storage values from prewarm execution
    storage: RwLock<StorageCache>,
    /// Contract code from prewarm execution
    code: RwLock<CodeCache>,
}

impl SharedPrewarmDB {
    pub fn new(inner: Arc<dyn Database>) -> Self {
        Self {
            inner,
            accounts: RwLock::new(FxHashMap::default()),
            storage: RwLock::new(FxHashMap::default()),
            code: RwLock::new(FxHashMap::default()),
        }
    }

    /// Merges modified accounts from a `GeneralizedDatabase` into the shared state.
    /// Called after each transaction execution in a prewarm thread.
    pub fn merge_from(&self, db: &GeneralizedDatabase) {
        // Use unwrap_or_else to recover from poisoned locks — prewarm is best-effort.
        let mut accounts = self.accounts.write().unwrap_or_else(|e| e.into_inner());
        let mut storage = self.storage.write().unwrap_or_else(|e| e.into_inner());

        for (addr, account) in &db.current_accounts_state {
            if account.is_unmodified() {
                continue;
            }

            // Preserve original storage_root from prior loads if available
            let storage_root = accounts
                .get(addr)
                .map(|s| s.storage_root)
                .unwrap_or_default();

            accounts.insert(
                *addr,
                AccountState {
                    nonce: account.info.nonce,
                    balance: account.info.balance,
                    code_hash: account.info.code_hash,
                    storage_root,
                },
            );

            for (key, value) in &account.storage {
                storage.insert((*addr, *key), *value);
            }
        }

        let mut codes = self.code.write().unwrap_or_else(|e| e.into_inner());
        for (hash, code) in &db.codes {
            codes.entry(*hash).or_insert_with(|| code.clone());
        }
    }

    fn read_accounts(&self) -> Result<RwLockReadGuard<'_, AccountCache>, DatabaseError> {
        self.accounts.read().map_err(poison_error_to_db_error)
    }

    fn read_storage(&self) -> Result<RwLockReadGuard<'_, StorageCache>, DatabaseError> {
        self.storage.read().map_err(poison_error_to_db_error)
    }

    fn read_code(&self) -> Result<RwLockReadGuard<'_, CodeCache>, DatabaseError> {
        self.code.read().map_err(poison_error_to_db_error)
    }
}

impl Database for SharedPrewarmDB {
    fn get_account_state(&self, address: Address) -> Result<AccountState, DatabaseError> {
        if let Some(state) = self.read_accounts()?.get(&address).copied() {
            return Ok(state);
        }
        self.inner.get_account_state(address)
    }

    fn get_storage_value(&self, address: Address, key: H256) -> Result<U256, DatabaseError> {
        if let Some(value) = self.read_storage()?.get(&(address, key)).copied() {
            return Ok(value);
        }
        self.inner.get_storage_value(address, key)
    }

    fn get_block_hash(&self, block_number: u64) -> Result<H256, DatabaseError> {
        self.inner.get_block_hash(block_number)
    }

    fn get_chain_config(&self) -> Result<ChainConfig, DatabaseError> {
        self.inner.get_chain_config()
    }

    fn get_account_code(&self, code_hash: H256) -> Result<Code, DatabaseError> {
        if let Some(code) = self.read_code()?.get(&code_hash).cloned() {
            return Ok(code);
        }
        self.inner.get_account_code(code_hash)
    }

    fn get_code_metadata(&self, code_hash: H256) -> Result<CodeMetadata, DatabaseError> {
        self.inner.get_code_metadata(code_hash)
    }
}
