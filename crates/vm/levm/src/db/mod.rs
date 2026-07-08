use crate::{errors::DatabaseError, precompiles::PrecompileCache};
use ethrex_common::{
    Address, H256, U256,
    types::{AccountState, ChainConfig, Code, CodeMetadata},
};
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
    /// Batch storage-slot lookup. Default: loop. Backends with a batched read
    /// path (e.g. rocksdb `multi_get_cf` on the storage flat key-value table)
    /// should override this and the caching layer above will dispatch to it.
    fn get_storage_values_batch(
        &self,
        keys: &[(Address, H256)],
    ) -> Result<Vec<U256>, DatabaseError> {
        keys.iter()
            .map(|&(addr, key)| self.get_storage_value(addr, key))
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

    /// Per-slot parallel point-gets, in `missing` order. Warm-optimal fan-out
    /// for normal-sized prefetch batches; bloated batches use the sorted batch
    /// multi_get instead (see `prefetch_storage`).
    #[cfg(all(feature = "rayon", not(feature = "eip-8025")))]
    fn point_get_storage_many(
        &self,
        missing: &[(Address, H256)],
    ) -> Result<Vec<U256>, DatabaseError> {
        use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
        missing
            .par_iter()
            .map(|&(addr, key)| self.inner.get_storage_value(addr, key))
            .collect()
    }

    #[cfg(not(all(feature = "rayon", not(feature = "eip-8025"))))]
    fn point_get_storage_many(
        &self,
        missing: &[(Address, H256)],
    ) -> Result<Vec<U256>, DatabaseError> {
        missing
            .iter()
            .map(|&(addr, key)| self.inner.get_storage_value(addr, key))
            .collect()
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

    /// Per-account parallel point-gets, in `missing` order. Warm-optimal fan-out
    /// for normal-sized prefetch batches; large batches use the sorted sharded
    /// multi_get instead (see `prefetch_accounts`).
    #[cfg(all(feature = "rayon", not(feature = "eip-8025")))]
    fn point_get_accounts_many(
        &self,
        missing: &[Address],
    ) -> Result<Vec<AccountState>, DatabaseError> {
        use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
        missing
            .par_iter()
            .map(|&addr| self.inner.get_account_state(addr))
            .collect()
    }

    #[cfg(not(all(feature = "rayon", not(feature = "eip-8025"))))]
    fn point_get_accounts_many(
        &self,
        missing: &[Address],
    ) -> Result<Vec<AccountState>, DatabaseError> {
        missing
            .iter()
            .map(|&addr| self.inner.get_account_state(addr))
            .collect()
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
        // Same gate as `prefetch_storage`: a large set of distinct COLD accounts is
        // queue-depth bound. The inner batch path on the rocksdb-backed
        // StoreVmDatabase used a single multi_get (queue depth 1, async_io off),
        // which collapses on cold account-heavy blocks (coldbench: ~13x slower than
        // the sharded batch). Route large/cold sets to the (now sharded) batch and
        // small/warm sets to parallel point-gets. The gate counts MISSING (cold)
        // accounts, so warm blocks stay on the point-get path however many accounts
        // they touch. Tunable.
        const BLOATED_BATCH_THRESHOLD: usize = 16_384;

        let states = if missing.len() >= BLOATED_BATCH_THRESHOLD {
            self.inner.get_account_states_batch(&missing)?
        } else {
            self.point_get_accounts_many(&missing)?
        };
        let mut cache = self.write_accounts()?;
        for (addr, state) in missing.into_iter().zip(states.into_iter()) {
            cache.entry(addr).or_insert(state);
        }
        Ok(())
    }

    fn prefetch_storage(&self, keys: &[(Address, H256)]) -> Result<(), DatabaseError> {
        // Filter out already-cached slots before issuing the batch read.
        let missing: Vec<(Address, H256)> = {
            let cache = self.read_storage()?;
            keys.iter()
                .copied()
                .filter(|k| !cache.contains_key(k))
                .collect()
        };
        if missing.is_empty() {
            return Ok(());
        }
        // Warm is the common case: a normal block touches relatively few storage
        // slots and they are usually cache-resident, where per-slot point-gets
        // (parallel fan-out) are warm-optimal. A block that instead reads a large
        // number of distinct COLD slots is queue-depth bound: a per-slot fan-out
        // is capped at ncpu reads in flight, and a single serial multi_get runs
        // at queue depth 1 (async_io is off in our build), so cold throughput
        // collapses (a sorted serial multi_get regressed bloated SLOAD ~4.5x).
        // The sharded batch path restores it (sorted shards share RocksDB data
        // blocks and run at high queue depth) and hardens validation against
        // storage-bloat DoS. The gate counts MISSING (uncached, i.e. cold) slots,
        // not total accesses, so a warm block never reaches it however many slots
        // it touches; that is what keeps the path off normal traffic. The sharded
        // win is already present once a block has this many cold slots (a cold
        // benchmark shows ~1.4x at 16k and growing with size), while the warm cost
        // it trades against is a few ms and effectively cannot fire, since warm
        // slots are not counted here. 16384 cold slots (~34M gas of cold reads)
        // sits above ordinary cold-block behavior yet below the large-state blocks
        // this targets. Tunable.
        const BLOATED_BATCH_THRESHOLD: usize = 16_384;

        let values = if missing.len() >= BLOATED_BATCH_THRESHOLD {
            // Dispatch to inner's batch path. For the rocksdb-backed
            // StoreVmDatabase this is a sharded parallel multi_get on
            // STORAGE_FLATKEYVALUE for the FKV-covered subset; the default impl
            // loops for other backends.
            self.inner.get_storage_values_batch(&missing)?
        } else {
            self.point_get_storage_many(&missing)?
        };
        let mut cache = self.write_storage()?;
        for (key, value) in missing.into_iter().zip(values.into_iter()) {
            cache.entry(key).or_insert(value);
        }
        Ok(())
    }
}
