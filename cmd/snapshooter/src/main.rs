use std::time::{Duration, SystemTime};

use ethrex::utils::default_datadir;
use ethrex_common::{types::AccountState, H256};
use ethrex_rlp::decode::RLPDecode;
use ethrex_storage::{EngineType, Store};
use ethrex_trie::Node;

fn main() {
    // Load already synced store
    let store = Store::new(default_datadir(), EngineType::RocksDB).expect("failed to create store");

    // Retrieve pivot block header (pivot should be the last executed block).
    let pivot_header = store
        .get_block_header(1375008)
        .expect("failed to get pivot header")
        .expect("pivot header not found in store");

    // Open the account state trie
    let account_state_trie = store
        .open_direct_state_trie(pivot_header.state_root)
        .expect("failed to open account state trie on pivot header state root");

    // Retrieve account state db
    let account_state_db = account_state_trie.db();

    let mut nodes_to_write = Vec::new();

    let start = SystemTime::now();
    println!("Traversing account state trie...");
    // Traverse account state trie
    store
        .open_direct_state_trie(pivot_header.state_root)
        .expect("failed to open account state trie on pivot header state root for traversal")
        .into_iter()
        .for_each(|(path, node)| {
            // Retrieve account state node
            let Node::Leaf(node) = node else {
                return;
            };

            let path_as_key = H256::from_slice(&path.to_bytes());

            // Retrieve the account state by decoding the node
            let account_state = AccountState::decode(&node.value).unwrap_or_else(|_| {
                panic!("failed to decode account state for node in path {path_as_key:#x}")
            });

            // Open the account state storage trie
            let storage_trie = store
                .open_direct_storage_trie(path_as_key, account_state.storage_root)
                .unwrap_or_else(|_| {
                    panic!(
                        "failed to open the account storage trie for account state root {:#x} in path {path_as_key:#x}", account_state.storage_root
                    )
                });

            // Retrieve the account state storage db
            let storage_trie_db = storage_trie.db();

            let mut storages_to_write = Vec::new();

            // Traverse account state storage trie
            store
                .open_direct_storage_trie(path_as_key, account_state.storage_root)
                .unwrap_or_else(|_| {
                    panic!(
                        "failed to open the account state storage trie for account state root {:#x} with path {path_as_key:#x}",
                        account_state.storage_root
                    )
                })
                .into_iter()
                .for_each(|(path, node)| {
                    // Retrieve the account state storage node
                    let Node::Leaf(node) = node else {
                        return;
                    };

                    // Add to the list of storage nodes to store
                    storages_to_write.push((path, node.value));

                    // Store every 100k batches account storage node batches
                    if storages_to_write.len() > 100_000 {
                        storage_trie_db
                            .put_batch(std::mem::take(&mut storages_to_write))
                            .expect("failed to store account state storage nodes 100k batch");
                    }
                });

            // Store the remining account storage nodes
            storage_trie_db
                .put_batch(storages_to_write)
                .expect("failed to store the remaining account state storage nodes");

            // Add account state node to the list of account state nodes to store
            nodes_to_write.push((path, node.value));

            // Store every in 100k account state nodes batches
            if nodes_to_write.len() > 100_000 {
                account_state_db
                    .put_batch(std::mem::take(&mut nodes_to_write))
                    .expect("failed to store account state nodes 100k batch");
            }
        });
    let elapsed = start.elapsed().expect("failed to get elapsed time");
    println!(
        "Traversed account state trie in: {}",
        format_duration(elapsed)
    );

    // Store the remaining account state nodes
    account_state_db
        .put_batch(nodes_to_write)
        .expect("failed to store the remaining account state nodes");
}

fn format_duration(duration: Duration) -> String {
    let total_seconds = duration.as_secs();
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;
    let milliseconds = duration.subsec_millis();

    if hours > 0 {
        return format!("{hours:02}h {minutes:02}m {seconds:02}s {milliseconds:03}ms");
    }

    if minutes == 0 {
        return format!("{seconds:02}s {milliseconds:03}ms");
    }

    format!("{minutes:02}m {seconds:02}s")
}
