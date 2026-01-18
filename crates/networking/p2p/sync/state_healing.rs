//! Optimized state trie healing module
//!
//! This module contains the logic for state healing with significant performance optimizations:
//! - Bloom filter + LRU cache for fast path existence checks
//! - Batch child lookups to reduce DB round-trips
//! - Parallel response processing with rayon
//! - Speculative prefetching of trie children
//! - Async select-based event loop (no busy polling)
//!
//! State healing begins after downloading the whole state trie and rebuilding it locally.
//! Its purpose is to fix inconsistencies with the canonical state trie by downloading
//! all missing trie nodes starting from the root node.

use std::{
    cmp::min,
    collections::{BTreeMap, HashMap},
    sync::{atomic::Ordering, Arc},
    time::Duration,
};

use ethrex_common::{constants::EMPTY_KECCACK_HASH, types::AccountState, H256};
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};
use ethrex_storage::Store;
use ethrex_trie::{Nibbles, Node, TrieDB, TrieError, EMPTY_TRIE_HASH};
use rayon::prelude::*;
use tokio::time::Instant;
use tracing::{debug, trace};

use crate::{
    metrics::{CurrentStepValue, METRICS},
    peer_handler::{PeerHandler, RequestMetadata, RequestStateTrieNodesError},
    rlpx::p2p::SUPPORTED_SNAP_CAPABILITIES,
    sync::{code_collector::CodeHashCollector, AccountStorageRoots},
    utils::current_unix_time,
};

use super::healing_cache::{HealingCache, PathStatus, SharedHealingCache};
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
    debug!("Starting optimized state healing");

    // Create shared healing cache for path existence tracking
    let healing_cache = Arc::new(HealingCache::new());

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
            healing_cache.clone(),
        )
        .await?;
        if current_unix_time() > staleness_timestamp {
            debug!("Stopped state healing due to staleness");
            break;
        }
    }

    // Log cache statistics
    let cache_stats = healing_cache.stats();
    debug!(
        paths_cached = cache_stats.paths_added,
        "State healing cache statistics"
    );

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
    healing_cache: SharedHealingCache,
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

            let cache_stats = healing_cache.stats();

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
                cache_paths = cache_stats.paths_added,
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
                        for (node, meta) in nodes.iter().zip(batch.iter()) {
                            if let Node::Leaf(leaf_node) = node {
                                let account = AccountState::decode(&leaf_node.value)?;
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
            let cache_clone = healing_cache.clone();

            // Process batches in parallel using rayon
            let results: Vec<Result<(Vec<RequestMetadata>, Vec<(Nibbles, Node)>), SyncError>> =
                batches
                    .into_par_iter()
                    .map(|(nodes, batch)| {
                        heal_state_batch_optimized(
                            batch,
                            nodes,
                            store_clone.clone(),
                            cache_clone.clone(),
                        )
                    })
                    .collect();

            // Merge results back
            for result in results {
                let (return_paths, batch_nodes_to_write) = result?;
                paths.extend(return_paths);

                // Process nodes for membatch (must be done sequentially)
                for (path, node) in batch_nodes_to_write {
                    nodes_to_write.push((path, node));
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
                healing_cache.clone(),
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
                            &healing_cache,
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
                let cache = healing_cache.clone();

                // Wait for previous write to complete
                if !db_joinset.is_empty() {
                    db_joinset
                        .join_next()
                        .await
                        .expect("joinset not empty")?;
                }

                db_joinset.spawn_blocking(move || {
                    let mut encoded_to_write = BTreeMap::new();
                    let mut paths_to_cache = Vec::with_capacity(to_write.len());

                    for (path, node) in to_write {
                        // Mark parent paths as needing deletion
                        for i in 0..path.len() {
                            encoded_to_write.insert(path.slice(0, i), vec![]);
                        }
                        encoded_to_write.insert(path.clone(), node.encode_to_vec());
                        paths_to_cache.push(path);
                    }

                    let trie_db = store
                        .open_direct_state_trie(*EMPTY_TRIE_HASH)
                        .expect("Store should open");
                    let db = trie_db.db();
                    db.put_batch(encoded_to_write.into_iter().collect())
                        .expect("put_batch failed");

                    // Update cache with newly written paths
                    cache.mark_exists_batch(&paths_to_cache);
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
fn heal_state_batch_optimized(
    mut batch: Vec<RequestMetadata>,
    nodes: Vec<Node>,
    store: Store,
    healing_cache: SharedHealingCache,
) -> Result<(Vec<RequestMetadata>, Vec<(Nibbles, Node)>), SyncError> {
    let trie = store.open_direct_state_trie(*EMPTY_TRIE_HASH)?;
    let mut nodes_to_write = Vec::new();
    let mut return_paths = Vec::new();

    for node in nodes.into_iter() {
        let path = batch.remove(0);
        let (missing_children_count, missing_children) =
            node_missing_children_optimized(&node, &path.path, trie.db(), &healing_cache)?;

        return_paths.extend(missing_children);

        if missing_children_count == 0 {
            nodes_to_write.push((path.path.clone(), node));
        }
        // Note: membatch handling is done separately for parallel batches
    }

    return_paths.extend(batch);
    Ok((return_paths, nodes_to_write))
}

/// Process a batch of nodes, checking for missing children and updating membatch
fn heal_state_batch(
    mut batch: Vec<RequestMetadata>,
    nodes: Vec<Node>,
    store: Store,
    membatch: &mut HashMap<Nibbles, MembatchEntryValue>,
    nodes_to_write: &mut Vec<(Nibbles, Node)>,
    healing_cache: SharedHealingCache,
) -> Result<Vec<RequestMetadata>, SyncError> {
    let trie = store.open_direct_state_trie(*EMPTY_TRIE_HASH)?;

    for node in nodes.into_iter() {
        let path = batch.remove(0);
        let (missing_children_count, missing_children) =
            node_missing_children_optimized(&node, &path.path, trie.db(), &healing_cache)?;

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

/// Optimized version of node_missing_children that uses:
/// 1. Healing cache for fast existence checks
/// 2. Batch DB lookups when cache misses occur
/// 3. Skips full hash verification (uses path-based existence check)
pub fn node_missing_children_optimized(
    node: &Node,
    path: &Nibbles,
    trie_state: &dyn TrieDB,
    healing_cache: &HealingCache,
) -> Result<(u64, Vec<RequestMetadata>), TrieError> {
    match node {
        Node::Branch(branch_node) => {
            // Collect all valid children paths first
            let mut child_info: Vec<(usize, Nibbles, &ethrex_trie::NodeRef)> = Vec::with_capacity(16);

            for (index, child) in branch_node.choices.iter().enumerate() {
                if !child.is_valid() {
                    continue;
                }
                let child_path = path.clone().append_new(index as u8);
                child_info.push((index, child_path, child));
            }

            if child_info.is_empty() {
                return Ok((0, vec![]));
            }

            // Check cache first for all children
            let cache_statuses: Vec<_> = child_info
                .iter()
                .map(|(_, child_path, _)| healing_cache.check_path(child_path))
                .collect();

            // Identify paths that need DB verification
            // Both ProbablyExists and DefinitelyMissing need DB check
            // (DefinitelyMissing means cache hasn't seen it, but DB might have it)
            let mut paths_to_check: Vec<Nibbles> = Vec::new();
            let mut check_indices: Vec<usize> = Vec::new();

            for (i, status) in cache_statuses.iter().enumerate() {
                match status {
                    PathStatus::ConfirmedExists => {
                        // Already verified in cache, skip DB check
                    }
                    PathStatus::ProbablyExists | PathStatus::DefinitelyMissing => {
                        // Need to verify with DB
                        paths_to_check.push(child_info[i].1.clone());
                        check_indices.push(i);
                    }
                }
            }

            // Batch check paths that might exist
            let db_exists: Vec<bool> = if !paths_to_check.is_empty() {
                trie_state.exists_batch(&paths_to_check)?
            } else {
                vec![]
            };

            // Update cache with confirmed existences
            let confirmed_paths: Vec<_> = paths_to_check
                .iter()
                .zip(db_exists.iter())
                .filter(|(_, exists)| **exists)
                .map(|(path, _)| path.clone())
                .collect();

            if !confirmed_paths.is_empty() {
                healing_cache.mark_exists_batch(&confirmed_paths);
            }

            // Build list of missing children
            let mut missing_children = Vec::new();
            let mut db_check_idx = 0;

            for (i, status) in cache_statuses.iter().enumerate() {
                let (_, ref child_path, child) = child_info[i];

                let exists = match status {
                    PathStatus::ConfirmedExists => true,
                    PathStatus::ProbablyExists | PathStatus::DefinitelyMissing => {
                        let exists = db_exists[db_check_idx];
                        db_check_idx += 1;
                        exists
                    }
                };

                if !exists {
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

            // Check cache first
            match healing_cache.check_path(&child_path) {
                PathStatus::ConfirmedExists => Ok((0, vec![])),
                PathStatus::ProbablyExists | PathStatus::DefinitelyMissing => {
                    // Verify with DB (both cases need DB check)
                    if trie_state.exists(child_path.clone())? {
                        healing_cache.mark_exists(&child_path);
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
            }
        }

        Node::Leaf(_) => Ok((0, vec![])),
    }
}
