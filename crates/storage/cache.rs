//! An in memory cache.

use std::sync::Arc;

use ethrex_common::{types::AccountState, H256, U256};

use crate::rlp::AccountStateRLP;

/// In-memory cache.
///
/// Can be cloned freely.
#[derive(Debug, Clone)]
pub struct Cache {
    pub accounts: moka::sync::Cache<H256, Arc<AccountState>>,
    pub storages: moka::sync::Cache<(H256, H256), U256>,
}

impl Cache {
    pub fn new(max_capacity_accounts: u64, max_capacity_storage: u64) -> Self {
        Self {
            accounts: moka::sync::Cache::new(max_capacity_accounts),
            storages: moka::sync::Cache::new(max_capacity_storage),
        }
    }
}
