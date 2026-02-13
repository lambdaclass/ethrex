//! This module contains the logic for state healing
//! State healing begins after we already downloaded the whole state trie and rebuilt it locally
//! It's purpose is to fix inconsistencies with the canonical state trie by downloading all the trie nodes that we don't have starting from the root node
//! The reason for these inconsistencies is that state download can spawn across multiple sync cycles each with a different pivot,
//! meaning that the resulting trie is made up of fragments of different state tries and is not consistent with any block's state trie
//! For each node downloaded, will add it to the trie's state and check if we have its children stored, if we don't we will download each missing child
//! Note that during this process the state trie for the pivot block and any prior pivot block will not be in a consistent state
//! This process will stop once it has fixed all trie inconsistencies or when the pivot becomes stale, in which case it can be resumed on the next cycle
//! All healed accounts will also have their bytecodes and storages healed by the corresponding processes

use std::{
    cmp::min,
    collections::{BTreeMap, HashMap},
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

use ethrex_common::{H256, constants::EMPTY_KECCACK_HASH, types::AccountState};
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};
use ethrex_storage::Store;
use ethrex_trie::{EMPTY_TRIE_HASH, Nibbles, Node, TrieDB, TrieError};
use tracing::{debug, trace};

use crate::{
    metrics::{CurrentStepValue, METRICS},
    peer_handler::{PeerHandler, RequestMetadata},
    rlpx::p2p::SUPPORTED_SNAP_CAPABILITIES,
    snap::{
        SnapError,
        constants::{MAX_IN_FLIGHT_REQUESTS, NODE_BATCH_SIZE, SHOW_PROGRESS_INTERVAL_DURATION},
        request_state_trienodes,
    },
    sync::{AccountStorageRoots, SyncError, code_collector::CodeHashCollector},
    utils::current_unix_time,
};

use super::types::{HealingQueueEntry, StateHealingQueue};

pub async fn heal_state_trie_wrap(
    state_root: H256,
    store: Store,
    peers: &PeerHandler,
    staleness_timestamp: u64,
    global_leafs_healed: &mut u64,
    storage_accounts: &mut AccountStorageRoots,
    code_hash_collector: &mut CodeHashCollector,
) -> Result<bool, SyncError> {
    let mut healing_done = false;
    METRICS.current_step.set(CurrentStepValue::HealingState);
    debug!("Starting state healing");
    while !healing_done {
        healing_done = heal_state_trie(
            state_root,
            store.clone(),
            peers.clone(),
            staleness_timestamp,
            global_leafs_healed,
            HashMap::new(),
            storage_accounts,
            code_hash_collector,
        )
        .await?;
        if current_unix_time() > staleness_timestamp {
            debug!("Stopped state healing due to staleness");
            break;
        }
    }
    debug!("Stopped state healing");
    Ok(healing_done)
}

/// Heals the trie given its state_root by fetching any missing nodes in it via p2p
/// Returns true if healing was fully completed or false if we need to resume healing on the next sync cycle
/// This method also stores modified storage roots in the db for heal_storage_trie
/// Note: downloaders only gets updated when heal_state_trie, once per snap cycle
#[allow(clippy::too_many_arguments)]
async fn heal_state_trie(
    state_root: H256,
    store: Store,
    mut peers: PeerHandler,
    staleness_timestamp: u64,
    global_leafs_healed: &mut u64,
    mut healing_queue: StateHealingQueue,
    storage_accounts: &mut AccountStorageRoots,
    code_hash_collector: &mut CodeHashCollector,
) -> Result<bool, SyncError> {
    // Add the current state trie root to the pending paths
    let mut paths: Vec<RequestMetadata> = vec![RequestMetadata {
        hash: state_root,
        path: Nibbles::default(), // We need to be careful, the root parent is a special case
        parent_path: Nibbles::default(),
    }];
    let mut last_update = Instant::now();
    let mut inflight_tasks: u64 = 0;
    let mut is_stale = false;
    let mut longest_path_seen = 0;
    let mut downloads_success = 0;
    let mut downloads_fail = 0;
    let mut leafs_healed = 0;
    let mut heals_per_cycle: u64 = 0;
    let mut nodes_to_write: Vec<(Nibbles, Node)> = Vec::new();
    let mut db_joinset = tokio::task::JoinSet::new();

    // channel to send the tasks to the peers
    let (task_sender, mut task_receiver) =
        tokio::sync::mpsc::channel::<(H256, Result<Vec<Node>, SnapError>, Vec<RequestMetadata>)>(
            1000,
        );
    // Contains both nodes and their corresponding paths to heal
    let mut nodes_to_heal = Vec::new();

    let mut logged_no_free_peers_count: u32 = 0;

    loop {
        if last_update.elapsed() >= SHOW_PROGRESS_INTERVAL_DURATION {
            let num_peers = peers
                .peer_table
                .peer_count_by_capabilities(&SUPPORTED_SNAP_CAPABILITIES)
                .await
                .unwrap_or(0);
            last_update = Instant::now();
            let downloads_rate =
                downloads_success as f64 / (downloads_success + downloads_fail) as f64;

            METRICS
                .global_state_trie_leafs_healed
                .store(*global_leafs_healed, Ordering::Relaxed);
            debug!(
                status = if is_stale { "stopping" } else { "in progress" },
                snap_peers = num_peers,
                inflight_tasks,
                longest_path_seen,
                leafs_healed,
                global_leafs_healed,
                downloads_rate,
                paths_to_go = paths.len(),
                pending_nodes = healing_queue.len(),
                heals_per_cycle,
                "State Healing",
            );
            downloads_success = 0;
            downloads_fail = 0;
        }

        // Dispatch multiple batches concurrently (up to MAX_IN_FLIGHT_REQUESTS)
        if !is_stale {
            dispatch_state_healing_batches(
                &mut paths,
                &mut inflight_tasks,
                &mut longest_path_seen,
                &mut peers,
                state_root,
                &task_sender,
                &mut logged_no_free_peers_count,
            )
            .await;
        }

        // Process all pending healed node batches
        while let Some((nodes, batch)) = nodes_to_heal.pop() {
            heals_per_cycle += 1;
            let return_paths = heal_state_batch(
                batch,
                nodes,
                store.clone(),
                &mut healing_queue,
                &mut nodes_to_write,
            )
            .inspect_err(|err| {
                debug!(error=?err, "We have found a sync error while trying to write to DB a batch")
            })?;
            paths.extend(return_paths);
        }

        let is_done = paths.is_empty() && nodes_to_heal.is_empty() && inflight_tasks == 0;

        if nodes_to_write.len() > 100_000 || is_done || is_stale {
            // PERF: reuse buffers?
            let to_write = std::mem::take(&mut nodes_to_write);
            let store = store.clone();
            // NOTE: we keep only a single task in the background to avoid out of order deletes
            if !db_joinset.is_empty() {
                db_joinset
                    .join_next()
                    .await
                    .expect("we just checked joinset is not empty")?;
            }
            db_joinset.spawn_blocking(move || {
                let mut encoded_to_write = BTreeMap::new();
                for (path, node) in to_write {
                    for i in 0..path.len() {
                        encoded_to_write.insert(path.slice(0, i), vec![]);
                    }
                    encoded_to_write.insert(path, node.encode_to_vec());
                }
                let trie_db = store
                    .open_direct_state_trie_no_wal(*EMPTY_TRIE_HASH)
                    .expect("Store should open");
                let db = trie_db.db();
                // PERF: use put_batch_no_alloc (note that it needs to remove nodes too)
                db.put_batch(encoded_to_write.into_iter().collect())
                    .expect("The put batch on the store failed");
            });
        }

        // End loop if we have no more paths to fetch nor nodes to heal and no inflight tasks
        if is_done {
            debug!("Nothing more to heal found");
            db_joinset.join_all().await;
            break;
        }

        // We check with a clock if we are stale
        if !is_stale && current_unix_time() > staleness_timestamp {
            debug!("state healing is stale");
            is_stale = true;
        }

        if is_stale && nodes_to_heal.is_empty() && inflight_tasks == 0 {
            debug!("Finished inflight tasks");
            db_joinset.join_all().await;
            break;
        }

        // Wait for a response or check staleness periodically
        if inflight_tasks > 0 {
            tokio::select! {
                Some((peer_id, response, batch)) = task_receiver.recv() => {
                    inflight_tasks -= 1;
                    match response {
                        Ok(nodes) => {
                            for (node, meta) in nodes.iter().zip(batch.iter()) {
                                if let Node::Leaf(node) = node {
                                    let account = AccountState::decode(&node.value)?;
                                    let account_hash =
                                        H256::from_slice(&meta.path.concat(&node.partial).to_bytes());

                                    if account.code_hash != *EMPTY_KECCACK_HASH {
                                        code_hash_collector.add(account.code_hash);
                                        code_hash_collector.flush_if_needed().await?;
                                    }

                                    storage_accounts.healed_accounts.insert(account_hash);
                                    let old_value = storage_accounts
                                        .accounts_with_storage_root
                                        .get_mut(&account_hash);
                                    if let Some((old_root, _)) = old_value {
                                        *old_root = None;
                                    }
                                }
                            }
                            leafs_healed += nodes
                                .iter()
                                .filter(|node| matches!(node, Node::Leaf(_)))
                                .count();
                            *global_leafs_healed += nodes
                                .iter()
                                .filter(|node| matches!(node, Node::Leaf(_)))
                                .count() as u64;
                            nodes_to_heal.push((nodes, batch));
                            downloads_success += 1;
                            peers.peer_table.record_success(&peer_id).await?;
                        }
                        Err(_) => {
                            paths.extend(batch);
                            downloads_fail += 1;
                            peers.peer_table.record_failure(&peer_id).await?;
                        }
                    }
                }
                _ = tokio::time::sleep(Duration::from_secs(1)) => {
                    // Timeout: re-check staleness and try dispatching again
                }
            }
        } else if !paths.is_empty() {
            // No peers available for dispatching, back off briefly
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
    debug!("State Healing stopped, signaling storage healer");
    // Save paths for the next cycle. If there are no paths left, clear it in case pivot becomes stale during storage
    // Send empty batch to signal that no more batches are incoming
    // bytecode_sender.send(vec![]).await?;
    // bytecode_fetcher_handle.await??;
    Ok(paths.is_empty())
}

/// Dispatches multiple state healing batches concurrently, up to MAX_IN_FLIGHT_REQUESTS.
#[allow(clippy::too_many_arguments)]
async fn dispatch_state_healing_batches(
    paths: &mut Vec<RequestMetadata>,
    inflight_tasks: &mut u64,
    longest_path_seen: &mut usize,
    peers: &mut PeerHandler,
    state_root: H256,
    task_sender: &tokio::sync::mpsc::Sender<(
        H256,
        Result<Vec<Node>, SnapError>,
        Vec<RequestMetadata>,
    )>,
    logged_no_free_peers_count: &mut u32,
) {
    while (*inflight_tasks as u32) < MAX_IN_FLIGHT_REQUESTS && !paths.is_empty() {
        let batch: Vec<RequestMetadata> =
            paths.drain(0..min(paths.len(), NODE_BATCH_SIZE)).collect();

        *longest_path_seen = usize::max(
            batch
                .iter()
                .map(|request_metadata| request_metadata.path.len())
                .max()
                .unwrap_or_default(),
            *longest_path_seen,
        );

        let Some((peer_id, connection)) = peers
            .peer_table
            .get_best_peer(&SUPPORTED_SNAP_CAPABILITIES)
            .await
            .inspect_err(
                |err| debug!(err=?err, "Error requesting a peer to perform state healing"),
            )
            .unwrap_or(None)
        else {
            // No peers available, put batch back and stop dispatching
            paths.extend(batch);
            if *logged_no_free_peers_count == 0 {
                trace!("We are missing peers in heal_state_trie");
                *logged_no_free_peers_count = 1000;
            }
            *logged_no_free_peers_count -= 1;
            break;
        };

        let tx = task_sender.clone();
        *inflight_tasks += 1;
        let peer_table = peers.peer_table.clone();

        tokio::spawn(async move {
            let response = request_state_trienodes(
                peer_id,
                connection,
                peer_table,
                state_root,
                batch.clone(),
            )
            .await;
            let _ = tx
                .send((peer_id, response, batch))
                .await
                .inspect_err(
                    |err| debug!(error=?err, "Failed to send state trie nodes response"),
                );
        });
    }
}

/// Receives a set of state trie paths, fetches their respective nodes, stores them,
/// and returns their children paths and the paths that couldn't be fetched so they can be returned to the queue
fn heal_state_batch(
    mut batch: Vec<RequestMetadata>,
    nodes: Vec<Node>,
    store: Store,
    healing_queue: &mut StateHealingQueue,
    nodes_to_write: &mut Vec<(Nibbles, Node)>, // TODO: change tuple to struct
) -> Result<Vec<RequestMetadata>, SyncError> {
    let trie = store.open_direct_state_trie(*EMPTY_TRIE_HASH)?;
    for node in nodes.into_iter() {
        let path = batch.remove(0);
        let (pending_children_count, pending_children) =
            node_pending_children(&node, &path.path, trie.db())?;
        batch.extend(pending_children);
        if pending_children_count == 0 {
            commit_node(
                node,
                &path.path,
                &path.parent_path,
                healing_queue,
                nodes_to_write,
            );
        } else {
            let entry = HealingQueueEntry {
                node: node.clone(),
                pending_children_count,
                parent_path: path.parent_path.clone(),
            };
            healing_queue.insert(path.path.clone(), entry);
        }
    }
    Ok(batch)
}

fn commit_node(
    node: Node,
    path: &Nibbles,
    parent_path: &Nibbles,
    healing_queue: &mut StateHealingQueue,
    nodes_to_write: &mut Vec<(Nibbles, Node)>,
) {
    nodes_to_write.push((path.clone(), node));

    if parent_path == path {
        return; // Case where we're saving the root
    }

    let mut healing_queue_entry = healing_queue.remove(parent_path).unwrap_or_else(|| {
        panic!("The parent should exist. Parent: {parent_path:?}, path: {path:?}")
    });

    healing_queue_entry.pending_children_count -= 1;
    if healing_queue_entry.pending_children_count == 0 {
        commit_node(
            healing_queue_entry.node,
            parent_path,
            &healing_queue_entry.parent_path,
            healing_queue,
            nodes_to_write,
        );
    } else {
        healing_queue.insert(parent_path.clone(), healing_queue_entry);
    }
}

/// Returns the partial paths to the node's children if they are not already part of the trie state
pub fn node_pending_children(
    node: &Node,
    path: &Nibbles,
    trie_state: &dyn TrieDB,
) -> Result<(usize, Vec<RequestMetadata>), TrieError> {
    let mut paths: Vec<RequestMetadata> = Vec::new();
    let mut pending_children_count: usize = 0;
    match &node {
        Node::Branch(node) => {
            for (index, child) in node.choices.iter().enumerate() {
                let child_path = path.clone().append_new(index as u8);
                if !child.is_valid() {
                    continue;
                }
                let validity = child
                    .get_node_checked(trie_state, child_path.clone())
                    .inspect_err(|_| {
                        debug!("Malformed data when doing get child of a branch node")
                    })?
                    .is_some();
                if validity {
                    continue;
                }

                pending_children_count += 1;
                paths.extend(vec![RequestMetadata {
                    hash: child.compute_hash().finalize(),
                    path: child_path,
                    parent_path: path.clone(),
                }]);
            }
        }
        Node::Extension(node) => {
            let child_path = path.concat(&node.prefix);
            if !node.child.is_valid() {
                return Ok((0, vec![]));
            }
            let validity = node
                .child
                .get_node_checked(trie_state, child_path.clone())
                .inspect_err(|_| debug!("Malformed data when doing get child of a branch node"))?
                .is_some();
            if validity {
                return Ok((0, vec![]));
            }
            pending_children_count += 1;

            paths.extend(vec![RequestMetadata {
                hash: node.child.compute_hash().finalize(),
                path: child_path,
                parent_path: path.clone(),
            }]);
        }
        _ => {}
    }
    Ok((pending_children_count, paths))
}
