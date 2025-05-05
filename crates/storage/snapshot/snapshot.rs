use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

use ethrex_common::{types::AccountState, H256};
use ethrex_trie::TrieDB;

use crate::{cache::Cache, Store};

#[derive(Debug)]
pub struct SnapshotTree {
    store: Store,
    cache: Cache,
    layers: Arc<RwLock<HashMap<H256, SnapshotLayer>>>,
}

// "disk layer"
#[derive(Debug)]
pub struct SnapshotLayer {
    state_root: H256,
    parent: Option<Box<SnapshotLayer>>,

    accounts: HashMap<H256, AccountState>,
    storages: HashMap<(H256, H256), Vec<u8>>,
}

impl SnapshotTree {
    pub fn new(store: Store) -> Self {
        // TODO: load existing snapshot from disk

        Self {
            store,
            cache: Cache::new(10_000, 50_000),
            layers: Default::default(),
        }
    }

    pub fn add(
        &self,
        block_hash: H256,
        parent_hash: H256,
        accounts: HashMap<H256, AccountState>,
        storages: HashMap<(H256, H256), H256>,
    ) {
    }
}

impl SnapshotLayer {
    pub fn new(store: Store, state_root: H256) {}

    pub fn get_account(&self, hash: H256) -> Option<&AccountState> {
        self.accounts.get(&hash)
    }

    pub fn get_storage(&self, account_hash: H256, storage_hash: H256) -> Option<&Vec<u8>> {
        self.storages.get(&(account_hash, storage_hash))
    }
}
