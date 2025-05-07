use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use ethrex_common::{types::AccountState, Bloom, BloomInput, H256, U256};
use tracing::debug;

use super::{DiskLayer, SnapshotLayer};

#[derive(Clone)]
pub struct DiffLayer {
    origin: Arc<DiskLayer>,
    parent: Arc<dyn SnapshotLayer>,
    root: H256,
    stale: Arc<AtomicBool>,
    accounts: Arc<HashMap<H256, Option<AccountState>>>, // None if deleted
    storage: Arc<HashMap<H256, HashMap<H256, U256>>>,
    /// tracks all diffed items up to disk layer
    diffed: Bloom,
}

impl DiffLayer {
    pub fn new(
        parent: Arc<dyn SnapshotLayer>,
        root: H256,
        accounts: HashMap<H256, Option<AccountState>>,
        storage: HashMap<H256, HashMap<H256, U256>>,
    ) -> Self {
        let mut layer = DiffLayer {
            origin: parent.origin(),
            parent,
            root,
            stale: AtomicBool::new(false).into(),
            accounts: Arc::new(accounts),
            storage: Arc::new(storage),
            diffed: Bloom::zero(),
        };

        layer.rebloom();

        layer
    }
}

impl DiffLayer {
    pub fn rebloom(&mut self) {
        self.diffed = self.parent.diffed().unwrap_or_default();

        for hash in self.accounts.keys() {
            self.diffed.accrue(BloomInput::Hash(hash.as_fixed_bytes()));
        }

        for (hash, slots) in self.storage.iter() {
            for slot in slots.keys() {
                let value = hash ^ slot;
                self.diffed.accrue(BloomInput::Hash(value.as_fixed_bytes()));
            }
        }
    }
}

impl SnapshotLayer for DiffLayer {
    fn root(&self) -> H256 {
        self.root
    }

    fn diffed(&self) -> Option<Bloom> {
        Some(self.diffed)
    }

    fn get_account(&self, hash: H256) -> Option<Option<AccountState>> {
        // todo: check stale

        let hit = self
            .diffed
            .contains_input(BloomInput::Hash(hash.as_fixed_bytes()));

        // If bloom misses we can skip diff layers
        if !hit {
            return self.origin.get_account(hash);
        }

        // Start traversing layers.
        self.get_account_traverse(hash, 0)
    }

    fn get_storage(&self, account_hash: H256, storage_hash: H256) -> Option<ethrex_common::U256> {
        // todo: check stale

        let bloom_hash = account_hash ^ storage_hash;
        let hit = self
            .diffed
            .contains_input(BloomInput::Hash(bloom_hash.as_fixed_bytes()));

        // If bloom misses we can skip diff layers
        if !hit {
            return self.origin.get_storage(account_hash, storage_hash);
        }

        // Start traversing layers.
        self.get_storage_traverse(account_hash, storage_hash, 0)
    }

    fn stale(&self) -> bool {
        self.stale.load(Ordering::Acquire)
    }

    fn mark_stale(&self) {
        self.stale.store(true, Ordering::SeqCst);
    }

    fn parent(&self) -> Option<Arc<dyn SnapshotLayer>> {
        Some(self.parent.clone())
    }

    fn update(
        &self,
        block: ethrex_common::H256,
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

    fn origin(&self) -> Arc<DiskLayer> {
        self.origin.clone()
    }

    // skips bloom checks, used if a higher layer bloom filter is hit
    fn get_account_traverse(&self, hash: H256, depth: usize) -> Option<Option<AccountState>> {
        // todo: check if its stale

        // If it's in this layer, return it.
        if let Some(value) = self.accounts.get(&hash) {
            debug!("Snapshot DiffLayer get_account_traverse hit at depth {depth}");
            return Some(value.clone());
        }

        // delegate to parent
        self.parent.get_account_traverse(hash, depth + 1)
    }

    fn get_storage_traverse(
        &self,
        account_hash: H256,
        storage_hash: H256,
        depth: usize,
    ) -> Option<U256> {
        // todo: check if its stale

        // If it's in this layer, return it.
        if let Some(value) = self
            .storage
            .get(&account_hash)
            .and_then(|x| x.get(&storage_hash))
        {
            debug!("Snapshot DiffLayer get_storage_traverse hit at depth {depth}");
            return Some(*value);
        }

        // delegate to parent
        self.parent
            .get_storage_traverse(account_hash, storage_hash, depth + 1)
    }
}
