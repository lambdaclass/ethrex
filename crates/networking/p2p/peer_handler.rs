use std::{
    collections::{HashSet, VecDeque},
    io::ErrorKind,
    sync::{Arc, atomic::Ordering},
    time::{Duration, SystemTime},
};

use bytes::Bytes;
use ethrex_common::{
    BigEndianHash, H256, U256,
    types::{AccountState, BlockBody, BlockHeader, Receipt, validate_block_body},
};
use ethrex_rlp::encode::RLPEncode;
use ethrex_trie::Nibbles;
use ethrex_trie::{Node, verify_range};
use rand::seq::SliceRandom;
use tokio::sync::Mutex;

use super::peer_score::PeerScores;
use crate::{
    kademlia::{Kademlia, PeerChannels, PeerData},
    metrics::METRICS,
    rlpx::{
        connection::server::CastMessage,
        eth::{
            blocks::{BlockBodies, BlockHeaders, GetBlockBodies, GetBlockHeaders, HashOrNumber},
            receipts::GetReceipts,
        },
        message::Message as RLPxMessage,
        p2p::{Capability, SUPPORTED_ETH_CAPABILITIES, SUPPORTED_SNAP_CAPABILITIES},
        snap::{
            AccountRange, AccountRangeUnit, ByteCodes, GetAccountRange, GetByteCodes,
            GetStorageRanges, GetTrieNodes, StorageRanges, TrieNodes,
        },
    },
    snap::encodable_to_proof,
    sync::{AccountStorageRoots, BlockSyncState, block_is_stale, update_pivot},
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

pub const MAX_HEADER_CHUNK: u64 = 500_000;

// How much we store in memory of request_account_range and request_storage_ranges
// before we dump it into the file. This tunes how much memory ethrex uses during
// the first steps of snap sync
pub const RANGE_FILE_CHUNK_SIZE: u64 = 1024 * 1024 * 512; // 512MB
pub const SNAP_LIMIT: usize = 64;

// Request as many as 128 block bodies per request
// this magic number is not part of the protocol and is taken from geth, see:
// https://github.com/ethereum/go-ethereum/blob/2585776aabbd4ae9b00050403b42afb0cee968ec/eth/downloader/downloader.go#L42-L43
//
// Note: We noticed that while bigger values are supported
// increasing them may be the cause of peers disconnection
pub const MAX_BLOCK_BODIES_TO_REQUEST: usize = 128;

/// An abstraction over the [Kademlia] containing logic to make requests to peers
#[derive(Debug, Clone)]
pub struct PeerHandler {
    pub peer_table: Kademlia,
    pub peer_scores: Arc<Mutex<PeerScores>>,
}

pub enum BlockRequestOrder {
    OldToNew,
    NewToOld,
}

#[derive(Clone)]
struct StorageTaskResult {
    start_index: usize,
    account_storages: Vec<Vec<(H256, U256)>>,
    peer_id: H256,
    remaining_start: usize,
    remaining_end: usize,
    remaining_hash_range: (H256, Option<H256>),
}
#[derive(Debug)]
struct StorageTask {
    start_index: usize,
    end_index: usize,
    start_hash: H256,
    // end_hash is None if the task is for the first big storage request
    end_hash: Option<H256>,
}

async fn ask_peer_head_number(
    peer_id: H256,
    peer_channel: &mut PeerChannels,
    sync_head: H256,
    retries: i32,
) -> Result<u64, PeerHandlerError> {
    // TODO: Better error handling
    trace!("Sync Log 11: Requesting sync head block number from peer {peer_id}");
    let request_id = rand::random();
    let request = RLPxMessage::GetBlockHeaders(GetBlockHeaders {
        id: request_id,
        startblock: HashOrNumber::Hash(sync_head),
        limit: 1,
        skip: 0,
        reverse: false,
    });

    peer_channel
        .connection
        .cast(CastMessage::BackendMessage(request.clone()))
        .await
        .map_err(|e| PeerHandlerError::SendMessageToPeer(e.to_string()))?;

    debug!("(Retry {retries}) Requesting sync head {sync_head:?} to peer {peer_id}");

    match tokio::time::timeout(Duration::from_millis(500), async move {
        peer_channel.receiver.lock().await.recv().await
    })
    .await
    {
        Ok(Some(RLPxMessage::BlockHeaders(BlockHeaders { id, block_headers }))) => {
            if id == request_id && !block_headers.is_empty() {
                let sync_head_number = block_headers
                    .last()
                    .ok_or(PeerHandlerError::BlockHeaders)?
                    .number;
                trace!(
                    "Sync Log 12: Received sync head block headers from peer {peer_id}, sync head number {sync_head_number}"
                );
                Ok(sync_head_number)
            } else {
                Err(PeerHandlerError::UnexpectedResponseFromPeer(peer_id))
            }
        }
        Ok(None) => Err(PeerHandlerError::ReceiveMessageFromPeer(peer_id)),
        Ok(_other_msgs) => Err(PeerHandlerError::UnexpectedResponseFromPeer(peer_id)),
        Err(_err) => Err(PeerHandlerError::ReceiveMessageFromPeerTimeout(peer_id)),
    }
}

impl PeerHandler {
    pub fn new(peer_table: Kademlia) -> PeerHandler {
        Self {
            peer_table,
            peer_scores: Default::default(),
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

    /// TODO: docs
    pub async fn get_peer_channel_with_highest_score(
        &self,
        capabilities: &[Capability],
        scores: &mut PeerScores,
    ) -> Option<(H256, PeerChannels)> {
        scores
            .get_peer_channel_with_highest_score(&self.peer_table, capabilities)
            .await
    }

    /// Returns the node id and the channel ends to an active peer connection that supports the given capability
    /// The peer is selected randomly, and doesn't guarantee that the selected peer is not currently busy
    /// If no peer is found, this method will try again after 10 seconds
    async fn get_peer_channel_with_retry(
        &self,
        capabilities: &[Capability],
    ) -> Option<(H256, PeerChannels)> {
        let mut peer_channels = self.peer_table.get_peer_channels(capabilities).await;

        peer_channels.shuffle(&mut rand::rngs::OsRng);

        peer_channels.first().cloned()
    }

    /// Requests block headers from any suitable peer, starting from the `start` block hash towards either older or newer blocks depending on the order
    /// Returns the block headers or None if:
    /// - There are no available peers (the node just started up or was rejected by all other nodes)
    /// - No peer returned a valid response in the given time and retry limits
    pub async fn request_block_headers(
        &self,
        start: u64,
        sync_head: H256,
    ) -> Option<Vec<BlockHeader>> {
        let start_time = SystemTime::now();
        *METRICS.current_step.lock().await = "Downloading Headers".to_string();

        let initial_downloaded_headers = METRICS.downloaded_headers.load(Ordering::Relaxed);

        let mut ret = Vec::<BlockHeader>::new();

        let mut sync_head_number = 0_u64;

        let sync_head_number_retrieval_start = SystemTime::now();

        info!("Retrieving sync head block number from peers");

        let mut retries = 1;

        while sync_head_number == 0 {
            if retries > 10 {
                // sync_head might be invalid
                return None;
            }
            let peers_table = self
                .peer_table
                .get_peer_channels(&SUPPORTED_ETH_CAPABILITIES)
                .await;

            for (peer_id, mut peer_channel) in peers_table {
                match ask_peer_head_number(peer_id, &mut peer_channel, sync_head, retries).await {
                    Ok(number) => {
                        sync_head_number = number;
                        if number != 0 {
                            break;
                        }
                    }
                    Err(err) => {
                        debug!(
                            "Sync Log 13: Failed to retrieve sync head block number from peer {peer_id}: {err}"
                        );
                    }
                }
            }

            retries += 1;
        }
        sync_head_number = sync_head_number.min(start + MAX_HEADER_CHUNK);

        let sync_head_number_retrieval_elapsed = sync_head_number_retrieval_start
            .elapsed()
            .unwrap_or_default();

        info!("Sync head block number retrieved");

        *METRICS.time_to_retrieve_sync_head_block.lock().await =
            Some(sync_head_number_retrieval_elapsed);
        METRICS
            .sync_head_block
            .store(sync_head_number, Ordering::Relaxed);
        METRICS
            .headers_to_download
            .store(sync_head_number + 1, Ordering::Relaxed);
        *METRICS.sync_head_hash.lock().await = sync_head;

        let block_count = sync_head_number + 1 - start;
        let chunk_count = if block_count < 800_u64 { 1 } else { 800_u64 };

        // 2) partition the amount of headers in `K` tasks
        let chunk_limit = block_count / chunk_count;

        // list of tasks to be executed
        let mut tasks_queue_not_started = VecDeque::<(u64, u64)>::new();

        for i in 0..chunk_count {
            tasks_queue_not_started.push_back((i * chunk_limit + start, chunk_limit));
        }

        // Push the reminder
        if block_count % chunk_count != 0 {
            tasks_queue_not_started
                .push_back((chunk_count * chunk_limit + start, block_count % chunk_count));
        }

        let mut downloaded_count = 0_u64;

        // channel to send the tasks to the peers
        let (task_sender, mut task_receiver) =
            tokio::sync::mpsc::channel::<(Vec<BlockHeader>, H256, PeerChannels, u64, u64)>(1000);

        let mut current_show = 0;

        // 3) create tasks that will request a chunk of headers from a peer

        info!("Starting to download block headers from peers");

        *METRICS.headers_download_start_time.lock().await = Some(SystemTime::now());

        let mut last_update = SystemTime::now();

        loop {
            if let Ok((headers, peer_id, _peer_channel, startblock, previous_chunk_limit)) =
                task_receiver.try_recv()
            {
                trace!("We received a download chunk from peer");
                if headers.is_empty() {
                    self.peer_scores.lock().await.free_peer(peer_id);
                    self.peer_scores.lock().await.record_failure(peer_id);

                    debug!("Failed to download chunk from peer. Downloader {peer_id} freed");

                    // reinsert the task to the queue
                    tasks_queue_not_started.push_back((startblock, previous_chunk_limit));

                    continue; // Retry with the next peer
                }

                downloaded_count += headers.len() as u64;

                METRICS
                    .downloaded_headers
                    .fetch_add(headers.len() as u64, Ordering::Relaxed);

                let batch_show = downloaded_count / 10_000;

                if current_show < batch_show {
                    debug!(
                        "Downloaded {} headers from peer {} (current count: {downloaded_count})",
                        headers.len(),
                        peer_id
                    );
                    current_show += 1;
                }
                // store headers!!!!
                ret.extend_from_slice(&headers);

                let downloaded_headers = headers.len() as u64;

                // reinsert the task to the queue if it was not completed
                if downloaded_headers < previous_chunk_limit {
                    let new_start = startblock + headers.len() as u64;

                    let new_chunk_limit = previous_chunk_limit - headers.len() as u64;

                    debug!(
                        "Task for ({startblock}, {new_chunk_limit}) was not completed, re-adding to the queue, {new_chunk_limit} remaining headers"
                    );

                    tasks_queue_not_started.push_back((new_start, new_chunk_limit));
                }

                self.peer_scores.lock().await.record_success(peer_id);
                self.peer_scores.lock().await.free_peer(peer_id);
                debug!("Downloader {peer_id} freed");
            }

            if last_update
                .elapsed()
                .expect("Last update is always in the past")
                >= Duration::from_secs(1)
            {
                debug!("Updating the peer scores table");
                self.peer_scores
                    .lock()
                    .await
                    .update_peers(&self.peer_table)
                    .await;
                last_update = SystemTime::now();
            }
            let Some((peer_id, mut peer_channel)) = self
                .peer_scores
                .lock()
                .await
                .get_peer_channel_with_highest_score(&self.peer_table, &SUPPORTED_SNAP_CAPABILITIES)
                .await
            else {
                trace!("We didn't get a peer from the table");
                continue;
            };

            let Some((startblock, chunk_limit)) = tasks_queue_not_started.pop_front() else {
                if downloaded_count >= block_count {
                    info!("All headers downloaded successfully");
                    break;
                }

                let batch_show = downloaded_count / 10_000;

                if current_show < batch_show {
                    current_show += 1;
                }

                continue;
            };

            let tx = task_sender.clone();
            self.peer_scores.lock().await.mark_in_use(peer_id);

            debug!("Downloader {peer_id} is now busy");

            // run download_chunk_from_peer in a different Tokio task
            tokio::spawn(async move {
                trace!(
                    "Sync Log 5: Requesting block headers from peer {peer_id}, chunk_limit: {chunk_limit}"
                );
                let headers = Self::download_chunk_from_peer(
                    peer_id,
                    &mut peer_channel,
                    startblock,
                    chunk_limit,
                )
                .await
                .inspect_err(|err| trace!("Sync Log 6: {peer_id} failed to download chunk: {err}"))
                .unwrap_or_default();

                tx.send((headers, peer_id, peer_channel, startblock, chunk_limit))
                    .await
                    .inspect_err(|err| {
                        error!("Failed to send headers result through channel. Error: {err}")
                    })
            });
        }

        METRICS.downloaded_headers.store(
            initial_downloaded_headers + downloaded_count,
            Ordering::Relaxed,
        );

        let elapsed = start_time.elapsed().unwrap_or_default();

        debug!(
            "Downloaded {} headers in {} seconds",
            ret.len(),
            format_duration(elapsed)
        );

        {
            let downloaded_headers = ret.len();
            let unique_headers = ret.iter().map(|h| h.hash()).collect::<HashSet<_>>();

            debug!(
                "Downloaded {} headers, unique: {}, duplicates: {}",
                downloaded_headers,
                unique_headers.len(),
                downloaded_headers - unique_headers.len()
            );

            match downloaded_headers.cmp(&unique_headers.len()) {
                std::cmp::Ordering::Equal => {
                    info!("All downloaded headers are unique");
                }
                std::cmp::Ordering::Greater => {
                    warn!(
                        "Downloaded headers contain duplicates, {} duplicates found",
                        downloaded_headers - unique_headers.len()
                    );
                }
                std::cmp::Ordering::Less => {
                    warn!("Downloaded headers are less than unique headers, something went wrong");
                }
            }
        }

        ret.sort_by(|x, y| x.number.cmp(&y.number));
        Some(ret)
    }

    /// given a peer id, a chunk start and a chunk limit, requests the block headers from the peer
    ///
    /// If it fails, returns an error message.
    async fn download_chunk_from_peer(
        peer_id: H256,
        peer_channel: &mut PeerChannels,
        startblock: u64,
        chunk_limit: u64,
    ) -> Result<Vec<BlockHeader>, PeerHandlerError> {
        debug!("Requesting block headers from peer {peer_id}");
        let request_id = rand::random();
        let request = RLPxMessage::GetBlockHeaders(GetBlockHeaders {
            id: request_id,
            startblock: HashOrNumber::Number(startblock),
            limit: chunk_limit,
            skip: 0,
            reverse: false,
        });
        let mut receiver = peer_channel.receiver.lock().await;

        // FIXME! modify the cast and wait for a `call` version
        peer_channel
            .connection
            .cast(CastMessage::BackendMessage(request))
            .await
            .map_err(|e| PeerHandlerError::SendMessageToPeer(e.to_string()))?;

        let block_headers = tokio::time::timeout(Duration::from_secs(2), async move {
            loop {
                match receiver.recv().await {
                    Some(RLPxMessage::BlockHeaders(BlockHeaders { id, block_headers }))
                        if id == request_id =>
                    {
                        return Some(block_headers);
                    }
                    // Ignore replies that don't match the expected id (such as late responses)
                    Some(_) => continue,
                    None => return None, // EOF
                }
            }
        })
        .await
        .map_err(|_| PeerHandlerError::BlockHeaders)?
        .ok_or(PeerHandlerError::BlockHeaders)?;

        if are_block_headers_chained(&block_headers, &BlockRequestOrder::OldToNew) {
            Ok(block_headers)
        } else {
            warn!("[SYNCING] Received invalid headers from peer: {peer_id}");
            Err(PeerHandlerError::InvalidHeaders)
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
        let block_hashes_len = block_hashes.len();
        let request_id = rand::random();
        let request = RLPxMessage::GetBlockBodies(GetBlockBodies {
            id: request_id,
            block_hashes: block_hashes.clone(),
        });
        let (peer_id, mut peer_channel) = self
            .get_peer_channel_with_retry(&SUPPORTED_ETH_CAPABILITIES)
            .await?;
        let mut receiver = peer_channel.receiver.lock().await;
        if let Err(err) = peer_channel
            .connection
            .cast(CastMessage::BackendMessage(request))
            .await
        {
            self.record_peer_failure(peer_id).await;
            debug!("Failed to send message to peer: {err:?}");
            return None;
        }
        if let Some(block_bodies) = tokio::time::timeout(Duration::from_secs(2), async move {
            loop {
                match receiver.recv().await {
                    Some(RLPxMessage::BlockBodies(BlockBodies { id, block_bodies }))
                        if id == request_id =>
                    {
                        return Some(block_bodies);
                    }
                    // Ignore replies that don't match the expected id (such as late responses)
                    Some(_) => continue,
                    None => return None,
                }
            }
        })
        .await
        .ok()
        .flatten()
        .and_then(|bodies| {
            // Check that the response is not empty and does not contain more bodies than the ones requested
            (!bodies.is_empty() && bodies.len() <= block_hashes_len).then_some(bodies)
        }) {
            self.record_peer_success(peer_id).await;
            return Some((block_bodies, peer_id));
        }

        warn!("[SYNCING] Didn't receive block bodies from peer, penalizing peer {peer_id}...");
        self.record_peer_failure(peer_id).await;
        None
    }

    /// Requests block bodies from any suitable peer given their block hashes
    /// Returns the block bodies or None if:
    /// - There are no available peers (the node just started up or was rejected by all other nodes)
    /// - No peer returned a valid response in the given time and retry limits
    pub async fn request_block_bodies(&self, block_hashes: Vec<H256>) -> Option<Vec<BlockBody>> {
        for _ in 0..REQUEST_RETRY_ATTEMPTS {
            if let Some((block_bodies, _)) =
                self.request_block_bodies_inner(block_hashes.clone()).await
            {
                return Some(block_bodies);
            }
        }
        None
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
        let block_hashes_len = block_hashes.len();
        for _ in 0..REQUEST_RETRY_ATTEMPTS {
            let request_id = rand::random();
            let request = RLPxMessage::GetReceipts(GetReceipts {
                id: request_id,
                block_hashes: block_hashes.clone(),
            });
            let (_, mut peer_channel) = self
                .get_peer_channel_with_retry(&SUPPORTED_ETH_CAPABILITIES)
                .await?;
            let mut receiver = peer_channel.receiver.lock().await;
            if let Err(err) = peer_channel
                .connection
                .cast(CastMessage::BackendMessage(request))
                .await
            {
                debug!("Failed to send message to peer: {err:?}");
                continue;
            }
            if let Some(receipts) = tokio::time::timeout(PEER_REPLY_TIMEOUT, async move {
                loop {
                    match receiver.recv().await {
                        Some(RLPxMessage::Receipts(receipts)) => {
                            if receipts.get_id() == request_id {
                                return Some(receipts.get_receipts());
                            }
                            return None;
                        }
                        // Ignore replies that don't match the expected id (such as late responses)
                        Some(_) => continue,
                        None => return None,
                    }
                }
            })
            .await
            .ok()
            .flatten()
            .and_then(|receipts|
                // Check that the response is not empty and does not contain more bodies than the ones requested
                (!receipts.is_empty() && receipts.len() <= block_hashes_len).then_some(receipts))
            {
                return Some(receipts);
            }
        }
        None
    }

    /// Requests an account range from any suitable peer given the state trie's root and the starting hash and the limit hash.
    /// Will also return a boolean indicating if there is more state to be fetched towards the right of the trie
    /// (Note that the boolean will be true even if the remaining state is ouside the boundary set by the limit hash)
    ///
    /// # Returns
    ///
    /// The account range or `None` if:
    ///
    /// - There are no available peers (the node just started up or was rejected by all other nodes)
    /// - No peer returned a valid response in the given time and retry limits
    pub async fn request_account_range(
        &self,
        start: H256,
        limit: H256,
        account_state_snapshots_dir: String,
        pivot_header: &mut BlockHeader,
        block_sync_state: &mut BlockSyncState,
    ) -> Result<(), PeerHandlerError> {
        *METRICS.current_step.lock().await = "Requesting Account Ranges".to_string();
        // 1) split the range in chunks of same length
        let start_u256 = U256::from_big_endian(&start.0);
        let limit_u256 = U256::from_big_endian(&limit.0);

        let chunk_count = 800;
        let chunk_size = (limit_u256 - start_u256) / chunk_count;

        // list of tasks to be executed
        let mut tasks_queue_not_started = VecDeque::<(H256, H256)>::new();
        for i in 0..(chunk_count as u64) {
            let chunk_start_u256 = chunk_size * i + start_u256;
            // We subtract one because ranges are inclusive
            let chunk_end_u256 = chunk_start_u256 + chunk_size - 1u64;
            let chunk_start = H256::from_uint(&(chunk_start_u256));
            let chunk_end = H256::from_uint(&(chunk_end_u256));
            tasks_queue_not_started.push_back((chunk_start, chunk_end));
        }
        // Modify the last chunk to include the limit
        let last_task = tasks_queue_not_started
            .back_mut()
            .ok_or(PeerHandlerError::NoTasks)?;
        last_task.1 = limit;

        // 2) request the chunks from peers

        let mut downloaded_count = 0_u64;
        let mut all_account_hashes = Vec::new();
        let mut all_accounts_state = Vec::new();

        // channel to send the tasks to the peers
        let (task_sender, mut task_receiver) =
            tokio::sync::mpsc::channel::<(Vec<AccountRangeUnit>, H256, Option<(H256, H256)>)>(1000);

        // channel to send the result of dumping accounts
        let (dump_account_result_sender, mut dump_account_result_receiver) =
            tokio::sync::mpsc::channel::<Result<(), DumpError>>(1000);

        info!("Starting to download account ranges from peers");

        *METRICS.account_tries_download_start_time.lock().await = Some(SystemTime::now());

        let mut completed_tasks = 0;
        let mut chunk_file = 0;
        let mut last_update: SystemTime = SystemTime::now();
        let mut we_are_missing_peers = false;

        loop {
            if all_accounts_state.len() * size_of::<AccountState>()
                >= RANGE_FILE_CHUNK_SIZE as usize
            {
                let current_account_hashes = std::mem::take(&mut all_account_hashes);
                let current_account_states = std::mem::take(&mut all_accounts_state);

                let account_state_chunk = current_account_hashes
                    .into_iter()
                    .zip(current_account_states)
                    .collect::<Vec<(H256, AccountState)>>()
                    .encode_to_vec();

                if !std::fs::exists(&account_state_snapshots_dir)
                    .map_err(|_| PeerHandlerError::NoStateSnapshotsDir)?
                {
                    std::fs::create_dir_all(&account_state_snapshots_dir)
                        .map_err(|_| PeerHandlerError::CreateStateSnapshotsDir)?;
                }

                let account_state_snapshots_dir_cloned = account_state_snapshots_dir.clone();
                let dump_account_result_sender_cloned = dump_account_result_sender.clone();
                tokio::task::spawn(async move {
                    let path = get_account_state_snapshot_file(
                        account_state_snapshots_dir_cloned,
                        chunk_file,
                    );
                    // TODO: check the error type and handle it properly
                    let result = dump_to_file(path, account_state_chunk);
                    dump_account_result_sender_cloned
                        .send(result)
                        .await
                        .inspect_err(|err| {
                            error!(
                                "Failed to send account dump result through channel. Error: {err}"
                            )
                        })
                });

                chunk_file += 1;
            }

            if last_update
                .elapsed()
                .expect("Time shouldn't be in the past")
                >= Duration::from_secs(1)
            {
                self.peer_scores
                    .lock()
                    .await
                    .update_peers(&self.peer_table)
                    .await;
                METRICS
                    .downloaded_account_tries
                    .store(downloaded_count, Ordering::Relaxed);
                last_update = SystemTime::now();
                if we_are_missing_peers {
                    debug!("We are missing peers to go full speed in request_account_range");
                }
                we_are_missing_peers = false;
            }

            if let Ok((accounts, peer_id, chunk_start_end)) = task_receiver.try_recv() {
                self.peer_scores.lock().await.free_peer(peer_id);

                if let Some((chunk_start, chunk_end)) = chunk_start_end {
                    if chunk_start <= chunk_end {
                        tasks_queue_not_started.push_back((chunk_start, chunk_end));
                    } else {
                        completed_tasks += 1;
                    }
                }
                if chunk_start_end.is_none() {
                    completed_tasks += 1;
                }
                if accounts.is_empty() {
                    self.peer_scores.lock().await.record_failure(peer_id);
                    continue;
                }
                self.peer_scores.lock().await.record_success(peer_id);

                downloaded_count += accounts.len() as u64;

                debug!(
                    "Downloaded {} accounts from peer {} (current count: {downloaded_count})",
                    accounts.len(),
                    peer_id
                );
                all_account_hashes.extend(accounts.iter().map(|unit| unit.hash));
                all_accounts_state.extend(
                    accounts
                        .iter()
                        .map(|unit| AccountState::from(unit.account.clone())),
                );
            }

            // Check if any dump account task finished
            // TODO: consider tracking in-flight (dump) tasks
            if let Ok(Err(dump_account_data)) = dump_account_result_receiver.try_recv() {
                if dump_account_data.error == ErrorKind::StorageFull {
                    return Err(PeerHandlerError::StorageFull);
                }
                // If the dumping failed, retry it
                let dump_account_result_sender_cloned = dump_account_result_sender.clone();
                tokio::task::spawn(async move {
                    let DumpError { path, contents, .. } = dump_account_data;
                    // Dump the account data
                    let result = dump_to_file(path, contents);
                    // Send the result through the channel
                    dump_account_result_sender_cloned
                        .send(result)
                        .await
                        .inspect_err(|err| {
                            error!(
                                "Failed to send account dump result through channel. Error: {err}"
                            )
                        })
                });
            }

            let Some((peer_id, peer_channel)) = self
                .peer_scores
                .lock()
                .await
                .get_peer_channel_with_highest_score(&self.peer_table, &SUPPORTED_SNAP_CAPABILITIES)
                .await
            else {
                trace!("We are missing peers in request_account_range_request");
                we_are_missing_peers = true;
                continue;
            };

            let Some((chunk_start, chunk_end)) = tasks_queue_not_started.pop_front() else {
                if completed_tasks >= chunk_count {
                    info!("All account ranges downloaded successfully");
                    break;
                }
                continue;
            };

            self.peer_scores.lock().await.mark_in_use(peer_id);
            let tx = task_sender.clone();

            if block_is_stale(pivot_header) {
                info!("request_account_range became stale, updating pivot");
                *pivot_header = update_pivot(
                    pivot_header.number,
                    pivot_header.timestamp,
                    self,
                    block_sync_state,
                )
                .await
                .expect("Should be able to update pivot")
            }

            tokio::spawn(PeerHandler::request_account_range_worker(
                peer_id,
                chunk_start,
                chunk_end,
                pivot_header.state_root,
                peer_channel,
                tx,
            ));
        }

        // TODO: This is repeated code, consider refactoring
        {
            let current_account_hashes = std::mem::take(&mut all_account_hashes);
            let current_account_states = std::mem::take(&mut all_accounts_state);

            let account_state_chunk = current_account_hashes
                .into_iter()
                .zip(current_account_states)
                .collect::<Vec<(H256, AccountState)>>()
                .encode_to_vec();

            if !std::fs::exists(&account_state_snapshots_dir)
                .map_err(|_| PeerHandlerError::NoStateSnapshotsDir)?
            {
                std::fs::create_dir_all(&account_state_snapshots_dir)
                    .map_err(|_| PeerHandlerError::CreateStateSnapshotsDir)?;
            }

            let path = get_account_state_snapshot_file(account_state_snapshots_dir, chunk_file);
            std::fs::write(path, account_state_chunk)
                .map_err(|_| PeerHandlerError::WriteStateSnapshotsDir(chunk_file))?;
        }

        METRICS
            .downloaded_account_tries
            .store(downloaded_count, Ordering::Relaxed);
        *METRICS.account_tries_download_end_time.lock().await = Some(SystemTime::now());

        Ok(())
    }

    #[allow(clippy::type_complexity)]
    async fn request_account_range_worker(
        free_peer_id: H256,
        chunk_start: H256,
        chunk_end: H256,
        state_root: H256,
        mut free_downloader_channels_clone: PeerChannels,
        tx: tokio::sync::mpsc::Sender<(Vec<AccountRangeUnit>, H256, Option<(H256, H256)>)>,
    ) -> Result<(), PeerHandlerError> {
        debug!(
            "Requesting account range from peer {free_peer_id}, chunk: {chunk_start:?} - {chunk_end:?}"
        );
        let request_id = rand::random();
        let request = RLPxMessage::GetAccountRange(GetAccountRange {
            id: request_id,
            root_hash: state_root,
            starting_hash: chunk_start,
            limit_hash: chunk_end,
            response_bytes: MAX_RESPONSE_BYTES,
        });
        let mut receiver = free_downloader_channels_clone.receiver.lock().await;
        if let Err(err) = (free_downloader_channels_clone.connection)
            .cast(CastMessage::BackendMessage(request))
            .await
        {
            error!("Failed to send message to peer: {err:?}");
            tx.send((Vec::new(), free_peer_id, Some((chunk_start, chunk_end))))
                .await
                .ok();
            return Ok(());
        }
        if let Some((accounts, proof)) = tokio::time::timeout(Duration::from_secs(2), async move {
            loop {
                if let RLPxMessage::AccountRange(AccountRange {
                    id,
                    accounts,
                    proof,
                }) = receiver.recv().await?
                {
                    if id == request_id {
                        return Some((accounts, proof));
                    }
                }
            }
        })
        .await
        .ok()
        .flatten()
        {
            if accounts.is_empty() {
                tx.send((Vec::new(), free_peer_id, Some((chunk_start, chunk_end))))
                    .await
                    .ok();
                return Ok(());
            }
            // Unzip & validate response
            let proof = encodable_to_proof(&proof);
            let (account_hashes, account_states): (Vec<_>, Vec<_>) = accounts
                .clone()
                .into_iter()
                .map(|unit| (unit.hash, AccountState::from(unit.account)))
                .unzip();
            let encoded_accounts = account_states
                .iter()
                .map(|acc| acc.encode_to_vec())
                .collect::<Vec<_>>();

            let Ok(should_continue) = verify_range(
                state_root,
                &chunk_start,
                &account_hashes,
                &encoded_accounts,
                &proof,
            ) else {
                tx.send((Vec::new(), free_peer_id, Some((chunk_start, chunk_end))))
                    .await
                    .ok();
                tracing::error!("Received invalid account range");
                return Ok(());
            };

            // If the range has more accounts to fetch, we send the new chunk
            let chunk_left = if should_continue {
                let last_hash = match account_hashes.last() {
                    Some(last_hash) => last_hash,
                    None => {
                        tx.send((Vec::new(), free_peer_id, Some((chunk_start, chunk_end))))
                            .await
                            .ok();
                        error!("Account hashes last failed, this shouldn't happen");
                        return Err(PeerHandlerError::AccountHashes);
                    }
                };
                let new_start_u256 = U256::from_big_endian(&last_hash.0) + 1;
                let new_start = H256::from_uint(&new_start_u256);
                Some((new_start, chunk_end))
            } else {
                None
            };
            tx.send((
                accounts
                    .into_iter()
                    .filter(|unit| unit.hash <= chunk_end)
                    .collect(),
                free_peer_id,
                chunk_left,
            ))
            .await
            .ok();
        } else {
            tracing::debug!("Failed to get account range");
            tx.send((Vec::new(), free_peer_id, Some((chunk_start, chunk_end))))
                .await
                .ok();
        }
        Ok::<(), PeerHandlerError>(())
    }

    /// Requests bytecodes for the given code hashes
    /// Returns the bytecodes or None if:
    /// - There are no available peers (the node just started up or was rejected by all other nodes)
    /// - No peer returned a valid response in the given time and retry limits
    pub async fn request_bytecodes(
        &self,
        all_bytecode_hashes: &[H256],
    ) -> Result<Option<Vec<Bytes>>, PeerHandlerError> {
        *METRICS.current_step.lock().await = "Requesting Bytecodes".to_string();
        const MAX_BYTECODES_REQUEST_SIZE: usize = 100;
        // 1) split the range in chunks of same length
        let chunk_count = 800;
        let chunk_size = all_bytecode_hashes.len() / chunk_count;

        // list of tasks to be executed
        // Types are (start_index, end_index, starting_hash)
        // NOTE: end_index is NOT inclusive
        let mut tasks_queue_not_started = VecDeque::<(usize, usize)>::new();
        for i in 0..chunk_count {
            let chunk_start = chunk_size * i;
            let chunk_end = chunk_start + chunk_size;
            tasks_queue_not_started.push_back((chunk_start, chunk_end));
        }
        // Modify the last chunk to include the limit
        let last_task = tasks_queue_not_started
            .back_mut()
            .ok_or(PeerHandlerError::NoTasks)?;
        last_task.1 = all_bytecode_hashes.len();

        // 2) request the chunks from peers
        let mut downloaded_count = 0_u64;
        let mut all_bytecodes = vec![Bytes::new(); all_bytecode_hashes.len()];

        // channel to send the tasks to the peers
        struct TaskResult {
            start_index: usize,
            bytecodes: Vec<Bytes>,
            peer_id: H256,
            remaining_start: usize,
            remaining_end: usize,
        }
        let (task_sender, mut task_receiver) = tokio::sync::mpsc::channel::<TaskResult>(1000);

        info!("Starting to download bytecodes from peers");

        METRICS
            .bytecodes_to_download
            .fetch_add(all_bytecode_hashes.len() as u64, Ordering::Relaxed);

        let mut completed_tasks = 0;
        let mut last_update = SystemTime::now();

        loop {
            if let Ok(result) = task_receiver.try_recv() {
                let TaskResult {
                    start_index,
                    bytecodes,
                    peer_id,
                    remaining_start,
                    remaining_end,
                } = result;
                self.peer_scores.lock().await.free_peer(peer_id);

                if remaining_start < remaining_end {
                    tasks_queue_not_started.push_back((remaining_start, remaining_end));
                } else {
                    completed_tasks += 1;
                }
                if bytecodes.is_empty() {
                    self.peer_scores.lock().await.record_failure(peer_id);
                    continue;
                }

                downloaded_count += bytecodes.len() as u64;

                self.peer_scores.lock().await.record_success(peer_id);

                debug!(
                    "Downloaded {} bytecodes from peer {peer_id} (current count: {downloaded_count})",
                    bytecodes.len(),
                );
                for (i, bytecode) in bytecodes.into_iter().enumerate() {
                    all_bytecodes[start_index + i] = bytecode;
                }
            }

            if last_update
                .elapsed()
                .expect("Should never be in the future")
                >= Duration::from_secs(1)
            {
                self.peer_scores
                    .lock()
                    .await
                    .update_peers(&self.peer_table)
                    .await;
                last_update = SystemTime::now();
            };

            let Some((peer_id, mut peer_channel)) = self
                .peer_scores
                .lock()
                .await
                .get_peer_channel_with_highest_score(&self.peer_table, &SUPPORTED_SNAP_CAPABILITIES)
                .await
            else {
                continue;
            };
            self.peer_scores.lock().await.mark_in_use(peer_id);

            let Some((chunk_start, chunk_end)) = tasks_queue_not_started.pop_front() else {
                if completed_tasks >= chunk_count {
                    info!("All bytecodes downloaded successfully");
                    break;
                }
                continue;
            };

            let tx = task_sender.clone();

            let hashes_to_request: Vec<_> = all_bytecode_hashes
                .iter()
                .skip(chunk_start)
                .take((chunk_end - chunk_start).min(MAX_BYTECODES_REQUEST_SIZE))
                .copied()
                .collect();

            tokio::spawn(async move {
                let empty_task_result = TaskResult {
                    start_index: chunk_start,
                    bytecodes: vec![],
                    peer_id,
                    remaining_start: chunk_start,
                    remaining_end: chunk_end,
                };
                debug!(
                    "Requesting bytecode from peer {peer_id}, chunk: {chunk_start:?} - {chunk_end:?}"
                );
                let request_id = rand::random();
                let request = RLPxMessage::GetByteCodes(GetByteCodes {
                    id: request_id,
                    hashes: hashes_to_request.clone(),
                    bytes: MAX_RESPONSE_BYTES,
                });
                let mut receiver = peer_channel.receiver.lock().await;
                if let Err(err) = (peer_channel.connection)
                    .cast(CastMessage::BackendMessage(request))
                    .await
                {
                    error!("Failed to send message to peer: {err:?}");
                    tx.send(empty_task_result).await.ok();
                    return;
                }
                if let Some(codes) = tokio::time::timeout(Duration::from_secs(2), async move {
                    loop {
                        match receiver.recv().await {
                            Some(RLPxMessage::ByteCodes(ByteCodes { id, codes }))
                                if id == request_id =>
                            {
                                return Some(codes);
                            }
                            Some(_) => continue,
                            None => return None,
                        }
                    }
                })
                .await
                .ok()
                .flatten()
                {
                    if codes.is_empty() {
                        tx.send(empty_task_result).await.ok();
                        // Too spammy
                        // tracing::error!("Received empty account range");
                        return;
                    }
                    // Validate response by hashing bytecodes
                    let validated_codes: Vec<Bytes> = codes
                        .into_iter()
                        .zip(hashes_to_request)
                        .take_while(|(b, hash)| keccak_hash::keccak(b) == *hash)
                        .map(|(b, _hash)| b)
                        .collect();
                    let result = TaskResult {
                        start_index: chunk_start,
                        remaining_start: chunk_start + validated_codes.len(),
                        bytecodes: validated_codes,
                        peer_id,
                        remaining_end: chunk_end,
                    };
                    tx.send(result).await.ok();
                } else {
                    tracing::debug!("Failed to get bytecode");
                    tx.send(empty_task_result).await.ok();
                }
            });
        }

        METRICS
            .downloaded_bytecodes
            .fetch_add(downloaded_count, Ordering::Relaxed);
        info!(
            "Finished downloading bytecodes, total bytecodes: {}",
            all_bytecode_hashes.len()
        );

        Ok(Some(all_bytecodes))
    }

    /// Requests storage ranges for accounts given their hashed address and storage roots, and the root of their state trie
    /// account_hashes & storage_roots must have the same length
    /// storage_roots must not contain empty trie hashes, we will treat empty ranges as invalid responses
    /// Returns true if the last account's storage was not completely fetched by the request
    /// Returns the list of hashed storage keys and values for each account's storage or None if:
    /// - There are no available peers (the node just started up or was rejected by all other nodes)
    /// - No peer returned a valid response in the given time and retry limits
    pub async fn request_storage_ranges(
        &self,
        account_storage_roots: &mut AccountStorageRoots,
        account_storages_snapshots_dir: String,
        mut chunk_index: u64,
        pivot_header: &mut BlockHeader,
    ) -> Result<u64, PeerHandlerError> {
        *METRICS.current_step.lock().await = "Requesting Storage Ranges".to_string();
        debug!("Starting request_storage_ranges function");
        // 1) split the range in chunks of same length
        let chunk_size = 300;
        let chunk_count = (account_storage_roots.accounts_with_storage_root.len() / chunk_size) + 1;

        // list of tasks to be executed
        // Types are (start_index, end_index, starting_hash)
        // NOTE: end_index is NOT inclusive
        let mut tasks_queue_not_started = VecDeque::<StorageTask>::new();
        for i in 0..chunk_count {
            let chunk_start = chunk_size * i;
            let chunk_end = (chunk_start + chunk_size)
                .min(account_storage_roots.accounts_with_storage_root.len());
            tasks_queue_not_started.push_back(StorageTask {
                start_index: chunk_start,
                end_index: chunk_end,
                start_hash: H256::zero(),
                end_hash: None,
            });
        }

        // 2) request the chunks from peers
        let mut all_account_storages =
            vec![vec![]; account_storage_roots.accounts_with_storage_root.len()];

        // channel to send the tasks to the peers
        let (task_sender, mut task_receiver) =
            tokio::sync::mpsc::channel::<StorageTaskResult>(1000);

        // channel to send the result of dumping storages
        let mut disk_joinset: tokio::task::JoinSet<Result<(), DumpError>> =
            tokio::task::JoinSet::new();

        let mut task_count = tasks_queue_not_started.len();
        let mut completed_tasks = 0;

        // TODO: in a refactor, delete this replace with a structure that can handle removes
        let mut accounts_done: Vec<H256> = Vec::new();
        let current_account_hashes = account_storage_roots
            .accounts_with_storage_root
            .iter()
            .map(|a| *a.0)
            .collect::<Vec<_>>();

        let mut last_update = SystemTime::now();
        debug!("Starting request_storage_ranges loop");
        loop {
            if all_account_storages.iter().map(Vec::len).sum::<usize>() * 64
                > RANGE_FILE_CHUNK_SIZE as usize
            {
                let current_account_storages = std::mem::take(&mut all_account_storages);
                all_account_storages =
                    vec![vec![]; account_storage_roots.accounts_with_storage_root.len()];

                let snapshot = current_account_hashes
                    .clone()
                    .into_iter()
                    .zip(current_account_storages)
                    .collect::<Vec<_>>()
                    .encode_to_vec();

                if !std::fs::exists(&account_storages_snapshots_dir)
                    .map_err(|_| PeerHandlerError::NoStorageSnapshotsDir)?
                {
                    std::fs::create_dir_all(&account_storages_snapshots_dir)
                        .map_err(|_| PeerHandlerError::CreateStorageSnapshotsDir)?;
                }
                let account_storages_snapshots_dir_cloned = account_storages_snapshots_dir.clone();
                if !disk_joinset.is_empty() {
                    debug!("Writing to disk");
                    disk_joinset
                        .join_next()
                        .await
                        .expect("Shouldn't be empty")
                        .expect("Shouldn't have a join error")
                        .inspect_err(|err| {
                            error!("We found this error while dumping to file {err:?}")
                        })
                        .map_err(PeerHandlerError::DumpError)?;
                }
                disk_joinset.spawn(async move {
                    let path = get_account_storages_snapshot_file(
                        account_storages_snapshots_dir_cloned,
                        chunk_index,
                    );
                    dump_to_file(path, snapshot)
                });

                chunk_index += 1;
            }

            if last_update
                .elapsed()
                .expect("Last update shouldn't be in the past")
                > Duration::from_secs(2)
            {
                debug!("Updating peer scores");
                self.peer_scores
                    .lock()
                    .await
                    .update_peers(&self.peer_table)
                    .await;
                last_update = SystemTime::now();
            }

            if let Ok(result) = task_receiver.try_recv() {
                let StorageTaskResult {
                    start_index,
                    mut account_storages,
                    peer_id,
                    remaining_start,
                    remaining_end,
                    remaining_hash_range: (hash_start, hash_end),
                } = result;
                completed_tasks += 1;

                self.peer_scores.lock().await.free_peer(peer_id);

                for account in &current_account_hashes[start_index..remaining_start] {
                    accounts_done.push(*account);
                }

                if remaining_start < remaining_end {
                    debug!("Failed to download entire chunk from peer {peer_id}");
                    if hash_start.is_zero() {
                        // Task is common storage range request
                        let task = StorageTask {
                            start_index: remaining_start,
                            end_index: remaining_end,
                            start_hash: H256::zero(),
                            end_hash: None,
                        };
                        tasks_queue_not_started.push_back(task);
                        task_count += 1;
                    } else if let Some(hash_end) = hash_end {
                        // Task was a big storage account result
                        if hash_start <= hash_end {
                            let task = StorageTask {
                                start_index: remaining_start,
                                end_index: remaining_end,
                                start_hash: hash_start,
                                end_hash: Some(hash_end),
                            };
                            tasks_queue_not_started.push_back(task);
                            task_count += 1;
                            accounts_done.push(current_account_hashes[remaining_start]);
                            account_storage_roots
                                .healed_accounts
                                .insert(current_account_hashes[start_index]);
                        }
                    } else {
                        if remaining_start + 1 < remaining_end {
                            let task = StorageTask {
                                start_index: remaining_start + 1,
                                end_index: remaining_end,
                                start_hash: H256::zero(),
                                end_hash: None,
                            };
                            tasks_queue_not_started.push_back(task);
                            task_count += 1;
                        }
                        // Task found a big storage account, so we split the chunk into multiple chunks
                        let start_hash_u256 = U256::from_big_endian(&hash_start.0);
                        let missing_storage_range = U256::MAX - start_hash_u256;

                        let slot_count = account_storages
                            .last()
                            .map(|v| v.len())
                            .ok_or(PeerHandlerError::NoAccountStorages)?
                            .max(1);
                        let storage_density = start_hash_u256 / slot_count;

                        let slots_per_chunk = U256::from(10000);
                        let chunk_size = storage_density
                            .checked_mul(slots_per_chunk)
                            .unwrap_or(U256::MAX);

                        let chunk_count = (missing_storage_range / chunk_size).as_usize().max(1);

                        for i in 0..chunk_count {
                            let start_hash_u256 = start_hash_u256 + chunk_size * i;
                            let start_hash = H256::from_uint(&start_hash_u256);
                            let end_hash = if i == chunk_count - 1 {
                                H256::repeat_byte(0xff)
                            } else {
                                let end_hash_u256 =
                                    start_hash_u256.checked_add(chunk_size).unwrap_or(U256::MAX);
                                H256::from_uint(&end_hash_u256)
                            };

                            let task = StorageTask {
                                start_index: remaining_start,
                                end_index: remaining_start + 1,
                                start_hash,
                                end_hash: Some(end_hash),
                            };
                            tasks_queue_not_started.push_back(task);
                            task_count += 1;
                        }
                        debug!("Split big storage account into {chunk_count} chunks.");
                    }
                }

                if account_storages.is_empty() {
                    self.peer_scores.lock().await.record_failure(peer_id);
                    continue;
                }
                if let Some(hash_end) = hash_end {
                    // This is a big storage account, and the range might be empty
                    if account_storages[0].len() == 1 && account_storages[0][0].0 > hash_end {
                        continue;
                    }
                }

                self.peer_scores.lock().await.record_success(peer_id);

                let n_storages = account_storages.len();
                let n_slots = account_storages
                    .iter()
                    .map(|storage| storage.len())
                    .sum::<usize>();

                METRICS
                    .downloaded_storage_slots
                    .fetch_add(n_slots as u64, Ordering::Relaxed);

                debug!("Downloaded {n_storages} storages ({n_slots} slots) from peer {peer_id}");
                debug!(
                    "Total tasks: {task_count}, completed tasks: {completed_tasks}, queued tasks: {}",
                    tasks_queue_not_started.len()
                );
                if account_storages.len() == 1 {
                    // We downloaded a big storage account
                    all_account_storages[start_index].extend(account_storages.remove(0));
                } else {
                    for (i, storage) in account_storages.into_iter().enumerate() {
                        all_account_storages[start_index + i] = storage;
                    }
                }
            }

            if block_is_stale(pivot_header) {
                info!("request_storage_ranges became stale, breaking");
                break;
            }

            let Some(task) = tasks_queue_not_started.pop_front() else {
                if completed_tasks >= task_count {
                    break;
                }
                continue;
            };

            let Some((peer_id, peer_channel)) = self
                .peer_scores
                .lock()
                .await
                .get_peer_channel_with_highest_score(&self.peer_table, &SUPPORTED_SNAP_CAPABILITIES)
                .await
            else {
                continue;
            };

            self.peer_scores.lock().await.mark_in_use(peer_id);

            let tx = task_sender.clone();

            let (chunk_account_hashes, chunk_storage_roots): (Vec<_>, Vec<_>) =
                account_storage_roots
                    .accounts_with_storage_root
                    .iter()
                    .skip(task.start_index)
                    .take(task.end_index - task.start_index)
                    .map(|(hash, root)| (*hash, *root))
                    .unzip();

            if task_count - completed_tasks < 30 {
                debug!(
                    "Assigning task: {task:?}, account_hash: {}, storage_root: {}",
                    chunk_account_hashes.first().unwrap_or(&H256::zero()),
                    chunk_storage_roots.first().unwrap_or(&H256::zero()),
                );
            }

            tokio::spawn(PeerHandler::request_storage_ranges_worker(
                task,
                peer_id,
                pivot_header.state_root,
                peer_channel,
                chunk_account_hashes,
                chunk_storage_roots,
                tx,
            ));
        }

        {
            let current_account_hashes = account_storage_roots
                .accounts_with_storage_root
                .iter()
                .map(|a| *a.0)
                .collect::<Vec<_>>();
            let current_account_storages = std::mem::take(&mut all_account_storages);

            let snapshot = current_account_hashes
                .into_iter()
                .zip(current_account_storages)
                .collect::<Vec<_>>()
                .encode_to_vec();

            if !std::fs::exists(&account_storages_snapshots_dir)
                .map_err(|_| PeerHandlerError::NoStorageSnapshotsDir)?
            {
                std::fs::create_dir_all(&account_storages_snapshots_dir)
                    .map_err(|_| PeerHandlerError::CreateStorageSnapshotsDir)?;
            }
            let account_storages_snapshots_dir_cloned = account_storages_snapshots_dir.clone();
            let path = get_account_storages_snapshot_file(
                account_storages_snapshots_dir_cloned,
                chunk_index,
            );
            std::fs::write(path, snapshot)
                .map_err(|_| PeerHandlerError::WriteStorageSnapshotsDir(chunk_index))?;
        }
        disk_joinset
            .join_all()
            .await
            .into_iter()
            .map(|result| {
                result
                    .inspect_err(|err| error!("We found this error while dumping to file {err:?}"))
            })
            .collect::<Result<Vec<()>, DumpError>>()
            .map_err(PeerHandlerError::DumpError)?;

        for account_done in accounts_done {
            account_storage_roots
                .accounts_with_storage_root
                .remove(&account_done);
        }

        Ok(chunk_index + 1)
    }

    async fn request_storage_ranges_worker(
        task: StorageTask,
        free_peer_id: H256,
        state_root: H256,
        mut free_downloader_channels_clone: PeerChannels,
        chunk_account_hashes: Vec<H256>,
        chunk_storage_roots: Vec<H256>,
        tx: tokio::sync::mpsc::Sender<StorageTaskResult>,
    ) -> Result<(), PeerHandlerError> {
        let start = task.start_index;
        let end = task.end_index;
        let start_hash = task.start_hash;

        let empty_task_result = StorageTaskResult {
            start_index: task.start_index,
            account_storages: Vec::new(),
            peer_id: free_peer_id,
            remaining_start: task.start_index,
            remaining_end: task.end_index,
            remaining_hash_range: (start_hash, task.end_hash),
        };
        let request_id = rand::random();
        let request = RLPxMessage::GetStorageRanges(GetStorageRanges {
            id: request_id,
            root_hash: state_root,
            account_hashes: chunk_account_hashes,
            starting_hash: start_hash,
            limit_hash: task.end_hash.unwrap_or(HASH_MAX),
            response_bytes: MAX_RESPONSE_BYTES,
        });
        let mut receiver = free_downloader_channels_clone.receiver.lock().await;
        if let Err(err) = (free_downloader_channels_clone.connection)
            .cast(CastMessage::BackendMessage(request))
            .await
        {
            error!("Failed to send message to peer: {err:?}");
            tx.send(empty_task_result).await.ok();
            return Ok(());
        }
        let request_result = tokio::time::timeout(Duration::from_secs(2), async move {
            loop {
                match receiver.recv().await {
                    Some(RLPxMessage::StorageRanges(StorageRanges { id, slots, proof }))
                        if id == request_id =>
                    {
                        return Some((slots, proof));
                    }
                    Some(_) => continue,
                    None => return None,
                }
            }
        })
        .await
        .ok()
        .flatten();
        let Some((slots, proof)) = request_result else {
            tracing::debug!("Failed to get storage range");
            tx.send(empty_task_result).await.ok();
            return Ok(());
        };
        if slots.is_empty() && proof.is_empty() {
            tx.send(empty_task_result).await.ok();
            tracing::debug!("Received empty storage range");
            return Ok(());
        }
        // Check we got some data and no more than the requested amount
        if slots.len() > chunk_storage_roots.len() || slots.is_empty() {
            tx.send(empty_task_result).await.ok();
            return Ok(());
        }
        // Unzip & validate response
        let proof = encodable_to_proof(&proof);
        let mut account_storages: Vec<Vec<(H256, U256)>> = vec![];
        let mut should_continue = false;
        // Validate each storage range
        let mut storage_roots = chunk_storage_roots.into_iter();
        let last_slot_index = slots.len() - 1;
        for (i, next_account_slots) in slots.into_iter().enumerate() {
            // We won't accept empty storage ranges
            if next_account_slots.is_empty() {
                // This shouldn't happen
                error!("Received empty storage range, skipping");
                tx.send(empty_task_result.clone()).await.ok();
                return Ok(());
            }
            let encoded_values = next_account_slots
                .iter()
                .map(|slot| slot.data.encode_to_vec())
                .collect::<Vec<_>>();
            let hashed_keys: Vec<_> = next_account_slots.iter().map(|slot| slot.hash).collect();

            let storage_root = match storage_roots.next() {
                Some(root) => root,
                None => {
                    tx.send(empty_task_result.clone()).await.ok();
                    error!("No storage root for account {i}");
                    return Err(PeerHandlerError::NoStorageRoots);
                }
            };

            // The proof corresponds to the last slot, for the previous ones the slot must be the full range without edge proofs
            if i == last_slot_index && !proof.is_empty() {
                let Ok(sc) = verify_range(
                    storage_root,
                    &start_hash,
                    &hashed_keys,
                    &encoded_values,
                    &proof,
                ) else {
                    tx.send(empty_task_result).await.ok();
                    return Ok(());
                };
                should_continue = sc;
            } else if verify_range(
                storage_root,
                &start_hash,
                &hashed_keys,
                &encoded_values,
                &[],
            )
            .is_err()
            {
                tx.send(empty_task_result.clone()).await.ok();
                return Ok(());
            }

            account_storages.push(
                next_account_slots
                    .iter()
                    .map(|slot| (slot.hash, slot.data))
                    .collect(),
            );
        }
        let (remaining_start, remaining_end, remaining_start_hash) = if should_continue {
            let last_account_storage = match account_storages.last() {
                Some(storage) => storage,
                None => {
                    tx.send(empty_task_result.clone()).await.ok();
                    error!("No account storage found, this shouldn't happen");
                    return Err(PeerHandlerError::NoAccountStorages);
                }
            };
            let (last_hash, _) = match last_account_storage.last() {
                Some(last_hash) => last_hash,
                None => {
                    tx.send(empty_task_result.clone()).await.ok();
                    error!("No last hash found, this shouldn't happen");
                    return Err(PeerHandlerError::NoAccountStorages);
                }
            };
            let next_hash_u256 = U256::from_big_endian(&last_hash.0).saturating_add(1.into());
            let next_hash = H256::from_uint(&next_hash_u256);
            (start + account_storages.len() - 1, end, next_hash)
        } else {
            (start + account_storages.len(), end, H256::zero())
        };
        let task_result = StorageTaskResult {
            start_index: start,
            account_storages,
            peer_id: free_peer_id,
            remaining_start,
            remaining_end,
            remaining_hash_range: (remaining_start_hash, task.end_hash),
        };
        tx.send(task_result).await.ok();
        Ok::<(), PeerHandlerError>(())
    }

    pub async fn request_state_trienodes(
        peer_channel: &mut PeerChannels,
        state_root: H256,
        paths: Vec<RequestMetadata>,
    ) -> Result<Vec<Node>, RequestStateTrieNodesError> {
        let expected_nodes = paths.len();
        // Keep track of peers we requested from so we can penalize unresponsive peers when we get a response
        // This is so we avoid penalizing peers due to requesting stale data

        let request_id = rand::random();
        let request = RLPxMessage::GetTrieNodes(GetTrieNodes {
            id: request_id,
            root_hash: state_root,
            // [acc_path, acc_path,...] -> [[acc_path], [acc_path]]
            paths: paths
                .iter()
                .map(|vec| vec![Bytes::from(vec.path.encode_compact())])
                .collect(),
            bytes: MAX_RESPONSE_BYTES,
        });
        let nodes =
            super::utils::send_message_and_wait_for_response(peer_channel, request, request_id)
                .await
                .map_err(RequestStateTrieNodesError::SendMessageError)?;

        if nodes.is_empty() || nodes.len() > expected_nodes {
            return Err(RequestStateTrieNodesError::InvalidData);
        }

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
        let request = RLPxMessage::GetTrieNodes(get_trie_nodes);
        super::utils::send_trie_nodes_messages_and_wait_for_reply(peer_channel, request, id)
            .await
            .map_err(|err| RequestStorageTrieNodes::SendMessageError(id, err))
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
        let request_id = rand::random();
        let request = RLPxMessage::GetBlockHeaders(GetBlockHeaders {
            id: request_id,
            startblock: HashOrNumber::Number(block_number),
            limit: 1,
            skip: 0,
            reverse: false,
        });
        info!("get_block_header: requesting header with number {block_number}");

        let mut receiver = peer_channel.receiver.lock().await;
        debug!("locked the receiver for the peer_channel");
        peer_channel
            .connection
            .cast(CastMessage::BackendMessage(request.clone()))
            .await
            .map_err(|e| PeerHandlerError::SendMessageToPeer(e.to_string()))?;

        let response =
            tokio::time::timeout(Duration::from_secs(5), async move { receiver.recv().await })
                .await;

        // TODO: we need to check, this seems a scenario where the peer channel does teardown
        // after we sent the backend message
        let Some(Ok(response)) = response
            .inspect_err(|_err| info!("Timeout while waiting for sync head from peer"))
            .transpose()
        else {
            warn!("The RLPxConnection closed the backend channel");
            return Ok(None);
        };

        match response {
            RLPxMessage::BlockHeaders(BlockHeaders { id, block_headers }) => {
                if id == request_id && !block_headers.is_empty() {
                    return Ok(Some(
                        block_headers
                            .last()
                            .ok_or(PeerHandlerError::BlockHeaders)?
                            .clone(),
                    ));
                }
            }
            _other_msgs => {
                info!("Received unexpected message from peer");
            }
        }

        Ok(None)
    }
}

/// Validates the block headers received from a peer by checking that the parent hash of each header
/// matches the hash of the previous one, i.e. the headers are chained
fn are_block_headers_chained(block_headers: &[BlockHeader], order: &BlockRequestOrder) -> bool {
    block_headers.windows(2).all(|headers| match order {
        BlockRequestOrder::OldToNew => headers[1].parent_hash == headers[0].hash(),
        BlockRequestOrder::NewToOld => headers[0].parent_hash == headers[1].hash(),
    })
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
