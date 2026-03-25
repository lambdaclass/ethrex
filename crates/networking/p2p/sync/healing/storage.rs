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

use crate::snap::mpt_stubs::{Nibbles, Node};
use bytes::Bytes;
use ethrex_common::{H256, types::AccountState};
use ethrex_crypto::NativeCrypto;
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode, error::RLPDecodeError};
use ethrex_storage::{Store, error::StoreError};
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
    _state_root: H256,
    _storage_accounts: &AccountStorageRoots,
    _peers: &mut PeerHandler,
    _store: Store,
    _healing_queue: StorageHealingQueue,
    _staleness_timestamp: u64,
    _global_leafs_healed: &mut u64,
) -> Result<bool, SyncError> {
    // MPT storage trie healing not supported on binary trie branch
    Ok(true)
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
    if (requests.len() as u32) < MAX_IN_FLIGHT_REQUESTS && !download_queue.is_empty() {
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

            if node.compute_hash(&NativeCrypto).finalize(&NativeCrypto) != node_request.hash {
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
    _store: &Store,
    _state_root: H256,
    _account_paths: &AccountStorageRoots,
) -> VecDeque<NodeRequest> {
    // MPT storage trie healing not supported on binary trie branch
    VecDeque::new()
}

/// Returns the full paths to the node's missing children and grandchildren
/// and the number of direct missing children
pub fn determine_pending_children(
    _node_response: &NodeResponse,
    _store: &Store,
) -> Result<(Vec<NodeRequest>, usize), StoreError> {
    // MPT storage trie healing not supported on binary trie branch
    Ok((vec![], 0))
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
