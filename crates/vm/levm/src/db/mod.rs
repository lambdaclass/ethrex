use crate::{errors::DatabaseError, precompiles::PrecompileCache};
use ethrex_common::{
    Address, H256, U256,
    types::{AccountState, AccountUpdate, ChainConfig, Code, CodeMetadata},
};
use rustc_hash::{FxHashMap, FxHashSet};
use std::sync::{
    Arc, OnceLock, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard,
    atomic::{AtomicU64, Ordering},
};

pub mod gen_db;

/// Distinct-cold-key count above which a prefetch routes to the sorted, sharded
/// batch read instead of per-key parallel point-gets. Shared by the account and
/// storage prefetch gates here and by the merkle trie-node prefetch gate in
/// `ethrex-blockchain`, so tuning it moves all three together. ~16384 cold
/// accesses is ~34M gas of cold reads: above ordinary cold blocks, below the
/// large-state blocks these paths target. Tunable.
pub const BLOATED_BATCH_THRESHOLD: usize = 16_384;

// Type aliases for cache storage maps
type AccountCache = FxHashMap<Address, AccountState>;
type StorageCache = FxHashMap<(Address, H256), U256>;
type CodeCache = FxHashMap<H256, Code>;
/// State a [`CachingDatabase`] held at the end of a block, moved out so the next block's
/// cache can start from it instead of empty.
#[derive(Default)]
pub struct CarriedEntries {
    pub accounts: AccountCache,
    pub storage: StorageCache,
    pub code: CodeCache,
    /// Sum of [`Code::size`] over `code`.
    pub code_bytes: u64,
    /// The block's state changes, in execution order: what the entries above need
    /// applied to describe the state after the block instead of before it.
    pub writes: Vec<AccountUpdate>,
}

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
    /// The account state this layer already holds in memory, without reading the backing
    /// store. Default: `None`, for layers that keep nothing.
    fn cached_account_state(&self, _address: Address) -> Option<AccountState> {
        None
    }
    /// The storage value this layer already holds in memory, without reading the backing
    /// store. Default: `None`.
    fn cached_storage_value(&self, _address: Address, _key: H256) -> Option<U256> {
        None
    }
    /// The bytecode this layer already holds in memory, without reading the backing store.
    /// Default: `None`.
    fn cached_account_code(&self, _code_hash: H256) -> Option<Code> {
        None
    }
    /// Records state changes the block's execution made, as it sends them to the
    /// merkleizer. Default: dropped.
    fn record_block_writes(&self, _updates: &[AccountUpdate]) {}
    /// Moves out everything this layer holds in memory, leaving it empty. Default: `None`,
    /// for layers that keep nothing.
    fn take_entries(&self) -> Option<CarriedEntries> {
        None
    }
    /// Adds `entries` this layer does not hold yet. They must describe the same state as
    /// this layer's backing store. Default: dropped.
    fn seed_entries(&self, _entries: CarriedEntries) {}
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
    /// Batch bytecode lookup, aligned to `code_hashes`. `None` means the hash is absent
    /// from the database. Default: loop. Backends with a batched read path (e.g. rocksdb
    /// `multi_get_cf` on the account-codes table) should override this; the caching layer
    /// above dispatches to it from [`Self::prefetch_codes`].
    fn get_account_codes_batch(
        &self,
        code_hashes: &[H256],
    ) -> Result<Vec<Option<Code>>, DatabaseError> {
        code_hashes
            .iter()
            .map(|h| self.get_account_code(*h).map(Some))
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

    /// Byte budget for warming bytecode ahead of execution, as measured by [`Code::size`].
    ///
    /// Bounds the bytecode a block warm reads speculatively, since a block access list
    /// does not say which of its addresses were accessed for their code. Backends holding
    /// a bytecode cache report its capacity, which is how much the node was configured to
    /// keep resident; the default is the floor for a backend without one.
    fn code_cache_budget_bytes(&self) -> u64 {
        64 * 1024 * 1024
    }

    /// Prefetch a batch of bytecodes into the cache. Default: sequential fallback.
    ///
    /// Returns the total [`Code::size`] warmed, so a caller warming a whole block can
    /// hold itself to [`Self::code_cache_budget_bytes`]. An absent hash is not an error:
    /// warming has nothing to do for it and the executor reports it if reached.
    fn prefetch_codes(&self, code_hashes: &[H256]) -> Result<u64, DatabaseError> {
        let mut warmed = 0u64;
        for &hash in code_hashes {
            if let Ok(code) = self.get_account_code(hash) {
                warmed = warmed.saturating_add(u64::try_from(code.size()).unwrap_or(u64::MAX));
            }
        }
        Ok(warmed)
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
    /// Sum of [`Code::size`] over `code`. Nothing is evicted from the map, so a long-lived
    /// instance (the mempool prewarmer keeps one for a whole slot) uses this to bound
    /// how much bytecode it keeps.
    code_bytes: AtomicU64,
    /// Shared precompile result cache (warmer populates, executor reuses).
    /// `None` when the cache is disabled via `BlockchainOptions::precompile_cache_enabled = false`.
    precompile_cache: Option<PrecompileCache>,
    /// Cached chain config (constant for the lifetime of this database)
    chain_config: OnceLock<ChainConfig>,
    /// State changes the block's execution made, kept so the cache can be carried to the
    /// next block.
    block_writes: std::sync::Mutex<Vec<AccountUpdate>>,
}

impl CachingDatabase {
    pub fn new(inner: Arc<dyn Database>, precompile_cache_enabled: bool) -> Self {
        Self {
            inner,
            accounts: RwLock::new(FxHashMap::default()),
            storage: RwLock::new(FxHashMap::default()),
            code: RwLock::new(FxHashMap::default()),
            code_bytes: AtomicU64::new(0),
            precompile_cache: precompile_cache_enabled.then(PrecompileCache::new),
            chain_config: OnceLock::new(),
            block_writes: std::sync::Mutex::new(Vec::new()),
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

    /// Total [`Code::size`] of the bytecode this cache holds.
    pub fn code_bytes(&self) -> u64 {
        self.code_bytes.load(Ordering::Relaxed)
    }

    /// Inserts `code` if absent and counts its size once.
    fn keep_code(&self, cache: &mut CodeCache, code_hash: H256, code: Code) {
        if let std::collections::hash_map::Entry::Vacant(entry) = cache.entry(code_hash) {
            let size = u64::try_from(code.size()).unwrap_or(u64::MAX);
            entry.insert(code);
            self.code_bytes.fetch_add(size, Ordering::Relaxed);
        }
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
    #[cfg(feature = "rayon")]
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

    #[cfg(not(feature = "rayon"))]
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
    #[cfg(feature = "rayon")]
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

    #[cfg(not(feature = "rayon"))]
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

/// Adds the entries of `other` that `map` lacks. Walks whichever of the two is smaller, so
/// that a large carried set added to a small cache, or the reverse, costs the small one's
/// size. Both describe the same state, so which copy of a key is kept does not matter.
fn add_missing<K: Eq + std::hash::Hash, V>(map: &mut FxHashMap<K, V>, mut other: FxHashMap<K, V>) {
    if other.len() > map.len() {
        std::mem::swap(map, &mut other);
    }
    for (key, value) in other {
        map.entry(key).or_insert(value);
    }
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
        self.keep_code(&mut *self.write_code()?, code_hash, code.clone());

        Ok(code)
    }

    fn get_code_metadata(&self, code_hash: H256) -> Result<CodeMetadata, DatabaseError> {
        // Answer from resident code when there is any. The BAL warmer loads the block's
        // bytecode into this cache, so a length read usually has the answer here under a
        // shared read lock. Falling straight through would instead take the store's
        // single mutex-guarded metadata cache, serializing every `EXTCODESIZE` across
        // the parallel executor's threads.
        if let Some(code) = self.read_code()?.get(&code_hash) {
            return Ok(CodeMetadata {
                length: u64::try_from(code.len()).unwrap_or(u64::MAX),
            });
        }
        self.inner.get_code_metadata(code_hash)
    }

    fn precompile_cache(&self) -> Option<&PrecompileCache> {
        self.precompile_cache.as_ref()
    }

    fn cached_account_state(&self, address: Address) -> Option<AccountState> {
        self.read_accounts().ok()?.get(&address).copied()
    }

    fn cached_storage_value(&self, address: Address, key: H256) -> Option<U256> {
        self.read_storage().ok()?.get(&(address, key)).copied()
    }

    fn cached_account_code(&self, code_hash: H256) -> Option<Code> {
        self.read_code().ok()?.get(&code_hash).cloned()
    }

    fn record_block_writes(&self, updates: &[AccountUpdate]) {
        if let Ok(mut writes) = self.block_writes.lock() {
            writes.extend_from_slice(updates);
        }
    }

    fn take_entries(&self) -> Option<CarriedEntries> {
        let writes = std::mem::take(&mut *self.block_writes.lock().ok()?);
        let accounts = std::mem::take(&mut *self.write_accounts().ok()?);
        let storage = std::mem::take(&mut *self.write_storage().ok()?);
        let (code, code_bytes) = {
            let mut code = self.write_code().ok()?;
            (
                std::mem::take(&mut *code),
                self.code_bytes.swap(0, Ordering::Relaxed),
            )
        };
        Some(CarriedEntries {
            accounts,
            storage,
            code,
            code_bytes,
            writes,
        })
    }

    fn seed_entries(&self, entries: CarriedEntries) {
        if let Ok(mut accounts) = self.write_accounts() {
            add_missing(&mut accounts, entries.accounts);
        }
        if let Ok(mut storage) = self.write_storage() {
            add_missing(&mut storage, entries.storage);
        }
        if let Ok(mut code) = self.write_code() {
            let mut other = entries.code;
            let mut bytes = self.code_bytes.load(Ordering::Relaxed);
            if other.len() > code.len() {
                std::mem::swap(&mut *code, &mut other);
                bytes = entries.code_bytes;
            }
            for (hash, bytecode) in other {
                if let std::collections::hash_map::Entry::Vacant(entry) = code.entry(hash) {
                    let size = u64::try_from(bytecode.size()).unwrap_or(u64::MAX);
                    bytes = bytes.saturating_add(size);
                    entry.insert(bytecode);
                }
            }
            self.code_bytes.store(bytes, Ordering::Relaxed);
        }
    }

    /// Warms every address through [`Self::prefetch_accounts`], then answers from the
    /// cache under one read lock. Repeated addresses cost a single database read, and a
    /// caller that wants the states pays one map lookup each rather than a full
    /// `get_account_state` round trip per address.
    ///
    /// The read lock spans the whole assembly, so callers should keep `addresses` to a
    /// bounded slice: a writer (an executor filling the same cache) waits on it.
    fn get_account_states_batch(
        &self,
        addresses: &[Address],
    ) -> Result<Vec<AccountState>, DatabaseError> {
        self.prefetch_accounts(addresses)?;
        let cache = self.read_accounts()?;
        addresses
            .iter()
            .map(|addr| {
                cache.get(addr).copied().ok_or_else(|| {
                    DatabaseError::Custom(format!("account {addr:?} missing after prefetch"))
                })
            })
            .collect()
    }

    fn code_cache_budget_bytes(&self) -> u64 {
        self.inner.code_cache_budget_bytes()
    }

    /// Fetches the uncached bytecodes in one batch and inserts them, reading each
    /// distinct hash once however many entries share it.
    ///
    /// Returns only the warmed byte count, not the bytecodes: the executor reads code
    /// through [`Self::get_account_code`], which this turns into a cache hit. Assembling
    /// a result vector here would hold the write lock that guards every executor code
    /// read for the length of the batch, to build something a warming caller discards.
    fn prefetch_codes(&self, code_hashes: &[H256]) -> Result<u64, DatabaseError> {
        let missing: Vec<H256> = {
            let cache = self.read_code()?;
            let mut seen: FxHashSet<H256> = FxHashSet::default();
            code_hashes
                .iter()
                .copied()
                .filter(|h| !cache.contains_key(h) && seen.insert(*h))
                .collect()
        };

        if missing.is_empty() {
            return Ok(0);
        }

        let codes = self.inner.get_account_codes_batch(&missing)?;
        let mut warmed = 0u64;
        let mut cache = self.write_code()?;
        for (hash, code) in missing.into_iter().zip(codes.into_iter()) {
            // An absent hash has nothing to warm; the executor reports it if reached.
            if let Some(code) = code {
                warmed = warmed.saturating_add(u64::try_from(code.size()).unwrap_or(u64::MAX));
                self.keep_code(&mut cache, hash, code);
            }
        }
        Ok(warmed)
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
        // they touch. See `BLOATED_BATCH_THRESHOLD`.
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
        // slots are not counted here. See `BLOATED_BATCH_THRESHOLD`.
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

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::as_conversions
)]
mod code_bytes_tests {
    use super::*;
    use bytes::Bytes;

    /// Serves a distinct 4 KiB contract for every code hash.
    struct BigCodeStore;

    fn code_for(hash: H256) -> Code {
        let mut bytecode = vec![0x5b; 4096];
        bytecode[..32].copy_from_slice(hash.as_bytes());
        Code::from_bytecode_unchecked(Bytes::from(bytecode), hash)
    }

    impl Database for BigCodeStore {
        fn get_account_state(&self, _: Address) -> Result<AccountState, DatabaseError> {
            Ok(AccountState::default())
        }
        fn get_storage_value(&self, _: Address, _: H256) -> Result<U256, DatabaseError> {
            Ok(U256::zero())
        }
        fn get_block_hash(&self, _: u64) -> Result<H256, DatabaseError> {
            Ok(H256::zero())
        }
        fn get_chain_config(&self) -> Result<ChainConfig, DatabaseError> {
            Ok(ChainConfig::default())
        }
        fn get_account_code(&self, code_hash: H256) -> Result<Code, DatabaseError> {
            Ok(code_for(code_hash))
        }
        fn get_code_metadata(&self, code_hash: H256) -> Result<CodeMetadata, DatabaseError> {
            Ok(CodeMetadata {
                length: code_for(code_hash).len() as u64,
            })
        }
    }

    /// `code_bytes` SHALL count every distinct code once, whether it was read on demand
    /// or prefetched, so the prewarmer's stop condition sees what the map really holds.
    #[test]
    fn code_bytes_counts_each_distinct_code_once() {
        let cache = CachingDatabase::new(Arc::new(BigCodeStore), false);
        let size = code_for(H256::zero()).size() as u64;
        let hashes: Vec<H256> = (1..=4u64).map(H256::from_low_u64_be).collect();

        cache.get_account_code(hashes[0]).unwrap();
        cache.get_account_code(hashes[0]).unwrap();
        assert_eq!(cache.code_bytes(), size);

        cache.prefetch_codes(&hashes).unwrap();
        cache.prefetch_codes(&hashes).unwrap();
        assert_eq!(cache.code_bytes(), 4 * size);
        assert_eq!(cache.read_code().unwrap().len(), 4);
    }

    /// Seeding SHALL leave the union of both entry sets, whichever of the two is larger,
    /// and `code_bytes` SHALL still count every distinct code once.
    #[test]
    fn seeding_keeps_the_union_and_counts_code_once() {
        let size = code_for(H256::zero()).size() as u64;
        let hash = H256::from_low_u64_be;
        let address = Address::from_low_u64_be;
        for (carried_codes, cached_codes) in [(1..=5u64, 5..=6u64), (5..=6u64, 1..=5u64)] {
            let previous = CachingDatabase::new(Arc::new(BigCodeStore), false);
            for i in carried_codes {
                previous.get_account_code(hash(i)).unwrap();
                previous.get_account_state(address(i)).unwrap();
                previous.get_storage_value(address(i), hash(i)).unwrap();
            }
            let entries = previous.take_entries().unwrap();
            assert_eq!(previous.code_bytes(), 0);
            assert_eq!(entries.code_bytes, entries.code.len() as u64 * size);

            let cache = CachingDatabase::new(Arc::new(BigCodeStore), false);
            for i in cached_codes {
                cache.get_account_code(hash(i)).unwrap();
                cache.get_account_state(address(i)).unwrap();
                cache.get_storage_value(address(i), hash(i)).unwrap();
            }
            cache.seed_entries(entries);

            assert_eq!(cache.read_code().unwrap().len(), 6);
            assert_eq!(cache.code_bytes(), 6 * size);
            assert_eq!(cache.read_accounts().unwrap().len(), 6);
            assert_eq!(cache.read_storage().unwrap().len(), 6);
        }
    }
}
