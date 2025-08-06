use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use crate::{peer_handler::PeerHandler, rlpx::snap::TrieNodes, utils::current_unix_time};
use ethrex_common::H256;
use ethrex_storage::{Store, error::StoreError};
use ethrex_trie::{EMPTY_TRIE_HASH, Nibbles, Node, NodeHash, TrieError};
use spawned_concurrency::{
    messages::Unused,
    tasks::{CallResponse, CastResponse, GenServer, GenServerHandle},
};
use tokio::sync::mpsc::{Receiver, Sender};
use tracing::{info, trace};

pub const LOGGING_INTERVAL: Duration = Duration::from_secs(2);

/// This struct stores the metadata we need when we request a node
#[derive(Debug, Clone)]
pub struct NodeResponse {
    /// Who is this node
    node: Node,
    /// What did we ask for
    node_request: NodeRequest,
}

/// This struct stores the metadata we need when we store a node in the memory bank before storing
#[derive(Debug)]
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
pub struct StorageHealerState {
    /// We use this to track which peers we have sent stuff to
    peer_handler: PeerHandler,
    /// With this we track how many requests are inflight to our peer
    /// This allows us to know if one is wildly out of time
    requests: HashMap<u64, ()>,
    /// This bool gets set up at the end of the processing, if we have
    /// committed the last node in the tree
    is_finished: bool,
    /// When we ask if we have finished, we check is the staleness
    /// If stale we stop
    staleness_timestamp: u64,
}

#[derive(Debug)]
pub struct StorageHealer {}

impl GenServer for StorageHealer {
    type CallMsg = StorageHealerCallMsg;
    type CastMsg = StorageHealerMsg;
    type OutMsg = StorageHealerOutMsg;
    type State = StorageHealerState;
    type Error = ();

    fn new() -> Self {
        Self {}
    }

    async fn handle_call(
        &mut self,
        _message: Self::CallMsg,
        _handle: &GenServerHandle<Self>,
        state: Self::State,
    ) -> CallResponse<Self> {
        // We only ask for IsFinished in the message, so we don't match it
        let is_finished = state.is_finished;
        let is_stale = current_unix_time() > state.staleness_timestamp;
        // Finished means that we have succesfully healed according to our algorithm
        // That means that we have commited the root_node of the tree
        if is_finished || is_stale {
            CallResponse::Stop(StorageHealerOutMsg::FinishedStale {
                is_finished,
                is_stale,
            })
        } else {
            CallResponse::Reply(
                state,
                StorageHealerOutMsg::FinishedStale {
                    is_finished,
                    is_stale,
                },
            )
        }
    }

    async fn handle_cast(
        &mut self,
        message: Self::CastMsg,
        _handle: &GenServerHandle<Self>,
        state: Self::State,
    ) -> CastResponse<Self> {
        match message {
            StorageHealerMsg::CheckUp => todo!(),
            StorageHealerMsg::TrieNodes(trie_nodes) => todo!(),
        }
        CastResponse::NoReply(state)
    }
}

/// This struct stores the metadata we need when we request a node
#[derive(Debug, Clone)]
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
pub fn storage_heal_trie(
    state_root: H256,
    account_paths: Vec<Nibbles>,
    peers: PeerHandler,
    store: Store,
    membatch: &mut Membatch,
    staleness_timestamp: u64,
) -> bool {
    // Logging stuff, we log during a given interval
    let mut last_update = Instant::now();

    let mut download_queue = get_initial_downloads(&account_paths);

    let mut node_processing_queue: Vec<NodeResponse> = Vec::new();

    // channel to send the download task to the peers
    let (task_sender, mut task_receiver) = tokio::sync::mpsc::channel::<Vec<NodeResponse>>(1000);

    loop {
        log_storage_heal(&mut last_update);

        if current_unix_time() > staleness_timestamp {
            return false;
        }

        // we request the trie nodes
        spawn_downloader_task(task_sender.clone(), &mut download_queue, &peers);

        // try_recv
        receive_data_from_tasks(&mut task_receiver, &mut node_processing_queue);

        // Then we process them
        // The coordinator for now is going to process them, although it may be more efficient if the writing to the database
        // is concurrentized
        process_data(
            &mut node_processing_queue,
            &mut download_queue,
            store.clone(),
            membatch,
        );

        if download_queue.is_empty() {
            return true;
        }
    }
}

fn receive_data_from_tasks(
    task_receiver: &mut Receiver<Vec<NodeResponse>>,
    node_processing_queue: &mut Vec<NodeResponse>,
) {
    todo!()
}

fn spawn_downloader_task(
    task_receiver: Sender<Vec<NodeResponse>>,
    download_queue: &mut Vec<NodeRequest>,
    peers: &PeerHandler,
) {
    todo!()
}

fn process_data(
    node_processing_queue: &mut Vec<NodeResponse>,
    download_queue: &mut Vec<NodeRequest>,
    store: Store,
    membatch: &mut Membatch,
) -> Result<(), StoreError> {
    while let Some(node_response) = node_processing_queue.pop() {
        trace!("We are processing node response {:?}", node_response);

        let (missing_children_nibbles, missing_children_count) =
            determine_missing_children(&node_response, store.clone(), membatch)?;

        if missing_children_count == 0 {
            // We flush to the database this node
            commit_node(&node_response, store.clone(), membatch);
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

fn get_initial_downloads(account_paths: &[Nibbles]) -> Vec<NodeRequest> {
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
        commit_node(&parent_entry.node_response, store, membatch)?;
    } else {
        membatch.insert(parent_key, parent_entry);
    }
    Ok(())
}
