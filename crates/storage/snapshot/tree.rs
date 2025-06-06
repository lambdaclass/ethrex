// Inspired by https://github.com/ethereum/go-ethereum/blob/f21adaf245e320a809f9bb6ec96c330726c9078f/core/state/snapshot/snapshot.go

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, RwLock},
};

use ethrex_common::{
    types::{AccountState, BlockHash},
    Address, Bloom, H256, U256,
};
use tracing::{error, info};

use crate::{api::StoreEngine, hash_address_fixed, hash_key};

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

    /// Used to add snapshot data to the database.
    ///
    /// Mainly called when initializing from the genesis.
    pub fn add_snapshot_data_to_db(
        &self,
        account_hashes: Vec<H256>,
        account_states: Vec<AccountState>,
        storage_keys: Vec<Vec<H256>>,
        storage_values: Vec<Vec<U256>>,
    ) {
        self.db
            .write_snapshot_account_batch_blocking(account_hashes.clone(), account_states)
            .expect("convert into a error");
        self.db
            .write_snapshot_storage_batches_blocking(account_hashes, storage_keys, storage_values)
            .expect("convert into a error");
    }

    /// Rebuilds the tree, marking all current layers stale, creating a new base disk layer from the given root.
    pub fn rebuild(&self, block_hash: BlockHash, state_root: H256) -> Result<(), SnapshotError> {
        let mut layers = self
            .layers
            .write()
            .map_err(|error| SnapshotError::LockError(error.to_string()))?;

        for layer in layers.values() {
            match layer {
                Layer::DiskLayer(disk_layer) => disk_layer.mark_stale(),
                Layer::DiffLayer(diff_layer) => diff_layer
                    .write()
                    .map_err(|error| SnapshotError::LockError(error.to_string()))?
                    .mark_stale(),
            };
        }

        layers.clear();
        let disk = Arc::new(DiskLayer::new(self.db.clone(), block_hash, state_root));
        layers.insert(block_hash, Layer::DiskLayer(disk.clone()));

        Ok(())
    }

    fn get_snapshot(&self, block_hash: H256) -> Option<Layer> {
        self.layers.read().unwrap().get(&block_hash).cloned()
    }

    /// Adds a new snapshot into the tree.
    ///
    /// Note: Storage keys must be hashed.
    pub fn add_snapshot(
        &self,
        block_hash: H256,
        block_state_root: H256,
        parent_block_hash: H256,
        accounts: HashMap<H256, Option<AccountState>>,
        storage: HashMap<H256, HashMap<H256, Option<U256>>>,
    ) -> Result<(), SnapshotError> {
        info!("Creating new diff snapshot");
        if block_hash == parent_block_hash {
            return Err(SnapshotError::SnapshotCycle);
        }

        if let Some(parent) = self.get_snapshot(parent_block_hash) {
            let snap = match parent {
                Layer::DiskLayer(parent) => {
                    parent.update(block_hash, block_state_root, accounts, storage)
                }
                Layer::DiffLayer(parent) => parent
                    .read()
                    .map_err(|error| SnapshotError::LockError(error.to_string()))?
                    .update(block_hash, block_state_root, accounts, storage),
            };

            self.layers
                .write()
                .map_err(|error| SnapshotError::LockError(error.to_string()))?
                .insert(block_hash, Layer::DiffLayer(Arc::new(RwLock::new(snap))));

            self.cap(block_hash, 128)?;

            Ok(())
        } else {
            error!(
                "Parent snaptshot not found, parent = {}, block = {}",
                parent_block_hash, block_hash
            );
            Err(SnapshotError::ParentSnapshotNotFound(
                parent_block_hash,
                block_hash,
            ))
        }
    }

    /// Returns how many layers the snapshot tree has.
    ///
    /// Mainly used for logging.
    pub fn len(&self) -> usize {
        self.layers.read().unwrap().len()
    }

    /// "Caps" the amount of layers, traversing downwards the snapshot tree
    /// from the head block until the number of allowed layers is passed.
    ///
    /// It's used to flatten the layers.
    fn cap(&self, head_block_hash: H256, layers_n: usize) -> Result<(), SnapshotError> {
        let diff = if let Some(diff) = self.get_snapshot(head_block_hash) {
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
            let mut layers = self
                .layers
                .write()
                .map_err(|error| SnapshotError::LockError(error.to_string()))?;
            let base = {
                Self::flatten_diff(head_block_hash, &mut layers)?
                    .read()
                    .unwrap()
                    .save_to_disk(&layers)?
            };
            layers.clear();
            layers.insert(head_block_hash, Layer::DiskLayer(base));
            return Ok(());
        }

        // Hold write lock the whole time for consistency in data.
        let mut layers = self
            .layers
            .write()
            .map_err(|error| SnapshotError::LockError(error.to_string()))?;

        let new_disk_layer = self.cap_layers(diff, layers_n, &mut layers)?;

        // Remove stale layers or ones that link into one.

        info!(
            "SnapshotTree: cap triggered, current layers: {}",
            layers.len()
        );

        let mut children: HashMap<H256, Vec<H256>> = HashMap::new();

        for (block_hash, snap) in layers.iter() {
            match snap {
                Layer::DiskLayer(_) => {}
                Layer::DiffLayer(diff_layer) => {
                    let parent = diff_layer
                        .read()
                        .map_err(|error| SnapshotError::LockError(error.to_string()))?
                        .parent();
                    let entry = children.entry(parent).or_default();
                    entry.push(*block_hash);
                }
            }
        }

        let mut to_remove: HashSet<H256> = HashSet::new();

        fn remove(
            block_hash: H256,
            children: &HashMap<H256, Vec<H256>>,
            to_remove: &mut HashSet<H256>,
        ) {
            if !to_remove.contains(&block_hash) {
                to_remove.insert(block_hash);
                if let Some(childs) = children.get(&block_hash) {
                    for child in childs {
                        remove(*child, children, to_remove);
                    }
                }
            }
        }

        for (block_hash, layer) in layers.iter() {
            match layer {
                Layer::DiskLayer(disk_layer) => {
                    if disk_layer.stale() {
                        remove(*block_hash, &children, &mut to_remove);
                    }
                }
                Layer::DiffLayer(diff_layer) => {
                    if diff_layer
                        .read()
                        .map_err(|error| SnapshotError::LockError(error.to_string()))?
                        .stale()
                    {
                        remove(*block_hash, &children, &mut to_remove);
                    }
                }
            }
        }

        for block_hash in to_remove.iter() {
            layers.remove(block_hash);
            children.remove(block_hash);
        }

        if let Some(base) = new_disk_layer {
            fn rebloom(
                block_hash: H256,
                layers: &HashMap<H256, Layer>,
                children: &HashMap<H256, Vec<H256>>,
                base: Arc<DiskLayer>,
                parent_diffed: Option<Bloom>,
            ) -> Result<(), SnapshotError> {
                if let Some(layer) = layers.get(&block_hash) {
                    let diffed = match layer {
                        Layer::DiskLayer(_) => None,
                        Layer::DiffLayer(layer) => {
                            let mut layer = layer
                                .write()
                                .map_err(|error| SnapshotError::LockError(error.to_string()))?;
                            layer.rebloom(base.block_hash, parent_diffed);
                            Some(layer.diffed)
                        }
                    };

                    if let Some(childs) = children.get(&block_hash) {
                        for child in childs {
                            rebloom(*child, layers, children, base.clone(), diffed)?;
                        }
                    }
                }
                Ok(())
            }
            info!(
                "SnapshotTree: changed disk layer block hash: {}",
                base.block_hash
            );
            rebloom(base.block_hash, &layers, &children, base, None)?;
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
                    let diff_value = diff_layer
                        .read()
                        .map_err(|error| SnapshotError::LockError(error.to_string()))?;
                    diff_wrapped = layers[&diff_value.parent()].clone();
                }
            }
        }

        let diff = match diff_wrapped {
            Layer::DiskLayer(_) => return Ok(None), // should be unreachable
            Layer::DiffLayer(diff) => diff,
        };

        let (parent, parent_block_hash) = {
            let diff_value = diff
                .read()
                .map_err(|error| SnapshotError::LockError(error.to_string()))?;
            (layers[&diff_value.parent()].clone(), diff_value.parent())
        };

        let parent = match parent {
            Layer::DiskLayer(_) => return Ok(None),
            Layer::DiffLayer(diff) => diff,
        };

        {
            // hold write lock until linked to new parent to avoid incorrect external reads
            let mut diff_value = diff
                .write()
                .map_err(|error| SnapshotError::LockError(error.to_string()))?;

            // flatten parent into grand parent.
            let flattened = Self::flatten_diff(parent_block_hash, layers)?;
            let flattened_block_hash = flattened
                .read()
                .map_err(|error| SnapshotError::LockError(error.to_string()))?
                .block_hash();
            layers.insert(flattened_block_hash, Layer::DiffLayer(flattened));
            diff_value.set_parent(flattened_block_hash);
        }

        // Persist the bottom most layer
        let base = { parent.read().unwrap().save_to_disk(layers)? };
        layers.insert(base.block_hash, Layer::DiskLayer(base.clone()));
        let mut diff_value = diff
            .write()
            .map_err(|error| SnapshotError::LockError(error.to_string()))?;
        diff_value.set_parent(base.block_hash());

        Ok(Some(base))
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
        if let Some(snapshot) = self.get_snapshot(block_hash) {
            let layers = self
                .layers
                .read()
                .map_err(|error| SnapshotError::LockError(error.to_string()))?;
            let address = hash_address_fixed(&address);

            match snapshot {
                Layer::DiskLayer(snapshot) => snapshot.get_account(address),
                Layer::DiffLayer(snapshot) => snapshot
                    .read()
                    .map_err(|error| SnapshotError::LockError(error.to_string()))?
                    .get_account(address, &layers),
            }
        } else {
            Err(SnapshotError::SnapshotNotFound(block_hash))
        }
    }

    /// Get a storage by its account and storage hash.
    ///
    /// Note: The result is valid if no Err is returned, this means Ok(None) means it doesn't really exist at all
    /// and no further checking is needed.
    ///
    /// Note: the given storage key must be a hash of the key.
    pub fn get_storage_at_hash(
        &self,
        block_hash: BlockHash,
        address: Address,
        storage_key: H256,
    ) -> Result<Option<U256>, SnapshotError> {
        if let Some(snapshot) = self.get_snapshot(block_hash) {
            let storage_key = H256::from_slice(&hash_key(&storage_key));
            let layers = self
                .layers
                .read()
                .map_err(|error| SnapshotError::LockError(error.to_string()))?;
            let address = hash_address_fixed(&address);

            let value = match snapshot {
                Layer::DiskLayer(snapshot) => snapshot.get_storage(address, storage_key)?,
                Layer::DiffLayer(snapshot) => {
                    let snapshot = snapshot
                        .read()
                        .map_err(|error| SnapshotError::LockError(error.to_string()))?;

                    snapshot.get_storage(address, storage_key, &layers)?
                }
            };

            return Ok(value);
        }

        Err(SnapshotError::SnapshotNotFound(block_hash))
    }

    pub fn flatten_diff(
        diff_block_hash: H256,
        layers: &mut HashMap<H256, Layer>,
    ) -> Result<Arc<RwLock<DiffLayer>>, SnapshotError> {
        let layer = match &layers[&diff_block_hash] {
            Layer::DiskLayer(_) => return Err(SnapshotError::DiskLayerFlatten),
            Layer::DiffLayer(diff) => diff.clone(),
        };

        // Get parent
        let parent = {
            let layer_value = layer
                .read()
                .map_err(|error| SnapshotError::LockError(error.to_string()))?;
            layers[&layer_value.parent()].clone()
        };

        // If parent is not a diff layer, layer is first in line, return layer.
        let parent = match parent {
            Layer::DiskLayer(_) => return Ok(layer),
            Layer::DiffLayer(diff) => diff,
        };

        // Flatten diff parent first.
        let parent = Self::flatten_diff(
            parent
                .read()
                .map_err(|error| SnapshotError::LockError(error.to_string()))?
                .block_hash(),
            layers,
        )?;

        let mut parent_value = parent
            .write()
            .map_err(|error| SnapshotError::LockError(error.to_string()))?;

        if parent_value.mark_stale() {
            // parent was stale, we flattened from different children
            return Err(SnapshotError::StaleSnapshot);
        }

        let layer_value = layer
            .read()
            .map_err(|error| SnapshotError::LockError(error.to_string()))?;
        parent_value.add_accounts(layer_value.accounts());
        parent_value.add_storage(layer_value.storage());

        // Return new combo parent

        let mut layer = DiffLayer::new(
            parent_value.parent(),
            parent_value.origin(),
            layer_value.block_hash(),
            layer_value.root(),
            parent_value.accounts(),
            parent_value.storage(),
        );

        layer.diffed = layer_value.diffed();

        Ok(Arc::new(RwLock::new(layer)))
    }
}

// Move the tests module outside the impl block
#[cfg(test)]
mod tests {
    use crate::store_db::in_memory::Store;

    use super::*;
    use ethrex_common::{types::AccountState, H256, U256};
    use std::sync::Arc;

    fn create_tree() -> SnapshotTree {
        SnapshotTree::new(Arc::new(Store::new()))
    }

    #[test]
    fn test_add_single_account_in_single_difflayer() {
        let tree = create_tree();
        let root = H256::from_low_u64_be(1);

        let address = Address::from_low_u64_be(1);

        let account_hash = hash_address_fixed(&address);
        let account_state = AccountState {
            nonce: 1,
            balance: U256::from(100),
            storage_root: H256::zero(),
            code_hash: H256::zero(),
        };

        // Add a disklayer to the tree
        tree.rebuild(H256::zero(), H256::zero()).unwrap();

        // Add a single account in a single difflayer
        tree.add_snapshot(
            root,
            root,
            H256::zero(),
            HashMap::from([(account_hash, Some(account_state.clone()))]),
            HashMap::new(),
        )
        .unwrap();

        // Retrieve the account and check it's there
        let retrieved_account = tree.get_account_state(root, address).unwrap();
        assert_eq!(retrieved_account, Some(account_state));
    }

    #[test]
    fn test_add_two_accounts_in_different_difflayers() {
        let tree = create_tree();
        tree.rebuild(H256::zero(), H256::zero()).unwrap();

        let root1 = H256::from_low_u64_be(1);
        let root2 = H256::from_low_u64_be(2);
        let address1 = Address::from_low_u64_be(1);
        let address2 = Address::from_low_u64_be(2);
        let account1_hash = hash_address_fixed(&address1);
        let account2_hash = hash_address_fixed(&address2);

        let account1_state = AccountState {
            nonce: 1,
            balance: U256::from(100),
            storage_root: H256::zero(),
            code_hash: H256::zero(),
        };

        let account2_state = AccountState {
            nonce: 2,
            balance: U256::from(200),
            storage_root: H256::zero(),
            code_hash: H256::zero(),
        };

        // Add the first account in the first difflayer
        tree.add_snapshot(
            root1,
            root1,
            H256::zero(),
            HashMap::from([(account1_hash, Some(account1_state.clone()))]),
            HashMap::new(),
        )
        .unwrap();

        // Add the second account in the second difflayer
        tree.add_snapshot(
            root2,
            root2,
            root1,
            HashMap::from([(account2_hash, Some(account2_state.clone()))]),
            HashMap::new(),
        )
        .unwrap();

        // Retrieve both accounts and check their values
        let retrieved_account1 = tree.get_account_state(root2, address1).unwrap();
        let retrieved_account2 = tree.get_account_state(root2, address2).unwrap();

        assert_eq!(retrieved_account1, Some(account1_state));
        assert_eq!(retrieved_account2, Some(account2_state));
    }

    #[test]
    fn test_override_account_in_second_difflayer() {
        let tree = create_tree();
        tree.rebuild(H256::zero(), H256::zero()).unwrap();
        let root1 = H256::from_low_u64_be(1);
        let root2 = H256::from_low_u64_be(2);
        let address = Address::from_low_u64_be(1);
        let account_hash = hash_address_fixed(&address);

        let account_state1 = AccountState {
            nonce: 1,
            balance: U256::from(100),
            storage_root: H256::zero(),
            code_hash: H256::zero(),
        };

        let account_state2 = AccountState {
            nonce: 2,
            balance: U256::from(200),
            storage_root: H256::zero(),
            code_hash: H256::zero(),
        };

        // Add the account in the first difflayer
        tree.add_snapshot(
            root1,
            root1,
            H256::zero(),
            HashMap::from([(account_hash, Some(account_state1.clone()))]),
            HashMap::new(),
        )
        .unwrap();

        // Override the account in the second difflayer
        tree.add_snapshot(
            root2,
            root2,
            root1,
            HashMap::from([(account_hash, Some(account_state2.clone()))]),
            HashMap::new(),
        )
        .unwrap();

        // Retrieve the account and check it returns the second value
        let retrieved_account = tree.get_account_state(root2, address).unwrap();
        assert_eq!(retrieved_account, Some(account_state2));

        // Retrieve it from the first hash and check it returns the first value
        let retrieved_account = tree.get_account_state(root1, address).unwrap();
        assert_eq!(retrieved_account, Some(account_state1));
    }

    #[test]
    fn test_override_account_storage_flattening() {
        let tree = create_tree();
        tree.rebuild(H256::zero(), H256::zero()).unwrap();
        let root1 = H256::from_low_u64_be(1);
        let root2 = H256::from_low_u64_be(2);

        let storage_root1 = H256::from_low_u64_be(0xbeef);
        let storage_root2 = H256::from_low_u64_be(0xfafa);

        let address = Address::from_low_u64_be(1);
        let account_hash = hash_address_fixed(&address);

        let account_state1 = AccountState {
            nonce: 1,
            balance: U256::from(100),
            storage_root: storage_root1,
            code_hash: H256::zero(),
        };

        let account_state2 = AccountState {
            nonce: 2,
            balance: U256::from(200),
            storage_root: storage_root2,
            code_hash: H256::zero(),
        };

        // Add the account in the first difflayer
        tree.add_snapshot(
            root1,
            root1,
            H256::zero(),
            HashMap::from([(account_hash, Some(account_state1.clone()))]),
            HashMap::from([(account_hash, {
                let mut map: HashMap<H256, Option<U256>> = HashMap::new();
                map.insert(
                    H256::from_slice(&hash_key(&H256::zero())),
                    Some(U256::one()),
                );
                map
            })]),
        )
        .unwrap();

        tree.add_snapshot(
            root2,
            root2,
            root1,
            HashMap::from([(account_hash, Some(account_state2.clone()))]),
            HashMap::from([(account_hash, {
                let mut map: HashMap<H256, Option<U256>> = HashMap::new();
                map.insert(
                    H256::from_slice(&hash_key(&H256::zero())),
                    Some(U256::zero()),
                );
                map
            })]),
        )
        .unwrap();

        tree.cap(root2, 1).unwrap();
        assert_eq!(tree.layers.read().unwrap().len(), 2);

        // Retrieve the account and check it returns the second value
        let retrieved_account = tree.get_account_state(root2, address).unwrap();
        assert_eq!(retrieved_account, Some(account_state2));

        let value = tree
            .get_storage_at_hash(root2, address, H256::zero())
            .unwrap();
        assert_eq!(value, Some(U256::zero()));

        // Retrieve it from the first hash and check it returns the first value
        let retrieved_account = tree.get_account_state(root1, address).unwrap();
        assert_eq!(retrieved_account, Some(account_state1));

        let value = tree
            .get_storage_at_hash(root1, address, H256::zero())
            .unwrap();
        assert_eq!(value, Some(U256::one()));
    }

    #[test]
    fn test_override_account_storage_in_second_difflayer() {
        let tree = create_tree();
        tree.rebuild(H256::zero(), H256::zero()).unwrap();
        let root1 = H256::from_low_u64_be(1);
        let root2 = H256::from_low_u64_be(2);

        let storage_root1 = H256::from_low_u64_be(0xbeef);
        let storage_root2 = H256::from_low_u64_be(0xfafa);

        let address = Address::from_low_u64_be(1);
        let account_hash = hash_address_fixed(&address);

        let account_state1 = AccountState {
            nonce: 1,
            balance: U256::from(100),
            storage_root: storage_root1,
            code_hash: H256::zero(),
        };

        let account_state2 = AccountState {
            nonce: 2,
            balance: U256::from(200),
            storage_root: storage_root2,
            code_hash: H256::zero(),
        };

        // Add the account in the first difflayer
        tree.add_snapshot(
            root1,
            root1,
            H256::zero(),
            HashMap::from([(account_hash, Some(account_state1.clone()))]),
            HashMap::from([(account_hash, {
                let mut map: HashMap<H256, Option<U256>> = HashMap::new();
                map.insert(
                    H256::from_slice(&hash_key(&H256::zero())),
                    Some(U256::one()),
                );
                map
            })]),
        )
        .unwrap();

        // Override the account in the second difflayer
        tree.add_snapshot(
            root2,
            root2,
            root1,
            HashMap::from([(account_hash, Some(account_state2.clone()))]),
            HashMap::from([(account_hash, {
                let mut map: HashMap<H256, Option<U256>> = HashMap::new();
                map.insert(
                    H256::from_slice(&hash_key(&H256::zero())),
                    Some(U256::zero()),
                );
                map
            })]),
        )
        .unwrap();

        // Retrieve the account and check it returns the second value
        let retrieved_account = tree.get_account_state(root2, address).unwrap();
        assert_eq!(retrieved_account, Some(account_state2));

        let value = tree
            .get_storage_at_hash(root2, address, H256::zero())
            .unwrap();
        assert_eq!(value, Some(U256::zero()));

        // Retrieve it from the first hash and check it returns the first value
        let retrieved_account = tree.get_account_state(root1, address).unwrap();
        assert_eq!(retrieved_account, Some(account_state1));

        let value = tree
            .get_storage_at_hash(root1, address, H256::zero())
            .unwrap();
        assert_eq!(value, Some(U256::one()));
    }

    // Utility function that creates account state and hashed address.
    fn acc(number: u64) -> (Address, AccountState) {
        let address = Address::from_low_u64_be(number);
        let account_state = AccountState {
            nonce: 1,
            balance: U256::from(number * 100),
            storage_root: H256::zero(),
            code_hash: H256::zero(),
        };
        (address, account_state)
    }

    #[test]
    fn a_deleted_account_should_return_none_when_querying_later_on() {
        let (address_1, acc_1) = acc(1);
        let hashed_1 = hash_address_fixed(&address_1);
        let (address_2, acc_2) = acc(2);
        let hashed_2 = hash_address_fixed(&address_2);

        let tree = create_tree();
        tree.rebuild(H256::zero(), H256::zero()).unwrap();

        // Add the account in the first difflayer
        let root1 = H256::from_low_u64_be(1);
        tree.add_snapshot(
            root1,
            root1,
            H256::zero(),
            HashMap::from([(hashed_1, Some(acc_1.clone()))]),
            HashMap::new(),
        )
        .unwrap();

        // Delete the account in the second difflayer
        let root2 = H256::from_low_u64_be(2);
        tree.add_snapshot(
            root2,
            root2,
            root1,
            HashMap::from([(hashed_1, None)]),
            HashMap::new(),
        )
        .unwrap();

        let root3 = H256::from_low_u64_be(3);
        tree.add_snapshot(
            root3,
            root3,
            root2,
            HashMap::from([(hashed_2, Some(acc_2.clone()))]),
            HashMap::new(),
        )
        .unwrap();

        // Retrieve the first account and check it returns None for the 2nd and 3rd blocks.
        let retrieved_account = tree.get_account_state(root3, address_1).unwrap();
        assert_eq!(retrieved_account, None);

        let retrieved_account = tree.get_account_state(root2, address_1).unwrap();
        assert_eq!(retrieved_account, None);

        let retrieved_account = tree.get_account_state(root1, address_1).unwrap();
        assert_eq!(retrieved_account, Some(acc_1));

        // Retrieve the other account from the latest block and check it's there.
        let retrieved_account = tree.get_account_state(root3, address_2).unwrap();
        assert_eq!(retrieved_account, Some(acc_2));

        // We now flatten the tree so that the first two layers are merged into the disk layer.
    }

    // Test that if we merge a none to the disklayer the account is removed from the disklayer.
    #[test]
    fn a_deleted_account_should_not_exist_in_disk_layer_after_flattening() {
        let (address_1, acc_1) = acc(1);
        let hashed_1 = hash_address_fixed(&address_1);

        let tree = create_tree();
        tree.rebuild(H256::zero(), H256::zero()).unwrap();

        // Add the account in the first difflayer
        let root1 = H256::from_low_u64_be(1);
        tree.add_snapshot(
            root1,
            root1,
            H256::zero(),
            HashMap::from([(hashed_1, Some(acc_1.clone()))]),
            HashMap::new(),
        )
        .unwrap();

        // Delete the account in the second difflayer
        let root2 = H256::from_low_u64_be(2);
        tree.add_snapshot(
            root2,
            root2,
            root1,
            HashMap::from([(hashed_1, None)]),
            HashMap::new(),
        )
        .unwrap();

        // Add a third snapshot with a different account
        let (address_2, acc_2) = acc(2);
        let hashed_2 = hash_address_fixed(&address_2);
        let root3 = H256::from_low_u64_be(3);

        tree.add_snapshot(
            root3,
            root3,
            root2,
            HashMap::from([(hashed_2, Some(acc_2.clone()))]),
            HashMap::new(),
        )
        .unwrap();

        // Flatten the tree
        tree.cap(root3, 1).unwrap();

        // Check that the account is not in the disklayer
        let retrieved_account = tree.get_account_state(root3, address_1).unwrap();
        assert_eq!(retrieved_account, None);

        // Check that the other account is still there
        let retrieved_account = tree.get_account_state(root3, address_2).unwrap();
        assert_eq!(retrieved_account, Some(acc_2.clone()));

        tree.cap(root3, 0).unwrap(); // Full commit to clear all difflayers

        // Check that the account is not in the disklayer
        let retrieved_account = tree.get_account_state(root3, address_1).unwrap();
        assert_eq!(retrieved_account, None);

        // Check that the other account is still there
        let retrieved_account = tree.get_account_state(root3, address_2).unwrap();
        assert_eq!(retrieved_account, Some(acc_2.clone()));
    }
}
