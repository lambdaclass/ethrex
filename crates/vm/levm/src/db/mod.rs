use crate::{errors::DatabaseError, precompiles::PrecompileCache};
use dashmap::DashMap;
use ethrex_common::{
    Address, H256, U256,
    types::{AccountState, ChainConfig, Code, CodeMetadata},
};
use ethrex_crypto::keccak::keccak_hash;
#[cfg(all(feature = "rayon", not(feature = "eip-8025")))]
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use rustc_hash::FxBuildHasher;
use std::sync::{Arc, OnceLock};

pub mod gen_db;

/// Cached account state plus its `keccak(address)`. Hoisting the hash here lets
/// downstream storage reads skip the inner database's own address-to-hash lookup
/// (and its associated lock).
#[derive(Clone, Copy)]
pub struct CachedAccount {
    pub state: AccountState,
    pub hashed_address: H256,
}

// DashMap-backed caches: per-shard locks let warmer writes and executor reads
// proceed concurrently without serializing on a single map-wide RwLock.
type AccountCache = DashMap<Address, CachedAccount, FxBuildHasher>;
type StorageCache = DashMap<(Address, H256), U256, FxBuildHasher>;
type CodeCache = DashMap<H256, Code, FxBuildHasher>;

pub trait Database: Send + Sync {
    fn get_account_state(&self, address: Address) -> Result<AccountState, DatabaseError>;
    /// Fetch account state and return its `keccak(address)` alongside. Lets callers
    /// reuse the hash the underlying database already computed (avoids a redundant
    /// keccak in the caller). Default impl computes the hash locally.
    fn get_account_state_with_hashed_address(
        &self,
        address: Address,
    ) -> Result<(AccountState, H256), DatabaseError> {
        let state = self.get_account_state(address)?;
        let hashed_address = H256::from(keccak_hash(address.to_fixed_bytes()));
        Ok((state, hashed_address))
    }
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
/// Thread-safe via DashMap - per-shard locks let warmer writes and executor
/// reads proceed concurrently without serializing on a single map-wide lock.
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
            accounts: DashMap::with_hasher(FxBuildHasher),
            storage: DashMap::with_hasher(FxBuildHasher),
            code: DashMap::with_hasher(FxBuildHasher),
            precompile_cache: precompile_cache_enabled.then(PrecompileCache::new),
            chain_config: OnceLock::new(),
        }
    }

    /// Look up the cached `(hashed_address, storage_root)` for an account if present.
    fn cached_account_hash_and_root(&self, address: Address) -> Option<(H256, H256)> {
        self.accounts
            .get(&address)
            .map(|cached| (cached.hashed_address, cached.state.storage_root))
    }
}

impl Database for CachingDatabase {
    fn get_account_state(&self, address: Address) -> Result<AccountState, DatabaseError> {
        if let Some(cached) = self.accounts.get(&address) {
            return Ok(cached.state);
        }

        // Cache miss: fetch state + hash from inner (which already has them) so
        // we don't recompute keccak(address) at this layer.
        let (state, hashed_address) = self.inner.get_account_state_with_hashed_address(address)?;
        self.accounts.entry(address).or_insert(CachedAccount {
            state,
            hashed_address,
        });
        Ok(state)
    }

    fn get_account_state_with_hashed_address(
        &self,
        address: Address,
    ) -> Result<(AccountState, H256), DatabaseError> {
        if let Some(cached) = self.accounts.get(&address) {
            return Ok((cached.state, cached.hashed_address));
        }
        let (state, hashed_address) = self.inner.get_account_state_with_hashed_address(address)?;
        self.accounts.entry(address).or_insert(CachedAccount {
            state,
            hashed_address,
        });
        Ok((state, hashed_address))
    }

    fn get_storage_value(&self, address: Address, key: H256) -> Result<U256, DatabaseError> {
        if let Some(value) = self.storage.get(&(address, key)) {
            return Ok(*value);
        }

        // Cache miss: if the account is already cached, use its hashed address
        // and storage_root to skip the inner database's address-to-hash lookup.
        let value = if let Some((hashed_address, storage_root)) =
            self.cached_account_hash_and_root(address)
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

        self.storage.insert((address, key), value);
        Ok(value)
    }

    fn get_storage_value_with_known_hash(
        &self,
        address: Address,
        hashed_address: H256,
        storage_root: H256,
        key: H256,
    ) -> Result<U256, DatabaseError> {
        if let Some(value) = self.storage.get(&(address, key)) {
            return Ok(*value);
        }
        let value = self.inner.get_storage_value_with_known_hash(
            address,
            hashed_address,
            storage_root,
            key,
        )?;
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
        if let Some(code) = self.code.get(&code_hash) {
            return Ok(code.clone());
        }

        let code = self.inner.get_account_code(code_hash)?;
        // Code contains Bytes which is ref-counted, clone is cheap.
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
        self.precompile_cache.as_ref()
    }

    #[cfg(all(feature = "rayon", not(feature = "eip-8025")))]
    fn prefetch_accounts(&self, addresses: &[Address]) -> Result<(), DatabaseError> {
        // Each task fetches + inserts independently — DashMap's per-shard locks
        // make the batched-write pattern unnecessary.
        addresses
            .par_iter()
            .try_for_each(|&addr| -> Result<(), DatabaseError> {
                if self.accounts.contains_key(&addr) {
                    return Ok(());
                }
                let (state, hashed_address) =
                    self.inner.get_account_state_with_hashed_address(addr)?;
                self.accounts.entry(addr).or_insert(CachedAccount {
                    state,
                    hashed_address,
                });
                Ok(())
            })
    }

    #[cfg(all(feature = "rayon", not(feature = "eip-8025")))]
    fn prefetch_storage(&self, keys: &[(Address, H256)]) -> Result<(), DatabaseError> {
        // No upfront snapshot needed — DashMap reads are lock-free per shard,
        // so each parallel task can look up the cached account hash directly.
        keys.par_iter()
            .try_for_each(|&(addr, key)| -> Result<(), DatabaseError> {
                if self.storage.contains_key(&(addr, key)) {
                    return Ok(());
                }
                let value = if let Some((hashed_address, storage_root)) =
                    self.cached_account_hash_and_root(addr)
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
                self.storage.insert((addr, key), value);
                Ok(())
            })
    }
}
