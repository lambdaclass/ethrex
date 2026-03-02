use crate::{
    metrics::{CurrentStepValue, METRICS},
    peer_handler::PeerHandler,
    rlpx::{
        p2p::SUPPORTED_SNAP_CAPABILITIES,
        snap::{GetTrieNodes, TrieNodes},
    },
    snap::{
        RequestStorageTrieNodesError,
        constants::{
            MAX_IN_FLIGHT_REQUESTS, MAX_RESPONSE_BYTES, SHOW_PROGRESS_INTERVAL_DURATION,
            STORAGE_BATCH_SIZE,
        },
        request_storage_trienodes,
    },
    sync::{AccountStorageRoots, SyncError},
    utils::current_unix_time,
};

use bytes::Bytes;
use ethrex_common::{H256, types::AccountState};
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode, error::RLPDecodeError};
use ethrex_storage::{Store, error::StoreError};
use ethrex_trie::{EMPTY_TRIE_HASH, Nibbles, Node};
use rand::random;
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use std::{
    collections::{HashMap, VecDeque},
    sync::atomic::Ordering,
    time::Instant,
};
use tokio::{sync::mpsc::error::TryRecvError, task::JoinSet};
use tokio::{
    sync::mpsc::{Sender, error::TrySendError},
    task::yield_now,
};
use tracing::{debug, trace, warn};

/// This struct stores the metadata we need when we request a node
#[derive(Debug, Clone)]
pub struct NodeResponse {
    /// Who is this node
    node: Node,
    /// What did we ask for
    node_request: NodeRequest,
}

/// This struct stores the metadata we need when we store a node in the memory bank before storing
#[derive(Debug, Clone)]
pub struct StorageHealingQueueEntry {
    /// What this node is
    node_response: NodeResponse,
    /// How many missing children this node has
    /// if this number is 0, it should be flushed to the db, not stored in memory
    pending_children_count: usize,
}

/// The healing queue key represents the account path and the storage path
type StorageHealingQueueKey = (Nibbles, Nibbles);

pub type StorageHealingQueue = HashMap<StorageHealingQueueKey, StorageHealingQueueEntry>;

#[derive(Debug, Clone)]
pub struct InflightRequest {
    requests: Vec<NodeRequest>,
    peer_id: H256,
}

#[derive(Debug, Clone)]
pub struct StorageHealer {
    last_update: Instant,
    /// We use this to track what is still to be downloaded
    /// After processing the nodes it may be left empty,
    /// but if we have too many requests in flight
    /// we may want to throttle the new requests
    download_queue: VecDeque<NodeRequest>,
    /// Arc<dyn> to the db, clone freely
    store: Store,
    /// Memory of everything stored
    healing_queue: StorageHealingQueue,
    /// With this we track how many requests are inflight to our peer
    /// This allows us to know if one is wildly out of time
    requests: HashMap<u64, InflightRequest>,
    /// When we ask if we have finished, we check is the staleness
    /// If stale we stop
    staleness_timestamp: u64,
    /// What state tree is our pivot at
    state_root: H256,

    /// Data for analytics
    maximum_length_seen: usize,
    leafs_healed: usize,
    roots_healed: usize,
    succesful_downloads: usize,
    failed_downloads: usize,
    empty_count: usize,
    disconnected_count: usize,
}

/// This struct stores the metadata we need when we request a node
#[derive(Debug, Clone, Default)]
pub struct NodeRequest {
    /// What account this belongs too (so what is the storage tree)
    acc_path: Nibbles,
    /// Where in the tree is this node located
    storage_path: Nibbles,
    /// What node needs this node
    parent: Nibbles,
    /// What hash was requested. We use this for validation
    hash: H256,
}

/// This algorithm 'heals' the storage trie. That is to say, it downloads data until all accounts have the storage indicated
/// by the storage root in their account state
/// We receive a list of the counts that we want to save, we heal by chunks of accounts.
/// We assume these accounts are not empty hash tries, but may or may not have their
/// Algorithmic rules:
/// - If a nodehash is present in the db, it and all of its children are present in the db
/// - If we are missing a node, we queue to download them.
/// - When a node is downloaded:
///    - if it has no missing children, we store it in the db
///    - if the node has missing children, we store it in our healing_queue, which is preserved between calls
pub async fn heal_storage_trie(
    state_root: H256,
    storage_accounts: &AccountStorageRoots,
    peers: &mut PeerHandler,
    store: Store,
    healing_queue: StorageHealingQueue,
    staleness_timestamp: u64,
    global_leafs_healed: &mut u64,
) -> Result<bool, SyncError> {
    METRICS.current_step.set(CurrentStepValue::HealingStorage);
    let download_queue = get_initial_downloads(&store, state_root, storage_accounts);
    debug!(
        initial_accounts_count = download_queue.len(),
        "Started Storage Healing",
    );
    let mut state = StorageHealer {
        last_update: Instant::now(),
        download_queue,
        store,
        healing_queue,
        requests: HashMap::new(),
        staleness_timestamp,
        state_root,
        maximum_length_seen: Default::default(),
        leafs_healed: Default::default(),
        roots_healed: Default::default(),
        succesful_downloads: Default::default(),
        failed_downloads: Default::default(),
        empty_count: Default::default(),
        disconnected_count: Default::default(),
    };

    // With this we track what's going on with the tasks in flight
    // Only really relevant right now for debugging purposes.
    // TODO: think if this is a better way to receiver the data
    // Not in the state because it's not clonable
    let mut requests_task_joinset: JoinSet<
        Result<u64, TrySendError<Result<TrieNodes, RequestStorageTrieNodesError>>>,
    > = JoinSet::new();

    let mut nodes_to_write: HashMap<H256, Vec<(Nibbles, Node)>> = HashMap::new();
    let mut db_joinset = tokio::task::JoinSet::new();

    // channel to send the tasks to the peers
    let (task_sender, mut task_receiver) =
        tokio::sync::mpsc::channel::<Result<TrieNodes, RequestStorageTrieNodesError>>(1000);

    let mut logged_no_free_peers_count = 0;

    loop {
        yield_now().await;
        if state.last_update.elapsed() >= SHOW_PROGRESS_INTERVAL_DURATION {
            METRICS
                .global_storage_tries_leafs_healed
                .store(*global_leafs_healed, Ordering::Relaxed);
            METRICS
                .healing_empty_try_recv
                .store(state.empty_count as u64, Ordering::Relaxed);
            state.last_update = Instant::now();
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
                succesful_downloads_percentage = state.succesful_downloads as f64
                    / (state.succesful_downloads as f64 + state.failed_downloads as f64),
                empty_count = state.empty_count,
                disconnected_count = state.disconnected_count,
                "We are storage healing",
            );
            //. Snap Peers {}. Inflight tasks {}. Download Queue {}. Maximum length {}. Leafs Healed {}. Global Leafs Healed {}. Roots Healed {}. Good Downloads {}. Good Download Percentage {}. Empty count {}. Disconnected Count {}.
            state.succesful_downloads = 0;
            state.failed_downloads = 0;
            state.empty_count = 0;
            state.disconnected_count = 0;
        }

        let is_done = state.requests.is_empty() && state.download_queue.is_empty();
        let is_stale = current_unix_time() > state.staleness_timestamp;

        if nodes_to_write.values().map(Vec::len).sum::<usize>() > 100_000 || is_done || is_stale {
            let to_write: Vec<_> = nodes_to_write.drain().collect();
            let store = state.store.clone();
            // NOTE: we keep only a single task in the background to avoid out of order deletes
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
                // PERF: use put_batch_no_alloc? (it needs to remove parent nodes too)
                spawned_rt::tasks::block_on(store.write_storage_trie_nodes_batch(encoded_to_write))
                    .expect("db write failed");
            });
        }

        if is_done {
            db_joinset.join_all().await;
            return Ok(true);
        }

        if is_stale {
            db_joinset.join_all().await;
            state.healing_queue = HashMap::new();
            return Ok(false);
        }

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

        let trie_nodes_result = match task_receiver.try_recv() {
            Ok(trie_nodes) => trie_nodes,
            Err(TryRecvError::Empty) => {
                state.empty_count += 1;
                continue;
            }
            Err(TryRecvError::Disconnected) => {
                state.disconnected_count += 1;
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
                    &mut state.healing_queue,
                    &mut state.leafs_healed,
                    global_leafs_healed,
                    &mut state.roots_healed,
                    &mut state.maximum_length_seen,
                    &mut nodes_to_write,
                )
                .expect("We shouldn't be getting store errors"); // TODO: if we have a store error we should stop
            }
            Err(RequestStorageTrieNodesError {
                request_id,
                source: _err,
            }) => {
                let inflight_request = state
                    .requests
                    .remove(&request_id)
                    .expect("request disappeared");
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

/// it grabs N peers to ask for data
async fn ask_peers_for_nodes(
    download_queue: &mut VecDeque<NodeRequest>,
    requests: &mut HashMap<u64, InflightRequest>,
    requests_task_joinset: &mut JoinSet<
        Result<u64, TrySendError<Result<TrieNodes, RequestStorageTrieNodesError>>>,
    >,
    peers: &mut PeerHandler,
    state_root: H256,
    task_sender: &Sender<Result<TrieNodes, RequestStorageTrieNodesError>>,
    logged_no_free_peers_count: &mut u32,
) {
    while (requests.len() as u32) < MAX_IN_FLIGHT_REQUESTS && !download_queue.is_empty() {
        let Some((peer_id, connection)) = peers
            .peer_table
            .get_best_peer(&SUPPORTED_SNAP_CAPABILITIES)
            .await
            .inspect_err(|err| debug!(?err, "Error requesting a peer to perform storage healing"))
            .unwrap_or(None)
        else {
            // Log ~ once every 10 seconds
            if *logged_no_free_peers_count == 0 {
                trace!("We are missing peers in heal_storage_trie");
                *logged_no_free_peers_count = 1000;
            }
            *logged_no_free_peers_count -= 1;
            // Sleep for a bit to avoid busy polling
            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
            break;
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
            let response = request_storage_trienodes(peer_id, connection, peer_table, gtn).await;
            // TODO: add error handling
            tx.try_send(response).inspect_err(
                |err| debug!(error=?err, "Failed to send state trie nodes response"),
            )?;
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
        trie_response_len=?trie_nodes.nodes.len(),
        "We are processing the nodes",
    );
    let Some(request) = requests.remove(&trie_nodes.id) else {
        debug!(
            ?trie_nodes,
            "No matching request found for received response"
        );
        return Ok(None);
    };

    let nodes_size = trie_nodes.nodes.len();
    if nodes_size == 0 {
        *failed_downloads += 1;
        peer_handler
            .peer_table
            .record_failure(&request.peer_id)
            .await?;

        download_queue.extend(request.requests);
        return Ok(None);
    }

    if request.requests.len() < nodes_size {
        warn!(
            peer = ?request.peer_id,
            requested = request.requests.len(),
            received = nodes_size,
            "Peer responded with more trie nodes than requested"
        );
        *failed_downloads += 1;
        peer_handler
            .peer_table
            .record_failure(&request.peer_id)
            .await?;
        download_queue.extend(request.requests);
        return Ok(None);
    }

    if let Ok(nodes) = request
        .requests
        .iter()
        .zip(trie_nodes.nodes.clone())
        .map(|(node_request, node_bytes)| {
            let node = Node::decode(&node_bytes).inspect_err(|err| {
                trace!(
                    peer=?request.peer_id,
                    ?node_request,
                    error=?err,
                    ?node_bytes,
                    "Decode Failed"
                )
            })?;

            if node.compute_hash().finalize() != node_request.hash {
                trace!(
                    peer=?request.peer_id,
                    ?node_request,
                    ?node_bytes,
                    "Node Hash failed"
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
    healing_queue: &mut StorageHealingQueue,
    leafs_healed: &mut usize,
    global_leafs_healed: &mut u64,
    roots_healed: &mut usize,
    maximum_length_seen: &mut usize,
    to_write: &mut HashMap<H256, Vec<(Nibbles, Node)>>,
) -> Result<(), StoreError> {
    while let Some(node_response) = node_processing_queue.pop() {
        trace!(?node_response, "We are processing node response");
        if let Node::Leaf(_) = &node_response.node {
            *leafs_healed += 1;
            *global_leafs_healed += 1;
        };

        *maximum_length_seen = usize::max(
            *maximum_length_seen,
            node_response.node_request.storage_path.len(),
        );

        let (pending_children_nibbles, pending_children_count) =
            determine_pending_children(&node_response, store).inspect_err(|err| {
                debug!(
                    error=?err,
                    ?node_response,
                    "Error in determine_pending_children"
                )
            })?;

        if pending_children_count == 0 {
            // We flush to the database this node
            commit_node(&node_response, healing_queue, roots_healed, to_write).inspect_err(
                |err| {
                    debug!(
                        error=?err,
                        ?node_response,
                        "Error in commit_node"
                    )
                },
            )?;
        } else {
            let key = (
                node_response.node_request.acc_path.clone(),
                node_response.node_request.storage_path.clone(),
            );
            healing_queue.insert(
                key,
                StorageHealingQueueEntry {
                    node_response: node_response.clone(),
                    pending_children_count,
                },
            );
            download_queue.extend(pending_children_nibbles);
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
        .expect("We should be able to open the store");
    let mut initial_requests: VecDeque<NodeRequest> = VecDeque::new();
    initial_requests.extend(
        account_paths
            .healed_accounts
            .par_iter()
            .filter_map(|acc_path| {
                // Accounts can be deleted from the trie after the healing process happens
                // This is an edge case where an account with value got deleted by
                // a self destruct contract creation step
                let rlp = trie
                    .get(acc_path.as_bytes())
                    .expect("We should be able to open the store")?;
                let account = AccountState::decode(&rlp).expect("We should have a valid account");
                if account.storage_root == *EMPTY_TRIE_HASH {
                    return None;
                }

                Some(NodeRequest {
                    acc_path: Nibbles::from_bytes(&acc_path.0),
                    storage_path: Nibbles::default(), // We need to be careful, the root parent is a special case
                    parent: Nibbles::default(),
                    hash: account.storage_root,
                })
            })
            .collect::<VecDeque<_>>(),
    );
    initial_requests
}

/// Returns the full paths to the node's missing children and grandchildren
/// and the number of direct missing children
pub fn determine_pending_children(
    node_response: &NodeResponse,
    store: &Store,
) -> Result<(Vec<NodeRequest>, usize), StoreError> {
    let mut paths = Vec::new();
    let mut count = 0;
    let node = node_response.node.clone();
    let trie = store
        .open_direct_storage_trie(
            H256::from_slice(&node_response.node_request.acc_path.to_bytes()),
            *EMPTY_TRIE_HASH,
        )
        .inspect_err(|_| {
            debug!("Malformed data when opening the storage trie in determine missing children")
        })?;
    let trie_state = trie.db();

    match &node {
        Node::Branch(node) => {
            for (index, child) in node.choices.iter().enumerate() {
                let child_path = node_response
                    .node_request
                    .storage_path
                    .append_new(index as u8);
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
                count += 1;

                paths.extend(vec![NodeRequest {
                    acc_path: node_response.node_request.acc_path.clone(),
                    storage_path: child_path,
                    parent: node_response.node_request.storage_path.clone(),
                    hash: child.compute_hash().finalize(),
                }]);
            }
        }
        Node::Extension(node) => {
            let child_path = node_response.node_request.storage_path.concat(&node.prefix);
            if !node.child.is_valid() {
                return Ok((vec![], 0));
            }
            let validity = node
                .child
                .get_node_checked(trie_state, child_path.clone())
                .inspect_err(|_| debug!("Malformed data when doing get child of a branch node"))?
                .is_some();

            if validity {
                return Ok((vec![], 0));
            }
            count += 1;

            paths.extend(vec![NodeRequest {
                acc_path: node_response.node_request.acc_path.clone(),
                storage_path: child_path,
                parent: node_response.node_request.storage_path.clone(),
                hash: node.child.compute_hash().finalize(),
            }]);
        }
        _ => {}
    }
    Ok((paths, count))
}

fn commit_node(
    node: &NodeResponse,
    healing_queue: &mut StorageHealingQueue,
    roots_healed: &mut usize,
    to_write: &mut HashMap<H256, Vec<(Nibbles, Node)>>,
) -> Result<(), StoreError> {
    let hashed_account = H256::from_slice(&node.node_request.acc_path.to_bytes());

    to_write
        .entry(hashed_account)
        .or_default()
        .push((node.node_request.storage_path.clone(), node.node.clone()));

    // Special case, we have just commited the root, we stop
    if node.node_request.storage_path == node.node_request.parent {
        trace!(
            "We have the parent of an account, this means we are the root. Storage healing should end."
        );
        *roots_healed += 1;
        return Ok(());
    }

    let parent_key = (
        node.node_request.acc_path.clone(),
        node.node_request.parent.clone(),
    );

    let mut parent_entry = healing_queue
        .remove(&parent_key)
        .expect("We are missing the parent from the healing_queue!");

    parent_entry.pending_children_count -= 1;

    if parent_entry.pending_children_count == 0 {
        commit_node(
            &parent_entry.node_response,
            healing_queue,
            roots_healed,
            to_write,
        )
    } else {
        healing_queue.insert(parent_key, parent_entry);
        Ok(())
    }
}
