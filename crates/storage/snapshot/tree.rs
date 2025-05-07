use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

use ethrex_common::{types::AccountState, H256, U256};
use tracing::{debug, error};

use crate::api::StoreEngine;

use super::{DiskLayer, SnapshotLayer};

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
    ) {
        debug!("Creating new diff snapshot");
        if block_root == parent_root {
            // TODO: return err here
            error!("Tried to create a snaptshot cycle");
            return;
        }

        let parent = self.snapshot(parent_root);

        let parent = if let Some(parent) = parent {
            parent
        } else {
            // TODO: return err here
            error!(
                "Parent snaptshot not found, parent = {}, block = {}",
                parent_root, block_root
            );
            return;
        };

        let snap = parent.update(block_root, accounts, storage);

        self.layers.write().unwrap().insert(block_root, snap);
    }

    pub fn len(&self) -> usize {
        self.layers.read().unwrap().len()
    }
}
