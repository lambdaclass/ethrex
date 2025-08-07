use crate::{
    kademlia::PeerChannels,
    peer_handler::{MAX_RESPONSE_BYTES, PEER_REPLY_TIMEOUT, PeerHandler},
    rlpx::{
        connection::server::CastMessage,
        message::Message,
        p2p::SUPPORTED_SNAP_CAPABILITIES,
        snap::{GetTrieNodes, TrieNodes},
    },
    sync::state_healing::{NODE_BATCH_SIZE, SHOW_PROGRESS_INTERVAL_DURATION},
    utils::current_unix_time,
};
use std::cell::OnceCell;

use bytes::Bytes;
use ethrex_common::{H256, types::AccountState};
use ethrex_rlp::error::RLPDecodeError;
use ethrex_storage::{Store, error::StoreError};
use ethrex_trie::{EMPTY_TRIE_HASH, Nibbles, Node, NodeHash, TrieError};
use rand::random;
use spawned_concurrency::{
    messages::Unused,
    tasks::{CallResponse, CastResponse, GenServer, GenServerHandle},
};
use std::{
    collections::{HashMap, VecDeque},
    time::{Duration, Instant},
};
use tokio::sync::mpsc::{Receiver, Sender};
use tracing::{info, trace, warn};

pub const LOGGING_INTERVAL: Duration = Duration::from_secs(2);
const MAX_IN_FLIGHT_REQUESTS: usize = 777;

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
    membatch: OnceCell<Membatch>,
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
}

impl GenServer for StorageHealer {
    type CallMsg = StorageHealerCallMsg;
    type CastMsg = StorageHealerMsg;
    type OutMsg = StorageHealerOutMsg;
    type Error = ();

    async fn handle_call(
        self,
        _message: StorageHealerCallMsg,
        _handle: &GenServerHandle<Self>,
    ) -> CallResponse<Self> {
        info!("Receiving a call");
        // We only ask for IsFinished in the message, so we don't match it
        let is_finished = self.requests.is_empty() && self.download_queue.is_empty();
        let is_stale = current_unix_time() > self.staleness_timestamp;
        info!("Are we finished? {is_finished}. Are we stale? {is_stale}");
        // Finished means that we have succesfully healed according to our algorithm
        // That means that we have commited the root_node of the tree
        if is_finished || is_stale {
            CallResponse::Stop(StorageHealerOutMsg::FinishedStale {
                is_finished,
                is_stale,
            })
        } else {
            CallResponse::Reply(
                self,
                StorageHealerOutMsg::FinishedStale {
                    is_finished,
                    is_stale,
                },
            )
        }
    }

    async fn handle_cast(
        mut self,
        message: Self::CastMsg,
        handle: &GenServerHandle<Self>,
    ) -> CastResponse<Self> {
        match message {
            StorageHealerMsg::CheckUp => {
                info!(
                    "We are storage healing. Inflight tasks {}. Download Queue {}. Maximum length {}. Leafs Healed {}. Roots Healed {}. Good Download Percentage {}",
                    self.requests.len(),
                    self.download_queue.len(),
                    self.maximum_length_seen,
                    self.leafs_healed,
                    self.roots_healed,
                    self.succesful_downloads as f64 / self.failed_downloads as f64
                );
                self.succesful_downloads = 0;
                self.failed_downloads = 0;
                clear_inflight_requests(
                    &mut self.requests,
                    &mut self.scored_peers,
                    &mut self.download_queue,
                    &mut self.failed_downloads,
                )
                .await;
                info!("We have cleared the inflight requests");
                ask_peers_for_nodes(
                    &mut self.download_queue,
                    &mut self.requests,
                    &self.peer_handler,
                    self.state_root,
                    &mut self.scored_peers,
                    handle.clone(),
                )
                .await;
                info!("We have asked the peers for nodes");
            }
            StorageHealerMsg::TrieNodes(trie_nodes) => {
                if let Some(mut nodes_from_peer) = zip_requeue_node_responses_score_peer(
                    &mut self.requests,
                    &mut self.scored_peers,
                    &mut self.download_queue,
                    trie_nodes,
                    &mut self.succesful_downloads,
                    &mut self.failed_downloads,
                ) {
                    process_node_responses(
                        &mut nodes_from_peer,
                        &mut self.download_queue,
                        self.store.clone(),
                        &mut self
                            .membatch
                            .get_mut()
                            .expect("We launched the storage healer without a membatch"),
                        &mut self.leafs_healed,
                        &mut self.roots_healed,
                    );
                };
                clear_inflight_requests(
                    &mut self.requests,
                    &mut self.scored_peers,
                    &mut self.download_queue,
                    &mut self.failed_downloads,
                )
                .await;
                ask_peers_for_nodes(
                    &mut self.download_queue,
                    &mut self.requests,
                    &self.peer_handler,
                    self.state_root,
                    &mut self.scored_peers,
                    handle.clone(),
                )
                .await;
            }
        }
        CastResponse::NoReply(self)
    }
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
    // /// What hash was requested. We can use this for validation
    // hash: H256 // this is a potential optimization, we ignore for now
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
pub async fn heal_storage_trie_wrap(
    state_root: H256,
    peers: PeerHandler,
    store: Store,
    membatch: OnceCell<Membatch>,
    staleness_timestamp: u64,
) -> bool {
    info!("Started Storage Healing");
    let accounts: Vec<(H256, AccountState)> = store
        .iter_accounts(state_root)
        .expect("We should be able to open the accoun")
        .collect();

    info!("Total accounts: {}", accounts.len());
    let filtered_accounts: Vec<(H256, AccountState)> = accounts
        .into_iter()
        .filter(|(_, state)| state.storage_root != *EMPTY_TRIE_HASH)
        .collect();
    info!("Total filtered accounts: {}", filtered_accounts.len());
    let mut account_path_nibbles: Vec<Nibbles> = filtered_accounts
        .into_iter()
        .map(|(hashed_key, _)| Nibbles::from_bytes(hashed_key.as_bytes()))
        .collect();
    heal_storage_trie(
        state_root,
        account_path_nibbles,
        peers.clone(),
        store.clone(),
        membatch.clone(),
        staleness_timestamp,
    )
    .await
}

pub async fn heal_storage_trie(
    state_root: H256,
    account_paths: Vec<Nibbles>,
    peers: PeerHandler,
    store: Store,
    membatch: OnceCell<Membatch>,
    staleness_timestamp: u64,
) -> bool {
    info!(
        "Started Storage Healing with {} accounts",
        account_paths.len()
    );
    let mut handle = StorageHealer::start(StorageHealer {
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
    });

    let mut is_finished = false;
    let mut is_stale = false;

    while !is_finished && !is_stale {
        handle.cast(StorageHealerMsg::CheckUp).await;
        let outmsg = handle
            .call(StorageHealerCallMsg::IsFinished)
            .await
            .expect("The genserver died prematurely");
        match outmsg {
            StorageHealerOutMsg::FinishedStale {
                is_finished: heal_finished,
                is_stale: heal_stale,
            } => {
                is_finished = heal_finished;
                is_stale = heal_stale;
            }
        }
        tokio::time::sleep(SHOW_PROGRESS_INTERVAL_DURATION);
    }
    if is_finished {
        info!("Storage healing finished succesfully.");
    } else {
        info!("Storage healing finished prematurely due to stalenss.");
    }
    is_finished
}

async fn clear_inflight_requests(
    requests: &mut HashMap<u64, InflightRequest>,
    scored_peers: &mut HashMap<H256, PeerScore>,
    download_queue: &mut VecDeque<NodeRequest>,
    failed_downloads: &mut usize,
) {
    // Inneficiant use extract_if when available for people (rust 1.88)
    requests.retain(|req_id, inflight_request| {
        if inflight_request.sent_time.elapsed() > PEER_REPLY_TIMEOUT {
            *failed_downloads += 1;
            download_queue.extend(inflight_request.requests.clone());
            scored_peers
                .entry(inflight_request.peer_id)
                .and_modify(|entry| {
                    entry.in_flight = false;
                    entry.score -= 1;
                });
            false
        } else {
            true
        }
    });
}

/// it grabs N peers to ask for data
async fn ask_peers_for_nodes(
    download_queue: &mut VecDeque<NodeRequest>,
    requests: &mut HashMap<u64, InflightRequest>,
    peers: &PeerHandler,
    state_root: H256,
    scored_peers: &mut HashMap<H256, PeerScore>,
    self_handler: GenServerHandle<StorageHealer>,
) {
    while requests.len() < MAX_IN_FLIGHT_REQUESTS && !download_queue.is_empty() {
        let Some(mut peer) =
            get_peer_with_highest_score_and_mark_it_as_occupied(peers, scored_peers).await
        else {
            warn!("We have no free peers for storage healing!");
            // If we have no peers we shrug our shoulders and wait until next free peer
            return;
        };
        let at = download_queue.len().saturating_sub(NODE_BATCH_SIZE);
        let download_chunk = download_queue.split_off(at);
        let req_id: u64 = random();
        requests.insert(
            req_id,
            InflightRequest {
                requests: download_chunk.clone().into(),
                peer_id: peer.0,
                sent_time: Instant::now(),
            },
        );
        peer.1
            .connection
            .cast(CastMessage::BackendRequest(
                Message::GetTrieNodes(GetTrieNodes {
                    id: req_id,
                    root_hash: state_root,
                    paths: download_chunk
                        .into_iter()
                        .map(|request| {
                            vec![
                                Bytes::from(request.acc_path.to_bytes()),
                                Bytes::from(request.storage_path.to_bytes()),
                            ]
                        })
                        .collect(),
                    bytes: MAX_RESPONSE_BYTES,
                }),
                self_handler.clone(),
            ))
            .await
            .expect("We should be able to send mesages to our peers");
    }
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
    let request = requests.remove(&trie_nodes.id)?;
    let peer = scored_peers
        .get_mut(&request.peer_id)
        .expect("Each time we request we should add to scored_peeers");
    peer.in_flight = false;

    let nodes_size = trie_nodes.nodes.len();

    if request.requests.len() < nodes_size {
        panic!("The node responded with more data than us!");
    }

    if let Ok(nodes) = request
        .requests
        .iter()
        .zip(trie_nodes.nodes)
        .map(|(node_request, node_bytes)| {
            Ok(NodeResponse {
                node_request: node_request.clone(),
                node: Node::decode_raw(&node_bytes)?,
            })
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
        *failed_downloads += 1;
        peer.score -= 1;
        download_queue.extend(request.requests.into_iter());
        None
    }
}

fn process_node_responses(
    node_processing_queue: &mut Vec<NodeResponse>,
    download_queue: &mut VecDeque<NodeRequest>,
    store: Store,
    membatch: &mut Membatch,
    leafs_healed: &mut usize,
    roots_healed: &mut usize,
) -> Result<(), StoreError> {
    while let Some(node_response) = node_processing_queue.pop() {
        trace!("We are processing node response {:?}", node_response);
        match &node_response.node {
            Node::Leaf(_) => *leafs_healed += 1,
            _ => {}
        };

        let (missing_children_nibbles, missing_children_count) =
            determine_missing_children(&node_response, store.clone(), membatch)?;

        if missing_children_count == 0 {
            // We flush to the database this node
            commit_node(&node_response, store.clone(), membatch, roots_healed);
        } else {
            let key = (
                node_response.node_request.acc_path.clone(),
                node_response.node_request.storage_path.clone(),
            );
            membatch.insert(
                key,
                MembatchEntry {
                    node_response: node_response.clone(),
                    missing_children_count: missing_children_count,
                },
            );
            download_queue.extend(missing_children_nibbles.iter().map(|children_key| {
                NodeRequest {
                    acc_path: children_key.0.clone(),
                    storage_path: children_key.1.clone(),
                    parent: node_response.node_request.storage_path.clone(),
                }
            }));
        }
    }

    Ok(())
}

fn log_storage_heal(last_update: &mut Instant) {
    if last_update.elapsed() > LOGGING_INTERVAL {
        info!("Storage Healing");
        *last_update = Instant::now();
    }
}

fn get_initial_downloads(account_paths: &[Nibbles]) -> VecDeque<NodeRequest> {
    account_paths
        .into_iter()
        .map(|acc_path| {
            NodeRequest {
                acc_path: acc_path.clone(),
                storage_path: Nibbles::default(), // We need to be careful, the root parent is a special case
                parent: Nibbles::default(),
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
) -> Result<(Vec<MembatchKey>, usize), StoreError> {
    let mut paths = Vec::new();
    let mut count = 0;
    let node = node_response.node.clone();
    let trie = store.open_state_trie(*EMPTY_TRIE_HASH)?;
    let trie_state = trie.db();
    match &node {
        Node::Branch(node) => {
            for (index, child) in node.choices.iter().enumerate() {
                if child.is_valid() && child.get_node(trie_state)?.is_none() {
                    count += 1;
                    paths.extend(determine_membatch_missing_children(
                        &node_response.node_request.acc_path,
                        &node_response
                            .node_request
                            .storage_path
                            .append_new(index as u8),
                        &child.compute_hash(),
                        membatch,
                        store.clone(),
                    )?);
                }
            }
        }
        Node::Extension(node) => {
            let hash = node.child.compute_hash();
            if node.child.is_valid() && node.child.get_node(trie_state)?.is_none() {
                count += 1;
                paths.extend(determine_membatch_missing_children(
                    &node_response.node_request.acc_path,
                    &node_response
                        .node_request
                        .parent
                        .concat(node.prefix.clone()),
                    &node.child.compute_hash(),
                    membatch,
                    store.clone(),
                )?);
            }
        }
        _ => {}
    }
    Ok((paths, count))
}

// This function searches for the nodes we have to download that are childs from the membatch
fn determine_membatch_missing_children(
    acc_path: &Nibbles,
    nibbles: &Nibbles,
    hash: &NodeHash,
    membatch: &Membatch,
    store: Store,
) -> Result<Vec<MembatchKey>, StoreError> {
    if let Some(membatch_entry) = membatch.get(&(acc_path.clone(), nibbles.clone())) {
        if membatch_entry.node_response.node.compute_hash() == *hash {
            determine_missing_children(&membatch_entry.node_response, store, membatch)
                .map(|(paths, count)| paths)
        } else {
            Ok(vec![(acc_path.clone(), nibbles.clone())])
        }
    } else {
        Ok(vec![(acc_path.clone(), nibbles.clone())])
    }
}

fn commit_node(
    node: &NodeResponse,
    store: Store,
    membatch: &mut Membatch,
    roots_healed: &mut usize,
) -> Result<(), StoreError> {
    let trie = store.clone().open_state_trie(*EMPTY_TRIE_HASH)?;
    let trie_db = trie.db();
    trie_db
        .put(node.node.compute_hash(), node.node.encode_raw())
        .map_err(StoreError::Trie)?; // we can have an error if 2 trees have the same nodes

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

    if !membatch.contains_key(&parent_key) {
        return Ok(());
    }

    let mut parent_entry = membatch
        .remove(&parent_key)
        .expect("We are missing the parent from the membatch!");

    parent_entry.missing_children_count -= 1;

    if parent_entry.missing_children_count == 0 {
        commit_node(&parent_entry.node_response, store, membatch, roots_healed)?;
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
