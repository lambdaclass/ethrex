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

use crate::{
    cache::Cache, hash_key, rlp::AccountStateRLP, store::hash_address_fixed, AccountUpdate, Store,
};

use super::{difflayer::DiffLayer, layer::SnapshotLayer};

#[derive(Clone)]
pub struct DiskLayer {
    state_trie: Arc<Trie>,
    store: Store,
    cache: Cache,
    root: H256,
    stale: Arc<AtomicBool>,
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
            .store
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
        ))
    }

    fn stale(&self) -> bool {
        self.stale.load(Ordering::Acquire)
    }

    fn origin(&self) -> Arc<DiskLayer> {
        Arc::new(self.clone())
    }

    fn diffed(&self) -> Option<Bloom> {
        None
    }

    fn get_account_traverse(&self, hash: H256, _depth: usize) -> Option<Option<AccountState>> {
        self.get_account(hash)
    }
}
