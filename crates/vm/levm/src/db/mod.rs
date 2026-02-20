use crate::errors::DatabaseError;
use dashmap::DashMap;
use ethrex_common::{
    Address, H256, U256,
    types::{AccountState, ChainConfig, Code, CodeMetadata},
};
use rustc_hash::FxBuildHasher;
use std::sync::Arc;

pub mod gen_db;

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
/// Thread-safe via DashMap — per-shard locking eliminates global read contention
/// between warmer threads and the executor.
///
/// This caching database is inspired by reth's overlay/proof worker cache.
pub struct CachingDatabase {
    inner: Arc<dyn Database>,
    /// Cached account states (balance, nonce, code_hash, storage_root)
    accounts: DashMap<Address, AccountState, FxBuildHasher>,
    /// Cached storage values
    storage: DashMap<(Address, H256), U256, FxBuildHasher>,
    /// Cached contract code
    code: DashMap<H256, Code, FxBuildHasher>,
}

impl CachingDatabase {
    pub fn new(inner: Arc<dyn Database>) -> Self {
        Self {
            inner,
            accounts: DashMap::with_hasher(FxBuildHasher),
            storage: DashMap::with_hasher(FxBuildHasher),
            code: DashMap::with_hasher(FxBuildHasher),
        }
    }
}

impl Database for CachingDatabase {
    fn get_account_state(&self, address: Address) -> Result<AccountState, DatabaseError> {
        if let Some(state) = self.accounts.get(&address).map(|r| *r) {
            return Ok(state);
        }

        let state = self.inner.get_account_state(address)?;
        self.accounts.insert(address, state);
        Ok(state)
    }

    fn get_storage_value(&self, address: Address, key: H256) -> Result<U256, DatabaseError> {
        if let Some(value) = self.storage.get(&(address, key)).map(|r| *r) {
            return Ok(value);
        }

        let value = self.inner.get_storage_value(address, key)?;
        self.storage.insert((address, key), value);
        Ok(value)
    }

    fn get_block_hash(&self, block_number: u64) -> Result<H256, DatabaseError> {
        self.inner.get_block_hash(block_number)
    }

    fn get_chain_config(&self) -> Result<ChainConfig, DatabaseError> {
        self.inner.get_chain_config()
    }

    fn get_account_code(&self, code_hash: H256) -> Result<Code, DatabaseError> {
        if let Some(code) = self.code.get(&code_hash).map(|r| r.clone()) {
            return Ok(code);
        }

        let code = self.inner.get_account_code(code_hash)?;
        self.code.insert(code_hash, code.clone());
        Ok(code)
    }

    fn get_code_metadata(&self, code_hash: H256) -> Result<CodeMetadata, DatabaseError> {
        self.inner.get_code_metadata(code_hash)
    }
}
