use std::{
    collections::{HashMap, VecDeque},
    fmt::Debug,
    io::ErrorKind,
    sync::Arc,
    time::{Duration, SystemTime},
};

use bytes::Bytes;
use ethrex_common::{
    BigEndianHash, H256, U256,
    types::{AccountState, BlockBody, BlockHash, BlockHeader, Receipt, validate_block_body},
};
use ethrex_rlp::encode::RLPEncode;
use ethrex_trie::Nibbles;
use ethrex_trie::Node;
use rand::random;
use spawned_concurrency::tasks::{
    CallResponse, CastResponse, GenServer, GenServerHandle,
    InitResult::{self, Success},
    send_interval,
};
use tokio::{sync::Mutex, time::Instant};

use crate::{
    kademlia::{Kademlia, PeerChannels, PeerData},
    metrics::METRICS,
    rlpx::{
        downloader::{
            Downloader, DownloaderCallRequest, DownloaderCallResponse, DownloaderCastRequest,
        },
        p2p::{Capability, SUPPORTED_ETH_CAPABILITIES, SUPPORTED_SNAP_CAPABILITIES},
        snap::{AccountRangeUnit, GetTrieNodes, TrieNodes},
    },
    sync::{AccountStorageRoots, BlockSyncState, block_is_stale},
    utils::{
        SendMessageError, dump_to_file, get_account_state_snapshot_file,
        get_account_storages_snapshot_file,
    },
};
use tracing::{debug, error, info, trace, warn};
pub const PEER_REPLY_TIMEOUT: Duration = Duration::from_secs(15);
pub const PEER_SELECT_RETRY_ATTEMPTS: u32 = 3;
pub const REQUEST_RETRY_ATTEMPTS: u32 = 5;
pub const MAX_RESPONSE_BYTES: u64 = 512 * 1024;
pub const HASH_MAX: H256 = H256([0xFF; 32]);
pub const CHUNK_COUNT: u64 = 800;
const MAX_BYTECODES_REQUEST_SIZE: usize = 100;

pub const SNAP_LIMIT: usize = 128;

// Request as many as 128 block bodies per request
// this magic number is not part of the protocol and is taken from geth, see:
// https://github.com/ethereum/go-ethereum/blob/2585776aabbd4ae9b00050403b42afb0cee968ec/eth/downloader/downloader.go#L42-L43
//
// Note: We noticed that while bigger values are supported
// increasing them may be the cause of peers disconnection
pub const MAX_BLOCK_BODIES_TO_REQUEST: usize = 128;

/// Holds information about connected peers, their performance and availability
#[derive(Debug, Clone)]
pub struct PeerInformation {
    pub score: i64,
    pub request_time: Option<Instant>,
}

impl Default for PeerInformation {
    fn default() -> Self {
        Self {
            score: 0,
            request_time: None,
        }
    }
}

impl PeerInformation {
    pub fn is_available(&self) -> bool {
        self.request_time.is_none()
    }
}

/// An abstraction over the [Kademlia] containing logic to make requests to peers
#[derive(Debug, Clone)]
pub struct PeerHandler {
    pub peer_table: Kademlia,
    pub peers_info: Arc<Mutex<HashMap<H256, PeerInformation>>>,
    pending_tasks: VecDeque<Task>,
    started_tasks: HashMap<H256, (Task, Instant)>,
    sync_state: InternalSyncState,
    pivot_header: BlockHeader,
}

#[derive(Clone, Debug)]
enum Task {
    Headers {
        start_block: u64,
        chunk_limit: u64,
    },
    AccountRanges {
        chunk_start: H256,
        chunk_end: H256,
    },
    StorageRanges {
        start_index: usize,
        end_index: usize,
        start_hash: H256,
        end_hash: Option<H256>,
    },
    Bytecode {
        chunk_start: usize,
        chunk_end: usize,
    },
}

#[derive(Clone)]
// Internal state used to handle the sync process
// in each of its steps
enum InternalSyncState {
    Idle,
    RetrievingHeaders {
        sync_head_number: u64,
        current_show: u64,
        acc_headers: Vec<BlockHeader>,
    },
    FinishedHeaders(Vec<BlockHeader>),
    RetrievingAccountRanges {
        account_state_snapshots_dir: String,
        chunk_file_index: u64,
        block_sync_state: BlockSyncState,
        completed_tasks: u64,
        all_account_hashes: Vec<H256>,
        all_accounts_state: Vec<AccountState>,
    },
    FinishedAccountRanges,
    RetrievingStorageRanges {
        account_storages_snapshots_dir: String,
        chunk_file_index: u64,
        account_storage_roots: AccountStorageRoots,
        all_account_storages: Vec<Vec<(H256, U256)>>,
        accounts_done: Vec<H256>,
        current_account_hashes: Vec<H256>,
        task_count: u64,
        completed_tasks: u64,
    },
    FinishedStorageRanges(u64, AccountStorageRoots),
    RetrievingBytecode {
        completed_tasks: u64,
        all_bytecode_hashes: Vec<H256>,
        all_bytecodes: Vec<Bytes>,
    },
    FinishedBytecode(Vec<Bytes>),
}

#[derive(Clone)]
// State exposed to the caller, prevents cloning middle states
// which is highly inneficient
pub enum SyncState {
    Idle,
    RetrievingHeaders,
    FinishedHeaders(Vec<BlockHeader>),
    RetrievingAccountRanges,
    FinishedAccountRanges,
    RetrievingStorageRanges,
    FinishedStorageRanges(u64, AccountStorageRoots),
    RetrievingBytecode,
    FinishedBytecode(Vec<Bytes>),
}

impl std::fmt::Display for InternalSyncState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InternalSyncState::Idle => write!(f, "Idle"),
            InternalSyncState::RetrievingHeaders { .. } => write!(f, "RetrievingHeaders"),
            InternalSyncState::FinishedHeaders(_) => write!(f, "FinishedHeaders"),
            InternalSyncState::RetrievingAccountRanges { .. } => {
                write!(f, "RetrievingAccountRanges")
            }
            InternalSyncState::FinishedAccountRanges => write!(f, "FinishedAccountRanges"),
            InternalSyncState::RetrievingStorageRanges { .. } => {
                write!(f, "RetrievingStorageRanges")
            }
            InternalSyncState::FinishedStorageRanges(_, _) => write!(f, "FinishedStorageRanges"),
            InternalSyncState::RetrievingBytecode { .. } => write!(f, "RetrievingBytecode"),
            InternalSyncState::FinishedBytecode(_) => write!(f, "FinishedBytecode"),
        }
    }
}

impl Debug for InternalSyncState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self)
    }
}

impl From<&InternalSyncState> for SyncState {
    fn from(state: &InternalSyncState) -> Self {
        match state {
            InternalSyncState::Idle => SyncState::Idle,
            InternalSyncState::RetrievingHeaders { .. } => SyncState::RetrievingHeaders,
            InternalSyncState::FinishedHeaders(headers) => {
                SyncState::FinishedHeaders(headers.clone())
            }
            InternalSyncState::RetrievingAccountRanges { .. } => SyncState::RetrievingAccountRanges,
            InternalSyncState::FinishedAccountRanges => SyncState::FinishedAccountRanges,
            InternalSyncState::RetrievingStorageRanges { .. } => SyncState::RetrievingStorageRanges,
            InternalSyncState::FinishedStorageRanges(index, roots) => {
                SyncState::FinishedStorageRanges(*index, roots.clone())
            }
            InternalSyncState::RetrievingBytecode { .. } => SyncState::RetrievingBytecode,
            InternalSyncState::FinishedBytecode(bytecodes) => {
                SyncState::FinishedBytecode(bytecodes.clone())
            }
        }
    }
}

pub enum BlockRequestOrder {
    OldToNew,
    NewToOld,
}

#[derive(Clone, Debug)]
pub struct StorageTaskResult {
    pub start_index: usize,
    pub account_storages: Vec<Vec<(H256, U256)>>,
    pub peer_id: H256,
    pub remaining_start: usize,
    pub remaining_end: usize,
    pub remaining_hash_range: (H256, Option<H256>),
}

#[derive(Clone, Debug)]
pub struct BytecodeTaskResult {
    pub start_index: usize,
    pub bytecodes: Vec<Bytes>,
    pub peer_id: H256,
    pub remaining_start: usize,
    pub remaining_end: usize,
}

#[derive(Debug)]
// TODO: add Storage actor?
struct StorageTask {
    start_index: usize,
    end_index: usize,
    start_hash: H256,
    // end_hash is None if the task is for the first big storage request
    end_hash: Option<H256>,
}

impl PeerHandler {
    pub fn new(peer_table: Kademlia) -> PeerHandler {
        Self {
            peer_table,
            peers_info: Default::default(),
            pending_tasks: vec![].into(),
            started_tasks: Default::default(),
            sync_state: InternalSyncState::Idle,
            pivot_header: BlockHeader::default(),
        }
    }

    /// Creates a dummy PeerHandler for tests where interacting with peers is not needed
    /// This should only be used in tests as it won't be able to interact with the node's connected peers
    pub fn dummy() -> PeerHandler {
        let dummy_peer_table = Kademlia::new();
        PeerHandler::new(dummy_peer_table)
    }

    // TODO: Implement the logic for recording peer successes
    /// Helper method to record successful peer response
    async fn record_peer_success(&self, _peer_id: H256) {}

    // TODO: Implement the logic for recording peer failures
    /// Helper method to record failed peer response
    async fn record_peer_failure(&self, _peer_id: H256) {}

    // TODO: Implement the logic for recording critical peer failures
    /// Helper method to record critical peer failure
    /// This is used when the peer returns invalid data or is otherwise unreliable
    async fn record_peer_critical_failure(&self, _peer_id: H256) {}

    /// Marks a peer as free (available for requests)
    async fn mark_peer_as_free(&self, peer_id: H256) {
        self.peers_info
            .lock()
            .await
            .entry(peer_id)
            .and_modify(|info| info.request_time = None);
        debug!("Downloader {peer_id} freed");
    }

    /// Marks a peer as busy (currently handling a request)
    async fn mark_peer_as_busy(&self, peer_id: H256) {
        self.peers_info
            .lock()
            .await
            .entry(peer_id)
            .and_modify(|info| info.request_time = Some(Instant::now()));
        debug!("Downloader {peer_id} marked as busy");
    }

    /// Helper function called in between snap sync steps.
    /// Prevents peers from being marked as busy indefinitely.
    async fn refresh_peers_availability(&self) {
        for (_, peer) in self.peers_info.lock().await.iter_mut() {
            peer.request_time = None;
        }
    }

    // TODO: once peer handler becomes an actor, call this periodically
    // TODO: redundant as `reset_timed_out_tasks`
    /// Helper function that frees peers after being busy
    /// for more than the tolerated time
    pub async fn reset_timed_out_busy_peers(&self) {
        for (_, peer) in self.peers_info.lock().await.iter_mut() {
            if peer
                .request_time
                .is_some_and(|time| time.elapsed() > PEER_REPLY_TIMEOUT)
            {
                debug!("Resetting peer that was busy for too long");
                peer.request_time = None;
            }
        }
    }

    pub async fn reset_timed_out_tasks(&mut self) {
        for (peer_id, (task, start_time)) in self.started_tasks.clone() {
            // TODO: HEAVY CLONE?
            if start_time.elapsed() > PEER_REPLY_TIMEOUT {
                debug!("Resetting task for peer {peer_id} that was busy for too long");
                self.pending_tasks.push_back(task);
                self.started_tasks.remove(&peer_id);
                self.mark_peer_as_free(peer_id).await;
            }
        }
    }

    /// TODO: docs
    pub async fn get_peer_channel_with_highest_score(
        &self,
        capabilities: &[Capability],
        peer_info: &mut HashMap<H256, PeerInformation>,
    ) -> Result<Option<(H256, PeerChannels)>, PeerHandlerError> {
        let (mut free_peer_id, mut free_peer_channel) = self
            .peer_table
            .get_peer_channels(capabilities)
            .await
            .first()
            .ok_or(PeerHandlerError::NoPeers)?
            .clone();

        let mut max_peer_id_score = i64::MIN;
        for (peer_id, channel) in self.peer_table.get_peer_channels(capabilities).await.iter() {
            let peer_info = peer_info.entry(*peer_id).or_default();
            if peer_info.score >= max_peer_id_score {
                free_peer_id = *peer_id;
                max_peer_id_score = peer_info.score;
                free_peer_channel = channel.clone();
            }
        }

        Ok(Some((free_peer_id, free_peer_channel.clone())))
    }

    async fn update_peers_info(&self) {
        let peer_channels = self
            .peer_table
            .get_peer_channels(&SUPPORTED_ETH_CAPABILITIES)
            .await;
        for (peer_id, _peer_channels) in &peer_channels {
            let mut peers_info = self.peers_info.lock().await;
            if peers_info.contains_key(peer_id) {
                // Peer is already in the downloaders list, skip it
                continue;
            }
            peers_info.insert(*peer_id, PeerInformation::default());

            debug!("{peer_id} added as downloader");
        }
    }

    // Retrieves a peer channel with supported capabilities
    async fn retrieve_peer_channels(
        &self,
        peer_id: H256,
        capabilities: &[Capability],
    ) -> Option<PeerChannels> {
        self.peer_table
            .get_peer_channels(capabilities)
            .await
            .iter()
            .find(|(id, _)| *id == peer_id)
            .map(|(_, peer_channels)| peer_channels.clone())
    }

    /// Returns a random available `Downloader` with supported capabilities,
    /// or None if there are no peers are available
    async fn get_random_downloader(&self, capabilities: &[Capability]) -> Option<Downloader> {
        // self.update_peers_info().await;

        let free_downloaders = self
            .peers_info
            .lock()
            .await
            .clone()
            .into_iter()
            .filter(|(_downloader_id, peer_info)| peer_info.is_available())
            .collect::<Vec<_>>();

        if free_downloaders.is_empty() {
            return None;
        }

        let free_peer_id = free_downloaders
            .get(random::<usize>() % free_downloaders.len())
            .map(|(peer_id, _)| *peer_id)?;

        let Some(free_downloader_channels) = self
            .retrieve_peer_channels(free_peer_id, capabilities)
            .await
        else {
            // The free downloader is not a peer of us anymore.
            debug!(
                "Downloader {free_peer_id} is not a peer anymore, removing it from the downloaders list"
            );
            self.peers_info.lock().await.remove(&free_peer_id);
            return None;
        };

        Some(Downloader::new(free_peer_id, free_downloader_channels))
    }

    /// Returns the best available `Downloader` with supported capabilities,
    /// or None if there are no peers are available
    async fn get_best_downloader(&self, capabilities: &[Capability]) -> Option<Downloader> {
        // self.update_peers_info().await;

        let free_downloaders = self
            .peers_info
            .lock()
            .await
            .clone()
            .into_iter()
            .filter(|(_downloader_id, peer_info)| peer_info.is_available())
            .collect::<Vec<_>>();

        if free_downloaders.is_empty() {
            return None;
        }

        let (mut free_peer_id, _) = free_downloaders[0];

        let peers_info = self.peers_info.lock().await;
        for (peer_id, _) in free_downloaders.iter() {
            if let (Some(peer_info), Some(free_peer_info)) =
                (peers_info.get(peer_id), peers_info.get(&free_peer_id))
            {
                if peer_info.score >= free_peer_info.score {
                    free_peer_id = *peer_id;
                }
            }
        }
        drop(peers_info);

        let Some(free_downloader_channels) = self
            .retrieve_peer_channels(free_peer_id, capabilities)
            .await
        else {
            // The free downloader is not a peer of us anymore.
            debug!(
                "Downloader {free_peer_id} is not a peer anymore, removing it from the downloaders list"
            );
            self.peers_info.lock().await.remove(&free_peer_id);
            return None;
        };

        Some(Downloader::new(free_peer_id, free_downloader_channels))
    }

    async fn update_pivot_header(&mut self) {
        let new_pivot_block_number = self.pivot_header.number + SNAP_LIMIT as u64 - 11;

        // TODO: possible permanent loop?
        loop {
            let peers_table = self
                .peer_table
                .get_peer_channels(&SUPPORTED_ETH_CAPABILITIES)
                .await;

            for (peer_id, peer_channels) in peers_table {
                let mut downloader = Downloader::new(peer_id, peer_channels).start();
                match downloader
                    .call(DownloaderCallRequest::BlockHeader {
                        block_number: new_pivot_block_number,
                    })
                    .await
                {
                    Ok(DownloaderCallResponse::BlockHeader(header)) => {
                        debug!("Updated pivot header to block number {}", header.number);
                        self.pivot_header = header;
                        return;
                    }
                    _ => {
                        debug!("Sync Log 14: Failed to update pivot block from peer {peer_id}");
                    }
                }
            }
        }
    }

    /// Internal method to request block bodies from any suitable peer given their block hashes
    /// Returns the block bodies or None if:
    /// - There are no available peers (the node just started up or was rejected by all other nodes)
    /// - The requested peer did not return a valid response in the given time limit
    async fn request_block_bodies_inner(
        &self,
        block_hashes: Vec<H256>,
    ) -> Option<(Vec<BlockBody>, H256)> {
        self.refresh_peers_availability().await;

        let available_downloader = loop {
            self.reset_timed_out_busy_peers().await;
            match self
                .get_random_downloader(&SUPPORTED_ETH_CAPABILITIES)
                .await
            {
                Some(downloader) => break downloader,
                None => {
                    debug!("No available downloader found, retrying...");
                    tokio::time::sleep(Duration::from_secs(10)).await;
                    continue;
                }
            }
        };

        let peer_id = available_downloader.peer_id();
        match available_downloader
            .start()
            .call(DownloaderCallRequest::BlockBodies { block_hashes })
            .await
        {
            Ok(DownloaderCallResponse::BlockBodies(block_bodies)) => {
                self.record_peer_success(peer_id).await;
                Some((block_bodies, peer_id))
            }
            _ => {
                warn!(
                    "[SYNCING] Didn't receive block bodies from peer, penalizing peer {peer_id}..."
                );
                self.record_peer_failure(peer_id).await;
                None
            }
        }
    }

    /// Requests block bodies from any suitable peer given their block headers and validates them
    /// Returns the requested block bodies or None if:
    /// - There are no available peers (the node just started up or was rejected by all other nodes)
    /// - No peer returned a valid response in the given time and retry limits
    /// - The block bodies are invalid given the block headers
    pub async fn request_and_validate_block_bodies(
        &self,
        block_headers: &[BlockHeader],
    ) -> Option<Vec<BlockBody>> {
        self.refresh_peers_availability().await;
        let block_hashes: Vec<H256> = block_headers.iter().map(|h| h.hash()).collect();

        for _ in 0..REQUEST_RETRY_ATTEMPTS {
            let Some((block_bodies, peer_id)) =
                self.request_block_bodies_inner(block_hashes.clone()).await
            else {
                continue; // Retry on empty response
            };
            let mut res = Vec::new();
            let mut validation_success = true;
            for (header, body) in block_headers[..block_bodies.len()].iter().zip(block_bodies) {
                if let Err(e) = validate_block_body(header, &body) {
                    warn!(
                        "Invalid block body error {e}, discarding peer {peer_id} and retrying..."
                    );
                    validation_success = false;
                    self.record_peer_critical_failure(peer_id).await;
                    break;
                }
                res.push(body);
            }
            // Retry on validation failure
            if validation_success {
                return Some(res);
            }
        }
        None
    }

    /// Requests all receipts in a set of blocks from any suitable peer given their block hashes
    /// Returns the lists of receipts or None if:
    /// - There are no available peers (the node just started up or was rejected by all other nodes)
    /// - No peer returned a valid response in the given time and retry limits
    pub async fn request_receipts(&self, block_hashes: Vec<H256>) -> Option<Vec<Vec<Receipt>>> {
        self.refresh_peers_availability().await;
        let mut attempts = 0;
        while attempts < REQUEST_RETRY_ATTEMPTS {
            let available_downloader = loop {
                self.reset_timed_out_busy_peers().await;
                match self
                    .get_random_downloader(&SUPPORTED_ETH_CAPABILITIES)
                    .await
                {
                    Some(downloader) => break downloader,
                    None => {
                        debug!("No available downloader found, retrying...");
                        tokio::time::sleep(Duration::from_secs(10)).await;
                        attempts += 1;
                        continue;
                    }
                }
            };

            if let Ok(DownloaderCallResponse::Receipts(Some(receipts))) = available_downloader
                .start()
                .call(DownloaderCallRequest::Receipts {
                    block_hashes: block_hashes.clone(),
                })
                .await
            {
                return Some(receipts);
            };
            attempts += 1;
        }
        None
    }

    pub async fn request_state_trienodes(
        peer_channel: &mut PeerChannels,
        state_root: H256,
        paths: Vec<RequestMetadata>,
    ) -> Result<Vec<Node>, RequestStateTrieNodesError> {
        // Keep track of peers we requested from so we can penalize unresponsive peers when we get a response
        // This is so we avoid penalizing peers due to requesting stale data

        let paths_bytes = paths
            .iter()
            .map(|vec| vec![Bytes::from(vec.path.encode_compact())])
            .collect();
        let available_downloader = Downloader::new(Default::default(), peer_channel.clone());
        match available_downloader
            .start()
            .call(DownloaderCallRequest::TrieNodes {
                root_hash: state_root,
                paths: paths_bytes,
            })
            .await
        {
            Ok(DownloaderCallResponse::TrieNodes(nodes)) => {
                for (index, node) in nodes.iter().enumerate() {
                    if node.compute_hash().finalize() != paths[index].hash {
                        error!(
                            "A peer is sending wrong data for the state trie node {:?}",
                            paths[index].path
                        );
                        return Err(RequestStateTrieNodesError::InvalidHash);
                    }
                }

                Ok(nodes)
            }
            _ => Err(RequestStateTrieNodesError::InvalidData),
        }
    }

    /// Requests storage trie nodes given the root of the state trie where they are contained and
    /// a hashmap mapping the path to the account in the state trie (aka hashed address) to the paths to the nodes in its storage trie (can be full or partial)
    /// Returns the nodes or None if:
    /// - There are no available peers (the node just started up or was rejected by all other nodes)
    /// - No peer returned a valid response in the given time and retry limits
    pub async fn request_storage_trienodes(
        peer_channel: &mut PeerChannels,
        get_trie_nodes: GetTrieNodes,
    ) -> Result<TrieNodes, RequestStorageTrieNodes> {
        // Keep track of peers we requested from so we can penalize unresponsive peers when we get a response
        // This is so we avoid penalizing peers due to requesting stale data
        let id = get_trie_nodes.id;
        let available_downloader = Downloader::new(Default::default(), peer_channel.clone());
        match available_downloader
            .start()
            .call(DownloaderCallRequest::TrieNodes {
                root_hash: get_trie_nodes.root_hash,
                paths: get_trie_nodes.paths,
            })
            .await
        {
            Ok(DownloaderCallResponse::TrieNodes(nodes)) => {
                // TODO: This might not be correct, verify
                let nodes = nodes
                    .iter()
                    .map(|node| Bytes::from(node.encode_raw()))
                    .collect();
                Ok(TrieNodes { id, nodes })
            }
            _ => Err(RequestStorageTrieNodes::SendMessageError(
                id,
                SendMessageError::PeerBusy,
            )), // TODO: THIS ERROR IS NOT ADECUATE
        }
    }

    /// Returns the PeerData for each connected Peer
    pub async fn read_connected_peers(&self) -> Vec<PeerData> {
        self.peer_table
            .peers
            .lock()
            .await
            .iter()
            .map(|(_, peer)| peer)
            .cloned()
            .collect()
    }

    pub async fn count_total_peers(&self) -> usize {
        self.peer_table.peers.lock().await.len()
    }

    // TODO: Implement the logic to remove a peer from the peer table
    pub async fn remove_peer(&self, _peer_id: H256) {}

    pub async fn get_block_header(
        &self,
        peer_channel: &mut PeerChannels,
        block_number: u64,
    ) -> Result<Option<BlockHeader>, PeerHandlerError> {
        let available_downloader = Downloader::new(Default::default(), peer_channel.clone());
        match available_downloader
            .start()
            .call(DownloaderCallRequest::BlockHeader { block_number })
            .await
        {
            Ok(DownloaderCallResponse::BlockHeader(block_header)) => Ok(Some(block_header)),
            _ => Ok(None),
        }
    }
}

#[derive(Clone)]
pub enum PeerHandlerCastMessage {
    UpdatePeers,
    AssignTasks,
    /// Called from a `Downloader` when a task is finished
    TaskFinished {
        peer_id: H256,
        response: DownloaderResponse,
    },
    UpdateState(InternalSyncState),
}

#[derive(Clone, Debug)]
pub enum DownloaderResponse {
    Headers(Vec<BlockHeader>),
    AccountRange(Vec<AccountRangeUnit>, Option<(H256, H256)>),
    StorageRange(StorageTaskResult),
    Bytecode(BytecodeTaskResult),
}

#[derive(Clone)]
pub enum PeerHandlerCallMessage {
    PivotHeader,
    CurrentState,
    DownloadHeaders(u64, H256),
    DownloadAccountRanges {
        start: H256,
        limit: H256,
        account_state_snapshots_dir: String,
        block_sync_state: BlockSyncState,
    },
    DownloadStorageRanges {
        storage_accounts: AccountStorageRoots,
        account_storages_snapshot_dir: String,
        chunk_index: u64,
    },
    DownloadBytecode(Vec<H256>),
    DownloadBlockBodies(Vec<BlockHash>),
}

#[derive(Clone)]
pub enum PeerHandlerCallResponse {
    PivotHeader(BlockHeader),
    CurrentState(SyncState),
    /// Use to signal that snap sync download request is in progress
    InProgress,
    BlockBodies(Vec<BlockBody>),
    // Possible errors
    SyncHeadNotFound,
    BlockBodiesNotFound,
}

impl GenServer for PeerHandler {
    type CastMsg = PeerHandlerCastMessage;
    type CallMsg = PeerHandlerCallMessage;
    type OutMsg = PeerHandlerCallResponse;
    type Error = ();

    async fn init(self, handle: &GenServerHandle<Self>) -> Result<InitResult<Self>, Self::Error> {
        let _peer_updater = send_interval(
            Duration::from_secs(5),
            handle.clone(),
            PeerHandlerCastMessage::UpdatePeers,
        );

        let _task_assigner = send_interval(
            Duration::from_millis(10),
            handle.clone(),
            PeerHandlerCastMessage::AssignTasks,
        );

        Ok(Success(self))
    }

    async fn handle_cast(
        &mut self,
        message: Self::CastMsg,
        handle: &GenServerHandle<Self>,
    ) -> spawned_concurrency::tasks::CastResponse {
        match message {
            PeerHandlerCastMessage::UpdatePeers => {
                self.update_peers_info().await;
            }
            PeerHandlerCastMessage::AssignTasks => {
                self.reset_timed_out_tasks().await;

                if self.pending_tasks.is_empty() {
                    return CastResponse::NoReply;
                }

                let capabilities = match self.sync_state {
                    InternalSyncState::RetrievingHeaders { .. } => &SUPPORTED_ETH_CAPABILITIES,
                    InternalSyncState::RetrievingAccountRanges { .. }
                    | InternalSyncState::RetrievingStorageRanges { .. } => {
                        &SUPPORTED_SNAP_CAPABILITIES
                    }
                    // Idle or finished states, should not get here
                    _ => &SUPPORTED_SNAP_CAPABILITIES,
                };

                while let Some(available_downloader) = match self.sync_state {
                    InternalSyncState::RetrievingHeaders { .. } => {
                        self.get_random_downloader(capabilities).await
                    }
                    InternalSyncState::RetrievingAccountRanges { .. }
                    | InternalSyncState::RetrievingStorageRanges { .. } => {
                        self.get_best_downloader(capabilities).await
                    }
                    // Idle or finished states, should not get here
                    _ => self.get_random_downloader(capabilities).await,
                } {
                    let Some(next_task) = self.pending_tasks.pop_front() else {
                        // No tasks to assign
                        return CastResponse::NoReply;
                    };

                    let peer_id = available_downloader.peer_id();
                    let mut downloader_handle = available_downloader.start();
                    match next_task {
                        Task::Headers {
                            start_block,
                            chunk_limit,
                        } => {
                            if !matches!(
                                self.sync_state,
                                InternalSyncState::RetrievingHeaders { .. }
                            ) {
                                // TODO: I don't know in which case this can happen
                                error!(
                                    "There's headers task, but the peer handler is not in the correct state"
                                );
                                return CastResponse::NoReply;
                            }

                            let response_handle = handle.clone();
                            downloader_handle
                                .cast(DownloaderCastRequest::Headers {
                                    response_handle,
                                    start_block,
                                    chunk_limit,
                                })
                                .await
                                .unwrap(); // TODO: handle unwrap
                        }
                        Task::AccountRanges {
                            chunk_start,
                            chunk_end,
                        } => {
                            let InternalSyncState::RetrievingAccountRanges {
                                account_state_snapshots_dir: _,
                                chunk_file_index: _,
                                block_sync_state: _,
                                completed_tasks,
                                all_account_hashes: _,
                                all_accounts_state: _,
                            } = &mut self.sync_state
                            else {
                                // TODO: This is a common occurrence when we finish downloading account ranges
                                debug!(
                                    "There's an account ranges task, but the peer handler is not in the correct state"
                                );
                                return CastResponse::NoReply;
                            };

                            // Prevent requesting empty tasks
                            if chunk_start == chunk_end {
                                *completed_tasks += 1;
                                continue;
                            }

                            if block_is_stale(&self.pivot_header) {
                                warn!("request_account_range became stale, updating pivot");
                                self.update_pivot_header().await;
                            }

                            let response_handle = handle.clone();
                            downloader_handle
                                .cast(DownloaderCastRequest::AccountRange {
                                    response_handle,
                                    root_hash: self.pivot_header.state_root,
                                    starting_hash: chunk_start,
                                    limit_hash: chunk_end,
                                })
                                .await
                                .unwrap(); // TODO: handle unwrap
                        }
                        Task::StorageRanges {
                            start_index,
                            end_index,
                            start_hash,
                            end_hash,
                        } => {
                            let InternalSyncState::RetrievingStorageRanges {
                                account_storages_snapshots_dir: _,
                                chunk_file_index: _,
                                account_storage_roots,
                                all_account_storages: _,
                                accounts_done: _,
                                current_account_hashes: _,
                                task_count: _,
                                completed_tasks,
                            } = &mut self.sync_state
                            else {
                                // TODO: I don't know in which case this can happen
                                error!(
                                    "There's an storage ranges task, but the peer handler is not in the correct state"
                                );
                                return CastResponse::NoReply;
                            };

                            // Prevent requesting empty tasks
                            if start_index == end_index {
                                *completed_tasks += 1;
                                continue;
                            }

                            if block_is_stale(&self.pivot_header) {
                                info!("request_storage_ranges became stale, breaking");
                                break;
                            }

                            let (chunk_account_hashes, chunk_storage_roots): (Vec<_>, Vec<_>) =
                                account_storage_roots
                                    .accounts_with_storage_root
                                    .iter()
                                    .skip(start_index)
                                    .take(end_index - start_index)
                                    .map(|(hash, root)| (*hash, *root))
                                    .unzip();

                            downloader_handle
                                .cast(DownloaderCastRequest::StorageRange {
                                    response_handle: handle.clone(),
                                    start_index,
                                    end_index,
                                    start_hash,
                                    end_hash,
                                    state_root: self.pivot_header.state_root,
                                    chunk_account_hashes,
                                    chunk_storage_roots,
                                })
                                .await
                                .unwrap();
                        }
                        Task::Bytecode {
                            chunk_start,
                            chunk_end,
                        } => {
                            let InternalSyncState::RetrievingBytecode {
                                completed_tasks,
                                all_bytecode_hashes,
                                all_bytecodes: _,
                            } = &mut self.sync_state
                            else {
                                error!(
                                    "There's a bytecode task, but the peer handler is not in the correct state"
                                );
                                return CastResponse::NoReply;
                            };

                            // Prevent requesting empty tasks
                            if chunk_start == chunk_end {
                                *completed_tasks += 1;
                                continue;
                            }

                            let hashes_to_request: Vec<H256> = all_bytecode_hashes
                                .iter()
                                .skip(chunk_start)
                                .take((chunk_end - chunk_start).min(MAX_BYTECODES_REQUEST_SIZE))
                                .copied()
                                .collect();

                            // Prevent requesting empty tasks
                            if hashes_to_request.is_empty() {
                                *completed_tasks += 1;
                                continue;
                            }

                            downloader_handle
                                .cast(DownloaderCastRequest::ByteCode {
                                    response_handle: handle.clone(),
                                    hashes_to_request,
                                    chunk_start,
                                    chunk_end,
                                })
                                .await
                                .unwrap();
                        }
                    }
                    debug!("Sent Downloader actor {peer_id} new request");
                    self.mark_peer_as_busy(peer_id).await;
                    self.started_tasks
                        .insert(peer_id, (next_task, Instant::now()));
                }
            }
            PeerHandlerCastMessage::TaskFinished { peer_id, response } => {
                self.mark_peer_as_free(peer_id).await;
                let Some((_, (requested_task, _))) = self.started_tasks.remove_entry(&peer_id)
                else {
                    // Should never happen
                    debug!(
                        "Received task finished from peer {peer_id} but we have no record of it"
                    );
                    return CastResponse::NoReply;
                };

                match response {
                    DownloaderResponse::Headers(headers) => {
                        if headers.is_empty() {
                            debug!("Peer {peer_id} returned empty headers");
                            if let Some(peer_info) = self.peers_info.lock().await.get_mut(&peer_id)
                            {
                                peer_info.score -= 1;
                            }
                            self.pending_tasks.push_back(requested_task);
                            return CastResponse::NoReply;
                        }
                        let downloaded_count = headers.len() as u64;
                        *METRICS.downloaded_headers.lock().await += downloaded_count;

                        if let InternalSyncState::RetrievingHeaders {
                            sync_head_number: _,
                            current_show,
                            acc_headers,
                        } = &mut self.sync_state
                        {
                            let batch_show = downloaded_count / 10_000;

                            if *current_show < batch_show {
                                debug!(
                                    "Downloaded {} headers from peer {} (current count: {downloaded_count})",
                                    headers.len(),
                                    peer_id
                                );
                                *current_show += 1;
                            }
                            acc_headers.extend_from_slice(&headers);
                        }

                        let downloaded_headers = headers.len() as u64;

                        // create a new task if the returned headers are less than the requested chunk limit
                        if let Task::Headers {
                            start_block,
                            chunk_limit,
                        } = requested_task
                        {
                            if downloaded_headers < chunk_limit {
                                let new_start = start_block + downloaded_headers;
                                debug!(
                                    "Task for ({start_block}, {chunk_limit}) was not completed, re-adding to the queue, {} remaining headers",
                                    chunk_limit - downloaded_headers
                                );
                                self.pending_tasks.push_back(Task::Headers {
                                    start_block: new_start,
                                    chunk_limit: chunk_limit - downloaded_headers,
                                });
                            } else {
                                if let InternalSyncState::RetrievingHeaders {
                                    sync_head_number,
                                    current_show: _,
                                    acc_headers,
                                } = &mut self.sync_state
                                {
                                    let pending = *sync_head_number + 1 - acc_headers.len() as u64;
                                    if pending == 0 {
                                        info!("Finished downloading all block headers");

                                        handle
                                            .clone()
                                            .cast(PeerHandlerCastMessage::UpdateState(
                                                InternalSyncState::FinishedHeaders(std::mem::take(
                                                    acc_headers,
                                                )),
                                            ))
                                            .await
                                            .unwrap();
                                    } else {
                                        debug!("{pending} headers remaining to download");
                                    }
                                }
                            }
                        }
                        if let Some(peer_info) = self.peers_info.lock().await.get_mut(&peer_id) {
                            peer_info.score += 1;
                        }
                    }
                    DownloaderResponse::AccountRange(accounts, chunk_start_end) => {
                        // TODO: WE ARE MISSING THE IF STATEMENT OF
                        // if let Ok(Err(dump_account_data)) = dump_account_result_receiver.try_recv() {

                        if let InternalSyncState::RetrievingAccountRanges {
                            account_state_snapshots_dir,
                            chunk_file_index,
                            block_sync_state: _,
                            completed_tasks,
                            all_account_hashes,
                            all_accounts_state,
                        } = &mut self.sync_state
                        {
                            if let Some((chunk_start, chunk_end)) = chunk_start_end {
                                if chunk_start <= chunk_end {
                                    self.pending_tasks.push_back(Task::AccountRanges {
                                        chunk_start,
                                        chunk_end,
                                    });
                                } else {
                                    *completed_tasks += 1;
                                }
                            }
                            if chunk_start_end.is_none() {
                                *completed_tasks += 1;
                            }

                            if accounts.is_empty() {
                                if let Some(peer_info) =
                                    self.peers_info.lock().await.get_mut(&peer_id)
                                {
                                    peer_info.score -= 1;
                                }
                                self.pending_tasks.push_back(requested_task);
                                return CastResponse::NoReply;
                            }
                            if let Some(peer_info) = self.peers_info.lock().await.get_mut(&peer_id)
                            {
                                peer_info.score += 1;
                            }

                            all_account_hashes.extend(accounts.iter().map(|unit| unit.hash));
                            all_accounts_state.extend(
                                accounts
                                    .iter()
                                    .map(|unit| AccountState::from(unit.account.clone())),
                            );

                            // TODO: MOVE THIS SOMEWHERE ELSE
                            if all_accounts_state.len() * size_of::<AccountState>()
                                >= 1024 * 1024 * 1024 * 8
                            {
                                let current_account_hashes = std::mem::take(all_account_hashes);
                                let current_account_states = std::mem::take(all_accounts_state);

                                let account_state_chunk = current_account_hashes
                                    .into_iter()
                                    .zip(current_account_states)
                                    .collect::<Vec<(H256, AccountState)>>()
                                    .encode_to_vec();

                                let account_state_snapshots_dir_cloned =
                                    account_state_snapshots_dir.clone();
                                // let dump_account_result_sender_cloned = dump_account_result_sender.clone();
                                let index = chunk_file_index.clone();
                                // TODO: re-add as a separate task
                                // tokio::task::spawn(async move {
                                let path = get_account_state_snapshot_file(
                                    account_state_snapshots_dir_cloned,
                                    index,
                                );
                                dump_to_file(path, account_state_chunk).unwrap(); // TODO: HANDLE UNWRAP
                                // });

                                *chunk_file_index += 1;
                            }
                            // TODO: MOVE THIS SOMEWHERE ELSE

                            if *completed_tasks >= CHUNK_COUNT {
                                info!("Finished downloading all account ranges");

                                // TODO: This is repeated code, consider refactoring
                                {
                                    let current_account_hashes = std::mem::take(all_account_hashes);
                                    let current_account_states = std::mem::take(all_accounts_state);

                                    let account_state_chunk = current_account_hashes
                                        .into_iter()
                                        .zip(current_account_states)
                                        .collect::<Vec<(H256, AccountState)>>()
                                        .encode_to_vec();

                                    let dir_cloned = account_state_snapshots_dir.clone();
                                    let path = get_account_state_snapshot_file(
                                        dir_cloned,
                                        *chunk_file_index,
                                    );
                                    std::fs::write(path, account_state_chunk).unwrap()
                                }

                                handle
                                    .clone()
                                    .cast(PeerHandlerCastMessage::UpdateState(
                                        InternalSyncState::FinishedAccountRanges,
                                    ))
                                    .await
                                    .unwrap();
                            }
                        }
                    }
                    DownloaderResponse::StorageRange(storage_task_result) => {
                        if let InternalSyncState::RetrievingStorageRanges {
                            account_storages_snapshots_dir,
                            chunk_file_index,
                            account_storage_roots,
                            all_account_storages,
                            accounts_done,
                            current_account_hashes,
                            task_count,
                            completed_tasks,
                        } = &mut self.sync_state
                        {
                            if all_account_storages.iter().map(Vec::len).sum::<usize>() * 64
                                > 1024 * 1024 * 1024 * 8
                            {
                                let current_account_storages = std::mem::take(all_account_storages);
                                *all_account_storages =
                                    vec![
                                        vec![];
                                        account_storage_roots.accounts_with_storage_root.len()
                                    ];

                                let snapshot = current_account_hashes
                                    .clone()
                                    .into_iter()
                                    .zip(current_account_storages)
                                    .collect::<Vec<_>>()
                                    .encode_to_vec();

                                let account_storages_snapshots_dir_cloned =
                                    account_storages_snapshots_dir.clone();
                                let chunk_index = *chunk_file_index;

                                // TODO: re-add as a separate task
                                // tokio::spawn(async move {
                                let path = get_account_storages_snapshot_file(
                                    account_storages_snapshots_dir_cloned,
                                    chunk_index,
                                );
                                dump_to_file(path, snapshot).unwrap();
                                // });
                                *chunk_file_index += 1;
                            }

                            let StorageTaskResult {
                                start_index,
                                mut account_storages,
                                peer_id,
                                remaining_start,
                                remaining_end,
                                remaining_hash_range: (hash_start, hash_end),
                            } = storage_task_result;
                            *completed_tasks += 1;

                            self.peers_info
                                .lock()
                                .await
                                .entry(peer_id)
                                .and_modify(|info| info.request_time = None);

                            for account in &current_account_hashes[start_index..remaining_start] {
                                accounts_done.push(*account);
                            }

                            if remaining_start < remaining_end {
                                trace!("Failed to download chunk from peer {peer_id}");
                                if hash_start.is_zero() {
                                    // Task is common storage range request
                                    self.pending_tasks.push_back(Task::StorageRanges {
                                        start_index: remaining_start,
                                        end_index: remaining_end,
                                        start_hash: H256::zero(),
                                        end_hash: None,
                                    });
                                    *task_count += 1;
                                } else if let Some(hash_end) = hash_end {
                                    // Task was a big storage account result
                                    if hash_start <= hash_end {
                                        self.pending_tasks.push_back(Task::StorageRanges {
                                            start_index: remaining_start,
                                            end_index: remaining_end,
                                            start_hash: hash_start,
                                            end_hash: Some(hash_end),
                                        });
                                        *task_count += 1;
                                        accounts_done.push(current_account_hashes[remaining_start]);
                                        account_storage_roots
                                            .healed_accounts
                                            .insert(current_account_hashes[start_index]);
                                    }
                                } else {
                                    if remaining_start + 1 < remaining_end {
                                        self.pending_tasks.push_back(Task::StorageRanges {
                                            start_index: remaining_start,
                                            end_index: remaining_start + 1,
                                            start_hash: hash_start,
                                            end_hash: None,
                                        });
                                        *task_count += 1;
                                    }
                                    // Task found a big storage account, so we split the chunk into multiple chunks
                                    let start_hash_u256 = U256::from_big_endian(&hash_start.0);
                                    let missing_storage_range = U256::MAX - start_hash_u256;

                                    let slot_count = account_storages
                                        .last()
                                        .map(|v| v.len())
                                        .unwrap() // TODO: Handle unwrap
                                        .max(1);
                                    let storage_density = start_hash_u256 / slot_count;

                                    let slots_per_chunk = U256::from(10000);
                                    let chunk_size = storage_density
                                        .checked_mul(slots_per_chunk)
                                        .unwrap_or(U256::MAX);

                                    let chunk_count =
                                        (missing_storage_range / chunk_size).as_usize().max(1);

                                    for i in 0..chunk_count {
                                        let start_hash_u256 = start_hash_u256 + chunk_size * i;
                                        let start_hash = H256::from_uint(&start_hash_u256);
                                        let end_hash = if i == chunk_count - 1 {
                                            H256::repeat_byte(0xff)
                                        } else {
                                            let end_hash_u256 = start_hash_u256
                                                .checked_add(chunk_size)
                                                .unwrap_or(U256::MAX);
                                            H256::from_uint(&end_hash_u256)
                                        };

                                        self.pending_tasks.push_back(Task::StorageRanges {
                                            start_index: remaining_start,
                                            end_index: remaining_start + 1,
                                            start_hash,
                                            end_hash: Some(end_hash),
                                        });
                                    }
                                    debug!("Split big storage account into {chunk_count} chunks.");
                                }
                            }

                            if account_storages.is_empty() {
                                self.peers_info.lock().await.entry(peer_id).and_modify(
                                    |peer_info| {
                                        peer_info.score -= 1;
                                    },
                                );
                                return CastResponse::NoReply;
                            }
                            if let Some(hash_end) = hash_end {
                                // This is a big storage account, and the range might be empty
                                if account_storages[0].len() == 1
                                    && account_storages[0][0].0 > hash_end
                                {
                                    return CastResponse::NoReply;
                                }
                            }

                            if let Some(peer_info) = self.peers_info.lock().await.get_mut(&peer_id)
                            {
                                if peer_info.score < 10 {
                                    peer_info.score += 1;
                                }
                            }

                            let n_storages = account_storages.len();
                            let n_slots = account_storages
                                .iter()
                                .map(|storage| storage.len())
                                .sum::<usize>();

                            debug!(
                                "Downloaded {n_storages} storages ({n_slots} slots) from peer {peer_id}"
                            );
                            debug!(
                                "Total tasks: {task_count}, completed tasks: {completed_tasks}, queued tasks: {}",
                                self.pending_tasks.len()
                            );
                            if account_storages.len() == 1 {
                                // We downloaded a big storage account
                                let acc = account_storages.remove(0);
                                all_account_storages[start_index].extend(acc);
                            } else {
                                for (i, storage) in account_storages.into_iter().enumerate() {
                                    all_account_storages[start_index + i] = storage;
                                }
                            }

                            // We finished downloading storage for all accounts
                            if completed_tasks >= task_count {
                                // TODO: move somewhere else
                                {
                                    let current_account_hashes = account_storage_roots
                                        .accounts_with_storage_root
                                        .iter()
                                        .map(|a| *a.0)
                                        .collect::<Vec<_>>();
                                    let current_account_storages =
                                        std::mem::take(all_account_storages);

                                    let snapshot = current_account_hashes
                                        .into_iter()
                                        .zip(current_account_storages)
                                        .collect::<Vec<_>>()
                                        .encode_to_vec();

                                    let account_storages_snapshots_dir_cloned =
                                        account_storages_snapshots_dir.clone();
                                    let path = get_account_storages_snapshot_file(
                                        account_storages_snapshots_dir_cloned,
                                        *chunk_file_index,
                                    );
                                    std::fs::write(path, snapshot).unwrap();
                                }

                                // TODO: RE-ENABLE
                                // disk_joinset
                                //     .join_all()
                                //     .await
                                //     .into_iter()
                                //     .map(|result| {
                                //         result
                                //             .inspect_err(|err| error!("We found this error while dumping to file {err:?}"))
                                //     })
                                //     .collect::<Result<Vec<()>, DumpError>>()
                                //     .map_err(PeerHandlerError::DumpError)?;

                                for account_done in accounts_done {
                                    account_storage_roots
                                        .accounts_with_storage_root
                                        .remove(&account_done);
                                }

                                handle
                                    .clone()
                                    .cast(PeerHandlerCastMessage::UpdateState(
                                        InternalSyncState::FinishedStorageRanges(
                                            *chunk_file_index + 1,
                                            std::mem::take(account_storage_roots),
                                        ),
                                    ))
                                    .await
                                    .unwrap();

                                info!("Finished downloading all storage ranges");
                            }
                        }
                    }
                    DownloaderResponse::Bytecode(bytecode_task_result) => {
                        if let InternalSyncState::RetrievingBytecode {
                            completed_tasks,
                            all_bytecode_hashes: _,
                            all_bytecodes,
                        } = &mut self.sync_state
                        {
                            let BytecodeTaskResult {
                                start_index,
                                bytecodes,
                                peer_id,
                                remaining_start,
                                remaining_end,
                            } = bytecode_task_result;
                            if bytecodes.is_empty() {
                                self.peers_info.lock().await.entry(peer_id).and_modify(
                                    |peer_info| {
                                        peer_info.score -= 1;
                                    },
                                );
                                self.pending_tasks.push_back(requested_task);
                                return CastResponse::NoReply;
                            }

                            if remaining_start < remaining_end {
                                self.pending_tasks.push_back(Task::Bytecode {
                                    chunk_start: remaining_start,
                                    chunk_end: remaining_end,
                                });
                            } else {
                                *completed_tasks += 1;
                            }

                            self.peers_info
                                .lock()
                                .await
                                .entry(peer_id)
                                .and_modify(|peer_info| {
                                    peer_info.score += 1;
                                });

                            for (i, bytecode) in bytecodes.into_iter().enumerate() {
                                all_bytecodes[start_index + i] = bytecode;
                            }

                            let chunk_count = 800; // TODO: move to a constant value
                            if *completed_tasks >= chunk_count {
                                info!("Finished downloading all bytecodes");
                                handle
                                    .clone()
                                    .cast(PeerHandlerCastMessage::UpdateState(
                                        InternalSyncState::FinishedBytecode(std::mem::take(
                                            all_bytecodes,
                                        )),
                                    ))
                                    .await
                                    .unwrap(); // TODO: handle unwrap
                            }
                        }
                    }
                }
            }
            PeerHandlerCastMessage::UpdateState(new_state) => {
                info!("Sync state updated: {} -> {}", self.sync_state, new_state);
                self.sync_state = new_state;
            }
        }
        CastResponse::NoReply
    }

    async fn handle_call(
        &mut self,
        message: Self::CallMsg,
        _handle: &GenServerHandle<Self>,
    ) -> CallResponse<Self> {
        match message {
            PeerHandlerCallMessage::PivotHeader => {
                if block_is_stale(&self.pivot_header) {
                    warn!("request_account_range became stale, updating pivot");
                    self.update_pivot_header().await;
                }
                CallResponse::Reply(PeerHandlerCallResponse::PivotHeader(
                    self.pivot_header.clone(),
                ))
            }
            PeerHandlerCallMessage::CurrentState => CallResponse::Reply(
                PeerHandlerCallResponse::CurrentState((&self.sync_state).into()), // TODO: CLONING STATE HERE IS COSTLY WITHOUT A GOOD REASON
            ),
            PeerHandlerCallMessage::DownloadHeaders(start, sync_head) => {
                // Retrieve sync head number
                let sync_head_number_retrieval_start = SystemTime::now();
                info!("Retrieving sync head block number from peers");

                let mut sync_head_number = 0_u64;
                let mut retries = 1;
                while sync_head_number == 0 {
                    if retries > 10 {
                        // sync_head might be invalid
                        return CallResponse::Reply(PeerHandlerCallResponse::SyncHeadNotFound);
                    }
                    let peers_table = self
                        .peer_table
                        .get_peer_channels(&SUPPORTED_ETH_CAPABILITIES)
                        .await;

                    for (peer_id, peer_channels) in peers_table {
                        let mut downloader = Downloader::new(peer_id, peer_channels).start();
                        match downloader
                            .call(DownloaderCallRequest::CurrentHead { sync_head })
                            .await
                        {
                            Ok(DownloaderCallResponse::CurrentHead(number)) => {
                                sync_head_number = number;
                                if number != 0 {
                                    break;
                                }
                            }
                            _ => {
                                debug!(
                                    "Sync Log 13: Failed to retrieve sync head block number from peer {peer_id}"
                                );
                            }
                        }
                    }

                    retries += 1;
                }

                let sync_head_number_retrieval_elapsed = sync_head_number_retrieval_start
                    .elapsed()
                    .unwrap_or_default();

                info!("Sync head block number retrieved");

                // Set pivot header to match sync head
                let mut retries = 1;
                self.pivot_header = loop {
                    if retries > 10 {
                        return CallResponse::Reply(PeerHandlerCallResponse::SyncHeadNotFound);
                    }

                    let peers_table = self
                        .peer_table
                        .get_peer_channels(&SUPPORTED_ETH_CAPABILITIES)
                        .await;

                    // Try all peers until one returns a header
                    if let Some(header) = futures::future::join_all(
                        peers_table.into_iter().map(|(peer_id, peer_channels)| async move {
                            let mut downloader = Downloader::new(peer_id, peer_channels).start();
                            match downloader
                                .call(DownloaderCallRequest::BlockHeader { block_number: sync_head_number })
                                .await
                            {
                                Ok(DownloaderCallResponse::BlockHeader(header)) => Some(header),
                                _ => {
                                    debug!("Sync Log 14: Failed to retrieve pivot header from peer {peer_id}");
                                    None
                                }
                            }
                        }),
                    )
                    .await
                    .into_iter()
                    .flatten()
                    .next()
                    {
                        break header;
                    }

                    retries += 1;
                };

                *METRICS.time_to_retrieve_sync_head_block.lock().await =
                    Some(sync_head_number_retrieval_elapsed);
                *METRICS.sync_head_block.lock().await = sync_head_number;
                *METRICS.headers_to_download.lock().await = sync_head_number + 1;
                *METRICS.sync_head_hash.lock().await = sync_head;

                let max_chunk_size = 800;
                let mut pending_tasks = VecDeque::new();

                let mut current_start = start;
                while current_start <= sync_head_number {
                    let remaining = sync_head_number + 1 - current_start;
                    let size = remaining.min(max_chunk_size);

                    pending_tasks.push_back(Task::Headers {
                        start_block: current_start,
                        chunk_limit: size,
                    });

                    current_start += size;
                }

                debug!(
                    "Created {} initial tasks for headers download",
                    pending_tasks.len()
                );
                self.pending_tasks = pending_tasks;
                self.started_tasks = HashMap::new();
                self.sync_state = InternalSyncState::RetrievingHeaders {
                    sync_head_number,
                    current_show: 0,
                    acc_headers: vec![],
                };

                CallResponse::Reply(PeerHandlerCallResponse::InProgress)
            }
            PeerHandlerCallMessage::DownloadAccountRanges {
                start,
                limit,
                account_state_snapshots_dir,
                block_sync_state,
            } => {
                // Create used directory if it doesn't exist
                if !std::fs::exists(&account_state_snapshots_dir).unwrap()
                // TODO: handle unwrap
                {
                    std::fs::create_dir_all(&account_state_snapshots_dir).unwrap(); // TODO: handle unwrap
                }

                // Create tasks
                // split the range in chunks of same length
                let start_u256 = U256::from_big_endian(&start.0);
                let limit_u256 = U256::from_big_endian(&limit.0);

                let chunk_size = (limit_u256 - start_u256) / CHUNK_COUNT;

                // list of tasks to be executed
                let mut pending_tasks = VecDeque::new();
                for i in 0..CHUNK_COUNT {
                    let chunk_start_u256 = chunk_size * i + start_u256;
                    // We subtract one because ranges are inclusive
                    let chunk_end_u256 = chunk_start_u256 + chunk_size - 1u64;
                    let chunk_start = H256::from_uint(&(chunk_start_u256));
                    let chunk_end = H256::from_uint(&(chunk_end_u256));
                    pending_tasks.push_back(Task::AccountRanges {
                        chunk_start,
                        chunk_end,
                    });
                }

                // Modify the last chunk to include the limit
                let last_task = pending_tasks.back_mut().unwrap(); // TODO: handle unwrap
                if let Task::AccountRanges {
                    chunk_start: _,
                    chunk_end,
                } = last_task
                {
                    *chunk_end = limit;
                };

                debug!(
                    "Created {} initial tasks for account ranges download",
                    pending_tasks.len()
                );
                self.pending_tasks = pending_tasks;
                self.started_tasks = HashMap::new();
                self.sync_state = InternalSyncState::RetrievingAccountRanges {
                    account_state_snapshots_dir,
                    chunk_file_index: 0,
                    block_sync_state,
                    completed_tasks: 0,
                    all_account_hashes: vec![],
                    all_accounts_state: vec![],
                };
                CallResponse::Reply(PeerHandlerCallResponse::InProgress)
            }
            PeerHandlerCallMessage::DownloadStorageRanges {
                storage_accounts,
                account_storages_snapshot_dir: account_storages_snapshots_dir,
                chunk_index,
            } => {
                if !std::fs::exists(&account_storages_snapshots_dir).unwrap() {
                    std::fs::create_dir_all(&account_storages_snapshots_dir).unwrap();
                }

                // 1) split the range in chunks of same length
                let chunk_size = 300;
                let chunk_count =
                    (storage_accounts.accounts_with_storage_root.len() / chunk_size) + 1;

                // list of tasks to be executed
                // Types are (start_index, end_index, starting_hash)
                // NOTE: end_index is NOT inclusive
                let mut pending_tasks = VecDeque::new();
                for i in 0..chunk_count {
                    let chunk_start = chunk_size * i;
                    let chunk_end = (chunk_start + chunk_size)
                        .min(storage_accounts.accounts_with_storage_root.len());
                    pending_tasks.push_back(Task::StorageRanges {
                        start_index: chunk_start,
                        end_index: chunk_end,
                        start_hash: H256::zero(),
                        end_hash: None,
                    });
                }

                let all_account_storages =
                    vec![vec![]; storage_accounts.accounts_with_storage_root.len()];

                let task_count = pending_tasks.len() as u64;
                let completed_tasks = 0;

                // TODO: in a refactor, delete this replace with a structure that can handle removes
                let accounts_done: Vec<H256> = Vec::new();
                let current_account_hashes = storage_accounts
                    .accounts_with_storage_root
                    .iter()
                    .map(|a| *a.0)
                    .collect::<Vec<_>>();

                self.pending_tasks = pending_tasks;
                self.started_tasks = HashMap::new();
                self.sync_state = InternalSyncState::RetrievingStorageRanges {
                    account_storages_snapshots_dir,
                    chunk_file_index: chunk_index,
                    account_storage_roots: storage_accounts,
                    all_account_storages,
                    accounts_done,
                    current_account_hashes,
                    task_count,
                    completed_tasks,
                };
                CallResponse::Reply(PeerHandlerCallResponse::InProgress)
            }
            PeerHandlerCallMessage::DownloadBytecode(bytecode_hashes) => {
                // 1) split the range in chunks of same length
                let chunk_count = 800;
                let chunk_size = bytecode_hashes.len() / chunk_count;

                // list of tasks to be executed
                // NOTE: end_index is NOT inclusive
                let mut tasks_queue_not_started = VecDeque::new();
                for i in 0..chunk_count {
                    let chunk_start = chunk_size * i;
                    let chunk_end = chunk_start + chunk_size;

                    tasks_queue_not_started.push_back(Task::Bytecode {
                        chunk_start,
                        chunk_end,
                    });
                }

                // Modify the last chunk to include the limit
                if let Some(Task::Bytecode {
                    chunk_start,
                    chunk_end: _,
                }) = tasks_queue_not_started.pop_back()
                {
                    tasks_queue_not_started.push_back(Task::Bytecode {
                        chunk_start,
                        chunk_end: bytecode_hashes.len(),
                    });
                }

                let bytecode_hashes_len = bytecode_hashes.len();
                self.started_tasks = HashMap::new();
                self.pending_tasks = tasks_queue_not_started;
                self.sync_state = InternalSyncState::RetrievingBytecode {
                    completed_tasks: 0,
                    all_bytecode_hashes: bytecode_hashes,
                    all_bytecodes: vec![Bytes::new(); bytecode_hashes_len],
                };

                CallResponse::Reply(PeerHandlerCallResponse::InProgress)
            }
            PeerHandlerCallMessage::DownloadBlockBodies(block_hashes) => {
                for _ in 0..REQUEST_RETRY_ATTEMPTS {
                    let available_downloader = loop {
                        self.reset_timed_out_busy_peers().await;
                        match self
                            .get_random_downloader(&SUPPORTED_ETH_CAPABILITIES)
                            .await
                        {
                            Some(downloader) => break downloader,
                            None => {
                                debug!("No available downloader found, retrying...");
                                tokio::time::sleep(Duration::from_secs(10)).await;
                                continue;
                            }
                        }
                    };

                    let peer_id = available_downloader.peer_id();
                    match available_downloader
                        .start()
                        .call(DownloaderCallRequest::BlockBodies {
                            block_hashes: block_hashes.clone(),
                        })
                        .await
                    {
                        Ok(DownloaderCallResponse::BlockBodies(block_bodies)) => {
                            self.record_peer_success(peer_id).await;
                            return CallResponse::Reply(PeerHandlerCallResponse::BlockBodies(
                                block_bodies,
                            ));
                        }
                        _ => {
                            warn!(
                                "[SYNCING] Didn't receive block bodies from peer, penalizing peer {peer_id}..."
                            );
                            self.record_peer_failure(peer_id).await;
                            continue;
                        }
                    }
                }
                CallResponse::Reply(PeerHandlerCallResponse::BlockBodiesNotFound)
            }
        }
    }
}

fn format_duration(duration: Duration) -> String {
    let total_seconds = duration.as_secs();
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;

    format!("{hours:02}h {minutes:02}m {seconds:02}s")
}

#[derive(Debug)]
pub struct DumpError {
    pub path: String,
    pub contents: Vec<u8>,
    pub error: ErrorKind,
}

#[derive(thiserror::Error, Debug)]
pub enum PeerHandlerError {
    #[error("Failed to send message to peer: {0}")]
    SendMessageToPeer(String),
    #[error("Failed to receive block headers")]
    BlockHeaders,
    #[error("Accounts state snapshots dir does not exist")]
    NoStateSnapshotsDir,
    #[error("Failed to create accounts state snapshots dir")]
    CreateStateSnapshotsDir,
    #[error("Failed to write account_state_snapshot chunk {0}")]
    WriteStateSnapshotsDir(u64),
    #[error("Accounts storage snapshots dir does not exist")]
    NoStorageSnapshotsDir,
    #[error("Failed to create accounts storage snapshots dir")]
    CreateStorageSnapshotsDir,
    #[error("Failed to write account_storages_snapshot chunk {0}")]
    WriteStorageSnapshotsDir(u64),
    #[error("Received unexpected response from peer {0}")]
    UnexpectedResponseFromPeer(H256),
    #[error("Failed to receive message from peer {0}")]
    ReceiveMessageFromPeer(H256),
    #[error("Timeout while waiting for message from peer {0}")]
    ReceiveMessageFromPeerTimeout(H256),
    #[error("No peers available")]
    NoPeers,
    #[error("Received invalid headers")]
    InvalidHeaders,
    #[error("Storage Full")]
    StorageFull,
    #[error("No tasks in queue")]
    NoTasks,
    #[error("No account hashes")]
    AccountHashes,
    #[error("No account storages")]
    NoAccountStorages,
    #[error("No storage roots")]
    NoStorageRoots,
    #[error("No response from peer")]
    NoResponseFromPeer,
    #[error("Dumping snapshots to disk failed {0:?}")]
    DumpError(DumpError),
}

#[derive(Debug, Clone, std::hash::Hash)]
pub struct RequestMetadata {
    pub hash: H256,
    pub path: Nibbles,
    /// What node is the parent of this node
    pub parent_path: Nibbles,
}

#[derive(Debug, thiserror::Error)]
pub enum RequestStateTrieNodesError {
    #[error("Send message error")]
    SendMessageError(SendMessageError),
    #[error("Invalid data")]
    InvalidData,
    #[error("Invalid Hash")]
    InvalidHash,
}

#[derive(Debug, thiserror::Error)]
pub enum RequestStorageTrieNodes {
    #[error("Send message error")]
    SendMessageError(u64, SendMessageError),
}
