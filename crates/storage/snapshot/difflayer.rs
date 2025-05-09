use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, RwLock,
    },
};

use ethrex_common::{types::AccountState, Bloom, BloomInput, H256, U256};
use tracing::debug;

use super::{
    disklayer::DiskLayer,
    error::SnapshotError,
    layer::{SnapshotLayer, SnapshotLayerImpl},
};

#[derive(Clone, Debug)]
pub struct DiffLayer {
    origin: Arc<DiskLayer>,
    parent: Arc<dyn SnapshotLayer>,
    root: H256,
    stale: Arc<AtomicBool>,
    accounts: Arc<RwLock<HashMap<H256, Option<AccountState>>>>, // None if deleted
    storage: Arc<RwLock<HashMap<H256, HashMap<H256, U256>>>>,
    /// tracks all diffed items up to disk layer
    diffed: Bloom,
}

impl DiffLayer {
    pub fn new(
        parent: Arc<dyn SnapshotLayer>,
        root: H256,
        accounts: HashMap<H256, Option<AccountState>>,
        storage: HashMap<H256, HashMap<H256, U256>>,
        diffed: Option<Bloom>,
    ) -> Self {
        let mut layer = DiffLayer {
            origin: parent.origin(),
            parent,
            root,
            stale: AtomicBool::new(false).into(),
            accounts: Arc::new(RwLock::new(accounts)),
            storage: Arc::new(RwLock::new(storage)),
            diffed: diffed.unwrap_or_default(),
        };

        layer.rebloom();

        layer
    }
}

impl DiffLayer {
    pub fn rebloom(&mut self) {
        self.diffed = self.parent.diffed().unwrap_or_default();

        {
            let accounts = self.accounts.read().unwrap();
            for hash in accounts.keys() {
                self.diffed.accrue(BloomInput::Hash(hash.as_fixed_bytes()));
            }
        }

        {
            let storage = self.storage.read().unwrap();
            for (hash, slots) in storage.iter() {
                for slot in slots.keys() {
                    let value = hash ^ slot;
                    self.diffed.accrue(BloomInput::Hash(value.as_fixed_bytes()));
                }
            }
        }
    }
}

impl SnapshotLayer for DiffLayer {
    fn root(&self) -> H256 {
        self.root
    }

    fn get_account(&self, hash: H256) -> Result<Option<Option<AccountState>>, SnapshotError> {
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

    fn get_storage(
        &self,
        account_hash: H256,
        storage_hash: H256,
    ) -> Result<Option<U256>, SnapshotError> {
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

    fn mark_stale(&self) -> bool {
        self.stale.swap(true, Ordering::SeqCst)
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
            None,
        ))
    }

    fn origin(&self) -> Arc<DiskLayer> {
        self.origin.clone()
    }
}

impl SnapshotLayerImpl for DiffLayer {
    fn diffed(&self) -> Option<Bloom> {
        Some(self.diffed)
    }

    // skips bloom checks, used if a higher layer bloom filter is hit
    fn get_account_traverse(
        &self,
        hash: H256,
        depth: usize,
    ) -> Result<Option<Option<AccountState>>, SnapshotError> {
        // todo: check if its stale

        // If it's in this layer, return it.
        {
            let accounts = self.accounts.read().unwrap();
            if let Some(value) = accounts.get(&hash) {
                debug!("Snapshot DiffLayer get_account_traverse hit at depth {depth}");
                return Ok(Some(value.clone()));
            }
        }

        // delegate to parent
        self.parent.get_account_traverse(hash, depth + 1)
    }

    fn get_storage_traverse(
        &self,
        account_hash: H256,
        storage_hash: H256,
        depth: usize,
    ) -> Result<Option<U256>, SnapshotError> {
        // todo: check if its stale

        // If it's in this layer, return it.
        {
            let storage = self.storage.read().unwrap();
            if let Some(value) = storage
                .get(&account_hash)
                .and_then(|x| x.get(&storage_hash))
            {
                debug!("Snapshot DiffLayer get_storage_traverse hit at depth {depth}");
                return Ok(Some(*value));
            }
        }

        // delegate to parent
        self.parent
            .get_storage_traverse(account_hash, storage_hash, depth + 1)
    }

    fn flatten(self: Arc<Self>) -> Arc<dyn SnapshotLayer> {
        // If parent doesn't have a parent it means its not a diff, so we return ourselves as the last diff.
        if self.parent.parent().is_none() {
            return self;
        }

        // Flatten diff parent.
        let parent = self.parent.clone().flatten();

        if parent.mark_stale() {
            // todo: make error
            panic!("parent was stale, we flattened from different children")
        }

        parent.add_accounts(self.accounts.read().unwrap().clone());
        parent.add_storage(self.storage.read().unwrap().clone());

        Arc::new(DiffLayer::new(
            parent.parent().unwrap(),
            self.root,
            self.accounts.read().unwrap().clone(),
            self.storage.read().unwrap().clone(),
            Some(self.diffed),
        ))
    }

    fn add_accounts(&self, accounts: HashMap<H256, Option<AccountState>>) {
        let mut accounts_self = self.accounts.write().unwrap();
        accounts_self.extend(accounts);
    }

    fn add_storage(&self, storage: HashMap<H256, HashMap<H256, U256>>) {
        let mut storage_self = self.storage.write().unwrap();

        for (address, st) in storage.iter() {
            let entry = storage_self.entry(*address).or_default();
            entry.extend(st);
        }
    }
}
