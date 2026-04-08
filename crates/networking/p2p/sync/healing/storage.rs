//! Storage trie healing (disabled on binary trie branch).

use crate::peer_handler::PeerHandler;
use crate::sync::{AccountStorageRoots, SyncError};
use ethrex_common::H256;
use ethrex_storage::Store;
use std::collections::HashMap;

pub type StorageHealingQueue = HashMap<(), ()>;

pub async fn heal_storage_trie(
    _state_root: H256,
    _storage_accounts: &AccountStorageRoots,
    _peers: &mut PeerHandler,
    _store: Store,
    _healing_queue: StorageHealingQueue,
    _staleness_timestamp: u64,
    _global_leafs_healed: &mut u64,
) -> Result<bool, SyncError> {
    // Storage trie healing is not supported on the binary trie branch.
    // Snap sync uses MPT-specific trie operations that don't exist here.
    Ok(true)
}
