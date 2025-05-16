//! An in memory cache.

use std::sync::Arc;

use ethrex_common::{types::AccountState, H256, U256};
use quick_cache::sync::Cache;

/// In-memory cache.
///
/// Can be cloned freely.
#[derive(Debug, Clone)]
pub struct DiskCache {
    pub accounts: Arc<Cache<H256, Option<AccountState>>>,
    pub storages: Arc<Cache<(H256, H256), Option<U256>>>,
}

impl DiskCache {
    pub fn new(max_capacity_accounts: usize, max_capacity_storage: usize) -> Self {
        Self {
            accounts: Arc::new(Cache::new(max_capacity_accounts)),
            storages: Arc::new(Cache::new(max_capacity_storage)),
        }
    }
}
