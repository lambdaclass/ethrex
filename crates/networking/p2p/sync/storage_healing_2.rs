use std::time::{Duration, Instant};

use crate::peer_handler::PeerHandler;
use ethrex_common::H256;
use ethrex_storage::Store;
use ethrex_trie::{EMPTY_TRIE_HASH, Nibbles, Node, NodeHash};
use tracing::info;

pub const LOGGING_INTERVAL: Duration = Duration::from_secs(2);

/// This struct stores the metadata we need when we request a node
pub struct NodeRequest {
    /// What account this belongs too (so what is the storage tree)
    acc_path: Nibbles,
    /// Where in the tree is this node located
    storage_path: Nibbles,
    /// What node needs this node
    parent: H256,
    // /// What hash was requested. We can use this for validation
    // hash: H256 // this is a potential optimization, we ignore for now
}

/// This struct stores the metadata we need when we store a node in the memory bank before storing
pub struct MembatchEntry {
    /// What node needs this node
    parent: H256,
    /// What this node is
    node: Node,
    /// How many missing children this node has
    /// if this number is 0, it should be flushed to the db, not stored in memory
    missing_children_count: usize,
}

/// This algorithm 'heals' the storage trie. That is to say, it downloads data until all accounts have the storage indicated
/// by the storage root in their account state
/// We receive a list of the counts that we want to save, we heal by chunks of accounts.
/// We assume these accounts are not empty hash tries, but may or may not have their
/// Algorithmic rules:
/// - If a nodehash is present in the db, it and all of it's children are present in the db
/// - If we are missing a node, we queue to download them.
/// - When a node is downloaded:
///    - if it has no missing children, we store it in the db
///    - if the node has missing childre, we store it in our membatch, wchich is preserved between calls
pub fn storage_heal_trie(
    state_root: H256,
    account_paths: Vec<Nibbles>,
    peers: PeerHandler,
    store: Store,
    staleness_timestamp: u64,
) {
    // Logging stuff, we log during a given interval
    let mut last_update = Instant::now();

    let mut download_queue: Vec<NodeRequest> = account_paths
        .into_iter()
        .map(|acc_path| {
            NodeRequest {
                acc_path,
                storage_path: Nibbles::default(), // We need to be careful, the root parent is a special case
                parent: H256::zero(),
            }
        })
        .collect();

    loop {
        if last_update.elapsed() > LOGGING_INTERVAL {
            info!("Storage Healing");
            last_update = Instant::now();
        }

        // we request the trie nodes
        let nodes = vec![8_u8, 10]; // Temporary imaginary data
        // Then we process them
        // The coordinator for now is going to process them, although it may be more efficient if the writing to the database
        // is concurrentized
    }
}
