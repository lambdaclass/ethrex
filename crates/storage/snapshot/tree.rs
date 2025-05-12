use std::{
    collections::HashMap,
    sync::{atomic::AtomicBool, Arc, RwLock},
};

use ethrex_common::{
    types::{AccountState, BlockHash},
    Address, H256, U256,
};
use tracing::{debug, error};

use crate::{api::StoreEngine, hash_address_fixed};

use super::{disklayer::DiskLayer, error::SnapshotError, layer::SnapshotLayer};

/// It consists of one persistent base
/// layer backed by a key-value store, on top of which arbitrarily many in-memory
/// diff layers are topped.
///
/// The memory diffs can form a tree with branching, but
/// the disk layer is singleton and common to all.
///
/// The goal of a state snapshot is twofold: to allow direct access to account and
/// storage data to avoid expensive multi-level trie lookups; and to allow sorted,
/// cheap iteration of the account/storage tries for sync aid.
#[derive(Clone, Debug)]
pub struct SnapshotTree {
    db: Arc<dyn StoreEngine>,
    layers: Arc<RwLock<HashMap<H256, Arc<dyn SnapshotLayer>>>>,
}

impl SnapshotTree {
    pub fn new(db: Arc<dyn StoreEngine>) -> Self {
        SnapshotTree {
            db,
            layers: Default::default(),
        }
    }

    /// Rebuilds the tree, marking all current layers stale, creating a new base disk layer from the given root.
    pub fn rebuild(&self, root: H256) {
        // TODO: mark all layers stale

        let mut layers = self.layers.write().unwrap();

        for layer in layers.values() {
            layer.mark_stale();
        }

        layers.clear();
        layers.insert(root, Arc::new(DiskLayer::new(self.db.clone(), root)));
    }

    pub fn snapshot(&self, block_root: H256) -> Option<Arc<dyn SnapshotLayer>> {
        self.layers.read().unwrap().get(&block_root).cloned()
    }

    /// Adds a new snapshot into the tree.
    pub fn update(
        &self,
        block_root: H256,
        parent_root: H256,
        accounts: HashMap<H256, Option<AccountState>>,
        storage: HashMap<H256, HashMap<H256, U256>>,
    ) -> Result<(), SnapshotError> {
        debug!("Creating new diff snapshot");
        if block_root == parent_root {
            return Err(SnapshotError::SnapshotCycle);
        }

        let parent = self.snapshot(parent_root);

        let parent = if let Some(parent) = parent {
            parent
        } else {
            error!(
                "Parent snaptshot not found, parent = {}, block = {}",
                parent_root, block_root
            );
            return Err(SnapshotError::ParentSnapshotNotFound(
                parent_root,
                block_root,
            ));
        };

        let snap = parent.update(block_root, accounts, storage);

        self.layers.write().unwrap().insert(block_root, snap);

        Ok(())
    }

    pub fn len(&self) -> usize {
        self.layers.read().unwrap().len()
    }

    /// "Caps" the amount of layers, traversing downwards the snapshot tree
    /// from the head block until the number of allowed layers is passed.
    ///
    /// It's used to flatten the layers.
    pub fn cap(&self, root: H256, layers: usize) -> Result<(), SnapshotError> {
        let diff = if let Some(diff) = self.snapshot(root) {
            diff
        } else {
            return Err(SnapshotError::SnapshotNotFound(root));
        };

        if diff.parent().is_none() {
            return Err(SnapshotError::SnapshotIsdiskLayer(root));
        }

        if layers == 0 {
            // Full commit
            let base = self.save_diff(diff.flatten(&self.layers.read().unwrap()))?;
            // TODO: save diff to disk?
            let mut layers = self.layers.write().unwrap();
            layers.clear();
            layers.insert(root, base);
            return Ok(());
        }

        Ok(())
    }

    fn cap_layers(
        &self,
        mut diff: Arc<dyn SnapshotLayer>,
        layers_n: usize,
    ) -> Option<Arc<DiskLayer>> {
        // Dive until end or disk layer.
        let mut layers = self.layers.write().unwrap();
        for _ in 0..(layers_n - 1) {
            if diff.parent().is_some() {
                diff = layers[&diff.parent().unwrap()].clone();
            } else {
                // Diff stack is shallow, no need to modify.
                return None;
            }
        }

        let parent = layers[&diff.parent().unwrap()].clone();

        // Stop if its disk layer.
        if parent.parent().is_none() {
            return None;
        }

        let flattened = parent.flatten(&layers);
        {
            layers.insert(flattened.root(), flattened.clone());
        }

        todo!()
    }

    /// Merges the diff into the disk layer.
    fn save_diff(&self, diff: Arc<dyn SnapshotLayer>) -> Result<Arc<DiskLayer>, SnapshotError> {
        let prev_disk = diff.origin();

        if prev_disk.mark_stale() {
            return Err(SnapshotError::StaleSnapshot);
        }

        // TODO: here we should save the diff to the db (in the future snapshots table) too.
        let accounts = diff.accounts();
        for (hash, acc) in accounts.iter() {
            prev_disk.cache.accounts.insert(*hash, acc.clone());
        }

        let storage = diff.storage();
        for (hash, storage) in storage.iter() {
            for (slot, value) in storage.iter() {
                prev_disk.cache.storages.insert((*hash, *slot), *value);
            }
        }

        let trie = Arc::new(self.db.open_state_trie(diff.root()));

        let disk = DiskLayer {
            state_trie: trie,
            db: self.db.clone(),
            cache: prev_disk.cache.clone(),
            root: diff.root(),
            stale: Arc::new(AtomicBool::new(false)),
        };
        Ok(Arc::new(disk))
    }

    pub fn get_account_state(
        &self,
        block_hash: BlockHash,
        address: Address,
    ) -> Result<Option<AccountState>, SnapshotError> {
        if let Some(snapshot) = self.snapshot(block_hash) {
            let layers = self.layers.read().unwrap();
            let address = hash_address_fixed(&address);
            let result = snapshot.get_account(address, &layers);

            match result {
                Ok(Some(value)) => Ok(value),
                Err(snapshot_error) => Err(snapshot_error),
                Ok(None) => Ok(None),
            }
        } else {
            Err(SnapshotError::SnapshotNotFound(block_hash))
        }
    }

    pub fn get_storage_at_hash(
        &self,
        block_hash: BlockHash,
        address: Address,
        storage_key: H256,
    ) -> Result<Option<U256>, SnapshotError> {
        if let Some(snapshot) = self.snapshot(block_hash) {
            let layers = self.layers.read().unwrap();
            let address = hash_address_fixed(&address);
            return snapshot.get_storage(address, storage_key, &layers);
        }

        Err(SnapshotError::SnapshotNotFound(block_hash))
    }
}
