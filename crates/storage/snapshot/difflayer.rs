// Inspired by https://github.com/ethereum/go-ethereum/blob/f21adaf245e320a809f9bb6ec96c330726c9078f/core/state/snapshot/difflayer.go

use std::{
    collections::HashMap,
    sync::{atomic::AtomicBool, Arc},
};

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
    // Origin (disk layer) block hash
    pub(crate) origin: H256,
    /// parent block hash
    parent: H256,
    block_hash: BlockHash,
    state_root: H256,
    stale: bool,
    accounts: HashMap<H256, Option<AccountState>>, // None if deleted
    storage: HashMap<H256, HashMap<H256, Option<U256>>>,
    /// tracks all diffed items up to disk layer
    pub(crate) diffed: Bloom,
}

impl DiffLayer {
    pub fn new(
        parent: H256,
        origin: H256,
        block_hash: BlockHash,
        state_root: H256,
        accounts: HashMap<H256, Option<AccountState>>,
        storage: HashMap<H256, HashMap<H256, Option<U256>>>,
    ) -> Self {
        DiffLayer {
            origin,
            parent,
            block_hash,
            state_root,
            stale: false,
            accounts,
            storage,
            diffed: Bloom::default(),
        }
    }

    /// Recreates the bloom filter of this layer, either using the parent diff filter as base or a new one.
    pub fn rebloom(&mut self, origin: H256, parent_diffed: Option<Bloom>) {
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
            return match &layers[&self.origin] {
                Layer::DiskLayer(disk_layer) => disk_layer.get_account(hash),
                Layer::DiffLayer(_) => unreachable!(),
            };
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
            return Ok(*value);
        }

        let bloom_hash = account_hash ^ storage_hash;
        let hit = self
            .diffed
            .contains_input(BloomInput::Hash(bloom_hash.as_fixed_bytes()));

        // If bloom misses we can skip diff layers
        if !hit {
            return match &layers[&self.origin] {
                Layer::DiskLayer(disk_layer) => disk_layer.get_storage(account_hash, storage_hash),
                Layer::DiffLayer(_) => unreachable!(),
            };
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
        accounts: HashMap<H256, Option<AccountState>>,
        storage: HashMap<H256, HashMap<H256, Option<U256>>>,
    ) -> DiffLayer {
        let mut layer = DiffLayer::new(
            self.block_hash,
            self.origin,
            block,
            state_root,
            accounts,
            storage,
        );

        layer.rebloom(self.origin, Some(self.diffed));

        layer
    }

    /// Merges the diff into the disk layer.
    ///
    /// Returning a new disk layer whose block hash is the diff block hash.
    ///
    /// Returns Err if the current disk layer is already marked stale.
    pub fn save_to_disk(
        &self,
        layers: &HashMap<H256, Layer>,
    ) -> Result<Arc<DiskLayer>, SnapshotError> {
        let prev_disk = match &layers[&self.origin()] {
            Layer::DiskLayer(disk_layer) => disk_layer.clone(),
            Layer::DiffLayer(_) => unreachable!(),
        };

        if prev_disk.mark_stale() {
            return Err(SnapshotError::StaleSnapshot);
        }

        // TODO: here we should save the diff layers to the db (in the future snapshots table) too.
        let accounts = self.accounts();

        let mut account_hashes = Vec::with_capacity(accounts.len());
        let mut account_states = Vec::with_capacity(accounts.len());

        for (hash, acc) in accounts.iter() {
            if let Some(acc) = acc {
                // TODO: Important, if acc is None it means it comes from a account update
                // with the removed flag, should we remove it from db too?
                account_hashes.push(*hash);
                account_states.push(acc.clone());
                prev_disk.cache.accounts.insert(*hash, Some(acc.clone()));
            } else {
                prev_disk.cache.accounts.remove(hash);
            }
        }

        prev_disk
            .db
            .write_snapshot_account_batch_blocking(account_hashes, account_states)
            .map_err(|e| SnapshotError::StoreError(Box::new(e)))?;

        let storage = self.storage();

        let mut account_hashes = Vec::with_capacity(storage.len());
        let mut storage_keys = Vec::with_capacity(storage.len());
        let mut storage_values = Vec::with_capacity(storage.len());

        for (account_hash, storage) in storage.iter() {
            account_hashes.push(*account_hash);
            let mut keys = Vec::new();
            let mut values = Vec::new();
            for (storage_hash, value) in storage.iter() {
                // TODO: Important, if acc is None it means it had a value of zero should we remove it from db too?
                match *value {
                    Some(v) => {
                        prev_disk
                            .cache
                            .storages
                            .insert((*account_hash, *storage_hash), Some(v));
                    }
                    // FIXME(fkrause98): Right now, None changes will be
                    // written as a U256::zero(), which is not **wrong**, but we
                    // can do better: A None value should be removed from the
                    // snapshot disk storage.
                    None => {
                        prev_disk
                            .cache
                            .storages
                            .remove(&(*account_hash, *storage_hash));
                    }
                }
                values.push(value.unwrap_or_else(U256::zero));
                keys.push(*storage_hash);
            }
            storage_values.push(values);
            storage_keys.push(keys);
        }

        prev_disk
            .db
            .write_snapshot_storage_batches_blocking(account_hashes, storage_keys, storage_values)
            .map_err(|e| SnapshotError::StoreError(Box::new(e)))?;

        let disk = DiskLayer {
            db: prev_disk.db.clone(),
            cache: prev_disk.cache.clone(),
            block_hash: self.block_hash(),
            state_root: self.root(),
            stale: Arc::new(AtomicBool::new(false)),
        };
        Ok(Arc::new(disk))
    }

    pub fn origin(&self) -> H256 {
        self.origin
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
            return Ok(value.clone());
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
            return Ok(*value);
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

    pub fn add_accounts(&mut self, accounts: HashMap<H256, Option<AccountState>>) {
        self.accounts.extend(accounts);
    }

    pub fn add_storage(&mut self, storage: HashMap<H256, HashMap<H256, Option<U256>>>) {
        for (address, st) in storage.iter() {
            let entry = self.storage.entry(*address).or_default();
            entry.extend(st);
        }
    }

    pub fn accounts(&self) -> HashMap<H256, Option<AccountState>> {
        self.accounts.clone()
    }

    pub fn storage(&self) -> HashMap<H256, HashMap<H256, Option<U256>>> {
        self.storage.clone()
    }

    pub fn set_parent(&mut self, parent: H256) {
        self.parent = parent;
    }
}
