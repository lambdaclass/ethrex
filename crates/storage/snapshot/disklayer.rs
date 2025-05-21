use core::fmt;
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Instant,
};

use crate::api::StoreEngine;
use ethrex_common::{
    types::{AccountState, BlockHash},
    H256, U256,
};
use ethrex_rlp::decode::RLPDecode;
use tracing::info;

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
    pub(super) generating: Arc<AtomicBool>,
}

impl fmt::Debug for DiskLayer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DiskLayer")
            .field("db", &self.db)
            .field("cache", &self.cache)
            .field("root", &self.state_root)
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
            cache: DiskCache::new(20000, 40000),
            stale: Arc::new(AtomicBool::new(false)),
            generating: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl DiskLayer {
    pub fn root(&self) -> H256 {
        self.state_root
    }

    pub fn get_account(&self, hash: H256) -> Result<Option<AccountState>, SnapshotError> {
        // Try to get the account from the cache.
        if let Some(value) = self.cache.accounts.get(&hash) {
            return Ok(value.clone());
        }

        // TODO: check that snapshot is done to make sure None is None?
        let account = self.db.get_account_snapshot(hash)?;

        self.cache.accounts.insert(hash, account.clone());

        Ok(account)
    }

    pub fn get_storage(
        &self,
        account_hash: H256,
        storage_hash: H256,
    ) -> Result<Option<U256>, SnapshotError> {
        // Look into the cache first.
        if let Some(value) = self.cache.storages.get(&(account_hash, storage_hash)) {
            return Ok(value);
        }

        // TODO: check that snapshot is done to make sure None is None?
        let value = self.db.get_storage_snapshot(account_hash, storage_hash)?;

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
        storage: HashMap<H256, HashMap<H256, U256>>,
    ) -> DiffLayer {
        let mut layer = DiffLayer::new(
            self.block_hash,
            self.clone(),
            block_hash,
            state_root,
            accounts,
            storage,
        );

        layer.rebloom(self.clone(), None);

        layer
    }

    pub fn stale(&self) -> bool {
        self.stale.load(Ordering::SeqCst)
    }

    pub fn mark_stale(&self) -> bool {
        self.stale.swap(true, Ordering::SeqCst)
    }

    // Starts in a blocking task the disk layer generation.
    pub fn start_generating(self: &Arc<Self>) {
        let layer = (*self).clone();
        tokio::task::spawn_blocking(move || layer.generate());
    }

    fn generate(self: Arc<Self>) {
        // Note: this method can call blocking methods because it's run outside the main thread.

        // todo: we should be able to stop mid generation in case disk layer changes?
        self.generating.store(true, Ordering::SeqCst);
        info!("Disk layer generating");
        let start = Instant::now();

        let state_trie = self.db.open_state_trie(self.state_root);

        let account_iter = state_trie.into_iter().content().map_while(|(path, value)| {
            Some((H256::from_slice(&path), AccountState::decode(&value).ok()?))
        });

        let mut account_hashes = Vec::with_capacity(1024);
        let mut account_states = Vec::with_capacity(1024);
        let mut storage_keys: Vec<Vec<H256>> = Vec::with_capacity(64);
        let mut storage_values: Vec<Vec<U256>> = Vec::with_capacity(64);

        // buffers
        let mut keys = Vec::with_capacity(32);
        let mut values = Vec::with_capacity(32);

        // TODO: figure out optimal
        // Write to db every ACCOUNT_BATCH's accounts processed.
        const ACCOUNT_BATCH: usize = 100;

        for (hash, state) in account_iter {
            keys.clear();
            values.clear();

            account_hashes.push(hash);
            let storage_root = state.storage_root;
            account_states.push(state);

            let storage_trie = self.db.open_storage_trie(hash, storage_root);
            let storage_iter = storage_trie.into_iter().content();

            for (storage_hash, value) in storage_iter {
                keys.push(H256::from_slice(&storage_hash));
                values.push(U256::from_big_endian(&value));
            }

            storage_keys.push(keys.clone());
            storage_values.push(values.clone());

            if account_hashes.len() >= ACCOUNT_BATCH {
                self.db
                    .write_snapshot_account_batch_blocking(
                        account_hashes.clone(),
                        account_states.clone(),
                    )
                    .expect("convert into a error");
                self.db
                    .write_snapshot_storage_batches_blocking(
                        account_hashes.clone(),
                        storage_keys.clone(),
                        storage_values.clone(),
                    )
                    .expect("convert into a error");
                account_hashes.clear();
                storage_keys.clear();
                storage_values.clear();
            }
        }

        if !account_hashes.is_empty() {
            self.db
                .write_snapshot_account_batch_blocking(
                    account_hashes.clone(),
                    account_states.clone(),
                )
                .expect("convert into a error");
            self.db
                .write_snapshot_storage_batches_blocking(
                    account_hashes.clone(),
                    storage_keys.clone(),
                    storage_values.clone(),
                )
                .expect("convert into a error");
            account_hashes.clear();
            storage_keys.clear();
            storage_values.clear();
        }

        self.generating.store(false, Ordering::SeqCst);
        info!(
            "Disk layer generation complete, done in {:?}",
            start.elapsed()
        );
    }
}
