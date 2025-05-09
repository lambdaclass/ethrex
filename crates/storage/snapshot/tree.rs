use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

use ethrex_common::{types::AccountState, H256, U256};
use tracing::{debug, error};

use crate::api::StoreEngine;

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
    pub fn cap(&self, root: H256, layers: usize) {
        let layer = if let Some(layer) = self.snapshot(root) {
            layer
        } else {
            return;
        };
    }
}
