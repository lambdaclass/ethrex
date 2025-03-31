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
use tokio_util::sync::CancellationToken;
use tracing::debug;

use crate::{peer_handler::PeerHandler, sync::node_missing_children};

const MINUMUM_STORAGES_IN_QUEUE: usize = 400;

use super::{SyncError, MAX_PARALLEL_FETCHES, NODE_BATCH_SIZE};

/// Waits for incoming hashed addresses from the receiver channel endpoint and queues the associated root nodes for state retrieval
/// Also retrieves their children nodes until we have the full storage trie stored
/// If the state becomes stale while fetching, returns its current queued account hashes
// Returns true if there are no more pending storages in the queue (aka storage healing was completed)
pub(crate) async fn storage_healer(
    state_root: H256,
    peers: PeerHandler,
    store: Store,
    cancel_token: CancellationToken,
) -> Result<bool, SyncError> {
    // Retrieve a batch of pending paths from the store
    // We won't be retrieving all of them as the read can become quite long and we may not end up using all of the paths in this cycle
    let mut pending_paths: BTreeMap<H256, Vec<Nibbles>> = store
        .get_storage_heal_paths(MINUMUM_STORAGES_IN_QUEUE)?
        .into_iter()
        .collect();
    // The pivot may become stale while the fetcher is active, we will still keep the process
    // alive until the end signal so we don't lose queued messages
    let mut stale = false;
    while !(stale || cancel_token.is_cancelled()) {
        // If we have few storages in queue, fetch more from the store
        if pending_paths.len() < MINUMUM_STORAGES_IN_QUEUE {
            pending_paths.extend(
                store
                    .get_storage_heal_paths(MINUMUM_STORAGES_IN_QUEUE)?
                    .into_iter(),
            );
        }
        // If we have enough pending storages to fill a batch
        // or if we have no more incoming batches, spawn a fetch process
        // If the pivot became stale don't process anything and just save incoming requests
        let mut storage_tasks = tokio::task::JoinSet::new();
        let mut task_num = 0;
        while !pending_paths.is_empty() && task_num < MAX_PARALLEL_FETCHES {
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
            for node in nodes.drain(..paths.len().min(nodes.len())) {
                let path = paths.remove(0);
                // Add children to batch
                let children = node_missing_children(&node, &path, trie.state())?;
                paths.extend(children);
                let hash = node.compute_hash();
                trie.state_mut().write_node(node, hash)?;
            }
            // Cut the loop if we ran out of nodes
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
