//! Optimized storage trie healing module
//!
//! This module heals storage tries with significant performance optimizations:
//! - Bloom filter + LRU cache for fast path existence checks
//! - Batch child lookups to reduce DB round-trips
//! - Parallel account processing with rayon
//! - Async select-based event loop (no busy polling)

use crate::{
    metrics::{CurrentStepValue, METRICS},
    peer_handler::{PeerHandler, RequestStorageTrieNodes, MAX_RESPONSE_BYTES},
    rlpx::{
        p2p::SUPPORTED_SNAP_CAPABILITIES,
        snap::{GetStorageRanges, GetTrieNodes, TrieNodes},
    },
    sync::{
        state_healing::{SHOW_PROGRESS_INTERVAL_DURATION, STORAGE_BATCH_SIZE},
        AccountStorageRoots, SyncError,
    },
    utils::current_unix_time,
};
use ethrex_storage::SnapSyncTrie;

use super::healing_cache::{HealingCache, PathStatus, SharedHealingCache};

use bytes::Bytes;
use ethrex_common::{types::AccountState, H256};
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode, error::RLPDecodeError};
use ethrex_storage::{error::StoreError, Store};
#[allow(unused_imports)]
use ethrex_trie::{Nibbles, Node, EMPTY_TRIE_HASH, TrieDB};
use rand::random;
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use std::{
    collections::{HashMap, VecDeque},
    sync::{atomic::Ordering, Arc},
    time::Duration,
};
use tokio::{
    sync::mpsc::{error::TrySendError, Sender},
    task::JoinSet,
};
use tracing::{debug, trace};

/// Maximum number of concurrent in-flight requests
const MAX_IN_FLIGHT_REQUESTS: u32 = 200;

/// Channel capacity for task responses
const TASK_CHANNEL_CAPACITY: usize = 2000;

/// This struct stores the metadata we need when we request a node
#[derive(Debug, Clone)]
pub struct NodeResponse {
    node: Node,
    node_request: NodeRequest,
}

/// This struct stores the metadata we need when we store a node in the memory bank before storing
#[derive(Debug, Clone)]
pub struct MembatchEntry {
    node_response: NodeResponse,
    missing_children_count: usize,
}

/// The membatch key represents the account path and the storage path
type MembatchKey = (Nibbles, Nibbles);

type Membatch = HashMap<MembatchKey, MembatchEntry>;

#[derive(Debug, Clone)]
pub struct InflightRequest {
    requests: Vec<NodeRequest>,
    peer_id: H256,
}

#[derive(Debug, Clone)]
pub struct StorageHealer {
    last_update: tokio::time::Instant,
    download_queue: VecDeque<NodeRequest>,
    store: Store,
    membatch: Membatch,
    requests: HashMap<u64, InflightRequest>,
    staleness_timestamp: u64,
    state_root: H256,
    healing_cache: SharedHealingCache,

    // Analytics data
    maximum_length_seen: usize,
    leafs_healed: usize,
    roots_healed: usize,
    succesful_downloads: usize,
    failed_downloads: usize,
    empty_count: usize,
    disconnected_count: usize,
}

#[derive(Debug, Clone, Default)]
pub struct NodeRequest {
    acc_path: Nibbles,
    storage_path: Nibbles,
    parent: Nibbles,
    hash: H256,
}

/// Heals storage tries with optimized caching and batch operations
pub async fn heal_storage_trie(
    state_root: H256,
    storage_accounts: &AccountStorageRoots,
    peers: &mut PeerHandler,
    store: Store,
    membatch: Membatch,
    staleness_timestamp: u64,
    global_leafs_healed: &mut u64,
) -> Result<bool, SyncError> {
    METRICS.current_step.set(CurrentStepValue::HealingStorage);

    // Create shared healing cache for this storage healing session
    let healing_cache = Arc::new(HealingCache::new());

    let download_queue = get_initial_downloads(&store, state_root, storage_accounts);
    debug!(
        initial_accounts_count = download_queue.len(),
        "Started optimized storage healing",
    );

    let mut state = StorageHealer {
        last_update: tokio::time::Instant::now(),
        download_queue,
        store,
        membatch,
        requests: HashMap::new(),
        staleness_timestamp,
        state_root,
        healing_cache,
        maximum_length_seen: 0,
        leafs_healed: 0,
        roots_healed: 0,
        succesful_downloads: 0,
        failed_downloads: 0,
        empty_count: 0,
        disconnected_count: 0,
    };

    let mut requests_task_joinset: JoinSet<
        Result<u64, TrySendError<Result<TrieNodes, RequestStorageTrieNodes>>>,
    > = JoinSet::new();

    let mut nodes_to_write: HashMap<H256, Vec<(Nibbles, Node)>> = HashMap::new();
    let mut db_joinset = tokio::task::JoinSet::new();

    let (task_sender, mut task_receiver) =
        tokio::sync::mpsc::channel::<Result<TrieNodes, RequestStorageTrieNodes>>(
            TASK_CHANNEL_CAPACITY,
        );

    let mut logged_no_free_peers_count = 0;

    loop {
        // Progress reporting
        if state.last_update.elapsed() >= SHOW_PROGRESS_INTERVAL_DURATION {
            METRICS
                .global_storage_tries_leafs_healed
                .store(*global_leafs_healed, Ordering::Relaxed);
            METRICS
                .healing_empty_try_recv
                .store(state.empty_count as u64, Ordering::Relaxed);
            state.last_update = tokio::time::Instant::now();

            let snap_peer_count = peers
                .peer_table
                .peer_count_by_capabilities(&SUPPORTED_SNAP_CAPABILITIES)
                .await
                .unwrap_or(0);

            let cache_stats = state.healing_cache.stats();

            debug!(
                snap_peer_count,
                inflight_requests = state.requests.len(),
                download_queue_len = state.download_queue.len(),
                maximum_depth = state.maximum_length_seen,
                leaves_healed = state.leafs_healed,
                global_leaves_healed = global_leafs_healed,
                roots_healed = state.roots_healed,
                succesful_downloads = state.succesful_downloads,
                cache_filter_hits = cache_stats.filter_hits,
                cache_lru_hits = cache_stats.lru_hits,
                "Storage Healing",
            );
            state.succesful_downloads = 0;
            state.failed_downloads = 0;
            state.empty_count = 0;
            state.disconnected_count = 0;
        }

        let is_done = state.requests.is_empty() && state.download_queue.is_empty();
        let is_stale = current_unix_time() > state.staleness_timestamp;

        // Write nodes to DB when batch is large enough
        if nodes_to_write.values().map(Vec::len).sum::<usize>() > 100_000 || is_done || is_stale {
            let to_write: Vec<_> = nodes_to_write.drain().collect();
            if !to_write.is_empty() {
                let store = state.store.clone();
                let cache = state.healing_cache.clone();

                if !db_joinset.is_empty() {
                    db_joinset.join_next().await;
                }

                db_joinset.spawn_blocking(move || {
                    let mut encoded_to_write = vec![];
                    let mut paths_to_cache = Vec::new();

                    for (hashed_account, nodes) in to_write {
                        let mut account_nodes = vec![];
                        for (path, node) in nodes {
                            for i in 0..path.len() {
                                account_nodes.push((path.slice(0, i), vec![]));
                            }
                            account_nodes.push((path.clone(), node.encode_to_vec()));

                            // Build cache key for storage path
                            let cache_key =
                                Nibbles::from_bytes(&hashed_account.0).concat(&path);
                            paths_to_cache.push(cache_key);
                        }
                        encoded_to_write.push((hashed_account, account_nodes));
                    }

                    spawned_rt::tasks::block_on(store.write_storage_trie_nodes_batch(encoded_to_write))
                        .expect("db write failed");

                    // Update cache with newly written paths
                    cache.mark_exists_batch(&paths_to_cache);
                });
            }
        }

        if is_done {
            db_joinset.join_all().await;
            let cache_stats = state.healing_cache.stats();
            debug!(
                filter_hits = cache_stats.filter_hits,
                lru_hits = cache_stats.lru_hits,
                paths_cached = cache_stats.paths_added,
                "Storage healing complete - cache statistics"
            );
            return Ok(true);
        }

        if is_stale {
            db_joinset.join_all().await;
            return Ok(false);
        }

        // Send new requests
        ask_peers_for_nodes(
            &mut state.download_queue,
            &mut state.requests,
            &mut requests_task_joinset,
            peers,
            state.state_root,
            &task_sender,
            &mut logged_no_free_peers_count,
        )
        .await;

        let _ = requests_task_joinset.try_join_next();

        // Use select! for efficient async waiting
        let trie_nodes_result = tokio::select! {
            biased;

            result = task_receiver.recv() => {
                match result {
                    Some(r) => r,
                    None => {
                        state.disconnected_count += 1;
                        continue;
                    }
                }
            }

            _ = tokio::time::sleep(Duration::from_micros(100)), if state.download_queue.is_empty() && !state.requests.is_empty() => {
                continue;
            }

            else => {
                state.empty_count += 1;
                continue;
            }
        };

        match trie_nodes_result {
            Ok(trie_nodes) => {
                let Some(mut nodes_from_peer) = zip_requeue_node_responses_score_peer(
                    &mut state.requests,
                    peers,
                    &mut state.download_queue,
                    &trie_nodes,
                    &mut state.succesful_downloads,
                    &mut state.failed_downloads,
                )
                .await?
                else {
                    continue;
                };

                process_node_responses(
                    &mut nodes_from_peer,
                    &mut state.download_queue,
                    &state.store,
                    &mut state.membatch,
                    &mut state.leafs_healed,
                    global_leafs_healed,
                    &mut state.roots_healed,
                    &mut state.maximum_length_seen,
                    &mut nodes_to_write,
                    &state.healing_cache,
                )
                .expect("Store error during processing");
            }
            Err(RequestStorageTrieNodes::RequestError(id, err)) => {
                let inflight_request = state.requests.remove(&id).expect("request disappeared");
                debug!(
                    ?err,
                    peer = ?inflight_request.peer_id,
                    request_count = inflight_request.requests.len(),
                    "GetTrieNodes request failed for storage healing"
                );
                state.failed_downloads += 1;
                state
                    .download_queue
                    .extend(inflight_request.requests.clone());
                peers
                    .peer_table
                    .record_failure(&inflight_request.peer_id)
                    .await?;
            }
        }
    }
}

async fn ask_peers_for_nodes(
    download_queue: &mut VecDeque<NodeRequest>,
    requests: &mut HashMap<u64, InflightRequest>,
    requests_task_joinset: &mut JoinSet<
        Result<u64, TrySendError<Result<TrieNodes, RequestStorageTrieNodes>>>,
    >,
    peers: &mut PeerHandler,
    state_root: H256,
    task_sender: &Sender<Result<TrieNodes, RequestStorageTrieNodes>>,
    logged_no_free_peers_count: &mut u32,
) {
    if (requests.len() as u32) < MAX_IN_FLIGHT_REQUESTS && !download_queue.is_empty() {
        let Some((peer_id, connection)) = peers
            .peer_table
            .get_best_peer(&SUPPORTED_SNAP_CAPABILITIES)
            .await
            .inspect_err(|err| debug!(?err, "Error requesting a peer for storage healing"))
            .unwrap_or(None)
        else {
            if *logged_no_free_peers_count == 0 {
                trace!("No peers available for storage healing");
                *logged_no_free_peers_count = 500;
            }
            *logged_no_free_peers_count -= 1;
            tokio::time::sleep(Duration::from_millis(10)).await;
            return;
        };

        let at = download_queue.len().saturating_sub(STORAGE_BATCH_SIZE);
        let download_chunk = download_queue.split_off(at);
        let req_id: u64 = random();
        let (paths, inflight_requests_data) = create_node_requests(download_chunk);

        requests.insert(
            req_id,
            InflightRequest {
                requests: inflight_requests_data,
                peer_id,
            },
        );

        let gtn = GetTrieNodes {
            id: req_id,
            root_hash: state_root,
            paths,
            bytes: MAX_RESPONSE_BYTES,
        };

        let tx = task_sender.clone();
        let peer_table = peers.peer_table.clone();

        requests_task_joinset.spawn(async move {
            let req_id = gtn.id;
            let response =
                PeerHandler::request_storage_trienodes(peer_id, connection, peer_table, gtn).await;
            tx.try_send(response)?;
            Ok(req_id)
        });
    }
}

fn create_node_requests(
    node_requests: VecDeque<NodeRequest>,
) -> (Vec<Vec<Bytes>>, Vec<NodeRequest>) {
    let mut mapped_requests: HashMap<Nibbles, Vec<NodeRequest>> = HashMap::new();

    for request in node_requests {
        mapped_requests
            .entry(request.acc_path.clone())
            .or_default()
            .push(request);
    }

    let mut inflight_request: Vec<NodeRequest> = Vec::new();

    let result: Vec<Vec<Bytes>> = mapped_requests
        .into_iter()
        .map(|(acc_path, request_vec)| {
            let response = [
                vec![Bytes::from(acc_path.to_bytes())],
                request_vec
                    .iter()
                    .map(|node_req| Bytes::from(node_req.storage_path.encode_compact()))
                    .collect(),
            ]
            .concat();
            inflight_request.extend(request_vec);
            response
        })
        .collect();

    (result, inflight_request)
}

async fn zip_requeue_node_responses_score_peer(
    requests: &mut HashMap<u64, InflightRequest>,
    peer_handler: &mut PeerHandler,
    download_queue: &mut VecDeque<NodeRequest>,
    trie_nodes: &TrieNodes,
    succesful_downloads: &mut usize,
    failed_downloads: &mut usize,
) -> Result<Option<Vec<NodeResponse>>, SyncError> {
    trace!(
        trie_response_len = ?trie_nodes.nodes.len(),
        "Processing storage trie nodes",
    );

    let Some(request) = requests.remove(&trie_nodes.id) else {
        debug!(?trie_nodes, "No matching request found for response");
        return Ok(None);
    };

    let nodes_size = trie_nodes.nodes.len();
    if nodes_size == 0 {
        debug!(
            peer = ?request.peer_id,
            requested_nodes = request.requests.len(),
            "Peer returned empty TrieNodes response - peer may not have requested state"
        );
        METRICS
            .healing_empty_peer_responses
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        *failed_downloads += 1;
        peer_handler
            .peer_table
            .record_failure(&request.peer_id)
            .await?;
        download_queue.extend(request.requests);
        return Ok(None);
    }

    if request.requests.len() < nodes_size {
        panic!("Peer responded with more data than requested!");
    }

    if let Ok(nodes) = request
        .requests
        .iter()
        .zip(trie_nodes.nodes.clone())
        .map(|(node_request, node_bytes)| {
            let node = Node::decode(&node_bytes).inspect_err(|err| {
                trace!(
                    peer = ?request.peer_id,
                    ?node_request,
                    error = ?err,
                    "Decode failed"
                )
            })?;

            if node.compute_hash().finalize() != node_request.hash {
                trace!(
                    peer = ?request.peer_id,
                    ?node_request,
                    "Node hash verification failed"
                );
                Err(RLPDecodeError::MalformedData)
            } else {
                Ok(NodeResponse {
                    node_request: node_request.clone(),
                    node,
                })
            }
        })
        .collect::<Result<Vec<NodeResponse>, RLPDecodeError>>()
    {
        if request.requests.len() > nodes_size {
            download_queue.extend(request.requests.into_iter().skip(nodes_size));
        }
        *succesful_downloads += 1;
        peer_handler
            .peer_table
            .record_success(&request.peer_id)
            .await?;
        Ok(Some(nodes))
    } else {
        *failed_downloads += 1;
        peer_handler
            .peer_table
            .record_failure(&request.peer_id)
            .await?;
        download_queue.extend(request.requests);
        Ok(None)
    }
}

#[allow(clippy::too_many_arguments)]
fn process_node_responses(
    node_processing_queue: &mut Vec<NodeResponse>,
    download_queue: &mut VecDeque<NodeRequest>,
    store: &Store,
    membatch: &mut Membatch,
    leafs_healed: &mut usize,
    global_leafs_healed: &mut u64,
    roots_healed: &mut usize,
    maximum_length_seen: &mut usize,
    to_write: &mut HashMap<H256, Vec<(Nibbles, Node)>>,
    healing_cache: &HealingCache,
) -> Result<(), StoreError> {
    while let Some(node_response) = node_processing_queue.pop() {
        trace!(?node_response, "Processing node response");

        if let Node::Leaf(_) = &node_response.node {
            *leafs_healed += 1;
            *global_leafs_healed += 1;
        }

        *maximum_length_seen = usize::max(
            *maximum_length_seen,
            node_response.node_request.storage_path.len(),
        );

        let (missing_children_nibbles, missing_children_count) =
            determine_missing_children_optimized(&node_response, store, healing_cache)?;

        if missing_children_count == 0 {
            commit_node(&node_response, membatch, roots_healed, to_write)?;
        } else {
            let key = (
                node_response.node_request.acc_path.clone(),
                node_response.node_request.storage_path.clone(),
            );
            membatch.insert(
                key,
                MembatchEntry {
                    node_response: node_response.clone(),
                    missing_children_count,
                },
            );
            download_queue.extend(missing_children_nibbles);
        }
    }

    Ok(())
}

fn get_initial_downloads(
    store: &Store,
    state_root: H256,
    account_paths: &AccountStorageRoots,
) -> VecDeque<NodeRequest> {
    let trie = store
        .open_locked_state_trie(state_root)
        .expect("Should be able to open store");

    account_paths
        .healed_accounts
        .par_iter()
        .filter_map(|acc_path| {
            let rlp = trie
                .get(acc_path.as_bytes())
                .expect("Should be able to read from store")?;
            let account = AccountState::decode(&rlp).expect("Should have valid account");

            if account.storage_root == *EMPTY_TRIE_HASH {
                return None;
            }

            Some(NodeRequest {
                acc_path: Nibbles::from_bytes(&acc_path.0),
                storage_path: Nibbles::default(),
                parent: Nibbles::default(),
                hash: account.storage_root,
            })
        })
        .collect()
}

/// Optimized version that uses healing cache for fast existence checks
pub fn determine_missing_children_optimized(
    node_response: &NodeResponse,
    store: &Store,
    healing_cache: &HealingCache,
) -> Result<(Vec<NodeRequest>, usize), StoreError> {
    let mut paths = Vec::new();
    let mut count = 0;
    let node = node_response.node.clone();

    let trie = store.open_direct_storage_trie(
        H256::from_slice(&node_response.node_request.acc_path.to_bytes()),
        *EMPTY_TRIE_HASH,
    )?;
    let trie_state = trie.db();

    match &node {
        Node::Branch(branch_node) => {
            // Collect all valid children
            let mut child_info: Vec<(usize, Nibbles, &ethrex_trie::NodeRef)> = Vec::with_capacity(16);

            for (index, child) in branch_node.choices.iter().enumerate() {
                if !child.is_valid() {
                    continue;
                }
                let child_path = node_response
                    .node_request
                    .storage_path
                    .append_new(index as u8);
                child_info.push((index, child_path, child));
            }

            if child_info.is_empty() {
                return Ok((vec![], 0));
            }

            // Build cache keys and check cache
            let cache_keys: Vec<Nibbles> = child_info
                .iter()
                .map(|(_, child_path, _)| {
                    // Storage cache key includes account path prefix
                    Nibbles::from_bytes(&node_response.node_request.acc_path.to_bytes())
                        .concat(child_path)
                })
                .collect();

            let cache_statuses: Vec<_> = cache_keys
                .iter()
                .map(|key| healing_cache.check_path(key))
                .collect();

            // Identify paths needing DB verification
            // Both ProbablyExists and DefinitelyMissing need DB check
            // (DefinitelyMissing means cache hasn't seen it, but DB might have it)
            let mut paths_to_check: Vec<Nibbles> = Vec::new();

            for (i, status) in cache_statuses.iter().enumerate() {
                match status {
                    PathStatus::ConfirmedExists => {}
                    PathStatus::ProbablyExists | PathStatus::DefinitelyMissing => {
                        paths_to_check.push(child_info[i].1.clone());
                    }
                }
            }

            // Batch check paths
            let db_exists: Vec<bool> = if !paths_to_check.is_empty() {
                trie_state
                    .exists_batch(&paths_to_check)
                    .map_err(|e| StoreError::Custom(format!("Trie error: {e}")))?
            } else {
                vec![]
            };

            // Update cache with confirmed existences
            // Build list of cache keys that correspond to paths that exist in DB
            let mut confirmed_keys: Vec<Nibbles> = Vec::new();
            let mut db_idx = 0;
            for (i, status) in cache_statuses.iter().enumerate() {
                if !matches!(status, PathStatus::ConfirmedExists) {
                    if db_exists[db_idx] {
                        confirmed_keys.push(cache_keys[i].clone());
                    }
                    db_idx += 1;
                }
            }

            if !confirmed_keys.is_empty() {
                healing_cache.mark_exists_batch(&confirmed_keys);
            }

            // Build missing children list
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
                    count += 1;
                    paths.push(NodeRequest {
                        acc_path: node_response.node_request.acc_path.clone(),
                        storage_path: child_path.clone(),
                        parent: node_response.node_request.storage_path.clone(),
                        hash: child.compute_hash().finalize(),
                    });
                }
            }
        }

        Node::Extension(ext_node) => {
            let child_path = node_response
                .node_request
                .storage_path
                .concat(&ext_node.prefix);

            if !ext_node.child.is_valid() {
                return Ok((vec![], 0));
            }

            // Build cache key
            let cache_key =
                Nibbles::from_bytes(&node_response.node_request.acc_path.to_bytes())
                    .concat(&child_path);

            match healing_cache.check_path(&cache_key) {
                PathStatus::ConfirmedExists => {}
                PathStatus::ProbablyExists | PathStatus::DefinitelyMissing => {
                    // Both cases need DB verification
                    if trie_state
                        .exists(child_path.clone())
                        .map_err(|e| StoreError::Custom(format!("Trie error: {e}")))?
                    {
                        healing_cache.mark_exists(&cache_key);
                    } else {
                        count = 1;
                        paths.push(NodeRequest {
                            acc_path: node_response.node_request.acc_path.clone(),
                            storage_path: child_path,
                            parent: node_response.node_request.storage_path.clone(),
                            hash: ext_node.child.compute_hash().finalize(),
                        });
                    }
                }
            }
        }

        _ => {}
    }

    Ok((paths, count))
}

fn commit_node(
    node: &NodeResponse,
    membatch: &mut Membatch,
    roots_healed: &mut usize,
    to_write: &mut HashMap<H256, Vec<(Nibbles, Node)>>,
) -> Result<(), StoreError> {
    let hashed_account = H256::from_slice(&node.node_request.acc_path.to_bytes());

    to_write
        .entry(hashed_account)
        .or_default()
        .push((node.node_request.storage_path.clone(), node.node.clone()));

    // Special case: root node
    if node.node_request.storage_path == node.node_request.parent {
        trace!("Committed storage root, healing should end for this account");
        *roots_healed += 1;
        return Ok(());
    }

    let parent_key = (
        node.node_request.acc_path.clone(),
        node.node_request.parent.clone(),
    );

    let mut parent_entry = membatch
        .remove(&parent_key)
        .expect("Parent missing from membatch!");

    parent_entry.missing_children_count -= 1;

    if parent_entry.missing_children_count == 0 {
        commit_node(&parent_entry.node_response, membatch, roots_healed, to_write)
    } else {
        membatch.insert(parent_key, parent_entry);
        Ok(())
    }
}

// ============================================================================
// SnapSyncTrie Storage Healing
// ============================================================================

/// Heals storage tries directly on SnapSyncTrie (for snap sync with ethrex_db backend).
///
/// Unlike the trie-node-based healing for the standard trie backend, this function
/// re-requests storage ranges for accounts that failed during initial download and
/// inserts them directly into the SnapSyncTrie.
///
/// # Arguments
/// * `state_root` - The expected state root from the pivot block
/// * `storage_accounts` - Contains the set of accounts that need healing
/// * `peers` - Peer handler for making snap protocol requests
/// * `snap_trie` - The SnapSyncTrie to insert healed storage into
/// * `staleness_timestamp` - Timestamp after which the pivot is considered stale
/// * `global_slots_healed` - Counter for tracking healed storage slots
///
/// # Returns
/// * `Ok(true)` if healing completed successfully
/// * `Ok(false)` if the pivot became stale during healing
/// * `Err(SyncError)` if healing failed
pub async fn heal_storage_trie_snap(
    state_root: H256,
    storage_accounts: &AccountStorageRoots,
    peers: &mut PeerHandler,
    snap_trie: &mut SnapSyncTrie,
    staleness_timestamp: u64,
    global_slots_healed: &mut u64,
) -> Result<bool, SyncError> {
    use tracing::info;

    let healed_accounts: Vec<H256> = storage_accounts.healed_accounts.iter().copied().collect();

    if healed_accounts.is_empty() {
        debug!("No accounts need storage healing");
        return Ok(true);
    }

    info!(
        "[SNAP SYNC] Starting storage healing for {} accounts",
        healed_accounts.len()
    );

    METRICS.current_step.set(CurrentStepValue::HealingStorage);

    let mut accounts_healed = 0usize;
    let mut slots_healed = 0u64;
    let mut last_progress_log = tokio::time::Instant::now();

    // Process accounts in batches to avoid overwhelming peers
    const ACCOUNTS_PER_BATCH: usize = 100;

    for batch in healed_accounts.chunks(ACCOUNTS_PER_BATCH) {
        // Check for staleness
        if current_unix_time() > staleness_timestamp {
            info!(
                "[SNAP SYNC] Storage healing interrupted due to stale pivot (healed {}/{} accounts)",
                accounts_healed,
                healed_accounts.len()
            );
            return Ok(false);
        }

        // Request storage ranges for this batch of accounts
        for account_hash in batch {
            // Get peer connection
            let Some((peer_id, mut connection)) = peers
                .peer_table
                .get_best_peer(&SUPPORTED_SNAP_CAPABILITIES)
                .await?
            else {
                debug!("No peers available for storage healing, retrying...");
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue;
            };

            // Request full storage range for this account
            let request = GetStorageRanges {
                id: rand::random(),
                root_hash: state_root,
                account_hashes: vec![*account_hash],
                starting_hash: H256::zero(),
                limit_hash: H256::repeat_byte(0xff),
                response_bytes: MAX_RESPONSE_BYTES,
            };

            match peers
                .request_storage_ranges_raw(&peer_id, &mut connection, request)
                .await
            {
                Ok(Some(response)) => {
                    // Process the storage slots
                    // response.slots is Vec<Vec<StorageSlot>> - one Vec<StorageSlot> per account
                    for slot in response.slots.into_iter().flatten() {
                        snap_trie.insert_storage(*account_hash, slot.hash, slot.data);
                        slots_healed += 1;
                    }
                    accounts_healed += 1;
                    peers.peer_table.record_success(&peer_id).await?;
                }
                Ok(None) => {
                    debug!(
                        "Empty response for account {:?}, peer may not have this data",
                        account_hash
                    );
                    peers.peer_table.record_failure(&peer_id).await?;
                }
                Err(e) => {
                    debug!(
                        "Failed to request storage for account {:?}: {:?}",
                        account_hash, e
                    );
                    peers.peer_table.record_failure(&peer_id).await?;
                }
            }
        }

        // Log progress periodically
        if last_progress_log.elapsed() >= SHOW_PROGRESS_INTERVAL_DURATION {
            info!(
                "[SNAP SYNC] Storage healing progress: {}/{} accounts, {} slots healed",
                accounts_healed,
                healed_accounts.len(),
                slots_healed
            );
            last_progress_log = tokio::time::Instant::now();
        }
    }

    *global_slots_healed += slots_healed;

    info!(
        "[SNAP SYNC] Storage healing complete: {} accounts, {} slots healed",
        accounts_healed, slots_healed
    );

    Ok(true)
}

/// Heals the account trie by requesting missing accounts from peers.
///
/// This function is called when the computed state root doesn't match the expected root.
/// It requests account ranges from peers and inserts any missing accounts into the snap trie.
///
/// # Arguments
/// * `expected_state_root` - The expected state root from the pivot block
/// * `peers` - Peer handler for making network requests
/// * `snap_trie` - The snap sync trie to heal
/// * `staleness_timestamp` - Unix timestamp when the pivot becomes stale
///
/// # Returns
/// * `Ok(true)` if healing completed successfully
/// * `Ok(false)` if the pivot became stale during healing
/// * `Err(SyncError)` if healing failed
pub async fn heal_accounts_snap(
    expected_state_root: H256,
    peers: &mut PeerHandler,
    snap_trie: &mut SnapSyncTrie,
    staleness_timestamp: u64,
) -> Result<bool, SyncError> {
    use crate::rlpx::snap::GetAccountRange;
    use tracing::info;

    info!("[SNAP SYNC] Starting account healing to fix state root mismatch");
    METRICS.current_step.set(CurrentStepValue::HealingState);

    let mut accounts_healed = 0u64;
    let mut last_progress_log = tokio::time::Instant::now();

    // Request accounts in ranges, starting from zero
    let mut current_start = H256::zero();
    let range_end = H256::repeat_byte(0xff);

    while current_start < range_end {
        // Check for staleness
        if current_unix_time() > staleness_timestamp {
            info!(
                "[SNAP SYNC] Account healing interrupted due to stale pivot (healed {} accounts)",
                accounts_healed
            );
            return Ok(false);
        }

        // Get peer connection
        let Some((peer_id, mut connection)) = peers
            .peer_table
            .get_best_peer(&SUPPORTED_SNAP_CAPABILITIES)
            .await?
        else {
            debug!("No peers available for account healing, retrying...");
            tokio::time::sleep(Duration::from_secs(1)).await;
            continue;
        };

        // Request account range
        let request = GetAccountRange {
            id: random(),
            root_hash: expected_state_root,
            starting_hash: current_start,
            limit_hash: range_end,
            response_bytes: MAX_RESPONSE_BYTES,
        };

        match peers
            .request_account_ranges_raw(&peer_id, &mut connection, request)
            .await
        {
            Ok(Some(response)) => {
                if response.accounts.is_empty() {
                    // No more accounts in this range
                    debug!("Received empty account range, healing complete");
                    break;
                }

                // Insert accounts into snap_trie
                for account in &response.accounts {
                    snap_trie.insert_account(
                        account.hash,
                        account.account.nonce,
                        account.account.balance,
                        account.account.storage_root,
                        account.account.code_hash,
                    );
                    accounts_healed += 1;
                }

                // Update start for next range (after last received account)
                if let Some(last) = response.accounts.last() {
                    // Increment by 1 to start after this account
                    let mut bytes = last.hash.to_fixed_bytes();
                    // Simple increment (handles overflow by wrapping, which is fine for healing)
                    for i in (0..32).rev() {
                        if bytes[i] < 255 {
                            bytes[i] += 1;
                            break;
                        }
                        bytes[i] = 0;
                    }
                    current_start = H256::from(bytes);
                } else {
                    break;
                }

                peers.peer_table.record_success(&peer_id).await?;
            }
            Ok(None) => {
                debug!("Empty or invalid response for account range request");
                peers.peer_table.record_failure(&peer_id).await?;
            }
            Err(e) => {
                debug!("Failed to request account range: {:?}", e);
                peers.peer_table.record_failure(&peer_id).await?;
                // Retry with different peer
                continue;
            }
        }

        // Log progress periodically
        if last_progress_log.elapsed() >= SHOW_PROGRESS_INTERVAL_DURATION {
            info!(
                "[SNAP SYNC] Account healing progress: {} accounts processed",
                accounts_healed
            );
            last_progress_log = tokio::time::Instant::now();
        }
    }

    info!(
        "[SNAP SYNC] Account healing complete: {} accounts processed",
        accounts_healed
    );

    Ok(true)
}
