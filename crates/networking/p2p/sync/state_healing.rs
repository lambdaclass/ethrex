//! State trie healing module
//!
//! This module contains the logic for state healing:
//! - Batch child lookups to reduce DB round-trips
//! - Parallel response processing with rayon
//! - Async select-based event loop (no busy polling)
//!
//! State healing begins after downloading the whole state trie and rebuilding it locally.
//! Its purpose is to fix inconsistencies with the canonical state trie by downloading
//! all missing trie nodes starting from the root node.

use std::{
    cmp::min,
    collections::{BTreeMap, HashMap},
    sync::atomic::Ordering,
    time::Duration,
};

use ethrex_common::{constants::EMPTY_KECCACK_HASH, types::AccountState, H256};
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};
use ethrex_storage::Store;
use ethrex_trie::{Nibbles, Node, TrieDB, TrieError, EMPTY_TRIE_HASH};
use rayon::prelude::*;
use tokio::time::Instant;
use tracing::{debug, trace, warn};

use crate::{
    metrics::{CurrentStepValue, METRICS},
    peer_handler::{PeerHandler, RequestMetadata, RequestStateTrieNodesError},
    rlpx::p2p::SUPPORTED_SNAP_CAPABILITIES,
    sync::{code_collector::CodeHashCollector, AccountStorageRoots},
    utils::current_unix_time,
};

use super::SyncError;

/// Max size of a batch to start a storage fetch request in queues
pub const STORAGE_BATCH_SIZE: usize = 2000;
/// Max size of a batch to start a node fetch request in queues - increased for faster healing
const NODE_BATCH_SIZE: usize = 4000;
/// Pace at which progress is shown via info tracing
pub const SHOW_PROGRESS_INTERVAL_DURATION: Duration = Duration::from_secs(2);
/// Maximum number of concurrent in-flight requests
const MAX_INFLIGHT_REQUESTS: u64 = 200;
/// Channel capacity for task responses
const TASK_CHANNEL_CAPACITY: usize = 5000;
/// Batch size threshold for parallel processing
const PARALLEL_BATCH_THRESHOLD: usize = 4;

#[derive(Debug)]
pub struct MembatchEntryValue {
    node: Node,
    children_not_in_storage_count: u64,
    parent_path: Nibbles,
}

/// Healing statistics for monitoring
#[derive(Debug, Default, Clone)]
struct HealingStats {
    downloads_success: u64,
    downloads_fail: u64,
    leafs_healed: u64,
}

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
#[allow(clippy::too_many_arguments)]
async fn heal_state_trie(
    state_root: H256,
    store: Store,
    mut peers: PeerHandler,
    staleness_timestamp: u64,
    global_leafs_healed: &mut u64,
    mut membatch: HashMap<Nibbles, MembatchEntryValue>,
    storage_accounts: &mut AccountStorageRoots,
    code_hash_collector: &mut CodeHashCollector,
) -> Result<bool, SyncError> {
    // Add the current state trie root to the pending paths
    let mut paths: Vec<RequestMetadata> = vec![RequestMetadata {
        hash: state_root,
        path: Nibbles::default(),
        parent_path: Nibbles::default(),
    }];

    let mut last_update = Instant::now();
    let mut inflight_tasks: u64 = 0;
    let mut is_stale = false;
    let mut longest_path_seen = 0;
    let mut stats = HealingStats::default();
    let mut nodes_to_write: Vec<(Nibbles, Node)> = Vec::new();
    let mut db_joinset = tokio::task::JoinSet::new();

    // Channel for task responses with increased capacity
    let (task_sender, mut task_receiver) = tokio::sync::mpsc::channel::<(
        H256,
        Result<Vec<Node>, RequestStateTrieNodesError>,
        Vec<RequestMetadata>,
    )>(TASK_CHANNEL_CAPACITY);

    // Batches of nodes waiting to be processed
    let mut nodes_to_heal: Vec<(Vec<Node>, Vec<RequestMetadata>)> = Vec::new();

    let mut logged_no_free_peers_count = 0;

    loop {
        // Progress reporting
        if last_update.elapsed() >= SHOW_PROGRESS_INTERVAL_DURATION {
            let num_peers = peers
                .peer_table
                .peer_count_by_capabilities(&SUPPORTED_SNAP_CAPABILITIES)
                .await
                .unwrap_or(0);
            last_update = Instant::now();
            let downloads_rate = if stats.downloads_success + stats.downloads_fail > 0 {
                stats.downloads_success as f64
                    / (stats.downloads_success + stats.downloads_fail) as f64
            } else {
                0.0
            };

            METRICS
                .global_state_trie_leafs_healed
                .store(*global_leafs_healed, Ordering::Relaxed);

            debug!(
                status = if is_stale { "stopping" } else { "in progress" },
                snap_peers = num_peers,
                inflight_tasks,
                longest_path_seen,
                leafs_healed = stats.leafs_healed,
                global_leafs_healed,
                downloads_rate,
                paths_to_go = paths.len(),
                pending_nodes = membatch.len(),
                "State Healing",
            );
            stats.downloads_success = 0;
            stats.downloads_fail = 0;
        }

        // Use tokio::select! for efficient async waiting instead of busy polling
        tokio::select! {
            biased;

            // Handle incoming responses first (higher priority)
            Some((peer_id, response, batch)) = task_receiver.recv() => {
                inflight_tasks -= 1;
                match response {
                    Ok(nodes) => {
                        // Process leaf nodes for account tracking
                        // Note: We use if-let Ok() instead of ? to avoid failing entire batch on decode error
                        for (node, meta) in nodes.iter().zip(batch.iter()) {
                            if let Node::Leaf(leaf_node) = node {
                                if let Ok(account) = AccountState::decode(&leaf_node.value) {
                                    let account_hash =
                                        H256::from_slice(&meta.path.concat(&leaf_node.partial).to_bytes());

                                    if account.code_hash != *EMPTY_KECCACK_HASH {
                                        code_hash_collector.add(account.code_hash);
                                        code_hash_collector.flush_if_needed().await?;
                                    }

                                    storage_accounts.healed_accounts.insert(account_hash);
                                    if let Some((old_root, _)) = storage_accounts
                                        .accounts_with_storage_root
                                        .get_mut(&account_hash)
                                    {
                                        *old_root = None;
                                    }
                                } else {
                                    warn!(
                                        "[SNAP SYNC] Failed to decode account state at path {:?}, skipping",
                                        meta.path
                                    );
                                }
                            }
                        }

                        let leaf_count = nodes.iter().filter(|n| matches!(n, Node::Leaf(_))).count();
                        stats.leafs_healed += leaf_count as u64;
                        *global_leafs_healed += leaf_count as u64;
                        nodes_to_heal.push((nodes, batch));
                        stats.downloads_success += 1;
                        peers.peer_table.record_success(&peer_id).await?;
                    }
                    Err(err) => {
                        debug!(
                            ?err,
                            peer = ?peer_id,
                            batch_size = batch.len(),
                            "GetTrieNodes request failed for state healing"
                        );
                        paths.extend(batch);
                        stats.downloads_fail += 1;
                        peers.peer_table.record_failure(&peer_id).await?;
                    }
                }
            }

            // Yield to allow other tasks to run, with a small timeout for scheduling new requests
            _ = tokio::time::sleep(Duration::from_micros(100)), if paths.is_empty() && nodes_to_heal.is_empty() && inflight_tasks > 0 => {
                // Just waiting for responses
            }

            else => {
                // No responses ready, continue with other work
            }
        }

        // Process multiple batches in parallel if we have enough
        if nodes_to_heal.len() >= PARALLEL_BATCH_THRESHOLD {
            let batches: Vec<_> = nodes_to_heal.drain(..).collect();
            let store_clone = store.clone();

            // Process batches in parallel using rayon
            let results: Vec<Result<(Vec<RequestMetadata>, Vec<(Nibbles, Node)>, Vec<(Nibbles, Node, u64, Nibbles)>), SyncError>> =
                batches
                    .into_par_iter()
                    .map(|(nodes, batch)| {
                        heal_state_batch_optimized(
                            batch,
                            nodes,
                            store_clone.clone(),
                        )
                    })
                    .collect();

            // Merge results back
            for result in results {
                let (return_paths, batch_nodes_to_write, incomplete_nodes) = result?;
                paths.extend(return_paths);

                // Process complete nodes for writing
                for (path, node) in batch_nodes_to_write {
                    nodes_to_write.push((path, node));
                }

                // Add incomplete nodes to membatch (must be done sequentially)
                for (path, node, missing_children_count, parent_path) in incomplete_nodes {
                    let entry = MembatchEntryValue {
                        node,
                        children_not_in_storage_count: missing_children_count,
                        parent_path,
                    };
                    membatch.insert(path, entry);
                }
            }
        } else if let Some((nodes, batch)) = nodes_to_heal.pop() {
            // Process single batch
            let return_paths = heal_state_batch(
                batch,
                nodes,
                store.clone(),
                &mut membatch,
                &mut nodes_to_write,
            )?;
            paths.extend(return_paths);
        }

        // Send new requests if we have capacity
        if !is_stale && inflight_tasks < MAX_INFLIGHT_REQUESTS && !paths.is_empty() {
            let batch: Vec<RequestMetadata> =
                paths.drain(0..min(paths.len(), NODE_BATCH_SIZE)).collect();

            if !batch.is_empty() {
                longest_path_seen = batch
                    .iter()
                    .map(|m| m.path.len())
                    .max()
                    .unwrap_or(0)
                    .max(longest_path_seen);

                // Check local DB first before requesting from peers
                let trie = store.open_direct_state_trie(*EMPTY_TRIE_HASH)?;
                let batch_paths: Vec<Nibbles> = batch.iter().map(|m| m.path.clone()).collect();
                let local_nodes = trie.db().get_batch(&batch_paths)?;

                let mut missing_from_local: Vec<RequestMetadata> = Vec::new();
                let mut found_locally: Vec<(RequestMetadata, Node)> = Vec::new();

                for (meta, local_node) in batch.into_iter().zip(local_nodes.into_iter()) {
                    if let Some(node_bytes) = local_node {
                        if !node_bytes.is_empty() {
                            if let Ok(node) = Node::decode(&node_bytes) {
                                found_locally.push((meta, node));
                                continue;
                            }
                        }
                    }
                    missing_from_local.push(meta);
                }

                // Process locally found nodes immediately (no peer request needed)
                if !found_locally.is_empty() {
                    let local_count = found_locally.len();
                    for (meta, node) in found_locally {
                        let (_, missing_children) = node_missing_children_optimized(
                            &node,
                            &meta.path,
                            trie.db(),
                        )?;
                        paths.extend(missing_children);

                        // Track leaf nodes for account processing
                        if let Node::Leaf(leaf_node) = &node {
                            if let Ok(account) = AccountState::decode(&leaf_node.value) {
                                let account_hash =
                                    H256::from_slice(&meta.path.concat(&leaf_node.partial).to_bytes());

                                if account.code_hash != *EMPTY_KECCACK_HASH {
                                    code_hash_collector.add(account.code_hash);
                                    code_hash_collector.flush_if_needed().await?;
                                }

                                storage_accounts.healed_accounts.insert(account_hash);
                                if let Some((old_root, _)) = storage_accounts
                                    .accounts_with_storage_root
                                    .get_mut(&account_hash)
                                {
                                    *old_root = None;
                                }
                            }
                            stats.leafs_healed += 1;
                            *global_leafs_healed += 1;
                        }

                        nodes_to_write.push((meta.path, node));
                    }
                    trace!(
                        local_count,
                        "Found nodes locally, skipping peer requests"
                    );
                }

                // Only request missing nodes from peers
                if !missing_from_local.is_empty() {
                    let Some((peer_id, connection)) = peers
                        .peer_table
                        .get_best_peer(&SUPPORTED_SNAP_CAPABILITIES)
                        .await
                        .unwrap_or(None)
                    else {
                        paths.extend(missing_from_local);

                        if logged_no_free_peers_count == 0 {
                            trace!("No peers available for state healing");
                            logged_no_free_peers_count = 500;
                        }
                        logged_no_free_peers_count -= 1;

                        tokio::time::sleep(Duration::from_millis(10)).await;
                        continue;
                    };

                    let tx = task_sender.clone();
                    inflight_tasks += 1;
                    let peer_table = peers.peer_table.clone();

                    tokio::spawn(async move {
                        let response = PeerHandler::request_state_trienodes(
                            peer_id,
                            connection,
                            peer_table,
                            state_root,
                            missing_from_local.clone(),
                        )
                        .await;
                        let _ = tx.send((peer_id, response, missing_from_local)).await;
                    });
                }
            }
        }

        let is_done = paths.is_empty() && nodes_to_heal.is_empty() && inflight_tasks == 0;

        // Write to DB when batch is large enough
        if nodes_to_write.len() > 250_000 || is_done || is_stale {
            let to_write = std::mem::take(&mut nodes_to_write);
            if !to_write.is_empty() {
                let store = store.clone();

                // Wait for previous write to complete
                if !db_joinset.is_empty() {
                    db_joinset
                        .join_next()
                        .await
                        .expect("joinset not empty")?;
                }

                db_joinset.spawn_blocking(move || {
                    let mut encoded_to_write = BTreeMap::new();

                    for (path, node) in to_write {
                        // Mark parent paths as needing deletion
                        for i in 0..path.len() {
                            encoded_to_write.insert(path.slice(0, i), vec![]);
                        }
                        encoded_to_write.insert(path, node.encode_to_vec());
                    }

                    let trie_db = store
                        .open_direct_state_trie(*EMPTY_TRIE_HASH)
                        .expect("Store should open");
                    let db = trie_db.db();
                    db.put_batch(encoded_to_write.into_iter().collect())
                        .expect("put_batch failed");
                });
            }
        }

        // Check termination conditions
        if is_done {
            debug!("State healing complete");
            db_joinset.join_all().await;
            break;
        }

        if !is_stale && current_unix_time() > staleness_timestamp {
            debug!("State healing is stale");
            is_stale = true;
        }

        if is_stale && nodes_to_heal.is_empty() && inflight_tasks == 0 {
            debug!("Finished in-flight tasks after staleness");
            db_joinset.join_all().await;
            break;
        }
    }

    Ok(paths.is_empty())
}

/// Optimized batch healing that returns results without modifying membatch
/// Used for parallel processing
/// Returns: (paths_to_fetch, nodes_to_write, incomplete_nodes_for_membatch)
fn heal_state_batch_optimized(
    mut batch: Vec<RequestMetadata>,
    nodes: Vec<Node>,
    store: Store,
) -> Result<(Vec<RequestMetadata>, Vec<(Nibbles, Node)>, Vec<(Nibbles, Node, u64, Nibbles)>), SyncError> {
    let trie = store.open_direct_state_trie(*EMPTY_TRIE_HASH)?;
    let mut nodes_to_write = Vec::new();
    let mut return_paths = Vec::new();
    // Incomplete nodes: (path, node, missing_children_count, parent_path)
    let mut incomplete_nodes = Vec::new();

    for node in nodes.into_iter() {
        let path = batch.remove(0);
        let (missing_children_count, missing_children) =
            node_missing_children_optimized(&node, &path.path, trie.db())?;

        return_paths.extend(missing_children);

        if missing_children_count == 0 {
            nodes_to_write.push((path.path.clone(), node));
        } else {
            // Store incomplete nodes to be added to membatch after parallel processing
            incomplete_nodes.push((path.path.clone(), node, missing_children_count, path.parent_path.clone()));
        }
    }

    return_paths.extend(batch);
    Ok((return_paths, nodes_to_write, incomplete_nodes))
}

/// Process a batch of nodes, checking for missing children and updating membatch
fn heal_state_batch(
    mut batch: Vec<RequestMetadata>,
    nodes: Vec<Node>,
    store: Store,
    membatch: &mut HashMap<Nibbles, MembatchEntryValue>,
    nodes_to_write: &mut Vec<(Nibbles, Node)>,
) -> Result<Vec<RequestMetadata>, SyncError> {
    let trie = store.open_direct_state_trie(*EMPTY_TRIE_HASH)?;

    for node in nodes.into_iter() {
        let path = batch.remove(0);
        let (missing_children_count, missing_children) =
            node_missing_children_optimized(&node, &path.path, trie.db())?;

        batch.extend(missing_children);

        if missing_children_count == 0 {
            commit_node(
                node,
                &path.path,
                &path.parent_path,
                membatch,
                nodes_to_write,
            );
        } else {
            let entry = MembatchEntryValue {
                node: node.clone(),
                children_not_in_storage_count: missing_children_count,
                parent_path: path.parent_path.clone(),
            };
            membatch.insert(path.path.clone(), entry);
        }
    }

    Ok(batch)
}

fn commit_node(
    node: Node,
    path: &Nibbles,
    parent_path: &Nibbles,
    membatch: &mut HashMap<Nibbles, MembatchEntryValue>,
    nodes_to_write: &mut Vec<(Nibbles, Node)>,
) {
    nodes_to_write.push((path.clone(), node));

    if parent_path == path {
        return; // Root node case
    }

    let mut membatch_entry = membatch.remove(parent_path).unwrap_or_else(|| {
        panic!("Parent should exist. Parent: {parent_path:?}, path: {path:?}")
    });

    membatch_entry.children_not_in_storage_count -= 1;
    if membatch_entry.children_not_in_storage_count == 0 {
        commit_node(
            membatch_entry.node,
            parent_path,
            &membatch_entry.parent_path,
            membatch,
            nodes_to_write,
        );
    } else {
        membatch.insert(parent_path.clone(), membatch_entry);
    }
}

/// Checks which children of a node are missing from the trie.
/// Uses batch DB lookups for efficiency.
pub fn node_missing_children_optimized(
    node: &Node,
    path: &Nibbles,
    trie_state: &dyn TrieDB,
) -> Result<(u64, Vec<RequestMetadata>), TrieError> {
    match node {
        Node::Branch(branch_node) => {
            // Collect all valid children paths first
            let mut child_info: Vec<(Nibbles, &ethrex_trie::NodeRef)> = Vec::with_capacity(16);

            for (index, child) in branch_node.choices.iter().enumerate() {
                if !child.is_valid() {
                    continue;
                }
                let child_path = path.clone().append_new(index as u8);
                child_info.push((child_path, child));
            }

            if child_info.is_empty() {
                return Ok((0, vec![]));
            }

            // Get all paths to check
            let paths_to_check: Vec<Nibbles> = child_info.iter().map(|(p, _)| p.clone()).collect();

            // Batch check DB
            let db_exists = trie_state.exists_batch(&paths_to_check)?;

            // Build list of missing children
            let mut missing_children = Vec::new();
            for (i, exists) in db_exists.iter().enumerate() {
                if !exists {
                    let (ref child_path, child) = child_info[i];
                    missing_children.push(RequestMetadata {
                        hash: child.compute_hash().finalize(),
                        path: child_path.clone(),
                        parent_path: path.clone(),
                    });
                }
            }

            Ok((missing_children.len() as u64, missing_children))
        }

        Node::Extension(ext_node) => {
            if !ext_node.child.is_valid() {
                return Ok((0, vec![]));
            }

            let child_path = path.concat(&ext_node.prefix);

            // Check DB
            if trie_state.exists(child_path.clone())? {
                Ok((0, vec![]))
            } else {
                Ok((
                    1,
                    vec![RequestMetadata {
                        hash: ext_node.child.compute_hash().finalize(),
                        path: child_path,
                        parent_path: path.clone(),
                    }],
                ))
            }
        }

        Node::Leaf(_) => Ok((0, vec![])),
    }
}
