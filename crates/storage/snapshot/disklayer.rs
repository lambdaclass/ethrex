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
use ethrex_rlp::decode::RLPDecode;

use super::{cache::DiskCache, difflayer::DiffLayer, error::SnapshotError, tree::Layers};

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
        }
    }
}

impl DiskLayer {
    pub fn root(&self) -> H256 {
        self.state_root
    }

    pub fn get_account(
        &self,
        hash: H256,
        _layers: &Layers,
    ) -> Result<Option<AccountState>, SnapshotError> {
        // Try to get the account from the cache.
        if let Some(value) = self.cache.accounts.get(&hash) {
            return Ok(value.clone());
        }

        // TODO: Right now we use the state trie, but the disk layer should use
        // it's own database table of snapshots for faster lookup.
        let state_trie = self.db.open_state_trie(self.state_root);

        let value = if let Some(value) = state_trie
            .get(hash)
            .ok()
            .flatten()
            .map(|x| AccountState::decode(&x))
        {
            value
        } else {
            self.cache.accounts.insert(hash, None);
            return Ok(None);
        };

        let value: AccountState = value?;

        self.cache.accounts.insert(hash, value.clone().into());

        Ok(Some(value))
    }

    pub fn get_storage(
        &self,
        account_hash: H256,
        storage_hash: H256,
        layers: &Layers,
    ) -> Result<Option<U256>, SnapshotError> {
        // Look into the cache first.
        if let Some(value) = self.cache.storages.get(&(account_hash, storage_hash)) {
            return Ok(value);
        }

        let account = if let Some(account) = self.get_account(account_hash, layers)? {
            account
        } else {
            self.cache
                .storages
                .insert((account_hash, storage_hash), None);
            return Ok(None);
        };

        // TODO: Right now we use the storage trie, but the disk layer should use
        // it's own database table of snapshots for faster lookup.

        let storage_trie = self
            .db
            .open_storage_trie(account_hash, account.storage_root);

        let value = if let Some(value) = storage_trie.get(storage_hash).ok().flatten() {
            value
        } else {
            self.cache
                .storages
                .insert((account_hash, storage_hash), None);
            return Ok(None);
        };
        let value: U256 = U256::decode(&value)?;

        self.cache
            .storages
            .insert((account_hash, storage_hash), Some(value));

        Ok(Some(value))
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

    pub async fn start_generating(self: &Arc<Self>) {
        tokio::spawn(self.clone().generate());
    }

    async fn generate(self: Arc<Self>) {
        // todo: we should be able to stop mid generation in case disk layer changes?
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
                    .write_snapshot_account_batch(account_hashes.clone(), account_states.clone())
                    .await
                    .expect("convert into a error");
                self.db
                    .write_snapshot_storage_batches(
                        account_hashes.clone(),
                        storage_keys.clone(),
                        storage_values.clone(),
                    )
                    .await
                    .expect("convert into a error");
                account_hashes.clear();
                storage_keys.clear();
                storage_values.clear();
            }
        }

        if !account_hashes.is_empty() {
            self.db
                .write_snapshot_account_batch(account_hashes.clone(), account_states.clone())
                .await
                .expect("convert into a error");
            self.db
                .write_snapshot_storage_batches(
                    account_hashes.clone(),
                    storage_keys.clone(),
                    storage_values.clone(),
                )
                .await
                .expect("convert into a error");
            account_hashes.clear();
            storage_keys.clear();
            storage_values.clear();
        }
    }
}
