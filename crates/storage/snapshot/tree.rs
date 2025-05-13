use std::{
    collections::{HashMap, HashSet},
    sync::{atomic::AtomicBool, Arc, RwLock},
};

use ethrex_common::{
    types::{AccountState, BlockHash},
    Address, H256, U256,
};
use tracing::{error, info};

use crate::{api::StoreEngine, hash_address_fixed};

use super::{difflayer::DiffLayer, disklayer::DiskLayer, error::SnapshotError};

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
    layers: Arc<RwLock<Layers>>,
}

#[derive(Debug, Clone)]
pub enum Layer {
    DiskLayer(Arc<DiskLayer>),
    DiffLayer(Arc<RwLock<DiffLayer>>),
}

pub type Layers = HashMap<H256, Layer>;

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
            match layer {
                Layer::DiskLayer(disk_layer) => disk_layer.mark_stale(),
                Layer::DiffLayer(diff_layer) => diff_layer.write().unwrap().mark_stale(),
            };
        }

        layers.clear();
        layers.insert(
            root,
            Layer::DiskLayer(Arc::new(DiskLayer::new(self.db.clone(), root))),
        );
    }

    fn snapshot(&self, block_root: H256) -> Option<Layer> {
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
        info!("Creating new diff snapshot");
        if block_root == parent_root {
            return Err(SnapshotError::SnapshotCycle);
        }

        if let Some(parent) = self.snapshot(parent_root) {
            let snap = match parent {
                Layer::DiskLayer(parent) => parent.update(block_root, accounts, storage),
                Layer::DiffLayer(parent) => {
                    parent.read().unwrap().update(block_root, accounts, storage)
                }
            };

            self.layers
                .write()
                .unwrap()
                .insert(block_root, Layer::DiffLayer(Arc::new(RwLock::new(snap))));

            Ok(())
        } else {
            error!(
                "Parent snaptshot not found, parent = {}, block = {}",
                parent_root, block_root
            );
            Err(SnapshotError::ParentSnapshotNotFound(
                parent_root,
                block_root,
            ))
        }
    }

    pub fn len(&self) -> usize {
        self.layers.read().unwrap().len()
    }

    /// "Caps" the amount of layers, traversing downwards the snapshot tree
    /// from the head block until the number of allowed layers is passed.
    ///
    /// It's used to flatten the layers.
    pub fn cap(&self, head_block_hash: H256, layers_n: usize) -> Result<(), SnapshotError> {
        let diff = if let Some(diff) = self.snapshot(head_block_hash) {
            match diff {
                Layer::DiskLayer(_) => {
                    return Err(SnapshotError::SnapshotIsdiskLayer(head_block_hash))
                }
                Layer::DiffLayer(diff) => diff,
            }
        } else {
            return Err(SnapshotError::SnapshotNotFound(head_block_hash));
        };

        if layers_n == 0 {
            // Full commit
            info!("SnapshotTree: cap full commit triggered, clearing snapshots");
            let mut layers = self.layers.write().unwrap();
            let base = self.save_diff(self.flatten_diff(head_block_hash, &mut layers)?)?;
            layers.clear();
            layers.insert(head_block_hash, Layer::DiskLayer(base));
            return Ok(());
        }

        // Hold write lock the whole time for consistency in data.
        let mut layers = self.layers.write().unwrap();

        let new_disk_layer = self.cap_layers(diff, layers_n, &mut layers)?;

        // Remove stale layers or ones that link into one.

        info!(
            "SnapshotTree: cap triggered, current layers: {}",
            layers.len()
        );

        let mut children: HashMap<H256, Vec<H256>> = HashMap::new();

        for (hash, snap) in layers.iter() {
            match snap {
                Layer::DiskLayer(_) => {}
                Layer::DiffLayer(diff_layer) => {
                    let parent = diff_layer.read().unwrap().parent();
                    let entry = children.entry(parent).or_default();
                    entry.push(*hash);
                }
            }
        }

        let mut to_remove: HashSet<H256> = HashSet::new();

        fn remove(root: H256, children: &HashMap<H256, Vec<H256>>, to_remove: &mut HashSet<H256>) {
            if !to_remove.contains(&root) {
                to_remove.insert(root);
                if let Some(childs) = children.get(&root) {
                    for child in childs {
                        remove(*child, children, to_remove);
                    }
                }
            }
        }

        for (root, snap) in layers.iter() {
            match snap {
                Layer::DiskLayer(disk_layer) => {
                    if disk_layer.stale() {
                        remove(*root, &children, &mut to_remove);
                    }
                }
                Layer::DiffLayer(diff_layer) => {
                    if diff_layer.read().unwrap().stale() {
                        remove(*root, &children, &mut to_remove);
                    }
                }
            }
        }

        for root in to_remove.iter() {
            layers.remove(root);
            children.remove(root);
        }

        if let Some(base) = new_disk_layer {
            fn rebloom(
                root: H256,
                layers: &HashMap<H256, Layer>,
                children: &HashMap<H256, Vec<H256>>,
                base: Arc<DiskLayer>,
            ) {
                if let Some(layer) = layers.get(&root) {
                    match layer {
                        Layer::DiskLayer(_) => {}
                        Layer::DiffLayer(layer) => {
                            layer.write().unwrap().rebloom(None, base.clone())
                        }
                    }
                }
                if let Some(childs) = children.get(&root) {
                    for child in childs {
                        rebloom(*child, layers, children, base.clone());
                    }
                }
            }
            info!("SnapshotTree: changed disk layer block hash: {}", base.root);
            rebloom(base.root, &layers, &children, base);
        }

        info!(
            "SnapshotTree: cap finished, current layers: {}",
            layers.len(),
        );

        Ok(())
    }

    /// Internal helper method to flatten diff layers.
    fn cap_layers(
        &self,
        diff: Arc<RwLock<DiffLayer>>,
        layers_n: usize,
        layers: &mut HashMap<H256, Layer>,
    ) -> Result<Option<Arc<DiskLayer>>, SnapshotError> {
        // Dive until end or disk layer.
        let mut diff_wrapped = Layer::DiffLayer(diff.clone());
        for _ in 0..(layers_n - 1) {
            match diff_wrapped {
                Layer::DiskLayer(_) => {
                    // Diff stack is shallow, no need to modify.
                    return Ok(None);
                }
                Layer::DiffLayer(diff_layer) => {
                    let diff_value = diff_layer.read().unwrap();
                    diff_wrapped = layers[&diff_value.parent()].clone();
                }
            }
        }

        let diff = match diff_wrapped {
            Layer::DiskLayer(_) => return Ok(None), // should be unreachable
            Layer::DiffLayer(diff) => diff,
        };

        let parent = {
            let diff_value = diff.read().unwrap();
            layers[&diff_value.parent()].clone()
        };

        let parent = match parent {
            Layer::DiskLayer(_) => return Ok(None),
            Layer::DiffLayer(diff) => diff,
        };

        {
            // hold write lock until linked to new parent to avoid incorrect external reads
            let mut diff_value = diff.write().unwrap();

            let parent_root = parent.read().unwrap().root();
            // flatten parent into grand parent.
            let flattened = self.flatten_diff(parent_root, layers)?;
            let flattened_root = match &flattened {
                Layer::DiskLayer(disk_layer) => disk_layer.root(),
                Layer::DiffLayer(diff_layer) => diff_layer.read().unwrap().root(),
            };
            layers.insert(flattened_root, flattened.clone());
            diff_value.set_parent(flattened_root);
        }

        // Persist the bottom most layer
        let base = self.save_diff(Layer::DiffLayer(parent))?;
        layers.insert(base.root, Layer::DiskLayer(base.clone()));
        let mut diff_value = diff.write().unwrap();
        diff_value.set_parent(base.root());

        Ok(Some(base))
    }

    /// Merges the diff into the disk layer.
    ///
    /// Returning a new disk layer whose root is the diff root.
    ///
    /// Returns Err if the current disk layer is already marked stale.
    fn save_diff(&self, diff: Layer) -> Result<Arc<DiskLayer>, SnapshotError> {
        let diff = match diff {
            Layer::DiskLayer(disk_layer) => {
                return Err(SnapshotError::SnapshotIsdiskLayer(disk_layer.root))
            }
            Layer::DiffLayer(diff) => diff,
        };
        let diff_value = diff.read().unwrap();
        let prev_disk = diff_value.origin();

        if prev_disk.mark_stale() {
            return Err(SnapshotError::StaleSnapshot);
        }

        // TODO: here we should save the diff to the db (in the future snapshots table) too.
        let accounts = diff_value.accounts();
        for (hash, acc) in accounts.iter() {
            prev_disk.cache.accounts.insert(*hash, acc.clone());
        }

        let storage = diff_value.storage();
        for (account_hash, storage) in storage.iter() {
            for (storage_hash, value) in storage.iter() {
                prev_disk
                    .cache
                    .storages
                    .insert((*account_hash, *storage_hash), *value);
            }
        }

        let trie = Arc::new(self.db.open_state_trie(diff_value.root()));

        let disk = DiskLayer {
            state_trie: trie,
            db: self.db.clone(),
            cache: prev_disk.cache.clone(),
            root: diff_value.root(),
            stale: Arc::new(AtomicBool::new(false)),
        };
        Ok(Arc::new(disk))
    }

    /// Get a account state by its hash.
    ///
    /// Note: The result is valid if no Err is returned, this means Ok(None) means it doesn't really exist at all
    /// and no further checking is needed.
    pub fn get_account_state(
        &self,
        block_hash: BlockHash,
        address: Address,
    ) -> Result<Option<AccountState>, SnapshotError> {
        if let Some(snapshot) = self.snapshot(block_hash) {
            let layers = self.layers.read().unwrap();
            let address = hash_address_fixed(&address);
            let result = match snapshot {
                Layer::DiskLayer(snapshot) => snapshot.get_account(address, &layers),
                Layer::DiffLayer(snapshot) => {
                    snapshot.read().unwrap().get_account(address, &layers)
                }
            };

            match result {
                Ok(Some(value)) => Ok(value),
                Err(snapshot_error) => Err(snapshot_error),
                Ok(None) => Ok(None),
            }
        } else {
            Err(SnapshotError::SnapshotNotFound(block_hash))
        }
    }

    /// Get a storage by its account and storage hash.
    ///
    /// Note: The result is valid if no Err is returned, this means Ok(None) means it doesn't really exist at all
    /// and no further checking is needed.
    pub fn get_storage_at_hash(
        &self,
        block_hash: BlockHash,
        address: Address,
        storage_key: H256,
    ) -> Result<Option<U256>, SnapshotError> {
        if let Some(snapshot) = self.snapshot(block_hash) {
            let layers = self.layers.read().unwrap();
            let address = hash_address_fixed(&address);
            return match snapshot {
                Layer::DiskLayer(snapshot) => snapshot.get_storage(address, storage_key, &layers),
                Layer::DiffLayer(snapshot) => {
                    snapshot
                        .read()
                        .unwrap()
                        .get_storage(address, storage_key, &layers)
                }
            };
        }

        Err(SnapshotError::SnapshotNotFound(block_hash))
    }

    pub fn flatten_diff(
        &self,
        diff: H256,
        layers: &mut HashMap<H256, Layer>,
    ) -> Result<Layer, SnapshotError> {
        let layer = match &layers[&diff] {
            Layer::DiskLayer(_) => return Err(SnapshotError::DiskLayerFlatten),
            Layer::DiffLayer(diff) => diff.clone(),
        };

        // If parent is not a diff layer, layer is first in line, return layer.
        let parent = {
            let layer_value = layer.read().unwrap();
            layers[&layer_value.parent()].clone()
        };

        let parent = match parent {
            Layer::DiskLayer(_) => return Ok(Layer::DiffLayer(layer)),
            Layer::DiffLayer(diff) => diff,
        };

        // Flatten diff parent first.
        let parent = match self.flatten_diff(parent.read().unwrap().root(), layers)? {
            Layer::DiskLayer(_) => unreachable!("only diff can be returned at this point"),
            Layer::DiffLayer(diff) => diff,
        };

        let mut parent_value = parent.write().unwrap();

        if parent_value.mark_stale() {
            // parent was stale, we flattened from different children
            return Err(SnapshotError::StaleSnapshot);
        }

        let layer_value = layer.read().unwrap();
        parent_value.add_accounts(layer_value.accounts());
        parent_value.add_storage(layer_value.storage());

        // Return new parent
        // TODO: new reblooms always, maybe we dont need in this case?
        Ok(Layer::DiffLayer(Arc::new(RwLock::new(DiffLayer::new(
            parent_value.parent(),
            parent_value.origin().clone(),
            layer_value.root(),
            parent_value.accounts(),
            parent_value.storage(),
            Some(layer_value.diffed()),
        )))))
    }
}
