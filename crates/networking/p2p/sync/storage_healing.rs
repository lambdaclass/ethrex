//! Optimized storage trie healing module
//!
//! This module heals storage tries with performance optimizations:
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


use bytes::Bytes;
use ethrex_common::{types::AccountState, H256, U256};
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode, error::RLPDecodeError};
use ethrex_storage::{error::StoreError, Store};
#[allow(unused_imports)]
use ethrex_trie::{Nibbles, Node, EMPTY_TRIE_HASH, TrieDB};
use rand::random;
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::atomic::Ordering,
    time::Duration,
};
use tokio::{
    sync::mpsc::{error::TrySendError, Sender},
    task::JoinSet,
};
use tracing::{debug, trace, warn};

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

/// Heals storage tries with batch operations
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

    let download_queue = get_initial_downloads(&store, state_root, storage_accounts);
    debug!(
        initial_accounts_count = download_queue.len(),
        "Started storage healing",
    );

    let mut state = StorageHealer {
        last_update: tokio::time::Instant::now(),
        download_queue,
        store,
        membatch,
        requests: HashMap::new(),
        staleness_timestamp,
        state_root,
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

            debug!(
                snap_peer_count,
                inflight_requests = state.requests.len(),
                download_queue_len = state.download_queue.len(),
                maximum_depth = state.maximum_length_seen,
                leaves_healed = state.leafs_healed,
                global_leaves_healed = global_leafs_healed,
                roots_healed = state.roots_healed,
                succesful_downloads = state.succesful_downloads,
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

                if !db_joinset.is_empty() {
                    db_joinset.join_next().await;
                }

                db_joinset.spawn_blocking(move || {
                    let mut encoded_to_write = vec![];

                    for (hashed_account, nodes) in to_write {
                        let mut account_nodes = vec![];
                        for (path, node) in nodes {
                            for i in 0..path.len() {
                                account_nodes.push((path.slice(0, i), vec![]));
                            }
                            account_nodes.push((path, node.encode_to_vec()));
                        }
                        encoded_to_write.push((hashed_account, account_nodes));
                    }

                    spawned_rt::tasks::block_on(store.write_storage_trie_nodes_batch(encoded_to_write))
                        .expect("db write failed");
                });
            }
        }

        if is_done {
            db_joinset.join_all().await;
            debug!("Storage healing complete");
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
            determine_missing_children_optimized(&node_response, store)?;

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

/// Determines which children of a node are missing from the storage trie.
/// Uses batch DB lookups for efficiency.
pub fn determine_missing_children_optimized(
    node_response: &NodeResponse,
    store: &Store,
) -> Result<(Vec<NodeRequest>, usize), StoreError> {
    let mut paths = Vec::new();
    let node = node_response.node.clone();

    let trie = store.open_direct_storage_trie(
        H256::from_slice(&node_response.node_request.acc_path.to_bytes()),
        *EMPTY_TRIE_HASH,
    )?;
    let trie_state = trie.db();

    match &node {
        Node::Branch(branch_node) => {
            // Collect all valid children
            let mut child_info: Vec<(Nibbles, &ethrex_trie::NodeRef)> = Vec::with_capacity(16);

            for (index, child) in branch_node.choices.iter().enumerate() {
                if !child.is_valid() {
                    continue;
                }
                let child_path = node_response
                    .node_request
                    .storage_path
                    .append_new(index as u8);
                child_info.push((child_path, child));
            }

            if child_info.is_empty() {
                return Ok((vec![], 0));
            }

            // Get all paths to check
            let paths_to_check: Vec<Nibbles> = child_info.iter().map(|(p, _)| p.clone()).collect();

            // Batch check DB
            let db_exists = trie_state
                .exists_batch(&paths_to_check)
                .map_err(|e| StoreError::Custom(format!("Trie error: {e}")))?;

            // Build missing children list
            for (i, exists) in db_exists.iter().enumerate() {
                if !exists {
                    let (ref child_path, child) = child_info[i];
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

            // Check DB
            if !trie_state
                .exists(child_path.clone())
                .map_err(|e| StoreError::Custom(format!("Trie error: {e}")))?
            {
                paths.push(NodeRequest {
                    acc_path: node_response.node_request.acc_path.clone(),
                    storage_path: child_path,
                    parent: node_response.node_request.storage_path.clone(),
                    hash: ext_node.child.compute_hash().finalize(),
                });
            }
        }

        _ => {}
    }

    let count = paths.len();
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
/// * `Ok((false, _))` if the pivot became stale during healing
/// * `Err(SyncError)` if healing failed
///
/// The returned HashSet contains accounts that were successfully queried (even if they had 0 slots).
/// Only these accounts should have their storage_root set to EMPTY_TRIE_HASH if they have no storage trie.
pub async fn heal_storage_trie_snap(
    state_root: H256,
    storage_accounts: &AccountStorageRoots,
    peers: &mut PeerHandler,
    snap_trie: &mut SnapSyncTrie,
    staleness_timestamp: u64,
    global_slots_healed: &mut u64,
) -> Result<(bool, HashSet<H256>), SyncError> {
    use tracing::info;

    let healed_accounts: Vec<H256> = storage_accounts.healed_accounts.iter().copied().collect();

    if healed_accounts.is_empty() {
        debug!("No accounts need storage healing");
        return Ok((true, HashSet::new()));
    }

    info!(
        "[SNAP SYNC] Starting storage healing for {} accounts",
        healed_accounts.len()
    );

    METRICS.current_step.set(CurrentStepValue::HealingStorage);

    let mut accounts_healed = 0usize;
    let mut slots_healed = 0u64;
    let mut last_progress_log = tokio::time::Instant::now();
    // Track accounts that were successfully queried (even if they had 0 slots)
    let mut successfully_queried: HashSet<H256> = HashSet::new();

    // Batch multiple accounts per request (protocol supports this)
    // Using 100 accounts per request to balance response size and efficiency
    const ACCOUNTS_PER_REQUEST: usize = 100;

    // Number of parallel requests to send at once
    const PARALLEL_REQUESTS: usize = 8;

    // Process accounts in parallel batches
    for parallel_batch in healed_accounts.chunks(ACCOUNTS_PER_REQUEST * PARALLEL_REQUESTS) {
        // Check for staleness before each parallel batch
        if current_unix_time() > staleness_timestamp {
            info!(
                "[SNAP SYNC] Storage healing interrupted due to stale pivot (healed {}/{} accounts)",
                accounts_healed,
                healed_accounts.len()
            );
            return Ok((false, successfully_queried));
        }

        // Split into sub-batches for parallel requests
        let sub_batches: Vec<Vec<H256>> = parallel_batch
            .chunks(ACCOUNTS_PER_REQUEST)
            .map(|c| c.to_vec())
            .collect();

        // Create a JoinSet for parallel requests
        // Returns: (accounts requested, slots received, peer_id, success flag)
        let mut join_set: JoinSet<Result<(Vec<H256>, Vec<(H256, H256, U256)>, H256, bool), SyncError>> = JoinSet::new();

        // Track accounts that couldn't be queued due to no peers
        let mut pending_accounts: Vec<Vec<H256>> = Vec::new();

        for sub_batch in sub_batches {
            let account_hashes = sub_batch;

            // Get peer connection for this request
            let Some((peer_id, connection)) = peers
                .peer_table
                .get_best_peer(&SUPPORTED_SNAP_CAPABILITIES)
                .await?
            else {
                debug!("No peers available for storage healing, will retry batch later");
                pending_accounts.push(account_hashes);
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            };

            let peer_table = peers.peer_table.clone();
            let request_state_root = state_root;

            // Spawn parallel request
            join_set.spawn(async move {
                let mut peer_table = peer_table; // Make mutable for record_success/record_failure
                let request = GetStorageRanges {
                    id: rand::random(),
                    root_hash: request_state_root,
                    account_hashes: account_hashes.clone(),
                    starting_hash: H256::zero(),
                    limit_hash: H256::repeat_byte(0xff),
                    response_bytes: MAX_RESPONSE_BYTES,
                };

                let result = PeerHandler::request_storage_ranges_static(
                    peer_id, connection, peer_table.clone(), request,
                )
                .await;

                match result {
                    Ok(Some(response)) => {
                        // Collect all slots with their account hashes
                        let mut all_slots = Vec::new();
                        let response_accounts = response.slots.len();
                        for (i, account_slots) in response.slots.into_iter().enumerate() {
                            if let Some(account_hash) = account_hashes.get(i) {
                                for slot in account_slots {
                                    all_slots.push((*account_hash, slot.hash, slot.data));
                                }
                            }
                        }
                        debug!(
                            "Storage healing response: requested {} accounts, got {} accounts, {} total slots",
                            account_hashes.len(), response_accounts, all_slots.len()
                        );
                        peer_table.record_success(&peer_id).await.ok();
                        Ok((account_hashes, all_slots, peer_id, true))
                    }
                    Ok(None) => {
                        debug!("Storage healing: peer returned None for {} accounts", account_hashes.len());
                        peer_table.record_failure(&peer_id).await.ok();
                        Ok((account_hashes, Vec::new(), peer_id, false))
                    }
                    Err(e) => {
                        debug!("Failed to request storage for batch of {} accounts: {:?}", account_hashes.len(), e);
                        peer_table.record_failure(&peer_id).await.ok();
                        Ok((account_hashes, Vec::new(), peer_id, false))
                    }
                }
            });
        }

        // Collect results from all parallel requests
        while let Some(result) = join_set.join_next().await {
            match result {
                Ok(Ok((account_hashes, slots, _peer_id, success))) => {
                    if success {
                        // Track all accounts in this batch as successfully queried
                        for account_hash in &account_hashes {
                            successfully_queried.insert(*account_hash);
                        }
                        // Insert all slots into the snap trie
                        for (account_hash, slot_hash, slot_data) in slots {
                            snap_trie.insert_storage(account_hash, slot_hash, slot_data);
                            slots_healed += 1;
                        }
                        accounts_healed += account_hashes.len();
                    } else {
                        debug!(
                            "Storage healing failed for batch of {} accounts (will NOT set to empty)",
                            account_hashes.len()
                        );
                    }
                }
                Ok(Err(e)) => {
                    debug!("Storage healing request failed: {:?}", e);
                }
                Err(e) => {
                    debug!("Storage healing task panicked: {:?}", e);
                }
            }
        }

        // Retry pending accounts that couldn't be queued due to no peers
        for pending_batch in pending_accounts {
            // Check staleness before retry
            if current_unix_time() > staleness_timestamp {
                debug!("Skipping pending account retry due to stale pivot");
                break;
            }

            let Some((peer_id, connection)) = peers
                .peer_table
                .get_best_peer(&SUPPORTED_SNAP_CAPABILITIES)
                .await?
            else {
                debug!("Still no peers for pending batch retry, skipping {} accounts", pending_batch.len());
                continue;
            };

            let request = GetStorageRanges {
                id: rand::random(),
                root_hash: state_root,
                account_hashes: pending_batch.clone(),
                starting_hash: H256::zero(),
                limit_hash: H256::repeat_byte(0xff),
                response_bytes: MAX_RESPONSE_BYTES,
            };

            match PeerHandler::request_storage_ranges_static(
                peer_id, connection, peers.peer_table.clone(), request,
            )
            .await
            {
                Ok(Some(response)) => {
                    // Track all accounts in this batch as successfully queried
                    for account_hash in &pending_batch {
                        successfully_queried.insert(*account_hash);
                    }
                    for (i, account_slots) in response.slots.into_iter().enumerate() {
                        if let Some(account_hash) = pending_batch.get(i) {
                            for slot in account_slots {
                                snap_trie.insert_storage(*account_hash, slot.hash, slot.data);
                                slots_healed += 1;
                            }
                        }
                    }
                    accounts_healed += pending_batch.len();
                    peers.peer_table.record_success(&peer_id).await?;
                }
                Ok(None) => {
                    debug!("Retry failed for batch of {} accounts (will NOT set to empty)", pending_batch.len());
                    peers.peer_table.record_failure(&peer_id).await?;
                }
                Err(e) => {
                    debug!("Retry error for batch: {:?} (will NOT set to empty)", e);
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
        "[SNAP SYNC] Storage healing complete: {} accounts, {} slots healed, {} successfully queried",
        accounts_healed, slots_healed, successfully_queried.len()
    );

    Ok((true, successfully_queried))
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
/// Returns (success: bool, accounts_with_storage: HashSet<H256>)
/// The accounts_with_storage set contains hashes of accounts that have non-empty storage
/// and need their storage to be re-healed.
pub async fn heal_accounts_snap(
    expected_state_root: H256,
    peers: &mut PeerHandler,
    snap_trie: &mut SnapSyncTrie,
    staleness_timestamp: u64,
) -> Result<(bool, std::collections::HashSet<H256>), SyncError> {
    use crate::rlpx::snap::GetAccountRange;
    use std::collections::HashSet;
    use tracing::info;

    info!("[SNAP SYNC] Starting account healing to fix state root mismatch");
    METRICS.current_step.set(CurrentStepValue::HealingState);

    let mut accounts_healed = 0u64;
    let mut accounts_with_storage: HashSet<H256> = HashSet::new();
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
            return Ok((false, accounts_with_storage));
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

                    // Track accounts with non-empty storage for subsequent storage healing
                    if account.account.storage_root != *EMPTY_TRIE_HASH {
                        accounts_with_storage.insert(account.hash);
                    }
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
        "[SNAP SYNC] Account healing complete: {} accounts processed, {} with non-empty storage",
        accounts_healed, accounts_with_storage.len()
    );

    Ok((true, accounts_with_storage))
}

/// Heals the state trie by traversing from the root and downloading missing nodes.
///
/// This function uses GetTrieNodes to walk the expected state trie from peers,
/// extracting account data from leaf nodes and inserting them into the SnapSyncTrie.
/// Unlike heal_accounts_snap which re-downloads all accounts via ranges, this
/// function only downloads the trie structure to find and fix specific missing accounts.
///
/// # Arguments
/// * `expected_state_root` - The state root we're trying to match
/// * `peers` - Peer handler for network requests
/// * `snap_trie` - The SnapSyncTrie to insert accounts into
/// * `staleness_timestamp` - Timestamp after which the pivot is considered stale
///
/// # Returns
/// * `Ok(true)` if healing completed successfully
/// * `Ok(false)` if the pivot became stale during healing
/// * `Err(SyncError)` if healing failed
pub async fn heal_state_trie_snap(
    expected_state_root: H256,
    peers: &mut PeerHandler,
    snap_trie: &mut SnapSyncTrie,
    staleness_timestamp: u64,
) -> Result<bool, SyncError> {
    use tracing::info;

    info!("[SNAP SYNC] Starting state trie healing via GetTrieNodes");
    METRICS.current_step.set(CurrentStepValue::HealingState);

    let mut accounts_healed = 0u64;
    let mut nodes_processed = 0u64;
    let mut paths_dropped = 0u64;
    let mut last_progress_log = tokio::time::Instant::now();
    let mut consecutive_empty_responses = 0u32;
    const MAX_RETRIES: u8 = 3;
    const MAX_CONSECUTIVE_EMPTY: u32 = 50; // Abort if too many consecutive empty responses

    // Queue of (path, hash, retry_count) tuples to fetch - start with root
    let mut paths_to_fetch: VecDeque<(Nibbles, H256, u8)> = VecDeque::new();
    paths_to_fetch.push_back((Nibbles::default(), expected_state_root, 0));

    // Queue of nodes to process locally (for inline nodes that don't need fetching)
    let mut nodes_to_process: VecDeque<(Nibbles, Node)> = VecDeque::new();

    // Track in-flight requests
    let mut inflight_requests = 0u32;
    let (response_tx, mut response_rx) = tokio::sync::mpsc::channel::<(
        H256, // peer_id
        Result<TrieNodes, String>,
        Vec<(Nibbles, H256, u8)>, // batch that was requested (with retry counts)
    )>(TASK_CHANNEL_CAPACITY);

    // Helper closure to process a node and extract children/accounts
    // Returns (accounts_found, nodes_processed_count)
    let mut process_node = |path: &Nibbles,
                            node: Node,
                            paths_to_fetch: &mut VecDeque<(Nibbles, H256, u8)>,
                            nodes_to_process: &mut VecDeque<(Nibbles, Node)>,
                            snap_trie: &mut SnapSyncTrie|
     -> (u64, u64) {
        let mut accounts_found = 0u64;
        let mut nodes_count = 1u64;

        match node {
            Node::Leaf(leaf_node) => {
                // This is an account - decode and insert
                if let Ok(account) = AccountState::decode(&leaf_node.value) {
                    // Compute full account hash from path + partial
                    let full_path = path.concat(&leaf_node.partial);
                    let path_bytes = full_path.to_bytes();

                    // Account hashes are always 32 bytes (64 nibbles)
                    // If path length is wrong, skip this node (shouldn't happen in valid trie)
                    if path_bytes.len() != 32 {
                        warn!(
                            "[SNAP SYNC] Invalid account path length: {} bytes (expected 32), path nibbles: {}, partial nibbles: {}",
                            path_bytes.len(),
                            path.len(),
                            leaf_node.partial.len()
                        );
                        return (accounts_found, nodes_count);
                    }
                    let account_hash = H256::from_slice(&path_bytes);

                    // Log account being inserted during healing (for debugging state root mismatches)
                    debug!(
                        "[SNAP SYNC] State healing: inserting account {:?} with nonce={}, storage_root={:?}, code_hash={:?}",
                        account_hash, account.nonce, account.storage_root, account.code_hash
                    );

                    // Insert into snap_trie
                    snap_trie.insert_account(
                        account_hash,
                        account.nonce,
                        account.balance,
                        account.storage_root,
                        account.code_hash,
                    );
                    accounts_found += 1;
                }
            }
            Node::Branch(branch_node) => {
                // Process all children
                for (i, choice) in branch_node.choices.iter().enumerate() {
                    let child_path = path.append_new(i as u8);
                    match choice {
                        // Hashed nodes need to be fetched from peers (skip empty/zero hash slots)
                        ethrex_trie::NodeRef::Hash(ethrex_trie::NodeHash::Hashed(child_hash)) => {
                            // Skip empty branch slots (zero hash means no child at this position)
                            if *child_hash != H256::zero() {
                                paths_to_fetch.push_back((child_path, *child_hash, 0));
                            }
                        }
                        // Inline nodes contain the encoded node data - decode and process locally
                        ethrex_trie::NodeRef::Hash(ethrex_trie::NodeHash::Inline(inline_data)) => {
                            let data = &inline_data.0[..inline_data.1 as usize];
                            if !data.is_empty() {
                                if let Ok(child_node) = Node::decode(data) {
                                    nodes_to_process.push_back((child_path, child_node));
                                }
                            }
                        }
                        // Embedded nodes are already decoded - process directly
                        ethrex_trie::NodeRef::Node(child_node, _) => {
                            nodes_to_process.push_back((child_path, (**child_node).clone()));
                        }
                    }
                }
            }
            Node::Extension(ext_node) => {
                let child_path = path.concat(&ext_node.prefix);
                match &ext_node.child {
                    // Hashed nodes need to be fetched from peers (skip empty/zero hash)
                    ethrex_trie::NodeRef::Hash(ethrex_trie::NodeHash::Hashed(child_hash)) => {
                        // Skip if zero hash (shouldn't happen for extensions, but be safe)
                        if *child_hash != H256::zero() {
                            paths_to_fetch.push_back((child_path, *child_hash, 0));
                        }
                    }
                    // Inline nodes contain the encoded node data - decode and process locally
                    ethrex_trie::NodeRef::Hash(ethrex_trie::NodeHash::Inline(inline_data)) => {
                        let data = &inline_data.0[..inline_data.1 as usize];
                        if !data.is_empty() {
                            if let Ok(child_node) = Node::decode(data) {
                                nodes_to_process.push_back((child_path, child_node));
                            }
                        }
                    }
                    // Embedded nodes are already decoded - process directly
                    ethrex_trie::NodeRef::Node(child_node, _) => {
                        nodes_to_process.push_back((child_path, (**child_node).clone()));
                    }
                }
            }
        }

        (accounts_found, nodes_count)
    };

    while !paths_to_fetch.is_empty() || !nodes_to_process.is_empty() || inflight_requests > 0 {
        // Check for staleness
        if current_unix_time() > staleness_timestamp {
            info!(
                "[SNAP SYNC] State trie healing interrupted due to stale pivot (healed {} accounts, {} nodes)",
                accounts_healed, nodes_processed
            );
            return Ok(false);
        }

        // Process any locally queued nodes first (inline nodes)
        while let Some((path, node)) = nodes_to_process.pop_front() {
            let (accounts, nodes) = process_node(
                &path,
                node,
                &mut paths_to_fetch,
                &mut nodes_to_process,
                snap_trie,
            );
            accounts_healed += accounts;
            nodes_processed += nodes;
        }

        // Process responses
        while let Ok((peer_id, result, batch)) = response_rx.try_recv() {
            inflight_requests -= 1;
            match result {
                Ok(trie_nodes) => {
                    let nodes_received = trie_nodes.nodes.len();
                    info!("[SNAP SYNC] Received {} nodes from peer for batch of {} paths", nodes_received, batch.len());

                    // Handle empty responses (0 nodes) - this indicates peer doesn't have the state
                    if nodes_received == 0 {
                        consecutive_empty_responses += 1;
                        peers.peer_table.record_failure(&peer_id).await?;

                        // Check if we should abort due to too many consecutive empty responses
                        if consecutive_empty_responses >= MAX_CONSECUTIVE_EMPTY {
                            warn!(
                                "[SNAP SYNC] Aborting state healing: {} consecutive empty responses - state root may be unavailable on peers",
                                consecutive_empty_responses
                            );
                            return Ok(false);
                        }

                        // Re-queue paths with incremented retry count, dropping those that exceed max
                        for (path, hash, retry_count) in batch {
                            if retry_count >= MAX_RETRIES {
                                debug!("[SNAP SYNC] Dropping path {:?} after {} retries", path, retry_count);
                                paths_dropped += 1;
                            } else {
                                paths_to_fetch.push_back((path, hash, retry_count + 1));
                            }
                        }
                        continue;
                    }

                    // Got actual nodes - record success and reset consecutive empty counter
                    peers.peer_table.record_success(&peer_id).await?;
                    consecutive_empty_responses = 0;

                    // Track paths that failed to decode (need to retry with different peer)
                    let mut failed_paths: Vec<(Nibbles, H256, u8)> = Vec::new();
                    let mut empty_count = 0u64;

                    // Process each node in the response
                    for (i, node_bytes) in trie_nodes.nodes.iter().enumerate() {
                        if i >= batch.len() {
                            break;
                        }
                        let (path, hash, retry_count) = &batch[i];

                        // Empty responses mean the node doesn't exist at this path.
                        // This is valid for sparse branch nodes - don't retry.
                        if node_bytes.is_empty() {
                            trace!("[SNAP SYNC] Empty response for path {:?} (hash {:?}), node doesn't exist", path, hash);
                            empty_count += 1;
                            continue;
                        }

                        // Decode the node and process it
                        match Node::decode(node_bytes) {
                            Ok(node) => {
                                // Debug: log what node type we got (at trace level to reduce noise)
                                match &node {
                                    Node::Leaf(_) => trace!("[SNAP SYNC] Decoded LEAF node at path {:?}", path),
                                    Node::Branch(b) => {
                                        let non_empty = b.choices.iter().filter(|c| c.is_valid()).count();
                                        trace!("[SNAP SYNC] Decoded BRANCH node at path {:?} with {} non-empty children", path, non_empty);
                                    }
                                    Node::Extension(e) => trace!("[SNAP SYNC] Decoded EXTENSION node at path {:?} with prefix {:?}", path, e.prefix),
                                }
                                let (accounts, nodes) = process_node(
                                    path,
                                    node,
                                    &mut paths_to_fetch,
                                    &mut nodes_to_process,
                                    snap_trie,
                                );
                                accounts_healed += accounts;
                                nodes_processed += nodes;
                            }
                            Err(e) => {
                                warn!("[SNAP SYNC] Failed to decode node at path {:?}: {:?}, bytes len: {}", path, e, node_bytes.len());
                                // Re-queue non-empty failed decodes to try with different peer
                                failed_paths.push((path.clone(), *hash, *retry_count));
                            }
                        }
                    }

                    // Log if many empty responses (might indicate state root mismatch)
                    if empty_count > 0 && nodes_received > 0 {
                        let empty_ratio = empty_count as f64 / nodes_received as f64;
                        if empty_ratio > 0.5 {
                            warn!(
                                "[SNAP SYNC] High ratio of empty responses: {}/{} ({:.1}%) - peer may not have this state root",
                                empty_count, nodes_received, empty_ratio * 100.0
                            );
                        }
                    }

                    // Re-queue paths that failed to decode (non-empty bytes that couldn't be decoded)
                    if !failed_paths.is_empty() {
                        debug!(
                            "[SNAP SYNC] Re-queueing {} paths that failed to decode",
                            failed_paths.len()
                        );
                        for (path, hash, retry_count) in failed_paths {
                            if retry_count >= MAX_RETRIES {
                                paths_dropped += 1;
                            } else {
                                paths_to_fetch.push_back((path, hash, retry_count + 1));
                            }
                        }
                    }

                    // Re-queue any paths that weren't returned by the peer (with incremented retry count)
                    if nodes_received < batch.len() {
                        debug!(
                            "[SNAP SYNC] Peer returned fewer nodes than requested ({}/{}), re-queueing missing",
                            nodes_received, batch.len()
                        );
                        for (path, hash, retry_count) in batch.into_iter().skip(nodes_received) {
                            if retry_count >= MAX_RETRIES {
                                paths_dropped += 1;
                            } else {
                                paths_to_fetch.push_back((path, hash, retry_count + 1));
                            }
                        }
                    }
                }
                Err(e) => {
                    debug!("GetTrieNodes request failed: {}", e);
                    peers.peer_table.record_failure(&peer_id).await?;
                    // Re-queue the failed batch with incremented retry count
                    for (path, hash, retry_count) in batch {
                        if retry_count >= MAX_RETRIES {
                            paths_dropped += 1;
                        } else {
                            paths_to_fetch.push_back((path, hash, retry_count + 1));
                        }
                    }
                }
            }
        }

        // Send new requests if we have capacity
        while inflight_requests < MAX_IN_FLIGHT_REQUESTS && !paths_to_fetch.is_empty() {
            // Build batch of paths to request
            let mut batch: Vec<(Nibbles, H256, u8)> = Vec::new();
            let mut paths_for_request: Vec<Bytes> = Vec::new();

            while batch.len() < 256 && !paths_to_fetch.is_empty() {
                if let Some((path, hash, retry_count)) = paths_to_fetch.pop_front() {
                    // Snap protocol expects compact (HP) encoded paths for partial paths (<32 bytes)
                    // and plain binary for full paths (32 bytes).
                    // Compact encoding packs nibbles and adds a prefix byte for odd/even length.
                    // Example: nibbles [1, 2, 3, 4] → [0x00, 0x12, 0x34] (even, extension)
                    // Example: nibbles [1, 2, 3] → [0x11, 0x23] (odd, extension)
                    let encoded = Bytes::from(path.encode_compact());
                    paths_for_request.push(encoded);
                    batch.push((path, hash, retry_count));
                }
            }

            if batch.is_empty() {
                break;
            }

            // Get a peer
            let Some((peer_id, mut connection)) = peers
                .peer_table
                .get_best_peer(&SUPPORTED_SNAP_CAPABILITIES)
                .await?
            else {
                // No peers available, re-queue and wait
                for item in batch {
                    paths_to_fetch.push_front(item);
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
                break;
            };

            // Send request
            // IMPORTANT: For state trie (account) nodes, each path must be its own pathset!
            // The snap protocol structure is: [[path1], [path2], [path3], ...]
            // NOT [[path1, path2, path3, ...]] which would be interpreted as prefix+suffixes
            let request = GetTrieNodes {
                id: random(),
                root_hash: expected_state_root,
                paths: paths_for_request.iter().map(|p| vec![p.clone()]).collect(),
                bytes: MAX_RESPONSE_BYTES,
            };

            // Debug: log what we're requesting (first request only or small batches)
            if batch.len() <= 3 || (accounts_healed == 0 && nodes_processed <= 1) {
                info!("[SNAP SYNC] Requesting {} paths under root {:?}",
                    batch.len(), expected_state_root);
                for (p, h, _) in batch.iter().take(5) {
                    info!("[SNAP SYNC]   path={:?} hash={:?}", p, h);
                }
                info!("[SNAP SYNC] Encoded paths (first 5): {:?}",
                    paths_for_request.iter().take(5).map(|p| hex::encode(p.as_ref())).collect::<Vec<_>>());
            }

            let response_tx_clone = response_tx.clone();
            let batch_clone = batch.clone();
            let peer_id_clone = peer_id;

            tokio::spawn(async move {
                use crate::rlpx::message::Message as RLPxMessage;
                let result = match connection
                    .outgoing_request(RLPxMessage::GetTrieNodes(request), Duration::from_secs(10))
                    .await
                {
                    Ok(RLPxMessage::TrieNodes(nodes)) => Ok(nodes),
                    Ok(_) => Err("Unexpected response type".to_string()),
                    Err(e) => Err(format!("{:?}", e)),
                };
                let _ = response_tx_clone.send((peer_id_clone, result, batch_clone)).await;
            });

            inflight_requests += 1;
        }

        // Log progress
        if last_progress_log.elapsed() >= SHOW_PROGRESS_INTERVAL_DURATION {
            info!(
                "[SNAP SYNC] State trie healing progress: {} accounts found, {} nodes processed, {} paths pending, {} dropped, {} consecutive empty",
                accounts_healed, nodes_processed, paths_to_fetch.len(), paths_dropped, consecutive_empty_responses
            );
            last_progress_log = tokio::time::Instant::now();
        }

        // Small sleep to avoid busy loop
        if paths_to_fetch.is_empty() && nodes_to_process.is_empty() && inflight_requests > 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    info!(
        "[SNAP SYNC] State trie healing complete: {} accounts found/inserted, {} nodes processed, {} paths dropped",
        accounts_healed, nodes_processed, paths_dropped
    );

    // Log warning if we found accounts during healing (means initial download was incomplete)
    if accounts_healed > 0 {
        warn!(
            "[SNAP SYNC] DEBUG: State trie healing inserted {} accounts that were missing from initial download!",
            accounts_healed
        );
    }

    // Log warning if many paths were dropped (state root may have been unavailable)
    if paths_dropped > 0 {
        warn!(
            "[SNAP SYNC] Dropped {} paths during state healing (state may be partially unavailable on peers)",
            paths_dropped
        );
    }

    Ok(true)
}
