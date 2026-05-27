use crate::{errors::DatabaseError, precompiles::PrecompileCache};
use ethrex_common::{
    Address, H256, U256,
    types::{AccountState, ChainConfig, Code, CodeMetadata},
};
use ethrex_crypto::keccak::keccak_hash;
#[cfg(all(feature = "rayon", not(feature = "eip-8025")))]
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use rustc_hash::FxHashMap;
use std::sync::{Arc, OnceLock, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

pub mod gen_db;

/// Cached account state plus its `keccak(address)`. Hoisting the hash here lets
/// downstream storage reads skip the inner database's own address-to-hash lookup
/// (and its associated lock).
#[derive(Clone, Copy)]
pub struct CachedAccount {
    pub state: AccountState,
    pub hashed_address: H256,
}

// Type aliases for cache storage maps
type AccountCache = FxHashMap<Address, CachedAccount>;
type StorageCache = FxHashMap<(Address, H256), U256>;
type CodeCache = FxHashMap<H256, Code>;

pub trait Database: Send + Sync {
    fn get_account_state(&self, address: Address) -> Result<AccountState, DatabaseError>;
    fn get_storage_value(&self, address: Address, key: H256) -> Result<U256, DatabaseError>;
    /// Storage read with caller-provided `keccak(address)` and `storage_root`.
    /// Lets implementations that wrap a higher-level cache (e.g. `CachingDatabase`)
    /// bypass the inner database's own address-to-hash lookup on the hot read path.
    /// Default impl ignores the hints and falls back to `get_storage_value`.
    fn get_storage_value_with_known_hash(
        &self,
        address: Address,
        _hashed_address: H256,
        _storage_root: H256,
        key: H256,
    ) -> Result<U256, DatabaseError> {
        self.get_storage_value(address, key)
    }
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
}

fn poison_error_to_db_error<T>(err: PoisonError<T>) -> DatabaseError {
    DatabaseError::Custom(format!("Cache lock poisoned: {err}"))
}

impl CachingDatabase {
    /// Compute and cache `keccak(address)` alongside the account state.
    fn cache_account(&self, address: Address, state: AccountState) -> Result<(), DatabaseError> {
        let hashed_address = H256::from(keccak_hash(address.to_fixed_bytes()));
        self.write_accounts()?
            .entry(address)
            .or_insert(CachedAccount {
                state,
                hashed_address,
            });
        Ok(())
    }

    /// Look up the cached `(hashed_address, storage_root)` for an account if present.
    fn cached_account_hash_and_root(
        &self,
        address: Address,
    ) -> Result<Option<(H256, H256)>, DatabaseError> {
        Ok(self
            .read_accounts()?
            .get(&address)
            .map(|cached| (cached.hashed_address, cached.state.storage_root)))
    }
}

impl Database for CachingDatabase {
    fn get_account_state(&self, address: Address) -> Result<AccountState, DatabaseError> {
        // Check cache first
        if let Some(cached) = self.read_accounts()?.get(&address).copied() {
            return Ok(cached.state);
        }

        // Cache miss: query underlying database
        let state = self.inner.get_account_state(address)?;
        self.cache_account(address, state)?;
        Ok(state)
    }

    fn get_storage_value(&self, address: Address, key: H256) -> Result<U256, DatabaseError> {
        // Check cache first
        if let Some(value) = self.read_storage()?.get(&(address, key)).copied() {
            return Ok(value);
        }

        // Cache miss: if the account is already cached, use its hashed address
        // and storage_root to skip the inner database's address-to-hash lookup
        // (and its associated lock acquisition).
        let value = if let Some((hashed_address, storage_root)) =
            self.cached_account_hash_and_root(address)?
        {
            self.inner.get_storage_value_with_known_hash(
                address,
                hashed_address,
                storage_root,
                key,
            )?
        } else {
            self.inner.get_storage_value(address, key)?
        };

        // Populate cache (U256 is Copy, no clone needed)
        self.write_storage()?.insert((address, key), value);

        Ok(value)
    }

    fn get_storage_value_with_known_hash(
        &self,
        address: Address,
        hashed_address: H256,
        storage_root: H256,
        key: H256,
    ) -> Result<U256, DatabaseError> {
        // Honour the storage cache first; otherwise forward the precomputed hash.
        if let Some(value) = self.read_storage()?.get(&(address, key)).copied() {
            return Ok(value);
        }
        let value = self.inner.get_storage_value_with_known_hash(
            address,
            hashed_address,
            storage_root,
            key,
        )?;
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

    #[cfg(all(feature = "rayon", not(feature = "eip-8025")))]
    fn prefetch_accounts(&self, addresses: &[Address]) -> Result<(), DatabaseError> {
        // Fetch from inner in parallel (no lock contention), compute keccak(address)
        // in the same parallel pass, then single write-lock to populate cache.
        let fetched: Vec<(Address, CachedAccount)> = addresses
            .par_iter()
            .map(|&addr| {
                self.inner.get_account_state(addr).map(|state| {
                    (
                        addr,
                        CachedAccount {
                            state,
                            hashed_address: H256::from(keccak_hash(addr.to_fixed_bytes())),
                        },
                    )
                })
            })
            .collect::<Result<_, _>>()?;
        let mut cache = self.write_accounts()?;
        for (addr, cached) in fetched {
            cache.entry(addr).or_insert(cached);
        }
        Ok(())
    }

    #[cfg(all(feature = "rayon", not(feature = "eip-8025")))]
    fn prefetch_storage(&self, keys: &[(Address, H256)]) -> Result<(), DatabaseError> {
        // Snapshot hashed_address + storage_root per account so the parallel
        // fetchers can use the precomputed hash and bypass the inner database's
        // address-to-hash lookup on every slot.
        let account_hints: FxHashMap<Address, (H256, H256)> = self
            .read_accounts()?
            .iter()
            .map(|(addr, cached)| (*addr, (cached.hashed_address, cached.state.storage_root)))
            .collect();

        let fetched: Vec<((Address, H256), U256)> = keys
            .par_iter()
            .map(|&(addr, key)| {
                let value = if let Some(&(hashed_address, storage_root)) = account_hints.get(&addr)
                {
                    self.inner.get_storage_value_with_known_hash(
                        addr,
                        hashed_address,
                        storage_root,
                        key,
                    )?
                } else {
                    self.inner.get_storage_value(addr, key)?
                };
                Ok(((addr, key), value))
            })
            .collect::<Result<_, _>>()?;
        let mut cache = self.write_storage()?;
        for (key, value) in fetched {
            cache.entry(key).or_insert(value);
        }
        Ok(())
    }
}
