use crate::{errors::DatabaseError, precompiles::PrecompileCache};
use ethrex_common::{
    Address, H256, U256,
    types::{AccountState, AccountUpdate, BlockHash, BlockNumber, ChainConfig, Code, CodeMetadata},
};
use lru::LruCache;
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use rustc_hash::{FxBuildHasher, FxHashSet};
use std::num::NonZeroUsize;
use std::sync::{Arc, OnceLock, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

pub mod gen_db;

// Bounded so the cache survives cross-block reuse without unbounded growth.
// Caps are first-pass values; revisit once we have benchmarks.
#[allow(clippy::unwrap_used)]
const ACCOUNT_CACHE_CAP: NonZeroUsize = NonZeroUsize::new(256 * 1024).unwrap();
#[allow(clippy::unwrap_used)]
const STORAGE_CACHE_CAP: NonZeroUsize = NonZeroUsize::new(1024 * 1024).unwrap();
#[allow(clippy::unwrap_used)]
const CODE_CACHE_CAP: NonZeroUsize = NonZeroUsize::new(4 * 1024).unwrap();

type AccountCache = LruCache<Address, AccountState, FxBuildHasher>;
type StorageCache = LruCache<(Address, H256), U256, FxBuildHasher>;
type CodeCache = LruCache<H256, Code, FxBuildHasher>;

pub trait Database: Send + Sync {
    fn get_account_state(&self, address: Address) -> Result<AccountState, DatabaseError>;
    fn get_storage_value(&self, address: Address, key: H256) -> Result<U256, DatabaseError>;
    fn get_block_hash(&self, block_number: u64) -> Result<H256, DatabaseError>;
    fn get_chain_config(&self) -> Result<ChainConfig, DatabaseError>;
    fn get_account_code(&self, code_hash: H256) -> Result<Code, DatabaseError>;
    fn get_code_metadata(&self, code_hash: H256) -> Result<CodeMetadata, DatabaseError>;
    /// Access the precompile cache, if available at this database layer.
    fn precompile_cache(&self) -> Option<&PrecompileCache> {
        None
    }
    /// Prefetch a batch of accounts into the cache. Default: sequential fallback.
    fn prefetch_accounts(&self, addresses: &[Address]) -> Result<(), DatabaseError> {
        for &addr in addresses {
            self.get_account_state(addr)?;
        }
        Ok(())
    }
    /// Prefetch a batch of storage slots into the cache. Default: sequential fallback.
    fn prefetch_storage(&self, keys: &[(Address, H256)]) -> Result<(), DatabaseError> {
        for &(addr, key) in keys {
            self.get_storage_value(addr, key)?;
        }
        Ok(())
    }
}

/// A database wrapper that caches state lookups for parallel pre-warming.
///
/// This enables parallel warming workers to share cached data, and allows
/// the sequential execution phase to reuse warmed state. Reduces redundant
/// database/trie lookups when multiple transactions touch the same accounts.
///
/// Thread-safe via RwLock - optimized for read-heavy concurrent access.
///
/// Inner DB is swappable (see [`CrossBlockCache`]) so the cache can be reused
/// across blocks; caller MUST ensure a swapped inner reflects the post-state
/// of the most recently promoted block.
///
/// This caching database is inspired by reth's overlay/proof worker cache.
pub struct CachingDatabase {
    /// `None` until `set_inner` is called. Reads error loudly in that state.
    inner: RwLock<Option<Arc<dyn Database>>>,
    /// Cached account states (balance, nonce, code_hash, storage_root)
    accounts: RwLock<AccountCache>,
    /// Cached storage values
    storage: RwLock<StorageCache>,
    /// Cached contract code
    code: RwLock<CodeCache>,
    /// Shared precompile result cache (warmer populates, executor reuses)
    precompile_cache: PrecompileCache,
    /// Cached chain config (constant for the lifetime of this database)
    chain_config: OnceLock<ChainConfig>,
}

impl Default for CachingDatabase {
    fn default() -> Self {
        Self::new()
    }
}

impl CachingDatabase {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(None),
            accounts: RwLock::new(LruCache::with_hasher(ACCOUNT_CACHE_CAP, FxBuildHasher)),
            storage: RwLock::new(LruCache::with_hasher(STORAGE_CACHE_CAP, FxBuildHasher)),
            code: RwLock::new(LruCache::with_hasher(CODE_CACHE_CAP, FxBuildHasher)),
            precompile_cache: PrecompileCache::new(),
            chain_config: OnceLock::new(),
        }
    }

    /// Set the inner database. When the cache holds entries, the new inner
    /// must be the post-state of the most recently promoted block (or the
    /// cache must be `clear_all`-ed first).
    pub fn set_inner(&self, inner: Arc<dyn Database>) {
        *self.inner.write().unwrap_or_else(|p| p.into_inner()) = Some(inner);
    }

    fn current_inner(&self) -> Result<Arc<dyn Database>, DatabaseError> {
        let guard = self.inner.read().map_err(poison_error_to_db_error)?;
        guard.as_ref().cloned().ok_or_else(|| {
            DatabaseError::Custom(
                "CachingDatabase: inner database not set; call set_inner() before use".to_string(),
            )
        })
    }

    /// Access the shared precompile result cache.
    pub fn precompile_cache(&self) -> &PrecompileCache {
        &self.precompile_cache
    }

    /// Drop all cached state (account / storage / code). `precompile_cache` and
    /// `chain_config` are intentionally retained: precompile outputs are
    /// input-deterministic and `ChainConfig` is constant for the process. Recovers
    /// from poisoned locks: clearing is the right action after a panicking writer.
    pub fn clear_all(&self) {
        self.accounts
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .clear();
        self.storage
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .clear();
        self.code.write().unwrap_or_else(|p| p.into_inner()).clear();
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
        // peek (not get) so a read lock is enough; we don't promote on hit.
        if let Some(state) = self.read_accounts()?.peek(&address).copied() {
            return Ok(state);
        }

        // Cache miss: query underlying database
        let state = self.current_inner()?.get_account_state(address)?;

        // Populate cache (AccountState is Copy, no clone needed)
        self.write_accounts()?.put(address, state);

        Ok(state)
    }

    fn get_storage_value(&self, address: Address, key: H256) -> Result<U256, DatabaseError> {
        if let Some(value) = self.read_storage()?.peek(&(address, key)).copied() {
            return Ok(value);
        }

        // Cache miss: query underlying database
        let value = self.current_inner()?.get_storage_value(address, key)?;

        // Populate cache (U256 is Copy, no clone needed)
        self.write_storage()?.put((address, key), value);

        Ok(value)
    }

    fn get_block_hash(&self, block_number: u64) -> Result<H256, DatabaseError> {
        // Block hashes don't benefit much from caching here
        // (they're already cached in StoreVmDatabase)
        self.current_inner()?.get_block_hash(block_number)
    }

    fn get_chain_config(&self) -> Result<ChainConfig, DatabaseError> {
        if let Some(cfg) = self.chain_config.get() {
            return Ok(*cfg);
        }
        let cfg = self.current_inner()?.get_chain_config()?;
        // Ignore set error: another thread may have raced us; re-read the winner.
        let _ = self.chain_config.set(cfg);
        Ok(*self.chain_config.get().unwrap_or(&cfg))
    }

    fn get_account_code(&self, code_hash: H256) -> Result<Code, DatabaseError> {
        if let Some(code) = self.read_code()?.peek(&code_hash).cloned() {
            return Ok(code);
        }

        // Cache miss: query underlying database
        let code = self.current_inner()?.get_account_code(code_hash)?;

        // Populate cache (Code contains Bytes which is ref-counted, clone is cheap)
        self.write_code()?.put(code_hash, code.clone());

        Ok(code)
    }

    fn get_code_metadata(&self, code_hash: H256) -> Result<CodeMetadata, DatabaseError> {
        // Delegate directly to the underlying database.
        // The underlying Store already has its own code_metadata_cache,
        // so we don't need to duplicate caching here.
        self.current_inner()?.get_code_metadata(code_hash)
    }

    fn precompile_cache(&self) -> Option<&PrecompileCache> {
        Some(&self.precompile_cache)
    }

    fn prefetch_accounts(&self, addresses: &[Address]) -> Result<(), DatabaseError> {
        // Fetch from inner in parallel (no lock contention), then single write-lock to populate cache.
        let inner = self.current_inner()?;
        let fetched: Vec<(Address, AccountState)> = addresses
            .par_iter()
            .map(|&addr| inner.get_account_state(addr).map(|s| (addr, s)))
            .collect::<Result<_, _>>()?;
        let mut cache = self.write_accounts()?;
        for (addr, state) in fetched {
            if !cache.contains(&addr) {
                cache.put(addr, state);
            }
        }
        Ok(())
    }

    fn prefetch_storage(&self, keys: &[(Address, H256)]) -> Result<(), DatabaseError> {
        // Fetch from inner in parallel (no lock contention), then single write-lock to populate cache.
        let inner = self.current_inner()?;
        let fetched: Vec<((Address, H256), U256)> = keys
            .par_iter()
            .map(|&(addr, key)| inner.get_storage_value(addr, key).map(|v| ((addr, key), v)))
            .collect::<Result<_, _>>()?;
        let mut cache = self.write_storage()?;
        for (key, value) in fetched {
            if !cache.contains(&key) {
                cache.put(key, value);
            }
        }
        Ok(())
    }
}

/// Cross-block state cache. Wraps a [`CachingDatabase`] with the lifecycle
/// metadata needed to keep cached state valid across blocks: the next block
/// must extend `last_committed`, otherwise the cache is invalidated. Writes
/// are deferred to [`Self::promote_block`], called only after a block has been
/// successfully executed AND stored.
pub struct CrossBlockCache {
    cache: Arc<CachingDatabase>,
    last_committed: RwLock<Option<(BlockNumber, BlockHash)>>,
}

impl CrossBlockCache {
    /// Empty cache with no inner database. Reads error until
    /// [`Self::set_inner`] is called.
    pub fn unset() -> Self {
        Self {
            cache: Arc::new(CachingDatabase::new()),
            last_committed: RwLock::new(None),
        }
    }

    /// Inner cache as `Arc<dyn Database>` for use by execution paths.
    /// Shared handle: warmer and executor see each other's writes.
    pub fn as_database(&self) -> Arc<dyn Database> {
        self.cache.clone()
    }

    /// Replace the inner database. See [`CachingDatabase::set_inner`] for the
    /// caller invariant.
    pub fn set_inner(&self, inner: Arc<dyn Database>) {
        self.cache.set_inner(inner);
    }

    /// True if `last_committed` matches the supplied parent. Used to detect
    /// reorgs / sibling blocks before reusing cached state for a new block.
    pub fn is_valid_for_parent(&self, parent_number: BlockNumber, parent_hash: BlockHash) -> bool {
        let snapshot = *self
            .last_committed
            .read()
            .unwrap_or_else(|p| p.into_inner());
        snapshot == Some((parent_number, parent_hash))
    }

    /// Drop all cached state and reset `last_committed`.
    pub fn invalidate(&self) {
        self.cache.clear_all();
        *self
            .last_committed
            .write()
            .unwrap_or_else(|p| p.into_inner()) = None;
    }

    /// Apply the post-state of a successfully executed-and-stored block.
    ///
    /// Accounts are evicted (the post-block `storage_root` is not cheaply
    /// available here, and a stale `storage_root` would lie to consumers).
    /// Touched slots are written through; wiped accounts have their cached
    /// slots evicted. Newly deployed code is inserted.
    pub fn promote_block(
        &self,
        block_number: BlockNumber,
        block_hash: BlockHash,
        account_updates: &[AccountUpdate],
    ) -> Result<(), DatabaseError> {
        let mut accounts = self.cache.write_accounts()?;
        let mut storage = self.cache.write_storage()?;
        let mut code = self.cache.write_code()?;

        // Wipe path is cold on Cancun+ (EIP-6780 prevents cross-tx storage
        // wipes); kept for pre-Cancun replay and L2s on older forks.
        let mut storage_wiped: FxHashSet<Address> = FxHashSet::default();

        for update in account_updates {
            accounts.pop(&update.address);

            if update.removed || update.removed_storage {
                storage_wiped.insert(update.address);
                continue;
            }

            for (slot, value) in &update.added_storage {
                storage.put((update.address, *slot), *value);
            }

            if let (Some(info), Some(c)) = (&update.info, &update.code) {
                code.put(info.code_hash, c.clone());
            }
        }

        if !storage_wiped.is_empty() {
            // LruCache lacks `retain`; collect-then-pop is unavoidable.
            let to_remove: Vec<(Address, H256)> = storage
                .iter()
                .filter_map(|(k, _)| storage_wiped.contains(&k.0).then_some(*k))
                .collect();
            for k in &to_remove {
                storage.pop(k);
            }
        }

        drop(accounts);
        drop(storage);
        drop(code);

        *self
            .last_committed
            .write()
            .unwrap_or_else(|p| p.into_inner()) = Some((block_number, block_hash));
        Ok(())
    }
}
