use crate::{
    kademlia::PeerChannels,
    peer_handler::{MAX_RESPONSE_BYTES, PeerHandler, RequestStorageTrieNodes},
    rlpx::{
        p2p::SUPPORTED_SNAP_CAPABILITIES,
        snap::{GetTrieNodes, TrieNodes},
    },
    sync::state_healing::{SHOW_PROGRESS_INTERVAL_DURATION, STORAGE_BATCH_SIZE},
    utils::current_unix_time,
};

use bytes::Bytes;
use ethrex_common::{H256, types::AccountState};
use ethrex_rlp::{encode::RLPEncode, error::RLPDecodeError};
use ethrex_storage::{Store, error::StoreError};
use ethrex_trie::{EMPTY_TRIE_HASH, Nibbles, Node, NodeHash, NodeRef};
use rand::random;
use std::{
    collections::{HashMap, VecDeque},
    time::{Duration, Instant},
};
use tokio::{sync::mpsc::error::TryRecvError, task::JoinSet};
use tokio::{
    sync::mpsc::{Sender, error::TrySendError},
    task::yield_now,
};
use tracing::{debug, error, info, trace};

pub const LOGGING_INTERVAL: Duration = Duration::from_secs(2);
const MAX_IN_FLIGHT_REQUESTS: u32 = 77;
const INFLIGHT_TIMEOUT: Duration = Duration::from_secs(7);

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
pub enum StorageHealerCallMsg {
    IsFinished,
}
#[derive(Debug, Clone)]
pub enum StorageHealerOutMsg {
    FinishedStale { is_finished: bool, is_stale: bool },
}

#[derive(Debug, Clone)]
pub enum StorageHealerMsg {
    /// Overloaded msg, checkup does two things
    /// It prints the status of the connection
    /// And if a request is timed out, we also clean it up
    CheckUp,
    /// This message is sent by a peer indicating what is needed to download
    /// We process the request
    TrieNodes(TrieNodes),
}

#[derive(Debug, Clone)]
pub struct InflightRequest {
    requests: Vec<NodeRequest>,
    peer_id: H256,
    sent_time: Instant,
}

#[derive(Debug, Clone)]
pub struct PeerScore {
    /// This tracks if a peer has a task in flight
    /// So we can't use it yet
    in_flight: bool,
    /// This tracks the score of a peer
    score: i64,
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
    /// We use this to track which peers we can send stuff to
    peer_handler: PeerHandler,
    /// We use this to track which peers are occupied, and we can't send stuff to
    /// Alongside their score for this situation
    scored_peers: HashMap<H256, PeerScore>,
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
    account_paths: &HashMap<H256, H256>,
    peers: PeerHandler,
    store: Store,
    membatch: Membatch,
    staleness_timestamp: u64,
    global_leafs_healed: &mut u64,
) -> bool {
    info!(
        "Started Storage Healing with {} accounts",
        account_paths.len()
    );
    let mut state = StorageHealer {
        last_update: Instant::now(),
        download_queue: get_initial_downloads(&account_paths),
        store,
        membatch,
        peer_handler: peers,
        scored_peers: HashMap::new(),
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
            state.last_update = Instant::now();
            info!(
                "We are storage healing. Snap Peers {}. Inflight tasks {}. Download Queue {}. Maximum length {}. Leafs Healed {}. Global Leafs Healed {global_leafs_healed}. Roots Healed {}. Good Download Percentage {}. Empty count {}. Disconnected Count {}. dhat stats {:?}",
                state
                    .peer_handler
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
                dhat::HeapStats::get()
            );
            state.succesful_downloads = 0;
            state.failed_downloads = 0;
            state.empty_count = 0;
            state.disconnected_count = 0;

            if state.download_queue.len() < 350 {
                info!(
                    "Logging the inflight requests at the end {:?}, taskset len {}",
                    state
                        .requests
                        .iter()
                        .map(|(key, value)| { (key, value.peer_id, value.sent_time) })
                        .collect::<Vec<_>>(),
                    requests_task_joinset.len()
                );
            }
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
            &state.peer_handler,
            state.state_root,
            &mut state.scored_peers,
            &task_sender,
        )
        .await;

        let result = requests_task_joinset.try_join_next();
        if let Some(res) = result {
            if state.download_queue.len() < 350 {
                info!("Tracking what happened to the tasks at the end {res:?}")
            }
        }

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
                    &mut state.scored_peers,
                    &mut state.download_queue,
                    trie_nodes.clone(), // TODO: remove unnecesary clone, needed now for log 🏗️🏗️
                    &mut state.succesful_downloads,
                    &mut state.failed_downloads,
                ) else {
                    if state.download_queue.len() < 350 {
                        info!(
                            "We received none from zip_requeue_node_responses_score_peer while low download value {trie_nodes:?}"
                        )
                    }
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
            Err(RequestStorageTrieNodes::SendMessageError(id, err)) => {
                let inflight_request = state.requests.remove(&id).expect("request disappeared");
                state.failed_downloads += 1;
                if state.download_queue.len() < 350 {
                    info!("In request {id} found error {err:?}");
                }
                state
                    .download_queue
                    .extend(inflight_request.requests.clone());
                state
                    .scored_peers
                    .entry(inflight_request.peer_id)
                    .and_modify(|entry| {
                        entry.in_flight = false;
                        entry.score -= 1;
                    });
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
    scored_peers: &mut HashMap<H256, PeerScore>,
    task_sender: &Sender<Result<TrieNodes, RequestStorageTrieNodes>>,
) {
    if (requests.len() as u32) < MAX_IN_FLIGHT_REQUESTS && !download_queue.is_empty() {
        let Some(mut peer) =
            get_peer_with_highest_score_and_mark_it_as_occupied(peers, scored_peers).await
        else {
            // warn!("We have no free peers for storage healing!"); way too spammy, moving to trace
            // If we have no peers we shrug our shoulders and wait until next free peer
            trace!("We have no free peers for storage healing!");
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
                peer_id: peer.0,
                sent_time: Instant::now(),
            },
        );
        let gtn = GetTrieNodes {
            id: req_id,
            root_hash: state_root,
            paths,
            bytes: MAX_RESPONSE_BYTES,
        };

        let tx = task_sender.clone();

        let logging_flag = download_queue.len() < 350;

        requests_task_joinset.spawn(async move {
            let req_id = gtn.id;
            // TODO: check errors to determine whether the current block is stale
            if logging_flag {
                info!("the task {req_id} has been started");
            }
            let response = PeerHandler::request_storage_trienodes(&mut peer.1, gtn, logging_flag).await;
            if logging_flag {
                info!(
                    "the task {req_id} has finished getting the request_storage_trienodes {response:?}"
                );
            }
            // TODO: add error handling
            tx.try_send(response).inspect_err(|err| {
                error!("Failed to send state trie nodes response. Error: {err}")
            })?;
            if logging_flag {
                info!("the task {req_id} has finished sending the thing");
            }
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

fn zip_requeue_node_responses_score_peer(
    requests: &mut HashMap<u64, InflightRequest>,
    scored_peers: &mut HashMap<H256, PeerScore>,
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
    let peer = scored_peers
        .get_mut(&request.peer_id)
        .expect("Each time we request we should add to scored_peeers");
    peer.in_flight = false;

    let nodes_size = trie_nodes.nodes.len();
    if nodes_size == 0 {
        if download_queue.len() < 350 {
            info!("We are receiving empty responses at the end of the download queue {request:?}");
        }
        *failed_downloads += 1;
        peer.score -= 1;
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
        if peer.score < 10 {
            peer.score += 1;
        }
        Some(nodes)
    } else {
        if download_queue.len() < 350 {
            info!("This should get logged already {request:?}");
        }
        *failed_downloads += 1;
        peer.score -= 1;
        download_queue.extend(request.requests);
        None
    }
}

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
            determine_missing_children(&node_response, store.clone(), membatch).inspect_err(
                |err| {
                    error!("{err} in determine missing children while searching {node_response:?}")
                },
            )?;

        if missing_children_count == 0 {
            // We flush to the database this node
            commit_node(
                &node_response,
                store.clone(),
                membatch,
                roots_healed,
                to_write,
            )
            .inspect_err(|err| error!("{err} in commit node while committing {node_response:?}"))?;
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

fn get_initial_downloads(account_paths: &HashMap<H256, H256>) -> VecDeque<NodeRequest> {
    account_paths
        .iter()
        .map(|(acc_path, storage_root)| {
            NodeRequest {
                acc_path: Nibbles::from_bytes(&acc_path.0),
                storage_path: Nibbles::default(), // We need to be careful, the root parent is a special case
                parent: Nibbles::default(),
                hash: *storage_root,
            }
        })
        .collect()
}

/// Returns the full paths to the node's missing children and grandchildren
/// and the number of direct missing children
pub fn determine_missing_children(
    node_response: &NodeResponse,
    store: Store,
    membatch: &Membatch,
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

                    // paths.extend(determine_membatch_missing_children(
                    //     NodeRequest {
                    //         acc_path: node_response.node_request.acc_path.clone(),
                    //         storage_path: node_response
                    //             .node_request
                    //             .storage_path
                    //             .append_new(index as u8),
                    //         parent: node_response.node_request.storage_path.clone(),
                    //     },
                    //     &child.compute_hash(),
                    //     membatch,
                    //     store.clone(),
                    //     nodes_to_write
                    // )?);

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
                // paths.extend(determine_membatch_missing_children(
                //     NodeRequest {
                //         acc_path: node_response.node_request.acc_path.clone(),
                //         storage_path: node_response
                //             .node_request
                //             .storage_path
                //             .concat(node.prefix.clone()),
                //         parent: node_response.node_request.storage_path.clone(),
                //     },
                //     &node.child.compute_hash(),
                //     membatch,
                //     store.clone(),
                //     nodes_to_write
                // )?);

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

// This function searches for the nodes we have to download that are childs from the membatch
fn determine_membatch_missing_children(
    node_request: NodeRequest,
    hash: &NodeHash,
    membatch: &Membatch,
    store: Store,
    nodes_to_write: &mut HashMap<H256, Vec<(NodeHash, Vec<u8>)>>,
) -> Result<Vec<NodeRequest>, StoreError> {
    if let Some(membatch_entry) = membatch.get(&(
        node_request.acc_path.clone(),
        node_request.storage_path.clone(),
    )) {
        if membatch_entry.node_response.node.compute_hash() == *hash {
            determine_missing_children(&membatch_entry.node_response, store, membatch)
                .map(|(paths, count)| paths)
        } else {
            Ok(vec![node_request])
        }
    } else {
        Ok(vec![node_request])
    }
}

fn commit_node(
    node: &NodeResponse,
    store: Store,
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
            store,
            membatch,
            roots_healed,
            to_write,
        )?;
    } else {
        membatch.insert(parent_key, parent_entry);
    }
    Ok(())
}

async fn get_peer_with_highest_score_and_mark_it_as_occupied(
    peers: &PeerHandler,
    scored_peers: &mut HashMap<H256, PeerScore>,
) -> Option<(H256, PeerChannels)> {
    let mut chosen_peer: Option<(H256, PeerChannels)> = None;
    let mut max_score = i64::MIN;

    for (peer_id, peer_channel) in peers
        .peer_table
        .get_peer_channels(&SUPPORTED_SNAP_CAPABILITIES)
        .await
    {
        if let Some(known_peer_score) = scored_peers.get_mut(&peer_id) {
            if known_peer_score.in_flight {
                continue;
            }
            if known_peer_score.score > max_score {
                chosen_peer = Some((peer_id, peer_channel));
                max_score = known_peer_score.score;
            }
        } else if chosen_peer.is_none() {
            chosen_peer = Some((peer_id, peer_channel));
            max_score = 0;
        }
    }

    if let Some((peer_id, _)) = chosen_peer {
        scored_peers
            .entry(peer_id)
            .and_modify(|peer_score| peer_score.in_flight = true)
            .or_insert(PeerScore {
                in_flight: true,
                score: 0,
            });
    }

    chosen_peer
}
