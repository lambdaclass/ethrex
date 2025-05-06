use std::{collections::HashMap, sync::Arc};

use ethrex_common::{types::AccountState, Bloom, H256, U256};

use super::DiskLayer;

// Snapshot layer methods.
pub trait SnapshotLayer: Send + Sync {
    /// Root hash for this snapshot.
    fn root(&self) -> H256;

    /// Get a account state  by its hash.
    ///
    /// Returned inner Option is None if deleted.
    fn get_account(&self, hash: H256) -> Option<Option<AccountState>>;

    /// Get a storage by its account and storage hash.
    fn get_storage(&self, account_hash: H256, storage_hash: H256) -> Option<U256>;

    // TODO: maybe move these to a private trait.

    fn stale(&self) -> bool;

    fn parent(&self) -> Option<Arc<dyn SnapshotLayer>>;

    /// Creates a new layer on top of the existing diff tree.
    fn update(
        &self,
        block: H256,
        accounts: HashMap<H256, Option<AccountState>>,
        storage: HashMap<H256, HashMap<H256, U256>>,
    ) -> Arc<dyn SnapshotLayer>;

    fn origin(&self) -> Arc<DiskLayer>;

    fn diffed(&self) -> Option<Bloom>;

    // skips bloom checks, used if a higher layer bloom filter is hit
    fn get_account_traverse(&self, hash: H256, depth: usize) -> Option<Option<AccountState>>;

    fn get_storage_traverse(
        &self,
        account_hash: H256,
        storage_hash: H256,
        depth: usize,
    ) -> Option<U256>;
}
