use crate::{
    kademlia::PeerChannels,
    metrics::METRICS,
    peer_handler::{MAX_RESPONSE_BYTES, PeerHandler, RequestStorageTrieNodes},
    rlpx::{
        p2p::SUPPORTED_SNAP_CAPABILITIES,
        snap::{GetTrieNodes, TrieNodes},
    },
    sync::{
        AccountStorageRoots,
        state_healing::{SHOW_PROGRESS_INTERVAL_DURATION, STORAGE_BATCH_SIZE},
    },
    utils::current_unix_time,
};

use bytes::Bytes;
use ethrex_common::{H256, types::AccountState};
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode, error::RLPDecodeError};
use ethrex_storage::{Store, error::StoreError};
use ethrex_trie::{EMPTY_TRIE_HASH, Nibbles, Node, NodeHash};
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
use tracing::{debug, error, info, trace};

const MAX_IN_FLIGHT_REQUESTS: u32 = 77;

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
pub struct MembatchEntry {
    /// What this node is
    node_response: NodeResponse,
    /// How many missing children this node has
    /// if this number is 0, it should be flushed to the db, not stored in memory
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
    last_update: Instant,
    /// We use this to track what is still to be downloaded
    /// After processing the nodes it may be left empty,
    /// but if we have too many requests in flight
    /// we may want to throttle the new requests
    download_queue: VecDeque<NodeRequest>,
    /// Arc<dyn> to the db, clone freely
    store: Store,
    /// Memory of everything stored
    membatch: Membatch,
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
/// - If a nodehash is present in the db, it and all of it's children are present in the db
/// - If we are missing a node, we queue to download them.
/// - When a node is downloaded:
///    - if it has no missing children, we store it in the db
///    - if the node has missing childre, we store it in our membatch, wchich is preserved between calls
pub async fn heal_storage_trie(
    state_root: H256,
    storage_accounts: &AccountStorageRoots,
    peers: &mut PeerHandler,
    store: Store,
    membatch: Membatch,
    staleness_timestamp: u64,
    global_leafs_healed: &mut u64,
) -> bool {
    *METRICS.current_step.lock().await = "Healing Storage".to_string();
    let download_queue = get_initial_downloads(&store, state_root, storage_accounts);
    info!(
        "Started Storage Healing with {} accounts",
        download_queue.len()
    );
    let mut state = StorageHealer {
        last_update: Instant::now(),
        download_queue,
        store,
        membatch,
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
        Result<u64, TrySendError<Result<TrieNodes, RequestStorageTrieNodes>>>,
    > = JoinSet::new();

    let mut nodes_to_write: HashMap<H256, Vec<(NodeHash, Vec<u8>)>> = HashMap::new();
    let mut db_joinset = tokio::task::JoinSet::new();

    // channel to send the tasks to the peers
    let (task_sender, mut task_receiver) =
        tokio::sync::mpsc::channel::<Result<TrieNodes, RequestStorageTrieNodes>>(1000);

    loop {
        yield_now().await;
        if state.last_update.elapsed() >= SHOW_PROGRESS_INTERVAL_DURATION {
            METRICS
                .global_storage_tries_leafs_healed
                .fetch_add(*global_leafs_healed, Ordering::Relaxed);
            METRICS
                .healing_empty_try_recv
                .store(state.empty_count as u64, Ordering::Relaxed);
            state.last_update = Instant::now();
            peers
                .peer_scores
                .lock()
                .await
                .update_peers(&peers.peer_table)
                .await;
            debug!(
                "We are storage healing. Snap Peers {}. Inflight tasks {}. Download Queue {}. Maximum length {}. Leafs Healed {}. Global Leafs Healed {global_leafs_healed}. Roots Healed {}. Good Download Percentage {}. Empty count {}. Disconnected Count {}.",
                peers
                    .peer_table
                    .get_peer_channels(&SUPPORTED_SNAP_CAPABILITIES)
                    .await
                    .len(),
                state.requests.len(),
                state.download_queue.len(),
                state.maximum_length_seen,
                state.leafs_healed,
                state.roots_healed,
                state.succesful_downloads as f64
                    / (state.succesful_downloads as f64 + state.failed_downloads as f64),
                state.empty_count,
                state.disconnected_count,
            );
            state.succesful_downloads = 0;
            state.failed_downloads = 0;
            state.empty_count = 0;
            state.disconnected_count = 0;
        }

        let is_done = state.requests.is_empty() && state.download_queue.is_empty();
        let is_stale = current_unix_time() > state.staleness_timestamp;

        if nodes_to_write.values().map(Vec::len).sum::<usize>() > 100_000 || is_done || is_stale {
            let to_write = nodes_to_write.drain().collect();
            let store = state.store.clone();
            if db_joinset.len() > 3 {
                db_joinset.join_next().await;
            }
            db_joinset.spawn_blocking(|| {
                spawned_rt::tasks::block_on(async move {
                    store
                        .write_storage_trie_nodes_batch(to_write)
                        .await
                        .expect("db write failed");
                })
            });
        }

        if is_done {
            db_joinset.join_all().await;
            return true;
        }

        if is_stale {
            db_joinset.join_all().await;
            state.membatch = HashMap::new();
            return false;
        }

        ask_peers_for_nodes(
            &mut state.download_queue,
            &mut state.requests,
            &mut requests_task_joinset,
            &peers,
            state.state_root,
            &task_sender,
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
                    trie_nodes.clone(), // TODO: remove unnecesary clone, needed now for log 🏗️🏗️
                    &mut state.succesful_downloads,
                    &mut state.failed_downloads,
                )
                .await
                else {
                    continue;
                };

                process_node_responses(
                    &mut nodes_from_peer,
                    &mut state.download_queue,
                    state.store.clone(),
                    &mut state.membatch,
                    &mut state.leafs_healed,
                    global_leafs_healed,
                    &mut state.roots_healed,
                    &mut state.maximum_length_seen,
                    &mut nodes_to_write,
                )
                .expect("We shouldn't be getting store errors"); // TODO: if we have a stor error we should stop
            }
            Err(RequestStorageTrieNodes::SendMessageError(id, _err)) => {
                let inflight_request = state.requests.remove(&id).expect("request disappeared");
                state.failed_downloads += 1;
                state
                    .download_queue
                    .extend(inflight_request.requests.clone());
                peers
                    .peer_scores
                    .lock()
                    .await
                    .record_failure(inflight_request.peer_id);
                peers
                    .peer_scores
                    .lock()
                    .await
                    .free_peer(inflight_request.peer_id);
            }
        }
    }
}

/// it grabs N peers to ask for data
async fn ask_peers_for_nodes(
    download_queue: &mut VecDeque<NodeRequest>,
    requests: &mut HashMap<u64, InflightRequest>,
    requests_task_joinset: &mut JoinSet<
        Result<u64, TrySendError<Result<TrieNodes, RequestStorageTrieNodes>>>,
    >,
    peers: &PeerHandler,
    state_root: H256,
    task_sender: &Sender<Result<TrieNodes, RequestStorageTrieNodes>>,
) {
    if (requests.len() as u32) < MAX_IN_FLIGHT_REQUESTS && !download_queue.is_empty() {
        let Some((peer_id, mut peer_channel)) = peers
            .peer_scores
            .lock()
            .await
            .get_peer_channel_with_highest_score(&peers.peer_table, &SUPPORTED_SNAP_CAPABILITIES)
            .await
        else {
            // warn!("We have no free peers for storage healing!"); way too spammy, moving to trace
            // If we have no peers we shrug our shoulders and wait until next free peer
            trace!("We have no free peers for storage healing!");
            return;
        };
        peers.peer_scores.lock().await.mark_in_use(peer_id);
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

        requests_task_joinset.spawn(async move {
            let req_id = gtn.id;
            // TODO: check errors to determine whether the current block is stale
            let response = PeerHandler::request_storage_trienodes(&mut peer_channel, gtn).await;
            // TODO: add error handling
            tx.try_send(response).inspect_err(|err| {
                error!("Failed to send state trie nodes response. Error: {err}")
            })?;
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
    trie_nodes: TrieNodes,
    succesful_downloads: &mut usize,
    failed_downloads: &mut usize,
) -> Option<Vec<NodeResponse>> {
    trace!(
        "We are processing the nodes, we received {} nodes from our peer",
        trie_nodes.nodes.len()
    );
    let Some(request) = requests.remove(&trie_nodes.id) else {
        info!("We received a response where we had a missing requests {trie_nodes:?}");
        return None;
    };
    peer_handler
        .peer_scores
        .lock()
        .await
        .free_peer(request.peer_id);

    let nodes_size = trie_nodes.nodes.len();
    if nodes_size == 0 {
        *failed_downloads += 1;
        peer_handler
            .peer_scores
            .lock()
            .await
            .record_failure(request.peer_id);
        download_queue.extend(request.requests);
        return None;
    }

    if request.requests.len() < nodes_size {
        panic!("The node responded with more data than us!");
    }

    if let Ok(nodes) = request
        .requests
        .iter()
        .zip(trie_nodes.nodes.clone())
        .map(|(node_request, node_bytes)| {
            let node = Node::decode_raw(&node_bytes).inspect_err(|err|{
                    info!("this peer {} request {node_request:?}, had this error {err:?}, and the raw node was {node_bytes:?}", request.peer_id)
                })?;

            if node.compute_hash().finalize() != node_request.hash {
                error!("this peer {} request {node_request:?}, sent us a valid node with the wrong hash, and the raw node was {node_bytes:?}", request.peer_id);
                Err(RLPDecodeError::MalformedData)
            } else {
                Ok(NodeResponse {
                    node_request: node_request.clone(),
                    node
                })
            }
        })
        .collect::<Result<Vec<NodeResponse>, RLPDecodeError>>()
    {
        if request.requests.len() > nodes_size {
            download_queue.extend(request.requests.into_iter().skip(nodes_size));
        }
        *succesful_downloads += 1;
        peer_handler.peer_scores.lock().await.record_success(request.peer_id);
        Some(nodes)
    } else {
        *failed_downloads += 1;
        peer_handler.peer_scores.lock().await.record_failure(request.peer_id);
        download_queue.extend(request.requests);
        None
    }
}

#[allow(clippy::too_many_arguments)]
fn process_node_responses(
    node_processing_queue: &mut Vec<NodeResponse>,
    download_queue: &mut VecDeque<NodeRequest>,
    store: Store,
    membatch: &mut Membatch,
    leafs_healed: &mut usize,
    global_leafs_healed: &mut u64,
    roots_healed: &mut usize,
    maximum_length_seen: &mut usize,
    to_write: &mut HashMap<H256, Vec<(NodeHash, Vec<u8>)>>,
) -> Result<(), StoreError> {
    while let Some(node_response) = node_processing_queue.pop() {
        trace!("We are processing node response {:?}", node_response);
        if let Node::Leaf(_) = &node_response.node {
            *leafs_healed += 1;
            *global_leafs_healed += 1;
        };

        *maximum_length_seen = usize::max(
            *maximum_length_seen,
            node_response.node_request.storage_path.len(),
        );

        let (missing_children_nibbles, missing_children_count) =
            determine_missing_children(&node_response, store.clone()).inspect_err(|err| {
                error!("{err} in determine missing children while searching {node_response:?}")
            })?;

        if missing_children_count == 0 {
            // We flush to the database this node
            commit_node(&node_response, membatch, roots_healed, to_write).inspect_err(|err| {
                error!("{err} in commit node while committing {node_response:?}")
            })?;
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
        .expect("We should be able to open the store");
    let mut initial_requests: VecDeque<NodeRequest> = VecDeque::new();
    initial_requests.extend(
        account_paths
            .healed_accounts
            .par_iter()
            .filter_map(|acc_path| {
                let rlp = trie
                    .get(&acc_path.to_fixed_bytes().to_vec())
                    .expect("We should be able to open the store")
                    .expect("This account should exist in the trie");
                let account = AccountState::decode(&rlp).expect("We should have a valid account");
                if account.storage_root == *EMPTY_TRIE_HASH {
                    return None;
                }
                if store
                    .contains_storage_node(*acc_path, account.storage_root)
                    .expect("We should be able to open the store")
                {
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
    initial_requests.extend(
        account_paths
            .accounts_with_storage_root
            .par_iter()
            .filter_map(|(acc_path, storage_root)| {
                if store
                    .contains_storage_node(*acc_path, *storage_root)
                    .expect("We should be able to open the store")
                {
                    return None;
                }
                Some(NodeRequest {
                    acc_path: Nibbles::from_bytes(&acc_path.0),
                    storage_path: Nibbles::default(), // We need to be careful, the root parent is a special case
                    parent: Nibbles::default(),
                    hash: *storage_root,
                })
            })
            .collect::<VecDeque<_>>(),
    );
    initial_requests
}

/// Returns the full paths to the node's missing children and grandchildren
/// and the number of direct missing children
pub fn determine_missing_children(
    node_response: &NodeResponse,
    store: Store,
) -> Result<(Vec<NodeRequest>, usize), StoreError> {
    let mut paths = Vec::new();
    let mut count = 0;
    let node = node_response.node.clone();
    let trie = store
        .open_storage_trie(
            H256::from_slice(&node_response.node_request.acc_path.to_bytes()),
            *EMPTY_TRIE_HASH,
        )
        .inspect_err(|_| {
            error!("Malformed data when opening the storage trie in determine missing children")
        })?;
    let trie_state = trie.db();
    match &node {
        Node::Branch(node) => {
            for (index, child) in node.choices.iter().enumerate() {
                if child.is_valid()
                    && child
                        .get_node(trie_state)
                        .inspect_err(|_| {
                            error!("Malformed data when doing get child of a branch node")
                        })?
                        .is_none()
                {
                    count += 1;

                    paths.extend(vec![NodeRequest {
                        acc_path: node_response.node_request.acc_path.clone(),
                        storage_path: node_response
                            .node_request
                            .storage_path
                            .append_new(index as u8),
                        parent: node_response.node_request.storage_path.clone(),
                        hash: child.compute_hash().finalize(),
                    }]);
                }
            }
        }
        Node::Extension(node) => {
            if node.child.is_valid()
                && node
                    .child
                    .get_node(trie_state)
                    .inspect_err(|_| {
                        error!("Malformed data when doing get child of an extension node")
                    })?
                    .is_none()
            {
                count += 1;

                paths.extend(vec![NodeRequest {
                    acc_path: node_response.node_request.acc_path.clone(),
                    storage_path: node_response
                        .node_request
                        .storage_path
                        .concat(node.prefix.clone()),
                    parent: node_response.node_request.storage_path.clone(),
                    hash: node.child.compute_hash().finalize(),
                }]);
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
    to_write: &mut HashMap<H256, Vec<(NodeHash, Vec<u8>)>>,
) -> Result<(), StoreError> {
    let hashed_account = H256::from_slice(&node.node_request.acc_path.to_bytes());
    to_write
        .entry(hashed_account)
        .or_default()
        .push((node.node.compute_hash(), node.node.encode_to_vec()));

    // Special case, we have just commited the root, we stop
    if node.node_request.storage_path == node.node_request.parent {
        trace!(
            "We have the parent of an account, this means we are the root. Storage healing should end."
        );
        *roots_healed += 1;
        return Ok(());
    }

    let parent_key: (Nibbles, Nibbles) = (
        node.node_request.acc_path.clone(),
        node.node_request.parent.clone(),
    );

    let mut parent_entry = membatch
        .remove(&parent_key)
        .expect("We are missing the parent from the membatch!");

    parent_entry.missing_children_count -= 1;

    if parent_entry.missing_children_count == 0 {
        commit_node(
            &parent_entry.node_response,
            membatch,
            roots_healed,
            to_write,
        )?;
    } else {
        membatch.insert(parent_key, parent_entry);
    }
    Ok(())
}
