use crate::{errors::DatabaseError, precompiles::PrecompileCache};
use ethrex_common::{
    Address, H256, U256,
    constants::EMPTY_TRIE_HASH,
    types::{AccountState, ChainConfig, Code, CodeMetadata},
};
#[cfg(all(feature = "rayon", not(feature = "eip-8025")))]
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use rustc_hash::FxHashMap;
use std::sync::{Arc, OnceLock, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

pub mod gen_db;

// Type aliases for cache storage maps
type AccountCache = FxHashMap<Address, AccountState>;
type StorageCache = FxHashMap<(Address, H256), U256>;
type CodeCache = FxHashMap<H256, Code>;
/// Touched-key snapshot returned by [`CachingDatabase::touched_keys_where`].
pub struct TouchedKeys {
    /// Touched accounts with their storage roots.
    ///
    /// A *merkle-patricia* storage root, and only meaningful as one. Past the
    /// EIP-8297 activation there are no per-account storage roots and this is
    /// `EMPTY_TRIE_HASH` for every account — which must not be read as "the
    /// account has no storage"; that question is
    /// `ethrex_vm::VmDatabase::has_storage`. The sole consumer,
    /// `ethrex_blockchain::prewarm::warm_merkle_paths`, is MPT-only by nature
    /// and refuses to run on an active chain for exactly this reason.
    pub accounts: Vec<(Address, H256)>,
    /// Touched storage slots as `(account address, slot key)`.
    pub slots: Vec<(Address, H256)>,
}

pub trait Database: Send + Sync {
    fn get_account_state(&self, address: Address) -> Result<AccountState, DatabaseError>;
    fn get_storage_value(&self, address: Address, key: H256) -> Result<U256, DatabaseError>;
    fn get_block_hash(&self, block_number: u64) -> Result<H256, DatabaseError>;
    fn get_chain_config(&self) -> Result<ChainConfig, DatabaseError>;
    fn get_account_code(&self, code_hash: H256) -> Result<Code, DatabaseError>;
    fn get_code_metadata(&self, code_hash: H256) -> Result<CodeMetadata, DatabaseError>;
    /// Whether `address` holds any storage at all. `false` for an account that
    /// is not there.
    ///
    /// # Why this is a method and not `storage_root != EMPTY_TRIE_HASH`
    ///
    /// Because on the EIP-8297 binary trie there is no per-account storage
    /// root to compare — storage there is leaves of one unified tree, not a
    /// subtrie per account — so a binary-backed read reports the empty root for
    /// every account and the comparison answers "no storage" for the whole
    /// chain. `ethrex_vm::VmDatabase::has_storage` documents this at length.
    ///
    /// # The default is right for MPT-shaped backends only
    ///
    /// It derives the answer from `storage_root`, which is correct for any
    /// backend that stores MPT accounts — including every test double in this
    /// workspace, which is why the default exists at all rather than being
    /// boilerplate on two dozen impls.
    ///
    /// **A layer that wraps another `Database` must override it and forward.**
    /// Inheriting the default there is not merely a missed optimisation: a
    /// wrapper's own `AccountState` (cached or logged) is MPT-shaped whatever
    /// the backend underneath it is, so deriving from it discards the real
    /// answer the backend had. `CachingDatabase` is the live example —
    /// `a_storage_only_account_exists_and_collides_on_both_paths` reads through
    /// it specifically to catch that.
    fn has_storage(&self, address: Address) -> Result<bool, DatabaseError> {
        Ok(self.get_account_state(address)?.storage_root != *EMPTY_TRIE_HASH)
    }

    /// Access the precompile cache, if available at this database layer.
    fn precompile_cache(&self) -> Option<&PrecompileCache> {
        None
    }
    /// Batch lookup. Default: loop. Backends with a batched read path (e.g. rocksdb
    /// `multi_get_cf` on the flat key-value table) should override this and the
    /// caching layer above will dispatch to it.
    fn get_account_states_batch(
        &self,
        addresses: &[Address],
    ) -> Result<Vec<AccountState>, DatabaseError> {
        addresses
            .iter()
            .map(|a| self.get_account_state(*a))
            .collect()
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
/// This caching database is inspired by reth's overlay/proof worker cache.
///
/// Besides the per-block warmer/executor sharing above, the mempool
/// prewarmer builds one instance per slot and publishes it across the block
/// boundary: `execute_block_pipeline` seeds the *next* block's execution
/// with it when the parent state and fork match (see
/// `ethrex-blockchain::prewarm`).
///
/// # Invariant
///
/// Because one instance is shared across the block boundary (and the
/// prewarmer may still be filling it while the next block executes), every
/// cached entry must be a pure function of the parent state root. A cache
/// layer whose entries also depend on the executing block (fork, number,
/// timestamp, ...) needs a matching handoff guard in
/// `execute_block_pipeline` — see `precompile_cache`, whose fork-dependent
/// entries are covered by the fork-equality check there.
pub struct CachingDatabase {
    inner: Arc<dyn Database>,
    /// Cached account states (balance, nonce, code_hash, storage_root)
    accounts: RwLock<AccountCache>,
    /// Cached "does this account hold storage" answers.
    ///
    /// Kept apart from `accounts` rather than folded into its value because the
    /// two are filled by different reads: `prefetch_accounts` warms `accounts`
    /// through the backend's batch path, which has no batched form of this
    /// question, so a combined entry would have to invent a placeholder for the
    /// flag and could then be mistaken for a real answer. A separate map is
    /// simply absent until asked.
    ///
    /// Satisfies the cross-block-boundary invariant above without a handoff
    /// guard: whether an account holds storage is a function of the parent
    /// state root alone, exactly as `accounts` is, and depends on nothing about
    /// the block being executed.
    has_storage: RwLock<FxHashMap<Address, bool>>,
    /// Cached storage values
    storage: RwLock<StorageCache>,
    /// Cached contract code
    code: RwLock<CodeCache>,
    /// Shared precompile result cache (warmer populates, executor reuses).
    /// `None` when the cache is disabled via `BlockchainOptions::precompile_cache_enabled = false`.
    precompile_cache: Option<PrecompileCache>,
    /// Cached chain config (constant for the lifetime of this database)
    chain_config: OnceLock<ChainConfig>,
}

impl CachingDatabase {
    pub fn new(inner: Arc<dyn Database>, precompile_cache_enabled: bool) -> Self {
        Self {
            inner,
            accounts: RwLock::new(FxHashMap::default()),
            has_storage: RwLock::new(FxHashMap::default()),
            storage: RwLock::new(FxHashMap::default()),
            code: RwLock::new(FxHashMap::default()),
            precompile_cache: precompile_cache_enabled.then(PrecompileCache::new),
            chain_config: OnceLock::new(),
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

    /// Snapshot of the touched key sets matching the given filters: cached
    /// accounts (with their storage roots) and cached storage slot keys. The
    /// filters let a caller that tracks already-processed keys collect only
    /// the delta, keeping the per-call allocation O(new) while the scan
    /// stays O(cache).
    pub fn touched_keys_where(
        &self,
        account_filter: &dyn Fn(&Address) -> bool,
        slot_filter: &dyn Fn(&(Address, H256)) -> bool,
    ) -> TouchedKeys {
        let accounts = self
            .accounts
            .read()
            .map(|a| {
                a.iter()
                    .filter(|(addr, _)| account_filter(addr))
                    .map(|(addr, st)| (*addr, st.storage_root))
                    .collect()
            })
            .unwrap_or_default();
        let storage = self
            .storage
            .read()
            .map(|s| s.keys().filter(|k| slot_filter(k)).copied().collect())
            .unwrap_or_default();
        TouchedKeys {
            accounts,
            slots: storage,
        }
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

    /// Forwarded to the backend, never derived from the cached `AccountState`.
    ///
    /// The cached state is MPT-shaped whatever the backend is: on a binary-trie
    /// chain its `storage_root` is `EMPTY_TRIE_HASH` for every account, so the
    /// trait's default would answer "no storage" for the whole chain past the
    /// activation, silently turning off EIP-7610 and the destroyed-account
    /// storage wipe. Only the backend knows.
    fn has_storage(&self, address: Address) -> Result<bool, DatabaseError> {
        if let Some(answer) = self
            .has_storage
            .read()
            .map_err(poison_error_to_db_error)?
            .get(&address)
            .copied()
        {
            return Ok(answer);
        }

        let answer = self.inner.has_storage(address)?;

        self.has_storage
            .write()
            .map_err(poison_error_to_db_error)?
            .insert(address, answer);

        Ok(answer)
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
        if let Some(cfg) = self.chain_config.get() {
            return Ok(*cfg);
        }
        let cfg = self.inner.get_chain_config()?;
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

    fn precompile_cache(&self) -> Option<&PrecompileCache> {
        self.precompile_cache.as_ref()
    }

    fn prefetch_accounts(&self, addresses: &[Address]) -> Result<(), DatabaseError> {
        // Filter out already-cached addresses before issuing the batch read.
        let missing: Vec<Address> = {
            let cache = self.read_accounts()?;
            addresses
                .iter()
                .copied()
                .filter(|a| !cache.contains_key(a))
                .collect()
        };
        if missing.is_empty() {
            return Ok(());
        }
        // Dispatch to inner's batch path. For the rocksdb-backed StoreVmDatabase
        // this collapses into a single multi_get on ACCOUNT_FLATKEYVALUE for the
        // FKV-covered subset; default impl loops for other backends.
        let states = self.inner.get_account_states_batch(&missing)?;
        let mut cache = self.write_accounts()?;
        for (addr, state) in missing.into_iter().zip(states.into_iter()) {
            cache.entry(addr).or_insert(state);
        }
        Ok(())
    }

    #[cfg(all(feature = "rayon", not(feature = "eip-8025")))]
    fn prefetch_storage(&self, keys: &[(Address, H256)]) -> Result<(), DatabaseError> {
        // Fetch from inner in parallel (no lock contention), then single write-lock to populate cache.
        let fetched: Vec<((Address, H256), U256)> = keys
            .par_iter()
            .map(|&(addr, key)| {
                self.inner
                    .get_storage_value(addr, key)
                    .map(|v| ((addr, key), v))
            })
            .collect::<Result<_, _>>()?;
        let mut cache = self.write_storage()?;
        for (key, value) in fetched {
            cache.entry(key).or_insert(value);
        }
        Ok(())
    }
}
