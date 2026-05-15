use crate::{errors::DatabaseError, precompiles::PrecompileCache};
use ethrex_common::{
    Address, H256, U256,
    types::{AccountState, AccountUpdate, BlockHash, BlockNumber, ChainConfig, Code, CodeMetadata},
};
use lru::LruCache;
#[cfg(all(feature = "rayon", not(feature = "eip-8025")))]
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use rustc_hash::{FxBuildHasher, FxHashSet};
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, RwLock};

pub mod gen_db;

// Bounded so the cache survives cross-block reuse without unbounded growth.
#[allow(clippy::unwrap_used)]
const ACCOUNT_CACHE_CAP: NonZeroUsize = NonZeroUsize::new(65_536).unwrap();
#[allow(clippy::unwrap_used)]
const STORAGE_CACHE_CAP: NonZeroUsize = NonZeroUsize::new(262_144).unwrap();
#[allow(clippy::unwrap_used)]
const CODE_CACHE_CAP: NonZeroUsize = NonZeroUsize::new(8_192).unwrap();

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

/// Read-through cache for state lookups, shared across the warming and
/// execution phases of a block. Inner DB is swappable (see [`CrossBlockCache`])
/// so the cache can be reused across blocks; caller MUST ensure a swapped
/// inner reflects the post-state of the most recently promoted block.
///
/// Inspired by reth's overlay/proof worker cache.
pub struct CachingDatabase {
    /// `None` until `set_inner` is called. Reads error loudly in that state.
    inner: RwLock<Option<Arc<dyn Database>>>,
    accounts: RwLock<AccountCache>,
    storage: RwLock<StorageCache>,
    code: RwLock<CodeCache>,
    /// Shared precompile result cache (warmer populates, executor reuses).
    /// `None` when the cache is disabled via `BlockchainOptions::precompile_cache_enabled = false`.
    precompile_cache: Option<PrecompileCache>,
    chain_config: OnceLock<ChainConfig>,
}

impl core::fmt::Debug for CachingDatabase {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CachingDatabase").finish_non_exhaustive()
    }
}

impl CachingDatabase {
    pub fn new(precompile_cache_enabled: bool) -> Self {
        Self {
            inner: RwLock::new(None),
            accounts: RwLock::new(LruCache::with_hasher(ACCOUNT_CACHE_CAP, FxBuildHasher)),
            storage: RwLock::new(LruCache::with_hasher(STORAGE_CACHE_CAP, FxBuildHasher)),
            code: RwLock::new(LruCache::with_hasher(CODE_CACHE_CAP, FxBuildHasher)),
            precompile_cache: precompile_cache_enabled.then(PrecompileCache::new),
            chain_config: OnceLock::new(),
        }
    }
}

impl CachingDatabase {
    /// Set the inner database. When the cache holds entries, the new inner
    /// must be the post-state of the most recently promoted block (or the
    /// cache must be `clear`-ed first).
    pub fn set_inner(&self, inner: Arc<dyn Database>) {
        *self.inner.write().unwrap_or_else(|p| p.into_inner()) = Some(inner);
    }

    fn current_inner(&self) -> Result<Arc<dyn Database>, DatabaseError> {
        self.inner
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
            .cloned()
            .ok_or_else(|| {
                DatabaseError::Custom(
                    "CachingDatabase: inner database not set; call set_inner() before use"
                        .to_string(),
                )
            })
    }

    /// Drop all cached state (account / storage / code). `precompile_cache` and
    /// `chain_config` are intentionally retained: precompile outputs are
    /// input-deterministic and `ChainConfig` is constant for the process.
    pub fn clear(&self) {
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
}

impl Database for CachingDatabase {
    fn get_account_state(&self, address: Address) -> Result<AccountState, DatabaseError> {
        // peek (not get) so a read lock is enough; we don't promote on hit.
        if let Some(state) = self
            .accounts
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .peek(&address)
            .copied()
        {
            return Ok(state);
        }

        let state = self.current_inner()?.get_account_state(address)?;
        self.accounts
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .put(address, state);
        Ok(state)
    }

    fn get_storage_value(&self, address: Address, key: H256) -> Result<U256, DatabaseError> {
        if let Some(value) = self
            .storage
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .peek(&(address, key))
            .copied()
        {
            return Ok(value);
        }

        let value = self.current_inner()?.get_storage_value(address, key)?;
        self.storage
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .put((address, key), value);
        Ok(value)
    }

    fn get_block_hash(&self, block_number: u64) -> Result<H256, DatabaseError> {
        // StoreVmDatabase already caches; no value adding another layer.
        self.current_inner()?.get_block_hash(block_number)
    }

    fn get_chain_config(&self) -> Result<ChainConfig, DatabaseError> {
        if let Some(cfg) = self.chain_config.get() {
            return Ok(*cfg);
        }
        let cfg = self.current_inner()?.get_chain_config()?;
        // Ignore set error: another thread may have raced us with the same value.
        let _ = self.chain_config.set(cfg);
        Ok(cfg)
    }

    fn get_account_code(&self, code_hash: H256) -> Result<Code, DatabaseError> {
        if let Some(code) = self
            .code
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .peek(&code_hash)
            .cloned()
        {
            return Ok(code);
        }

        let code = self.current_inner()?.get_account_code(code_hash)?;
        self.code
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .put(code_hash, code.clone());
        Ok(code)
    }

    fn get_code_metadata(&self, code_hash: H256) -> Result<CodeMetadata, DatabaseError> {
        // Store already has a code_metadata_cache; no value duplicating here.
        self.current_inner()?.get_code_metadata(code_hash)
    }

    fn precompile_cache(&self) -> Option<&PrecompileCache> {
        self.precompile_cache.as_ref()
    }

    #[cfg(all(feature = "rayon", not(feature = "eip-8025")))]
    fn prefetch_accounts(&self, addresses: &[Address]) -> Result<(), DatabaseError> {
        let inner = self.current_inner()?;
        let fetched: Vec<(Address, AccountState)> = addresses
            .par_iter()
            .map(|&addr| inner.get_account_state(addr).map(|s| (addr, s)))
            .collect::<Result<_, _>>()?;
        let mut cache = self.accounts.write().unwrap_or_else(|p| p.into_inner());
        for (addr, state) in fetched {
            if !cache.contains(&addr) {
                cache.put(addr, state);
            }
        }
        Ok(())
    }

    #[cfg(all(feature = "rayon", not(feature = "eip-8025")))]
    fn prefetch_storage(&self, keys: &[(Address, H256)]) -> Result<(), DatabaseError> {
        let inner = self.current_inner()?;
        let fetched: Vec<((Address, H256), U256)> = keys
            .par_iter()
            .map(|&(addr, key)| inner.get_storage_value(addr, key).map(|v| ((addr, key), v)))
            .collect::<Result<_, _>>()?;
        let mut cache = self.storage.write().unwrap_or_else(|p| p.into_inner());
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
/// are deferred to [`BlockSession::promote`], called only after a block has
/// been successfully executed AND stored.
///
/// Concurrency: [`Self::begin_block`] holds `pipeline_lock` for the entire
/// `set_inner → execute → promote` window. Callers that share the cache via
/// `Arc<Blockchain>` (L2 peer actors, full sync vs engine API) are serialized
/// at the session boundary — block execution is inherently sequential against
/// a single chain tip, so this matches the only safe ordering.
#[derive(Debug)]
pub struct CrossBlockCache {
    cache: Arc<CachingDatabase>,
    last_committed: RwLock<Option<(BlockNumber, BlockHash)>>,
    /// Serializes block-pipeline sessions. `Mutex` (not `TokioMutex`) because
    /// the pipeline is sync and the lock must not be held across `.await`.
    pipeline_lock: Mutex<()>,
}

/// Holds the pipeline lock and the caching DB for one block's execution.
/// Call [`Self::promote`] on success; drop the session on failure to release
/// the lock without writing the block's post-state.
pub struct BlockSession<'a> {
    cache: &'a CrossBlockCache,
    _guard: MutexGuard<'a, ()>,
}

impl<'a> BlockSession<'a> {
    /// Shared caching DB handle for warmer + executor threads.
    pub fn as_database(&self) -> Arc<dyn Database> {
        self.cache.cache.clone()
    }

    /// Apply this block's post-state to the cache and release the session.
    pub fn promote(
        self,
        block_number: BlockNumber,
        block_hash: BlockHash,
        account_updates: &[AccountUpdate],
    ) {
        self.cache
            .promote_inner(block_number, block_hash, account_updates);
    }
}

impl CrossBlockCache {
    /// Empty cache with no inner database. Reads error until the first
    /// [`Self::begin_block`] call.
    pub fn empty(precompile_cache_enabled: bool) -> Self {
        Self {
            cache: Arc::new(CachingDatabase::new(precompile_cache_enabled)),
            last_committed: RwLock::new(None),
            pipeline_lock: Mutex::new(()),
        }
    }

    /// Acquire a session for processing one block. Atomically:
    /// 1. Locks the pipeline (serializes against other in-flight blocks).
    /// 2. Invalidates the cache if `last_committed` doesn't match the parent.
    /// 3. Points the cache's inner DB at the parent's state.
    ///
    /// Poison recovery is safe: `last_committed` is only written by
    /// [`BlockSession::promote`], which runs after the block is stored. A
    /// mid-pipeline panic leaves it pointing at the previously committed
    /// block — exactly what the next parent-mismatch check expects.
    pub fn begin_block(
        &self,
        parent_number: BlockNumber,
        parent_hash: BlockHash,
        inner: Arc<dyn Database>,
    ) -> BlockSession<'_> {
        let guard = self
            .pipeline_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        if !self.is_valid_for_parent(parent_number, parent_hash) {
            self.cache.clear();
            *self
                .last_committed
                .write()
                .unwrap_or_else(|p| p.into_inner()) = None;
        }
        self.cache.set_inner(inner);

        BlockSession {
            cache: self,
            _guard: guard,
        }
    }

    fn is_valid_for_parent(&self, parent_number: BlockNumber, parent_hash: BlockHash) -> bool {
        let snapshot = *self
            .last_committed
            .read()
            .unwrap_or_else(|p| p.into_inner());
        snapshot == Some((parent_number, parent_hash))
    }

    /// Apply the post-state of a successfully executed-and-stored block.
    ///
    /// Accounts are evicted (the post-block `storage_root` is not cheaply
    /// available here, and a stale `storage_root` would lie to consumers).
    /// Touched slots are written through; wiped accounts have their cached
    /// slots evicted. Newly deployed code is inserted.
    fn promote_inner(
        &self,
        block_number: BlockNumber,
        block_hash: BlockHash,
        account_updates: &[AccountUpdate],
    ) {
        let mut accounts = self
            .cache
            .accounts
            .write()
            .unwrap_or_else(|p| p.into_inner());
        let mut storage = self
            .cache
            .storage
            .write()
            .unwrap_or_else(|p| p.into_inner());
        let mut code = self.cache.code.write().unwrap_or_else(|p| p.into_inner());

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

            if let Some(c) = &update.code {
                code.put(c.hash, c.clone());
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

        *self
            .last_committed
            .write()
            .unwrap_or_else(|p| p.into_inner()) = Some((block_number, block_hash));
    }
}
