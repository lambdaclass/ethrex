use std::{collections::HashMap, sync::Arc};

use ethrex_common::{
    types::{AccountState, BlockHash},
    Bloom, BloomInput, H256, U256,
};

use super::{
    disklayer::DiskLayer,
    error::SnapshotError,
    tree::{Layer, Layers},
};

#[derive(Clone, Debug)]
pub struct DiffLayer {
    origin: Arc<DiskLayer>,
    /// parent block hash
    parent: H256,
    block_hash: BlockHash,
    state_root: H256,
    stale: bool,
    accounts: HashMap<H256, AccountState>, // None if deleted
    storage: HashMap<H256, HashMap<H256, U256>>,
    /// tracks all diffed items up to disk layer
    pub(crate) diffed: Bloom,
}

impl DiffLayer {
    pub fn new(
        parent: H256,
        origin: Arc<DiskLayer>,
        block_hash: BlockHash,
        state_root: H256,
        accounts: HashMap<H256, AccountState>,
        storage: HashMap<H256, HashMap<H256, U256>>,
    ) -> Self {
        DiffLayer {
            origin: origin.clone(),
            parent,
            block_hash,
            state_root,
            stale: false,
            accounts,
            storage,
            diffed: Bloom::default(),
        }
    }
}

impl DiffLayer {
    /// Recreates the bloom filter of this layer, either using the parent diff filter as base or a new one.
    pub fn rebloom(&mut self, origin: Arc<DiskLayer>, parent_diffed: Option<Bloom>) {
        // Set the new origin that triggered a rebloom.
        self.origin = origin;

        // Use parent diffed or create new one.
        self.diffed = parent_diffed.unwrap_or_default();

        {
            for hash in self.accounts.keys() {
                self.diffed.accrue(BloomInput::Hash(hash.as_fixed_bytes()));
            }
        }

        {
            for (account_hash, slots) in self.storage.iter() {
                for storage_hash in slots.keys() {
                    let value = account_hash ^ storage_hash;
                    self.diffed.accrue(BloomInput::Hash(value.as_fixed_bytes()));
                }
            }
        }
    }
}

impl DiffLayer {
    pub fn root(&self) -> H256 {
        self.state_root
    }

    pub fn block_hash(&self) -> H256 {
        self.block_hash
    }

    pub fn get_account(
        &self,
        hash: H256,
        layers: &Layers,
    ) -> Result<Option<AccountState>, SnapshotError> {
        if self.stale {
            return Err(SnapshotError::StaleSnapshot);
        }

        let hit = self
            .diffed
            .contains_input(BloomInput::Hash(hash.as_fixed_bytes()));

        // If bloom misses we can skip diff layers
        if !hit {
            return self.origin.get_account(hash);
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
        if self.stale {
            return Err(SnapshotError::StaleSnapshot);
        }

        if let Some(value) = self
            .storage
            .get(&account_hash)
            .and_then(|x| x.get(&storage_hash))
        {
            return Ok(Some(*value));
        }

        let bloom_hash = account_hash ^ storage_hash;
        let hit = self
            .diffed
            .contains_input(BloomInput::Hash(bloom_hash.as_fixed_bytes()));

        // If bloom misses we can skip diff layers
        if !hit {
            return self.origin.get_storage(account_hash, storage_hash);
        }

        // Start traversing layers.
        self.get_storage_traverse(account_hash, storage_hash, layers)
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
        block: BlockHash,
        state_root: H256,
        accounts: HashMap<H256, AccountState>,
        storage: HashMap<H256, HashMap<H256, U256>>,
    ) -> DiffLayer {
        let mut layer = DiffLayer::new(
            self.block_hash,
            self.origin.clone(),
            block,
            state_root,
            accounts,
            storage,
        );

        layer.rebloom(self.origin.clone(), Some(self.diffed));

        layer
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
    ) -> Result<Option<AccountState>, SnapshotError> {
        if self.stale {
            return Err(SnapshotError::StaleSnapshot);
        }

        // If it's in this layer, return it.
        if let Some(value) = self.accounts.get(&hash) {
            return Ok(Some(value.clone()));
        }

        // delegate to parent
        match &layers[&self.parent] {
            Layer::DiskLayer(disk_layer) => disk_layer.get_account(hash),
            Layer::DiffLayer(diff_layer) => diff_layer
                .read()
                .map_err(|error| SnapshotError::LockError(error.to_string()))?
                .get_account_traverse(hash, layers),
        }
    }

    pub fn get_storage_traverse(
        &self,
        account_hash: H256,
        storage_hash: H256,
        layers: &Layers,
    ) -> Result<Option<U256>, SnapshotError> {
        if self.stale {
            return Err(SnapshotError::StaleSnapshot);
        }

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
            Layer::DiskLayer(disk_layer) => disk_layer.get_storage(account_hash, storage_hash),
            Layer::DiffLayer(diff_layer) => diff_layer
                .read()
                .map_err(|error| SnapshotError::LockError(error.to_string()))?
                .get_storage_traverse(account_hash, storage_hash, layers),
        }
    }

    pub fn add_accounts(&mut self, accounts: HashMap<H256, AccountState>) {
        self.accounts.extend(accounts);
    }

    pub fn add_storage(&mut self, storage: HashMap<H256, HashMap<H256, U256>>) {
        for (address, st) in storage.iter() {
            let entry = self.storage.entry(*address).or_default();
            entry.extend(st);
        }
    }

    pub fn accounts(&self) -> HashMap<H256, AccountState> {
        self.accounts.clone()
    }

    pub fn storage(&self) -> HashMap<H256, HashMap<H256, U256>> {
        self.storage.clone()
    }

    pub fn set_parent(&mut self, parent: H256) {
        self.parent = parent;
    }
}
