use std::{collections::HashMap, sync::Arc};

use ethrex_common::{types::AccountState, H256, U256};

use crate::rlp::AccountStateRLP;

use super::DiskLayer;

// Snapshot layer methods.
pub trait SnapshotLayer: Send + Sync {
    /// Root hash for this snapshot.
    fn root(&self) -> H256;

    /// Get a account state  by its hash.
    fn get_account(&self, hash: H256) -> Option<AccountState>;

    /// Get a storage value by its account and storage hash.
    fn get_storage(&self, account_hash: H256, storage_hash: H256) -> Option<U256>;

    // TODO: maybe move these to a private trait.

    fn stale(&self) -> bool;

    fn parent(&self) -> Option<Arc<dyn SnapshotLayer>>;

    /// Creates a new layer on top of the existing diff tree.
    fn update(
        &self,
        block: H256,
        accounts: HashMap<H256, AccountState>,
        storage: HashMap<H256, HashMap<H256, U256>>,
    ) -> Arc<dyn SnapshotLayer>;

    fn origin(&self) -> Arc<DiskLayer>;
}
