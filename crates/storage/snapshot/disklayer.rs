use core::fmt;
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use ethrex_common::{types::AccountState, H256, U256};
use ethrex_rlp::decode::RLPDecode;
use ethrex_trie::Trie;

use crate::{api::StoreEngine, cache::Cache, rlp::AccountStateRLP};

use super::{difflayer::DiffLayer, error::SnapshotError, tree::Layers};

#[derive(Clone)]
pub struct DiskLayer {
    pub(super) state_trie: Arc<Trie>,
    pub(super) db: Arc<dyn StoreEngine>,
    pub(super) cache: Cache,
    pub(super) root: H256,
    pub(super) stale: Arc<AtomicBool>,
}

impl fmt::Debug for DiskLayer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DiskLayer")
            .field("db", &self.db)
            .field("cache", &self.cache)
            .field("root", &self.root)
            .field("stale", &self.stale)
            .finish_non_exhaustive()
    }
}

impl DiskLayer {
    pub fn new(db: Arc<dyn StoreEngine>, root: H256) -> Self {
        let trie = Arc::new(db.open_state_trie(root));

        Self {
            state_trie: trie,
            root,
            db,
            cache: Cache::new(10000, 10000),
            stale: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl DiskLayer {
    pub fn root(&self) -> H256 {
        self.root
    }

    pub fn get_account(
        &self,
        hash: H256,
        _layers: &Layers,
    ) -> Result<Option<Option<AccountState>>, SnapshotError> {
        if let Some(value) = self.cache.accounts.get(&hash) {
            return Ok(Some(value.clone()));
        }

        let value = if let Some(value) = self
            .state_trie
            .get(hash)
            .ok()
            .flatten()
            .map(AccountStateRLP::from_bytes)
        {
            value
        } else {
            return Ok(None);
        };

        let value: AccountState = value.to();

        self.cache.accounts.insert(hash, value.clone().into());

        Ok(Some(Some(value)))
    }

    pub fn get_storage(
        &self,
        account_hash: H256,
        storage_hash: H256,
        layers: &Layers,
    ) -> Result<Option<U256>, SnapshotError> {
        if let Some(value) = self.cache.storages.get(&(account_hash, storage_hash)) {
            return Ok(Some(value));
        }

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

    pub fn parent(&self) -> Option<H256> {
        None
    }

    pub fn update(
        self: Arc<Self>, // import self is like this
        block: H256,
        accounts: HashMap<H256, Option<AccountState>>,
        storage: HashMap<H256, HashMap<H256, U256>>,
    ) -> DiffLayer {
        let mut layer = DiffLayer::new(self.root, self.clone(), block, accounts, storage);

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
