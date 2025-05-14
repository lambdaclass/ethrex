use core::fmt;
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use crate::{api::StoreEngine, cache::Cache};
use ethrex_common::{
    types::{AccountState, BlockHash},
    H256, U256,
};
use ethrex_rlp::decode::RLPDecode;
use tracing::debug;

use super::{difflayer::DiffLayer, error::SnapshotError, tree::Layers};

#[derive(Clone)]
pub struct DiskLayer {
    pub(super) db: Arc<dyn StoreEngine>,
    pub(super) cache: Cache,
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
            cache: Cache::new(10000, 10000),
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
    ) -> Result<Option<Option<AccountState>>, SnapshotError> {
        debug!("get_account disk layer hit for {}", hash);
        if let Some(value) = self.cache.accounts.get(&hash) {
            debug!("get_account disk layer cache hit for {}", hash);
            return Ok(Some(value.clone()));
        }
        debug!("get_account disk layer cache miss for {}", hash);

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

        Ok(Some(Some(value)))
    }

    pub fn get_storage(
        &self,
        account_hash: H256,
        storage_hash: H256,
        layers: &Layers,
    ) -> Result<Option<U256>, SnapshotError> {
        debug!(
            "get_storage disk layer hit for {} / {}",
            account_hash, storage_hash
        );
        if let Some(value) = self.cache.storages.get(&(account_hash, storage_hash)) {
            debug!(
                "get_storage disk layer cache hit for {} / {}",
                account_hash, storage_hash
            );
            return Ok(Some(value));
        }
        debug!(
            "get_storage disk layer cache miss for {} / {}",
            account_hash, storage_hash
        );

        let account = if let Some(Some(account)) = self.get_account(account_hash, layers)? {
            account
        } else {
            return Ok(None);
        };

        let storage_trie = self
            .db
            .open_storage_trie(account_hash, account.storage_root);

        let value = if let Some(value) = storage_trie.get(storage_hash).ok().flatten() {
            value
        } else {
            return Ok(None);
        };
        let value: U256 = U256::decode(&value)?;

        self.cache
            .storages
            .insert((account_hash, storage_hash), value);

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

        layer.rebloom(self.clone());

        layer
    }

    pub fn stale(&self) -> bool {
        self.stale.load(Ordering::SeqCst)
    }

    pub fn mark_stale(&self) -> bool {
        self.stale.swap(true, Ordering::SeqCst)
    }
}
