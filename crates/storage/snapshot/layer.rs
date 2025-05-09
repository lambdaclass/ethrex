use std::{collections::HashMap, fmt::Debug, sync::Arc};

use ethrex_common::{types::AccountState, Bloom, H256, U256};

use super::{disklayer::DiskLayer, error::SnapshotError};

// Snapshot layer methods.
pub trait SnapshotLayer: SnapshotLayerImpl + Send + Sync + Debug {
    /// Root hash for this snapshot.
    fn root(&self) -> H256;

    /// Get a account state  by its hash.
    ///
    /// Returned inner Option is None if deleted.
    fn get_account(&self, hash: H256) -> Result<Option<Option<AccountState>>, SnapshotError>;

    /// Get a storage by its account and storage hash.
    fn get_storage(
        &self,
        account_hash: H256,
        storage_hash: H256,
    ) -> Result<Option<U256>, SnapshotError>;

    // TODO: maybe move these to a private trait.

    fn stale(&self) -> bool;

    fn mark_stale(&self) -> bool;

    fn parent(&self) -> Option<Arc<dyn SnapshotLayer>>;

    /// Creates a new layer on top of the existing diff tree.
    fn update(
        &self,
        block: H256,
        accounts: HashMap<H256, Option<AccountState>>,
        storage: HashMap<H256, HashMap<H256, U256>>,
    ) -> Arc<dyn SnapshotLayer>;

    fn origin(&self) -> Arc<DiskLayer>;
}

/// Methods used internally between layers.
pub trait SnapshotLayerImpl: Send + Sync + Debug {
    fn diffed(&self) -> Option<Bloom>;

    fn set_parent(&self, parent: Arc<dyn SnapshotLayer>);

    /// Returns the accounts hashmap, expensive, used when saving to disk layer.
    fn accounts(&self) -> HashMap<H256, Option<AccountState>>;
    /// Returns the storage hashmap, expensive, used when saving to disk layer.
    fn storage(&self) -> HashMap<H256, HashMap<H256, U256>>;

    // skips bloom checks, used if a higher layer bloom filter is hit
    fn get_account_traverse(
        &self,
        hash: H256,
        depth: usize,
    ) -> Result<Option<Option<AccountState>>, SnapshotError>;

    fn get_storage_traverse(
        &self,
        account_hash: H256,
        storage_hash: H256,
        depth: usize,
    ) -> Result<Option<U256>, SnapshotError>;

    /// Flatten diff layers.
    fn flatten(self: Arc<Self>) -> Arc<dyn SnapshotLayer>;

    fn add_accounts(&self, accounts: HashMap<H256, Option<AccountState>>);

    fn add_storage(&self, storage: HashMap<H256, HashMap<H256, U256>>);
}
