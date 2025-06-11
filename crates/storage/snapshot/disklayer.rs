// Inspired by https://github.com/ethereum/go-ethereum/blob/f21adaf245e320a809f9bb6ec96c330726c9078f/core/state/snapshot/disklayer.go

use core::fmt;
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use crate::api::StoreEngine;
use ethrex_common::{
    types::{AccountState, BlockHash},
    H256, U256,
};

use super::{cache::DiskCache, difflayer::DiffLayer, error::SnapshotError};

/// A disk layer is the bottom most layer.
///
/// It looks into the database for the account or storage data,
/// using in addition a fast concurrent cache to store the results.
#[derive(Clone)]
pub struct DiskLayer {
    pub(super) db: Arc<dyn StoreEngine>,
    pub(super) cache: DiskCache,
    pub(super) block_hash: BlockHash,
    pub(super) state_root: H256,
    pub(super) stale: Arc<AtomicBool>,
}

impl fmt::Debug for DiskLayer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DiskLayer")
            .field("db", &self.db)
            .field("cache", &self.cache)
            .field("block_hash", &self.block_hash)
            .field("state_root", &self.state_root)
            .field("stale", &self.stale)
            .finish_non_exhaustive()
    }
}

impl DiskLayer {
    pub fn new(db: Arc<dyn StoreEngine>, block_hash: BlockHash, state_root: H256) -> Self {
        Self {
            block_hash,
            state_root,
            db,
            cache: DiskCache::new(10000, 20000),
            stale: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn get_account(&self, hash: H256) -> Result<Option<AccountState>, SnapshotError> {
        if self.stale() {
            return Err(SnapshotError::StaleSnapshot);
        }

        // Try to get the account from the cache.
        if let Some(value) = self.cache.accounts.get(&hash) {
            return Ok(value.clone());
        }

        let account = self
            .db
            .get_account_snapshot(hash)
            .map_err(|e| SnapshotError::StoreError(Box::new(e)))?;

        self.cache.accounts.insert(hash, account.clone());

        Ok(account)
    }

    pub fn get_storage(
        &self,
        account_hash: H256,
        storage_hash: H256,
    ) -> Result<Option<U256>, SnapshotError> {
        if self.stale() {
            return Err(SnapshotError::StaleSnapshot);
        }

        // Look into the cache first.
        if let Some(value) = self.cache.storages.get(&(account_hash, storage_hash)) {
            return Ok(value);
        }

        let value = self
            .db
            .get_storage_snapshot(account_hash, storage_hash)
            .map_err(|e| SnapshotError::StoreError(Box::new(e)))?;

        self.cache
            .storages
            .insert((account_hash, storage_hash), value);

        Ok(value)
    }

    pub fn block_hash(&self) -> H256 {
        self.block_hash
    }

    pub fn update(
        self: Arc<Self>, // import self is like this
        block_hash: BlockHash,
        state_root: H256,
        accounts: HashMap<H256, Option<AccountState>>,
        storage: HashMap<H256, HashMap<H256, Option<U256>>>,
    ) -> DiffLayer {
        let mut layer = DiffLayer::new(
            self.block_hash,
            self.block_hash,
            block_hash,
            state_root,
            accounts,
            storage,
        );

        layer.rebloom(self.block_hash, None);

        layer
    }

    pub fn stale(&self) -> bool {
        self.stale.load(Ordering::SeqCst)
    }

    pub fn mark_stale(&self) -> bool {
        self.stale.swap(true, Ordering::SeqCst)
    }
}
