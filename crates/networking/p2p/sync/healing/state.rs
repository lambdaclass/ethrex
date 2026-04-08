//! State trie healing (disabled on binary trie branch).

use crate::peer_handler::PeerHandler;
use crate::sync::AccountStorageRoots;
use crate::sync::code_collector::CodeHashCollector;
use crate::sync::SyncError;
use ethrex_common::H256;
use ethrex_storage::Store;

pub async fn heal_state_trie_wrap(
    _state_root: H256,
    _store: Store,
    _peers: &PeerHandler,
    _staleness_timestamp: u64,
    _global_leafs_healed: &mut u64,
    _storage_accounts: &mut AccountStorageRoots,
    _code_hash_collector: &mut CodeHashCollector,
) -> Result<bool, SyncError> {
    // State trie healing is not supported on the binary trie branch.
    // Snap sync uses MPT-specific trie operations that don't exist here.
    Ok(true)
}
