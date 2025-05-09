use core::fmt;
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use ethrex_common::{types::AccountState, Bloom, H256, U256};
use ethrex_rlp::decode::RLPDecode;
use ethrex_trie::Trie;
use tracing::debug;

use crate::{api::StoreEngine, cache::Cache, rlp::AccountStateRLP};

use super::{
    difflayer::DiffLayer,
    layer::{SnapshotLayer, SnapshotLayerImpl},
};

#[derive(Clone)]
pub struct DiskLayer {
    state_trie: Arc<Trie>,
    db: Arc<dyn StoreEngine>,
    cache: Cache,
    root: H256,
    stale: Arc<AtomicBool>,
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

impl SnapshotLayer for DiskLayer {
    fn root(&self) -> H256 {
        self.root
    }

    fn get_account(&self, hash: H256) -> Option<Option<AccountState>> {
        if let Some(value) = self.cache.accounts.get(&hash) {
            return Some(value.clone());
        }

        let value = self
            .state_trie
            .get(hash)
            .ok()
            .flatten()
            .map(AccountStateRLP::from_bytes)?;

        let value: AccountState = value.to();

        self.cache.accounts.insert(hash, value.clone().into());

        Some(Some(value))
    }

    fn get_storage(&self, account_hash: H256, storage_hash: H256) -> Option<U256> {
        if let Some(value) = self.cache.storages.get(&(account_hash, storage_hash)) {
            return Some(value);
        }

        let account = self.get_account(account_hash)??;

        let storage_trie = self
            .db
            .open_storage_trie(account_hash, account.storage_root);

        let value: U256 = U256::decode(&storage_trie.get(storage_hash).ok().flatten()?).ok()?;

        self.cache
            .storages
            .insert((account_hash, storage_hash), value);

        Some(value)
    }

    fn parent(&self) -> Option<Arc<dyn SnapshotLayer>> {
        None
    }

    fn update(
        &self,
        block: H256,
        accounts: HashMap<H256, Option<AccountState>>,
        storage: HashMap<H256, HashMap<H256, U256>>,
    ) -> Arc<dyn SnapshotLayer> {
        Arc::new(DiffLayer::new(
            Arc::new(self.clone()),
            block,
            accounts,
            storage,
            None,
        ))
    }

    fn stale(&self) -> bool {
        self.stale.load(Ordering::SeqCst)
    }

    fn mark_stale(&self) -> bool {
        self.stale.swap(true, Ordering::SeqCst)
    }

    fn origin(&self) -> Arc<DiskLayer> {
        Arc::new(self.clone())
    }
}

impl SnapshotLayerImpl for DiskLayer {
    fn diffed(&self) -> Option<Bloom> {
        None
    }

    fn get_account_traverse(&self, hash: H256, depth: usize) -> Option<Option<AccountState>> {
        debug!("Snapshot DiskLayer get_account_traverse called at depth {depth}");
        self.get_account(hash)
    }

    fn get_storage_traverse(
        &self,
        account_hash: H256,
        storage_hash: H256,
        depth: usize,
    ) -> Option<U256> {
        debug!("Snapshot DiskLayer get_storage_traverse called at depth {depth}");
        self.get_storage(account_hash, storage_hash)
    }

    fn flatten(self: Arc<Self>) -> Arc<dyn SnapshotLayer> {
        self.clone()
    }

    fn add_accounts(&self, _accounts: HashMap<H256, Option<AccountState>>) {}

    fn add_storage(&self, _storage: HashMap<H256, HashMap<H256, U256>>) {}
}
