use crate::{errors::DatabaseError, precompiles::PrecompileCache};
use ethrex_common::{
    Address, H256, U256,
    types::{AccountState, AccountUpdate, BlockHash, BlockNumber, ChainConfig, Code, CodeMetadata},
};
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use rustc_hash::{FxHashMap, FxHashSet};
use std::sync::{
    Arc, OnceLock, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard,
    atomic::{AtomicBool, Ordering},
};

pub mod gen_db;

// Type aliases for cache storage maps.
//
// TODO: investigate replacing FxHashMap with `schnellru::LruMap` for bounded
// eviction (storage especially) once we have benchmarks. See plan in
// `track-perf-commit/cross-block-caches/plan.md` (Sizing section).
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
/// The inner database is swappable so that the cache itself can survive across
/// blocks (see [`CrossBlockCache`]). Reads consult the cache first; on a miss,
/// the current `inner` is queried and the value is populated. The caller MUST
/// ensure that whenever `inner` is swapped, it points at a database whose state
/// matches what the cache currently holds (i.e. the post-state of the most
/// recently promoted block).
///
/// This caching database is inspired by reth's overlay/proof worker cache.
pub struct CachingDatabase {
    /// Underlying database for cache misses. Swappable via [`Self::set_inner`]
    /// so the cache can be reused across blocks while the read-through target
    /// follows the current parent state.
    inner: RwLock<Arc<dyn Database>>,
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

impl CachingDatabase {
    pub fn new(inner: Arc<dyn Database>) -> Self {
        Self {
            inner: RwLock::new(inner),
            accounts: RwLock::new(FxHashMap::default()),
            storage: RwLock::new(FxHashMap::default()),
            code: RwLock::new(FxHashMap::default()),
            precompile_cache: PrecompileCache::new(),
            chain_config: OnceLock::new(),
        }
    }

    /// Replace the inner database. Must be called when no other thread is reading
    /// through this cache (e.g. between blocks, before warmer/executor threads
    /// are spawned).
    pub fn set_inner(&self, inner: Arc<dyn Database>) -> Result<(), DatabaseError> {
        *self.inner.write().map_err(poison_error_to_db_error)? = inner;
        Ok(())
    }

    /// Snapshot the current inner database. Returned `Arc` is independent of
    /// future `set_inner` calls.
    fn current_inner(&self) -> Result<Arc<dyn Database>, DatabaseError> {
        self.inner
            .read()
            .map_err(poison_error_to_db_error)
            .map(|guard| guard.clone())
    }

    /// Access the shared precompile result cache.
    pub fn precompile_cache(&self) -> &PrecompileCache {
        &self.precompile_cache
    }

    /// Drop all cached state. Used by [`CrossBlockCache::invalidate`] on reorg
    /// or parent mismatch. Recovers from poisoned locks since clearing is the
    /// right action when a previous mutator panicked.
    pub fn clear_all(&self) {
        if let Ok(mut a) = self.accounts.write() {
            a.clear();
        }
        if let Ok(mut s) = self.storage.write() {
            s.clear();
        }
        if let Ok(mut c) = self.code.write() {
            c.clear();
        }
    }

    fn read_accounts(&self) -> Result<RwLockReadGuard<'_, AccountCache>, DatabaseError> {
        self.accounts.read().map_err(poison_error_to_db_error)
    }

    pub(crate) fn write_accounts(
        &self,
    ) -> Result<RwLockWriteGuard<'_, AccountCache>, DatabaseError> {
        self.accounts.write().map_err(poison_error_to_db_error)
    }

    fn read_storage(&self) -> Result<RwLockReadGuard<'_, StorageCache>, DatabaseError> {
        self.storage.read().map_err(poison_error_to_db_error)
    }

    pub(crate) fn write_storage(
        &self,
    ) -> Result<RwLockWriteGuard<'_, StorageCache>, DatabaseError> {
        self.storage.write().map_err(poison_error_to_db_error)
    }

    fn read_code(&self) -> Result<RwLockReadGuard<'_, CodeCache>, DatabaseError> {
        self.code.read().map_err(poison_error_to_db_error)
    }

    pub(crate) fn write_code(&self) -> Result<RwLockWriteGuard<'_, CodeCache>, DatabaseError> {
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
        let state = self.current_inner()?.get_account_state(address)?;

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
        let value = self.current_inner()?.get_storage_value(address, key)?;

        // Populate cache (U256 is Copy, no clone needed)
        self.write_storage()?.insert((address, key), value);

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
        // Check cache first
        if let Some(code) = self.read_code()?.get(&code_hash).cloned() {
            return Ok(code);
        }

        // Cache miss: query underlying database
        let code = self.current_inner()?.get_account_code(code_hash)?;

        // Populate cache (Code contains Bytes which is ref-counted, clone is cheap)
        self.write_code()?.insert(code_hash, code.clone());

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
            cache.entry(addr).or_insert(state);
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
            cache.entry(key).or_insert(value);
        }
        Ok(())
    }
}

/// Placeholder database used when a [`CrossBlockCache`] is constructed before
/// the first block is processed. Any read errors out — `set_inner` MUST be
/// called before any execution path consults the cache.
struct UnsetInner;

impl Database for UnsetInner {
    fn get_account_state(&self, _: Address) -> Result<AccountState, DatabaseError> {
        Err(unset_inner_error())
    }
    fn get_storage_value(&self, _: Address, _: H256) -> Result<U256, DatabaseError> {
        Err(unset_inner_error())
    }
    fn get_block_hash(&self, _: u64) -> Result<H256, DatabaseError> {
        Err(unset_inner_error())
    }
    fn get_chain_config(&self) -> Result<ChainConfig, DatabaseError> {
        Err(unset_inner_error())
    }
    fn get_account_code(&self, _: H256) -> Result<Code, DatabaseError> {
        Err(unset_inner_error())
    }
    fn get_code_metadata(&self, _: H256) -> Result<CodeMetadata, DatabaseError> {
        Err(unset_inner_error())
    }
}

fn unset_inner_error() -> DatabaseError {
    DatabaseError::Custom(
        "CrossBlockCache: inner database not set; call set_inner() before use".to_string(),
    )
}

/// Cross-block state cache.
///
/// Wraps a [`CachingDatabase`] and adds the lifecycle metadata needed to keep
/// the cache valid across multiple block executions:
///
/// - `last_committed` records the `(number, hash)` of the most recent block
///   whose post-state was promoted into the cache. The next block must extend
///   this one (i.e. its `parent_hash == last_committed.hash`); otherwise the
///   cache is invalidated.
/// - `legacy_clear` is a hint set during execution of pre-EIP-6780 SELFDESTRUCT
///   blocks. The promotion path uses it to wipe the storage cache wholesale
///   when a legacy storage clear happened (rare; pre-Cancun only).
///
/// Reads delegate to the inner [`CachingDatabase`]; writes are deferred to
/// `promote_block`, which is only called after the block has been successfully
/// executed AND stored. If anything fails between execution and `store_block`,
/// `promote_block` is not invoked, so the cache is never poisoned with state
/// from an invalid block.
///
/// See `track-perf-commit/cross-block-caches/plan.md` and Nethermind PR #10959
/// for the upstream design we're translating.
pub struct CrossBlockCache {
    cache: Arc<CachingDatabase>,
    /// `(parent_number, parent_hash)` whose post-state matches what's cached.
    /// `None` until the first successful promotion (or after invalidation).
    last_committed: RwLock<Option<(BlockNumber, BlockHash)>>,
    /// Hint set during execution of a block containing a pre-EIP-6780
    /// SELFDESTRUCT. Reset by `promote_block` / `invalidate`.
    legacy_clear: AtomicBool,
}

impl CrossBlockCache {
    pub fn new(inner: Arc<dyn Database>) -> Self {
        Self {
            cache: Arc::new(CachingDatabase::new(inner)),
            last_committed: RwLock::new(None),
            legacy_clear: AtomicBool::new(false),
        }
    }

    /// Construct without an initial backing database. The cache is empty and
    /// any read will fail loudly until [`Self::set_inner`] is called. Used so
    /// `Blockchain` can hold an `Arc<CrossBlockCache>` field that's set up at
    /// the start of every block.
    pub fn unset() -> Self {
        Self::new(Arc::new(UnsetInner))
    }

    /// Construct from an existing `CachingDatabase`. Used when the inner cache
    /// is shared with other consumers (e.g. warming threads that take an
    /// `Arc<dyn Database>`).
    pub fn from_cache(cache: Arc<CachingDatabase>) -> Self {
        Self {
            cache,
            last_committed: RwLock::new(None),
            legacy_clear: AtomicBool::new(false),
        }
    }

    /// The wrapped [`CachingDatabase`]. Cloning the returned `Arc` is cheap.
    pub fn cache(&self) -> Arc<CachingDatabase> {
        self.cache.clone()
    }

    /// Replace the underlying database (typically with the StoreVmDatabase for
    /// the new block's parent). Caller must ensure no other thread is reading
    /// through this cache during the swap.
    pub fn set_inner(&self, inner: Arc<dyn Database>) -> Result<(), DatabaseError> {
        self.cache.set_inner(inner)
    }

    /// True if the cache's `last_committed` matches the supplied parent
    /// `(number, hash)`. Used to detect reorgs / sibling-block scenarios.
    pub fn is_valid_for_parent(&self, parent_number: BlockNumber, parent_hash: BlockHash) -> bool {
        let snapshot = self
            .last_committed
            .read()
            .map(|g| *g)
            .unwrap_or_else(|p| *p.into_inner());
        match snapshot {
            Some((n, h)) => n == parent_number && h == parent_hash,
            None => false,
        }
    }

    /// Drop all cached state, reset metadata. Safe to call at any time.
    pub fn invalidate(&self) {
        self.cache.clear_all();
        self.legacy_clear.store(false, Ordering::Relaxed);
        if let Ok(mut g) = self.last_committed.write() {
            *g = None;
        }
    }

    /// Mark that the current in-progress block contains a pre-EIP-6780
    /// SELFDESTRUCT. Called by the execution path so the next `promote_block`
    /// knows it has to wipe storage. The flag is cleared on `promote_block` /
    /// `invalidate`.
    pub fn mark_legacy_clear(&self) {
        self.legacy_clear.store(true, Ordering::Relaxed);
    }

    /// Apply the post-state of a successfully executed-and-stored block.
    ///
    /// Promotion strategy:
    /// - **Accounts**: invalidated (removed from cache). Next read repopulates
    ///   from the new parent state. We do this because the post-block
    ///   `storage_root` is not cheaply available here, and inserting an account
    ///   with a stale `storage_root` would lie to consumers who read it.
    /// - **Storage**: the new value for every touched `(address, slot)` is
    ///   written through. Slots for fully-removed accounts (or accounts whose
    ///   storage trie was wiped) are evicted.
    /// - **Code**: every newly deployed code is inserted.
    ///
    /// `last_committed` is set to `(block_number, block_hash)` on success.
    pub fn promote_block(
        &self,
        block_number: BlockNumber,
        block_hash: BlockHash,
        account_updates: &[AccountUpdate],
    ) -> Result<(), DatabaseError> {
        let legacy_clear = self.legacy_clear.swap(false, Ordering::Relaxed);

        let mut accounts = self.cache.write_accounts()?;
        let mut storage = self.cache.write_storage()?;
        let mut code = self.cache.write_code()?;

        if legacy_clear {
            // Pre-EIP-6780 SELFDESTRUCT can wipe storage in a way the
            // per-account update list doesn't cleanly express. Be safe: drop
            // every cached storage slot. (This branch is rare — pre-Cancun
            // only.)
            storage.clear();
        }

        // Collect addresses whose storage was wholesale wiped this block so we
        // can evict their cached slots in one pass.
        let mut storage_wiped: FxHashSet<Address> = FxHashSet::default();

        for update in account_updates {
            // Always drop the cached account: we don't know the post-state
            // `storage_root` cheaply. The next read refetches from the parent
            // state of the next block, which will be this block's post-state.
            accounts.remove(&update.address);

            if update.removed {
                storage_wiped.insert(update.address);
                continue;
            }

            if update.removed_storage {
                storage_wiped.insert(update.address);
            }

            // Write through every touched slot (covers both updates and
            // post-EIP-6780 SELFDESTRUCT-then-recreate within a block).
            for (slot, value) in &update.added_storage {
                storage.insert((update.address, *slot), *value);
            }

            if let (Some(info), Some(c)) = (&update.info, &update.code) {
                code.insert(info.code_hash, c.clone());
            }
        }

        if !storage_wiped.is_empty() {
            storage.retain(|(addr, _), _| !storage_wiped.contains(addr));
        }

        drop(accounts);
        drop(storage);
        drop(code);

        match self.last_committed.write() {
            Ok(mut g) => *g = Some((block_number, block_hash)),
            Err(p) => {
                // Recover from poison: a previous panic in a mutator left the
                // lock poisoned, but the value behind it is safe to overwrite
                // with the freshly-promoted block.
                let mut g = p.into_inner();
                *g = Some((block_number, block_hash));
            }
        }
        Ok(())
    }
}

impl Database for CrossBlockCache {
    fn get_account_state(&self, address: Address) -> Result<AccountState, DatabaseError> {
        self.cache.get_account_state(address)
    }

    fn get_storage_value(&self, address: Address, key: H256) -> Result<U256, DatabaseError> {
        self.cache.get_storage_value(address, key)
    }

    fn get_block_hash(&self, block_number: u64) -> Result<H256, DatabaseError> {
        self.cache.get_block_hash(block_number)
    }

    fn get_chain_config(&self) -> Result<ChainConfig, DatabaseError> {
        self.cache.get_chain_config()
    }

    fn get_account_code(&self, code_hash: H256) -> Result<Code, DatabaseError> {
        self.cache.get_account_code(code_hash)
    }

    fn get_code_metadata(&self, code_hash: H256) -> Result<CodeMetadata, DatabaseError> {
        self.cache.get_code_metadata(code_hash)
    }

    fn precompile_cache(&self) -> Option<&PrecompileCache> {
        Some(self.cache.precompile_cache())
    }

    fn prefetch_accounts(&self, addresses: &[Address]) -> Result<(), DatabaseError> {
        self.cache.prefetch_accounts(addresses)
    }

    fn prefetch_storage(&self, keys: &[(Address, H256)]) -> Result<(), DatabaseError> {
        self.cache.prefetch_storage(keys)
    }
}
