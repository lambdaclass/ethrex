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

use crate::{api::StoreEngine, cache::Cache, rlp::AccountStateRLP};

use super::{
    difflayer::DiffLayer,
    error::SnapshotError,
    layer::{SnapshotLayer, SnapshotLayerImpl},
};

#[derive(Clone)]
pub struct DiskLayer {
    pub(super) state_trie: Arc<Trie>,
    pub(super) db: Arc<dyn StoreEngine>,
    pub(super)  cache: Cache,
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

impl SnapshotLayer for DiskLayer {
    fn root(&self) -> H256 {
        self.root
    }

    fn get_account(&self, hash: H256) -> Result<Option<Option<AccountState>>, SnapshotError> {
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

    fn get_storage(
        &self,
        account_hash: H256,
        storage_hash: H256,
    ) -> Result<Option<U256>, SnapshotError> {
        if let Some(value) = self.cache.storages.get(&(account_hash, storage_hash)) {
            return Ok(Some(value));
        }

        let account = if let Some(Some(account)) = self.get_account(account_hash)? {
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

    fn get_account_traverse(
        &self,
        hash: H256,
        _depth: usize,
    ) -> Result<Option<Option<AccountState>>, SnapshotError> {
        self.get_account(hash)
    }

    fn get_storage_traverse(
        &self,
        account_hash: H256,
        storage_hash: H256,
        _depth: usize,
    ) -> Result<Option<U256>, SnapshotError> {
        self.get_storage(account_hash, storage_hash)
    }

    fn flatten(self: Arc<Self>) -> Arc<dyn SnapshotLayer> {
        self.clone()
    }

    fn add_accounts(&self, _accounts: HashMap<H256, Option<AccountState>>) {}

    fn add_storage(&self, _storage: HashMap<H256, HashMap<H256, U256>>) {}

    // Only used on diff layers
    fn accounts(&self) -> HashMap<H256, Option<AccountState>> {
        HashMap::default()
    }

    // Only used on diff layers
    fn storage(&self) -> HashMap<H256, HashMap<H256, U256>> {
        HashMap::default()
    }
}
