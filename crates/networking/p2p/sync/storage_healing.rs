//! This module contains the logic for storage healing during snap sync
//! It becomes active as soon as state sync begins and acts like a queue, waiting for storages in need of healing to be advertised
//! It will receive storages from the storage_fetcher queue that couldn't be downloaded due to the pivot becoming stale,
//! and also storages belonging to newly healed accounts from the state healing
//! For each storage received, the process will first queue their root nodes and then queue all the missing children from each node fetched in the same way as state healing
//! Even if the pivot becomes stale, the healer will remain active and listening until a termination signal (an empty batch) is received

use std::{
    collections::{BTreeMap, HashMap},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use ethrex_common::H256;
use ethrex_storage::Store;
use ethrex_trie::{EMPTY_TRIE_HASH, Nibbles, NodeHash};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info};

use crate::{peer_handler::PeerHandler, sync::node_missing_children};

/// Minumum amount of storages to keep in the storage healer queue
/// More paths will be read from the Store if the amount goes below this value
const MINUMUM_STORAGES_IN_QUEUE: usize = 400;

use super::{MAX_PARALLEL_FETCHES, NODE_BATCH_SIZE, SHOW_PROGRESS_INTERVAL_DURATION, SyncError};
use ethrex_rlp::encode::RLPEncode;

/// Waits for incoming hashed addresses from the receiver channel endpoint and queues the associated root nodes for state retrieval
/// Also retrieves their children nodes until we have the full storage trie stored
/// If the state becomes stale while fetching, returns its current queued account hashes
// Returns true if there are no more pending storages in the queue (aka storage healing was completed)
pub(crate) async fn storage_healer(
    state_root: H256,
    peers: PeerHandler,
    store: Store,
    cancel_token: CancellationToken,
    state_healing_ended: Arc<AtomicBool>,
) -> Result<bool, SyncError> {
    // List of paths in need of healing, grouped by hashed address
    let mut pending_paths = BTreeMap::<H256, Vec<Nibbles>>::new();
    let mut stale = false;
    let mut last_update = Instant::now();
    let healing_start = Instant::now();
    let mut total_healed = 0;
    while !(stale || cancel_token.is_cancelled()) {
        if true
        /*last_update.elapsed() >= SHOW_PROGRESS_INTERVAL_DURATION*/
        {
            last_update = Instant::now();
            let speed = healing_start
                .elapsed()
                .as_millis()
                .checked_div((total_healed / 100) as u128)
                .unwrap_or(9999);
            info!(
                "Storage Healing in Progress, pending paths: {}, healing speed: {}ms/100nodes",
                pending_paths.len(),
                speed
            );
        }
        // If we have few storages in queue, fetch more from the store
        // We won't be retrieving all of them as the read can become quite long and we may not end up using all of the paths in this cycle
        if pending_paths.len() < MINUMUM_STORAGES_IN_QUEUE {
            pending_paths.extend(
                store
                    .take_storage_heal_paths(MINUMUM_STORAGES_IN_QUEUE)
                    .await?
                    .into_iter(),
            );
        }
        // If we have no more pending paths even after reading from the store, and state healing has finished, cut the loop
        if pending_paths.is_empty() && state_healing_ended.load(Ordering::Relaxed) {
            break;
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
                let (key, val) = pending_paths.pop_first().ok_or(SyncError::CorruptPath)?;
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
            let (remaining, is_stale, nodes_healed) = res?;
            total_healed += nodes_healed;
            pending_paths.extend(remaining);
            stale |= is_stale;
        }
    }
    let healing_complete = pending_paths.is_empty();
    // Store pending paths
    store
        .set_storage_heal_paths(pending_paths.into_iter().collect())
        .await?;
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
) -> Result<(BTreeMap<H256, Vec<Nibbles>>, bool, usize), SyncError> {
    if let Some(mut nodes) = peers
        .request_storage_trienodes(state_root, batch.clone())
        .await
    {
        let nodes_received = nodes.len();
        debug!("Received {} storage nodes", nodes.len());
        // Sort nodes by trie & update current batch
        let mut nodes_to_commit = HashMap::new();
        for (acc_path, paths) in batch.iter_mut() {
            // TODO: check if we can do this without opening a trie
            let trie = store.open_storage_trie(*acc_path, *EMPTY_TRIE_HASH)?;
            // Collect fetched nodes for that particular trie
            let trie_nodes = nodes
                .drain(..paths.len().min(nodes.len()))
                .collect::<Vec<_>>();
            // Collect missing children paths for the fetched nodes (And also remove the fetched paths from the batch)
            let missing_children = trie_nodes
                .iter()
                .zip(paths.drain(..trie_nodes.len()))
                .map(|(node, path)| node_missing_children(node, &path, trie.db()))
                .collect::<Result<Vec<_>, _>>()?;
            // Add the missing children paths of the nodes we fetched to the batch
            paths.extend(missing_children.into_iter().flatten());
            // Push nodes to commit list
            let trie_nodes = trie_nodes
                .into_iter()
                .filter_map(|node| {
                    match node.compute_hash() {
                        hash @ NodeHash::Hashed(_) => Some((hash, node.encode_to_vec())),
                        // Filter out inline nodes
                        NodeHash::Inline(_) => None,
                    }
                })
                .collect();
            nodes_to_commit.insert(*acc_path, trie_nodes);

            if nodes.is_empty() {
                break;
            }
        }
        store.commit_storage_nodes(nodes_to_commit).await?;
        // Return remaining and added paths to be added to the queue
        // Filter out the storages we completely fetched
        batch.retain(|_, v| !v.is_empty());
        return Ok((batch, false, nodes_received));
    }
    // Pivot became stale, lets inform the fetcher
    Ok((batch, true, 0))
}
