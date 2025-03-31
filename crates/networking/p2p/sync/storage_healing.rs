//! This module contains the logic for storage healing during snap sync
//! It becomes active as soon as state sync begins and acts like a queue, waiting for storages in need of healing to be advertised
//! It will receive storages from the storage_fetcher queue that couldn't be downloaded due to the pivot becoming stale,
//! and also storages belonging to newly healed accounts from the state healing
//! For each storage received, the process will first queue their root nodes and then queue all the missing children from each node fetched in the same way as state healing
//! Even if the pivot becomes stale, the healer will remain active and listening until a termination signal (an empty batch) is received

use std::collections::BTreeMap;

use ethrex_common::H256;
use ethrex_storage::Store;
use ethrex_trie::{Nibbles, EMPTY_TRIE_HASH};
use tokio::sync::mpsc::Receiver;
use tracing::debug;

use crate::{peer_handler::PeerHandler, sync::node_missing_children};

use super::{SyncError, MAX_CHANNEL_READS, MAX_PARALLEL_FETCHES, NODE_BATCH_SIZE};

/// Waits for incoming hashed addresses from the receiver channel endpoint and queues the associated root nodes for state retrieval
/// Also retrieves their children nodes until we have the full storage trie stored
/// If the state becomes stale while fetching, returns its current queued account hashes
// Returns true if there are no more pending storages in the queue (aka storage healing was completed)
pub(crate) async fn storage_healer(
    state_root: H256,
    mut receiver: Receiver<Vec<H256>>,
    peers: PeerHandler,
    store: Store,
) -> Result<bool, SyncError> {
    let mut pending_paths: BTreeMap<H256, Vec<Nibbles>> = store
        .get_storage_heal_paths()?
        .unwrap_or_default()
        .into_iter()
        .collect();
    // The pivot may become stale while the fetcher is active, we will still keep the process
    // alive until the end signal so we don't lose queued messages
    let mut stale = false;
    let mut incoming = true;
    while incoming {
        // If we have enough pending storages to fill a batch
        // or if we have no more incoming batches, spawn a fetch process
        // If the pivot became stale don't process anything and just save incoming requests
        let mut storage_tasks = tokio::task::JoinSet::new();
        let mut task_num = 0;
        while !stale && !pending_paths.is_empty() && task_num < MAX_PARALLEL_FETCHES {
            let mut next_batch: BTreeMap<H256, Vec<Nibbles>> = BTreeMap::new();
            // Fill batch
            let mut batch_size = 0;
            while batch_size < NODE_BATCH_SIZE && !pending_paths.is_empty() {
                let (key, val) = pending_paths.pop_first().unwrap();
                batch_size += val.len();
                next_batch.insert(key, val);
            }
            storage_tasks.spawn(heal_storage_batch(
                state_root,
                next_batch.clone(),
                peers.clone(),
                store.clone(),
            ));
            task_num += 1;
        }
        // Add unfetched paths to queue and handle stale signal
        for res in storage_tasks.join_all().await {
            let (remaining, is_stale) = res?;
            pending_paths.extend(remaining);
            stale |= is_stale;
        }

        // Read incoming requests that are already awaiting on the receiver
        // Don't wait for requests unless we have no pending paths left
        if incoming && (!receiver.is_empty() || pending_paths.is_empty()) {
            // Fetch incoming requests
            let mut msg_buffer = vec![];
            if receiver.recv_many(&mut msg_buffer, MAX_CHANNEL_READS).await != 0 {
                for account_hashes in msg_buffer {
                    if !account_hashes.is_empty() {
                        pending_paths.extend(
                            account_hashes
                                .into_iter()
                                .map(|acc_path| (acc_path, vec![Nibbles::default()])),
                        );
                    } else {
                        // Empty message signaling no more bytecodes to sync
                        incoming = false
                    }
                }
            } else {
                // Disconnect
                incoming = false
            }
        }
    }
    let healing_complete = pending_paths.is_empty();
    // Store pending paths
    store.set_storage_heal_paths(pending_paths.into_iter().collect())?;
    Ok(healing_complete)
}

/// Receives a set of storage trie paths (grouped by their corresponding account's state trie path),
/// fetches their respective nodes, stores them, and returns their children paths and the paths that couldn't be fetched so they can be returned to the queue
/// Also returns a boolean indicating if the pivot became stale during the request
async fn heal_storage_batch(
    state_root: H256,
    mut batch: BTreeMap<H256, Vec<Nibbles>>,
    peers: PeerHandler,
    store: Store,
) -> Result<(BTreeMap<H256, Vec<Nibbles>>, bool), SyncError> {
    if let Some(mut nodes) = peers
        .request_storage_trienodes(state_root, batch.clone())
        .await
    {
        debug!("Received {} storage nodes", nodes.len());
        // Process the nodes for each account path
        for (acc_path, paths) in batch.iter_mut() {
            let mut trie = store.open_storage_trie(*acc_path, *EMPTY_TRIE_HASH);
            // Get the corresponding nodes
            let trie_nodes: Vec<ethrex_trie::Node> =
                nodes.drain(..paths.len().min(nodes.len())).collect();
            // Add children to batch
            let children = trie_nodes
                .iter()
                .zip(paths.drain(..paths.len().min(nodes.len())))
                .map(|(node, path)| node_missing_children(node, &path, trie.state()))
                .collect::<Result<Vec<_>, _>>()?;
            paths.extend(children.into_iter().flatten());
            // Write nodes to trie
            trie.state_mut().write_node_batch(&nodes)?;
            if nodes.is_empty() {
                break;
            }
        }
        // Return remaining and added paths to be added to the queue
        // Filter out the storages we completely fetched
        batch.retain(|_, v| !v.is_empty());
        return Ok((batch, false));
    }
    // Pivot became stale, lets inform the fetcher
    Ok((batch, true))
}
