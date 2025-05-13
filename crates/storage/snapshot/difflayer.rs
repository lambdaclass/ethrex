use std::{collections::HashMap, sync::Arc};

use ethrex_common::{types::AccountState, Bloom, BloomInput, H256, U256};
use tracing::debug;

use super::{
    disklayer::DiskLayer,
    error::SnapshotError,
    tree::{Layer, Layers},
};

#[derive(Clone, Debug)]
pub struct DiffLayer {
    origin: Arc<DiskLayer>,
    parent: H256,
    root: H256,
    stale: bool,
    accounts: HashMap<H256, Option<AccountState>>, // None if deleted
    storage: HashMap<H256, HashMap<H256, U256>>,
    /// tracks all diffed items up to disk layer
    diffed: Bloom,
}

impl DiffLayer {
    pub fn new(
        parent: H256,
        origin: Arc<DiskLayer>,
        root: H256,
        accounts: HashMap<H256, Option<AccountState>>,
        storage: HashMap<H256, HashMap<H256, U256>>,
        diffed: Option<Bloom>,
    ) -> Self {
        let mut layer = DiffLayer {
            origin: origin.clone(),
            parent,
            root,
            stale: false,
            accounts,
            storage,
            diffed: diffed.unwrap_or_default(),
        };

        layer.rebloom(diffed, origin);

        layer
    }
}

impl DiffLayer {
    pub fn rebloom(&mut self, parent_diffed: Option<Bloom>, new_origin: Arc<DiskLayer>) {
        self.diffed = parent_diffed.unwrap_or_default();

        // Set the new origin that triggered a rebloom.
        self.origin = new_origin;

        {
            for hash in self.accounts.keys() {
                self.diffed.accrue(BloomInput::Hash(hash.as_fixed_bytes()));
            }
        }

        {
            for (hash, slots) in self.storage.iter() {
                for slot in slots.keys() {
                    let value = hash ^ slot;
                    self.diffed.accrue(BloomInput::Hash(value.as_fixed_bytes()));
                }
            }
        }
    }
}

impl DiffLayer {
    pub fn root(&self) -> H256 {
        self.root
    }

    pub fn get_account(
        &self,
        hash: H256,
        layers: &Layers,
    ) -> Result<Option<Option<AccountState>>, SnapshotError> {
        // todo: check stale

        let hit = self
            .diffed
            .contains_input(BloomInput::Hash(hash.as_fixed_bytes()));

        // If bloom misses we can skip diff layers
        if !hit {
            return self.origin.get_account(hash, layers);
        }

        // Start traversing layers.
        self.get_account_traverse(hash, layers)
    }

    pub fn get_storage(
        &self,
        account_hash: H256,
        storage_hash: H256,
        layers: &Layers,
    ) -> Result<Option<U256>, SnapshotError> {
        // todo: check stale

        let bloom_hash = account_hash ^ storage_hash;
        let hit = self
            .diffed
            .contains_input(BloomInput::Hash(bloom_hash.as_fixed_bytes()));

        // If bloom misses we can skip diff layers
        if !hit {
            return self.origin.get_storage(account_hash, storage_hash, layers);
        }

        // Start traversing layers.
        self.get_storage_traverse(account_hash, storage_hash, 0, layers)
    }

    pub fn stale(&self) -> bool {
        self.stale
    }

    pub fn mark_stale(&mut self) -> bool {
        let old = self.stale;
        self.stale = true;
        old
    }

    pub fn parent(&self) -> H256 {
        self.parent
    }

    pub fn update(
        &self,
        block: ethrex_common::H256,
        accounts: HashMap<H256, Option<AccountState>>,
        storage: HashMap<H256, HashMap<H256, U256>>,
    ) -> DiffLayer {
        DiffLayer::new(
            self.root,
            self.origin.clone(),
            block,
            accounts,
            storage,
            Some(self.diffed),
        )
    }

    pub fn origin(&self) -> Arc<DiskLayer> {
        self.origin.clone()
    }

    pub fn diffed(&self) -> Bloom {
        self.diffed
    }

    // skips bloom checks, used if a higher layer bloom filter is hit
    pub fn get_account_traverse(
        &self,
        hash: H256,
        layers: &Layers,
    ) -> Result<Option<Option<AccountState>>, SnapshotError> {
        // todo: check if its stale

        // If it's in this layer, return it.
        if let Some(value) = self.accounts.get(&hash) {
            return Ok(Some(value.clone()));
        }

        // delegate to parent
        match &layers[&self.parent] {
            Layer::DiskLayer(disk_layer) => disk_layer.get_account(hash, layers),
            Layer::DiffLayer(diff_layer) => diff_layer
                .read()
                .unwrap()
                .get_account_traverse(hash, layers),
        }
    }

    pub fn get_storage_traverse(
        &self,
        account_hash: H256,
        storage_hash: H256,
        depth: usize,
        layers: &Layers,
    ) -> Result<Option<U256>, SnapshotError> {
        // todo: check if its stale

        // If it's in this layer, return it.
        if let Some(value) = self
            .storage
            .get(&account_hash)
            .and_then(|x| x.get(&storage_hash))
        {
            return Ok(Some(*value));
        }

        // delegate to parent
        match &layers[&self.parent] {
            Layer::DiskLayer(disk_layer) => {
                disk_layer.get_storage(account_hash, storage_hash, layers)
            }
            Layer::DiffLayer(diff_layer) => diff_layer.read().unwrap().get_storage_traverse(
                account_hash,
                storage_hash,
                depth + 1,
                layers,
            ),
        }
    }

    pub fn add_accounts(&mut self, accounts: HashMap<H256, Option<AccountState>>) {
        self.accounts.extend(accounts);
    }

    pub fn add_storage(&mut self, storage: HashMap<H256, HashMap<H256, U256>>) {
        for (address, st) in storage.iter() {
            let entry = self.storage.entry(*address).or_default();
            entry.extend(st);
        }
    }

    pub fn accounts(&self) -> HashMap<H256, Option<AccountState>> {
        self.accounts.clone()
    }

    pub fn storage(&self) -> HashMap<H256, HashMap<H256, U256>> {
        self.storage.clone()
    }

    pub fn set_parent(&mut self, parent: H256) {
        self.parent = parent;
    }
}
