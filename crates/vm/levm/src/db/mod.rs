use crate::{errors::DatabaseError, precompiles::PrecompileCache};
use dashmap::DashMap;
use ethrex_common::{
    Address, H256, U256,
    types::{AccountState, ChainConfig, Code, CodeMetadata},
};
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use rustc_hash::FxBuildHasher;
use std::sync::{Arc, OnceLock};

pub mod gen_db;

// Type aliases for cache storage maps. Sharded concurrent maps replace the old
// `RwLock<FxHashMap>` design — under the parallel BAL path, 16 rayon workers all
// hammered the single account RwLock, contributing ~11% of CPU to lock
// contention (perf measured 2026-04-28). DashMap shards by key hash so workers
// touching different addresses don't contend.
type AccountCache = DashMap<Address, AccountState, FxBuildHasher>;
type StorageCache = DashMap<(Address, H256), U256, FxBuildHasher>;
type CodeCache = DashMap<H256, Code, FxBuildHasher>;

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
/// This caching database is inspired by reth's overlay/proof worker cache.
pub struct CachingDatabase {
    inner: Arc<dyn Database>,
    /// Cached account states (balance, nonce, code_hash, storage_root)
    accounts: AccountCache,
    /// Cached storage values
    storage: StorageCache,
    /// Cached contract code
    code: CodeCache,
    /// Shared precompile result cache (warmer populates, executor reuses)
    precompile_cache: PrecompileCache,
    /// Cached chain config (constant for the lifetime of this database)
    chain_config: OnceLock<ChainConfig>,
}

impl CachingDatabase {
    pub fn new(inner: Arc<dyn Database>) -> Self {
        Self {
            inner,
            accounts: AccountCache::with_hasher(FxBuildHasher),
            storage: StorageCache::with_hasher(FxBuildHasher),
            code: CodeCache::with_hasher(FxBuildHasher),
            precompile_cache: PrecompileCache::new(),
            chain_config: OnceLock::new(),
        }
    }

    /// Access the shared precompile result cache.
    pub fn precompile_cache(&self) -> &PrecompileCache {
        &self.precompile_cache
    }
}

impl Database for CachingDatabase {
    fn get_account_state(&self, address: Address) -> Result<AccountState, DatabaseError> {
        // Check cache first (per-shard lock; non-contended for distinct addresses)
        if let Some(state) = self.accounts.get(&address).map(|r| *r) {
            return Ok(state);
        }

        // Cache miss: query underlying database
        let state = self.inner.get_account_state(address)?;

        // Populate cache (AccountState is Copy, no clone needed)
        self.accounts.insert(address, state);

        Ok(state)
    }

    fn get_storage_value(&self, address: Address, key: H256) -> Result<U256, DatabaseError> {
        // Check cache first
        if let Some(value) = self.storage.get(&(address, key)).map(|r| *r) {
            return Ok(value);
        }

        // Cache miss: query underlying database
        let value = self.inner.get_storage_value(address, key)?;

        // Populate cache (U256 is Copy, no clone needed)
        self.storage.insert((address, key), value);

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
        if let Some(code) = self.code.get(&code_hash).map(|r| r.clone()) {
            return Ok(code);
        }

        // Cache miss: query underlying database
        let code = self.inner.get_account_code(code_hash)?;

        // Populate cache (Code contains Bytes which is ref-counted, clone is cheap)
        self.code.insert(code_hash, code.clone());

        Ok(code)
    }

    fn get_code_metadata(&self, code_hash: H256) -> Result<CodeMetadata, DatabaseError> {
        // Delegate directly to the underlying database.
        // The underlying Store already has its own code_metadata_cache,
        // so we don't need to duplicate caching here.
        self.inner.get_code_metadata(code_hash)
    }

    fn precompile_cache(&self) -> Option<&PrecompileCache> {
        Some(&self.precompile_cache)
    }

    fn prefetch_accounts(&self, addresses: &[Address]) -> Result<(), DatabaseError> {
        // Fetch from inner in parallel; insert into sharded cache without serializing
        // through a single write lock — DashMap inserts touch only the matching shard.
        addresses
            .par_iter()
            .try_for_each(|&addr| -> Result<(), DatabaseError> {
                let state = self.inner.get_account_state(addr)?;
                self.accounts.entry(addr).or_insert(state);
                Ok(())
            })
    }

    fn prefetch_storage(&self, keys: &[(Address, H256)]) -> Result<(), DatabaseError> {
        keys.par_iter()
            .try_for_each(|&(addr, key)| -> Result<(), DatabaseError> {
                let value = self.inner.get_storage_value(addr, key)?;
                self.storage.entry((addr, key)).or_insert(value);
                Ok(())
            })
    }
}
