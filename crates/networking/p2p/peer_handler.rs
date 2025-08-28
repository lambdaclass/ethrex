use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    io::ErrorKind,
    sync::Arc,
    time::{Duration, SystemTime},
};

use bytes::Bytes;
use ethrex_common::{
    BigEndianHash, H256, U256,
    types::{AccountState, BlockBody, BlockHeader, Receipt, validate_block_body},
};
use ethrex_rlp::encode::RLPEncode;
use ethrex_trie::Nibbles;
use ethrex_trie::Node;
use rand::random;
use spawned_concurrency::tasks::GenServer;
use tokio::{sync::Mutex, time::Instant};

use crate::{
    kademlia::{Kademlia, PeerChannels, PeerData},
    metrics::METRICS,
    rlpx::{
        downloader::{
            Downloader, DownloaderCallRequest, DownloaderCallResponse, DownloaderCastRequest,
        },
        p2p::{Capability, SUPPORTED_SNAP_CAPABILITIES},
        snap::AccountRangeUnit,
    },
    utils::{dump_to_file, get_account_state_snapshot_file, get_account_storages_snapshot_file},
};
use tracing::{debug, error, info, trace, warn};
pub const PEER_REPLY_TIMEOUT: Duration = Duration::from_secs(15);
pub const PEER_SELECT_RETRY_ATTEMPTS: usize = 3;
pub const REQUEST_RETRY_ATTEMPTS: usize = 5;
pub const HASH_MAX: H256 = H256([0xFF; 32]);

pub const SNAP_LIMIT: usize = 20;

pub const MIN_PEER_SCORE_THRESHOLD: i64 = -20;

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
    last_peer_timeout_check: Arc<Mutex<Instant>>,
    pub peer_table: Kademlia,
    pub peers_info: Arc<Mutex<HashMap<H256, PeerInformation>>>,
}

/// Holds information about connected peers, their performance and availability
#[derive(Debug, Clone)]
pub struct PeerInformation {
    pub score: i64,
    pub available: bool,
    pub request_time: Option<Instant>,
}

impl Default for PeerInformation {
    fn default() -> Self {
        Self {
            score: 0,
            available: true,
            request_time: None,
        }
    }
}

// channel to send the tasks to the peers
pub struct BytecodeRequestTaskResult {
    pub(crate) start_index: usize,
    pub(crate) bytecodes: Vec<Bytes>,
    pub(crate) peer_id: H256,
    pub(crate) remaining_start: usize,
    pub(crate) remaining_end: usize,
}

#[derive(Clone)]
pub struct StorageRequestTaskResult {
    pub(crate) start_index: usize,
    pub(crate) account_storages: Vec<Vec<(H256, U256)>>,
    pub(crate) peer_id: H256,
    pub(crate) remaining_start: usize,
    pub(crate) remaining_end: usize,
    pub(crate) remaining_hash_range: (H256, Option<H256>),
}

pub enum BlockRequestOrder {
    OldToNew,
    NewToOld,
}

impl PeerHandler {
    pub fn new(peer_table: Kademlia) -> PeerHandler {
        Self {
            peer_table,
            peers_info: Default::default(),
            last_peer_timeout_check: Arc::new(Mutex::new(Instant::now())),
        }
    }

    /// Creates a dummy PeerHandler for tests where interacting with peers is not needed
    /// This should only be used in tests as it won't be able to interact with the node's connected peers
    pub fn dummy() -> PeerHandler {
        let dummy_peer_table = Kademlia::new();
        PeerHandler::new(dummy_peer_table)
    }

    async fn mark_peer_as_free(&self, peer_id: H256) {
        self.peers_info
            .lock()
            .await
            .entry(peer_id)
            .and_modify(|info| {
                info.available = true;
                info.request_time = None
            });
    }

    async fn mark_peer_as_busy(&self, peer_id: H256) {
        self.peers_info
            .lock()
            .await
            .entry(peer_id)
            .and_modify(|info| {
                info.available = false;
                info.request_time = Some(Instant::now())
            });
    }

    /// Helper function called in between syncing steps.
    /// Guarantees that no peer is left as unavailable.
    async fn refresh_peers_availability(&self) {
        for peer_info in self.peers_info.lock().await.values_mut() {
            peer_info.available = true;
            peer_info.request_time = None;
            // Give badly scoring peers a new chance
            if peer_info.score <= MIN_PEER_SCORE_THRESHOLD {
                peer_info.score = 0;
            }
        }
    }

    // TODO: Implement the logic for recording peer successes
    /// Helper method to record successful peer response
    async fn record_peer_success(&self, peer_id: H256) {
        self.peers_info
            .lock()
            .await
            .entry(peer_id)
            .and_modify(|info| info.score = info.score.saturating_add(1));
    }

    /// Helper method to record failed peer response
    async fn record_peer_failure(&self, peer_id: H256) {
        self.peers_info
            .lock()
            .await
            .entry(peer_id)
            .and_modify(|info| info.score = info.score.saturating_sub(1));
    }

    /// Helper method to record critical peer failure
    /// This is used when the peer returns invalid data or is otherwise unreliable
    async fn record_peer_critical_failure(&self, peer_id: H256) {
        self.peers_info
            .lock()
            .await
            .entry(peer_id)
            .and_modify(|info| info.score = info.score.saturating_sub(MIN_PEER_SCORE_THRESHOLD));
    }

    /// TODO: docs
    pub async fn get_peer_channel_with_highest_score(
        &self,
        capabilities: &[Capability],
        scores: &mut HashMap<H256, i64>,
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
            let peer_id_score = scores.entry(*peer_id).or_default();
            if *peer_id_score >= max_peer_id_score {
                free_peer_id = *peer_id;
                max_peer_id_score = *peer_id_score;
                free_peer_channel = channel.clone();
            }
        }

        Ok(Some((free_peer_id, free_peer_channel.clone())))
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
        self.refresh_peers_availability().await;

        let start_time = SystemTime::now();

        let initial_downloaded_headers = *METRICS.downloaded_headers.lock().await;

        let mut ret = Vec::<BlockHeader>::new();

        let mut sync_head_number = 0_u64;

        let sync_head_number_retrieval_start = SystemTime::now();

        info!("Retrieving sync head block number from peers");

        while sync_head_number == 0 {
            let available_downloader = loop {
                if let Some(downloader) = self.get_random_available_downloader().await {
                    break downloader;
                } else {
                    debug!("No peers available to retrieve sync head");
                }
            };
            let peer_id = available_downloader.peer_id();

            match available_downloader
                .start()
                .call(DownloaderCallRequest::CurrentHead { sync_head })
                .await
            {
                Ok(DownloaderCallResponse::CurrentHead(current_head_number)) => {
                    sync_head_number = current_head_number;
                    self.record_peer_success(peer_id).await;
                    self.mark_peer_as_free(peer_id).await;
                    break;
                }
                _ => {
                    trace!("Failed to retrieve sync head block number from peer {peer_id}");
                    self.record_peer_failure(peer_id).await;
                }
            }
            self.mark_peer_as_free(peer_id).await;
        }

        let sync_head_number_retrieval_elapsed = sync_head_number_retrieval_start
            .elapsed()
            .unwrap_or_default();

        info!("Sync head block number retrieved");

        *METRICS.time_to_retrieve_sync_head_block.lock().await =
            Some(sync_head_number_retrieval_elapsed);
        *METRICS.sync_head_block.lock().await = sync_head_number;
        *METRICS.headers_to_download.lock().await = sync_head_number + 1;
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
        let mut metrics_downloaded_count = 0_u64;

        // channel to send the tasks to the peers
        let (task_sender, mut task_receiver) =
            tokio::sync::mpsc::channel::<(Vec<BlockHeader>, H256, u64, u64)>(1000);

        let mut current_show = 0;

        // 3) create tasks that will request a chunk of headers from a peer

        info!("Starting to download block headers from peers");

        *METRICS.headers_download_start_time.lock().await = Some(SystemTime::now());

        let mut last_metrics_update = SystemTime::now();

        self.refresh_peers_availability().await;
        loop {
            self.reset_timed_out_busy_peers().await;

            let new_last_metrics_update = last_metrics_update
                .elapsed()
                .unwrap_or(Duration::from_secs(1));

            if new_last_metrics_update >= Duration::from_secs(1) {
                *METRICS.header_downloads_tasks_queued.lock().await =
                    tasks_queue_not_started.len() as u64;

                *METRICS.total_header_downloaders.lock().await =
                    self.peers_info.lock().await.len() as u64;
            }

            if let Ok((headers, peer_id, startblock, previous_chunk_limit)) =
                task_receiver.try_recv()
            {
                self.mark_peer_as_free(peer_id).await;
                if headers.is_empty() {
                    trace!("Failed to download chunk from peer {peer_id}");
                    self.record_peer_failure(peer_id).await;
                    // reinsert the task to the queue
                    tasks_queue_not_started.push_back((startblock, previous_chunk_limit));
                    continue; // Retry with the next peer
                }

                downloaded_count += headers.len() as u64;
                metrics_downloaded_count += headers.len() as u64;

                if new_last_metrics_update >= Duration::from_secs(1) {
                    *METRICS.downloaded_headers.lock().await += metrics_downloaded_count;
                    metrics_downloaded_count = 0;
                }

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
                self.record_peer_success(peer_id).await;
            }

            let available_downloader = self.get_best_available_downloader().await;
            let Some(available_downloader) = available_downloader else {
                debug!("No free downloaders available, waiting for a peer to finish, retrying");
                continue;
            };

            let Some((start_block, chunk_limit)) = tasks_queue_not_started.pop_front() else {
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

            if let Err(_) = available_downloader
                .start()
                .cast(DownloaderCastRequest::Headers {
                    task_sender: task_sender.clone(),
                    start_block,
                    chunk_limit,
                })
                .await
            {
                tasks_queue_not_started.push_front((start_block, chunk_limit));
            }

            // 4) assign the tasks to the peers
            //     4.1) launch a tokio task with the chunk and a peer ready (giving the channels)

            // TODO!!! spawn a task to download the chunk, calling `download_chunk_from_peer`

            if new_last_metrics_update >= Duration::from_secs(1) {
                last_metrics_update = SystemTime::now();
            }
        }

        let downloaders_count = self.peers_info.lock().await.len() as u64;
        *METRICS.header_downloads_tasks_queued.lock().await = tasks_queue_not_started.len() as u64;
        *METRICS.free_header_downloaders.lock().await = downloaders_count;
        *METRICS.total_header_downloaders.lock().await = downloaders_count;
        *METRICS.downloaded_headers.lock().await = initial_downloaded_headers + downloaded_count;

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

    /// Internal method to request block bodies from any suitable peer given their block hashes
    /// Returns the block bodies or None if:
    /// - There are no available peers (the node just started up or was rejected by all other nodes)
    /// - The requested peer did not return a valid response in the given time limit
    async fn request_block_bodies_inner(
        &self,
        block_hashes: Vec<H256>,
    ) -> Option<(Vec<BlockBody>, H256)> {
        self.refresh_peers_availability().await;
        let Some(available_downloader) = self.get_best_available_downloader().await else {
            debug!("No free downloaders available to request block bodies");
            return None;
        };

        let peer_id = available_downloader.peer_id();
        match available_downloader
            .start()
            .call(DownloaderCallRequest::BlockBodies { block_hashes })
            .await
        {
            Ok(DownloaderCallResponse::BlockBodies(block_bodies)) => {
                self.record_peer_success(peer_id).await;
                self.mark_peer_as_free(peer_id).await;
                Some((block_bodies, peer_id))
            }
            _ => {
                warn!(
                    "[SYNCING] Didn't receive block bodies from peer, penalizing peer {peer_id}..."
                );
                self.record_peer_failure(peer_id).await;
                self.mark_peer_as_free(peer_id).await;
                None
            }
        }
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
        for _ in 0..REQUEST_RETRY_ATTEMPTS {
            let available_downloader = self.get_best_available_downloader().await?;
            let peer_id = available_downloader.peer_id();
            match available_downloader
                .start()
                .call(DownloaderCallRequest::Receipts {
                    block_hashes: block_hashes.clone(),
                })
                .await
            {
                Ok(DownloaderCallResponse::Receipts(Some(receipts))) => {
                    return {
                        self.record_peer_success(peer_id).await;
                        Some(receipts)
                    };
                }
                _ => {
                    self.record_peer_failure(peer_id).await;
                    continue;
                }
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
        state_root: H256,
        start: H256,
        limit: H256,
        account_state_snapshots_dir: String,
    ) -> Result<(), PeerHandlerError> {
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

        let mut last_metrics_update = SystemTime::now();
        let mut completed_tasks = 0;
        let mut chunk_file = 0;

        self.refresh_peers_availability().await;
        loop {
            self.reset_timed_out_busy_peers().await;
            if all_accounts_state.len() * size_of::<AccountState>() >= 1024 * 1024 * 1024 * 8 {
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

            let new_last_metrics_update = last_metrics_update
                .elapsed()
                .unwrap_or(Duration::from_secs(1));

            if new_last_metrics_update >= Duration::from_secs(1) {
                *METRICS.accounts_downloads_tasks_queued.lock().await =
                    tasks_queue_not_started.len() as u64;
                *METRICS.total_accounts_downloaders.lock().await =
                    self.peers_info.lock().await.len() as u64;
                *METRICS.downloaded_account_tries.lock().await = downloaded_count;
            }

            if let Ok((accounts, peer_id, chunk_start_end)) = task_receiver.try_recv() {
                self.mark_peer_as_free(peer_id).await;

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
                    self.record_peer_failure(peer_id).await;
                    continue;
                }
                self.record_peer_success(peer_id).await;

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

            let available_downloader = self.get_best_available_downloader().await;
            let Some(available_downloader) = available_downloader else {
                debug!("No free downloaders available, waiting for a peer to finish, retrying");
                continue;
            };

            let Some((chunk_start, chunk_end)) = tasks_queue_not_started.pop_front() else {
                if completed_tasks >= chunk_count {
                    info!("All account ranges downloaded successfully");
                    break;
                }
                continue;
            };

            if let Err(_) = available_downloader
                .start()
                .cast(DownloaderCastRequest::AccountRange {
                    task_sender: task_sender.clone(),
                    root_hash: state_root,
                    starting_hash: chunk_start,
                    limit_hash: chunk_end,
                })
                .await
            {
                tasks_queue_not_started.push_front((chunk_start, chunk_end));
            }

            if new_last_metrics_update >= Duration::from_secs(1) {
                info!("{:?} pending account tasks", tasks_queue_not_started.len());
                info!("completed tasks {completed_tasks} chunk count {chunk_count}");
                last_metrics_update = SystemTime::now();
            }
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

        let downloaders_count = self.peers_info.lock().await.len() as u64;
        *METRICS.accounts_downloads_tasks_queued.lock().await =
            tasks_queue_not_started.len() as u64;
        *METRICS.total_accounts_downloaders.lock().await = downloaders_count;
        *METRICS.downloaded_account_tries.lock().await = downloaded_count;
        *METRICS.free_accounts_downloaders.lock().await = downloaders_count;
        *METRICS.account_tries_download_end_time.lock().await = Some(SystemTime::now());

        Ok(())
    }

    /// Requests bytecodes for the given code hashes
    /// Returns the bytecodes or None if:
    /// - There are no available peers (the node just started up or was rejected by all other nodes)
    /// - No peer returned a valid response in the given time and retry limits
    pub async fn request_bytecodes(
        &self,
        all_bytecode_hashes: &[H256],
    ) -> Result<Option<Vec<Bytes>>, PeerHandlerError> {
        self.refresh_peers_availability().await;

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

        let mut downloaded_count = 0_u64;
        let mut all_bytecodes = vec![Bytes::new(); all_bytecode_hashes.len()];

        let (task_sender, mut task_receiver) =
            tokio::sync::mpsc::channel::<BytecodeRequestTaskResult>(1000);

        info!("Starting to download bytecodes from peers");

        *METRICS.bytecodes_to_download.lock().await = all_bytecode_hashes.len() as u64;
        *METRICS.bytecode_download_start_time.lock().await = Some(SystemTime::now());

        let mut last_metrics_update = SystemTime::now();
        let mut completed_tasks = 0;
        loop {
            self.reset_timed_out_busy_peers().await;

            let new_last_metrics_update = last_metrics_update
                .elapsed()
                .unwrap_or(Duration::from_secs(1));

            if new_last_metrics_update >= Duration::from_secs(1) {
                *METRICS.bytecode_downloads_tasks_queued.lock().await =
                    tasks_queue_not_started.len() as u64;
                *METRICS.total_bytecode_downloaders.lock().await =
                    self.peers_info.lock().await.len() as u64;
                *METRICS.downloaded_bytecodes.lock().await = downloaded_count;
            }

            if let Ok(result) = task_receiver.try_recv() {
                let BytecodeRequestTaskResult {
                    start_index,
                    bytecodes,
                    peer_id,
                    remaining_start,
                    remaining_end,
                } = result;

                self.mark_peer_as_free(peer_id).await;

                if remaining_start < remaining_end {
                    tasks_queue_not_started.push_back((remaining_start, remaining_end));
                } else {
                    completed_tasks += 1;
                }
                if bytecodes.is_empty() {
                    self.record_peer_failure(peer_id).await;
                    continue;
                }

                downloaded_count += bytecodes.len() as u64;
                self.record_peer_success(peer_id).await;
                debug!(
                    "Downloaded {} bytecodes from peer {peer_id} (current count: {downloaded_count})",
                    bytecodes.len(),
                );
                for (i, bytecode) in bytecodes.into_iter().enumerate() {
                    all_bytecodes[start_index + i] = bytecode;
                }
            }

            let Some(available_downloader) = self.get_best_available_downloader().await else {
                debug!("No free downloaders available, waiting for a peer to finish, retrying");
                continue;
            };

            let Some((chunk_start, chunk_end)) = tasks_queue_not_started.pop_front() else {
                if completed_tasks >= chunk_count {
                    info!("All bytecodes downloaded successfully");
                    break;
                }
                continue;
            };

            let tx = task_sender.clone();

            let hashes_to_request = all_bytecode_hashes
                .iter()
                .skip(chunk_start)
                .take((chunk_end - chunk_start).min(MAX_BYTECODES_REQUEST_SIZE))
                .copied()
                .collect();

            if let Err(_) = available_downloader
                .start()
                .cast(DownloaderCastRequest::ByteCode {
                    task_sender: tx.clone(),
                    hashes_to_request,
                    chunk_start,
                    chunk_end,
                })
                .await
            {
                tasks_queue_not_started.push_front((chunk_start, chunk_end));
            }

            if new_last_metrics_update >= Duration::from_secs(1) {
                last_metrics_update = SystemTime::now();
            }
        }

        let downloaders_count = self.peers_info.lock().await.len() as u64;
        *METRICS.bytecode_downloads_tasks_queued.lock().await =
            tasks_queue_not_started.len() as u64;
        *METRICS.total_bytecode_downloaders.lock().await = downloaders_count;
        *METRICS.downloaded_bytecodes.lock().await = downloaded_count;
        *METRICS.free_bytecode_downloaders.lock().await = downloaders_count;

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
        state_root: H256,
        account_storage_roots: Vec<(H256, H256)>,
        account_storages_snapshots_dir: String,
        mut chunk_index: u64,
        downloaded_count: &mut u64,
    ) -> Result<u64, PeerHandlerError> {
        self.refresh_peers_availability().await;

        // 1) split the range in chunks of same length
        let chunk_size = 300;
        let chunk_count = (account_storage_roots.len() / chunk_size) + 1;

        // list of tasks to be executed
        // Types are (start_index, end_index, starting_hash)
        // NOTE: end_index is NOT inclusive
        #[derive(Debug)]
        struct Task {
            start_index: usize,
            end_index: usize,
            start_hash: H256,
            // end_hash is None if the task is for the first big storage request
            end_hash: Option<H256>,
        }
        let mut tasks_queue_not_started = VecDeque::<Task>::new();
        for i in 0..chunk_count {
            let chunk_start = chunk_size * i;
            let chunk_end = (chunk_start + chunk_size).min(account_storage_roots.len());
            tasks_queue_not_started.push_back(Task {
                start_index: chunk_start,
                end_index: chunk_end,
                start_hash: H256::zero(),
                end_hash: None,
            });
        }

        let mut all_account_storages = vec![vec![]; account_storage_roots.len()];

        // channel to send the tasks to the peers
        let (task_sender, mut task_receiver) =
            tokio::sync::mpsc::channel::<StorageRequestTaskResult>(1000);

        // channel to send the result of dumping storages
        let (dump_storage_result_sender, mut dump_storage_result_receiver) =
            tokio::sync::mpsc::channel::<Result<(), DumpError>>(1000);

        let mut last_metrics_update = SystemTime::now();
        let mut task_count = tasks_queue_not_started.len();
        let mut completed_tasks = 0;

        loop {
            if all_account_storages.iter().map(Vec::len).sum::<usize>() * 64
                > 1024 * 1024 * 1024 * 8
            {
                let current_account_hashes = account_storage_roots
                    .iter()
                    .map(|a| a.0)
                    .collect::<Vec<_>>();
                let current_account_storages = std::mem::take(&mut all_account_storages);
                all_account_storages = vec![vec![]; account_storage_roots.len()];

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
                let dump_account_result_sender_cloned = dump_storage_result_sender.clone();
                tokio::task::spawn(async move {
                    let path = get_account_storages_snapshot_file(
                        account_storages_snapshots_dir_cloned,
                        chunk_index,
                    );
                    let result = dump_to_file(path, snapshot);
                    dump_account_result_sender_cloned
                        .send(result)
                        .await
                        .inspect_err(|err| {
                            error!(
                                "Failed to send storage dump result through channel. Error: {err}"
                            )
                        })
                });

                chunk_index += 1;
            }

            let new_last_metrics_update = last_metrics_update
                .elapsed()
                .unwrap_or(Duration::from_secs(1));

            if new_last_metrics_update >= Duration::from_secs(1) {
                *METRICS.storages_downloads_tasks_queued.lock().await =
                    tasks_queue_not_started.len() as u64;
                *METRICS.total_storages_downloaders.lock().await =
                    self.peers_info.lock().await.len() as u64;
                *METRICS.downloaded_storage_tries.lock().await = *downloaded_count;
            }

            if let Ok(result) = task_receiver.try_recv() {
                let StorageRequestTaskResult {
                    start_index,
                    mut account_storages,
                    peer_id,
                    remaining_start,
                    remaining_end,
                    remaining_hash_range: (hash_start, hash_end),
                } = result;
                completed_tasks += 1;

                self.mark_peer_as_free(peer_id).await;

                if remaining_start < remaining_end {
                    trace!("Failed to download chunk from peer {peer_id}");
                    if hash_start.is_zero() {
                        // Task is common storage range request
                        let task = Task {
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
                            let task = Task {
                                start_index: remaining_start,
                                end_index: remaining_end,
                                start_hash: hash_start,
                                end_hash: Some(hash_end),
                            };
                            tasks_queue_not_started.push_back(task);
                            task_count += 1;
                        }
                    } else {
                        if remaining_start + 1 < remaining_end {
                            let task = Task {
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

                            let task = Task {
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
                    self.record_peer_failure(peer_id).await;
                    continue;
                }
                if let Some(hash_end) = hash_end {
                    // This is a big storage account, and the range might be empty
                    if account_storages[0].len() == 1 && account_storages[0][0].0 > hash_end {
                        continue;
                    }
                }

                self.record_peer_success(peer_id).await;

                *downloaded_count += account_storages.len() as u64;
                // If we didn't finish downloading the account, don't count it
                if !hash_start.is_zero() {
                    *downloaded_count -= 1;
                }

                let n_storages = account_storages.len();
                let n_slots = account_storages
                    .iter()
                    .map(|storage| storage.len())
                    .sum::<usize>();

                *METRICS.downloaded_storage_slots.lock().await += n_slots as u64;

                debug!(
                    "Downloaded {n_storages} storages ({n_slots} slots) from peer {peer_id} (current count: {downloaded_count})"
                );
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

            // Check if any write storage task finished
            if let Ok(Err(dump_storage_data)) = dump_storage_result_receiver.try_recv() {
                if dump_storage_data.error == ErrorKind::StorageFull {
                    return Err(PeerHandlerError::StorageFull);
                }
                // If the dumping failed, retry it
                let dump_storage_result_sender_cloned = dump_storage_result_sender.clone();
                tokio::task::spawn(async move {
                    let DumpError { path, contents, .. } = dump_storage_data;
                    // Write the storage data
                    let result = dump_to_file(path, contents);
                    // Send the result through the channel
                    dump_storage_result_sender_cloned
                        .send(result)
                        .await
                        .inspect_err(|err| {
                            error!(
                                "Failed to send storage dump result through channel. Error: {err}"
                            )
                        })
                });
            }

            let Some(available_downloader) = self.get_best_available_downloader().await else {
                debug!("No free downloaders available, waiting for a peer to be free, retrying");
                continue;
            };

            let Some(task) = tasks_queue_not_started.pop_front() else {
                if completed_tasks >= task_count {
                    break;
                }
                continue;
            };

            let tx = task_sender.clone();

            let (chunk_account_hashes, chunk_storage_roots): (Vec<_>, Vec<_>) =
                account_storage_roots
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

            if let Err(_) = available_downloader
                .start()
                .cast(DownloaderCastRequest::StorageRanges {
                    task_sender: tx.clone(),
                    start_index: task.start_index,
                    end_index: task.end_index,
                    start_hash: task.start_hash,
                    end_hash: task.end_hash,
                    state_root,
                    chunk_account_hashes,
                    chunk_storage_roots,
                })
                .await
            {
                tasks_queue_not_started.push_front(task);
            }

            if new_last_metrics_update >= Duration::from_secs(1) {
                last_metrics_update = SystemTime::now();
            }
        }

        {
            let current_account_hashes = account_storage_roots
                .iter()
                .map(|a| a.0)
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

        let peers_count = self.peers_info.lock().await.len();
        *METRICS.storages_downloads_tasks_queued.lock().await =
            tasks_queue_not_started.len() as u64;
        *METRICS.total_storages_downloaders.lock().await = peers_count as u64;
        *METRICS.downloaded_storage_tries.lock().await = *downloaded_count;
        *METRICS.free_storages_downloaders.lock().await = peers_count as u64;

        Ok(chunk_index + 1)
    }

    /// Requests state trie nodes given the root of the trie where they are contained and their path (be them full or partial)
    /// Returns the nodes or None if:
    /// - There are no available peers (the node just started up or was rejected by all other nodes)
    /// - No peer returned a valid response in the given time and retry limits
    pub async fn request_state_trienodes(
        &self,
        state_root: H256,
        paths: Vec<Nibbles>,
    ) -> Option<Vec<Node>> {
        for _ in 0..REQUEST_RETRY_ATTEMPTS {
            let available_downloader = loop {
                if let Some(downloader) = self.get_best_available_downloader().await {
                    break downloader;
                }
            };

            let paths: Vec<Vec<Bytes>> = paths
                .iter()
                .map(|vec| vec![Bytes::from(vec.encode_compact())])
                .collect();

            let peer_id = available_downloader.peer_id();
            match available_downloader
                .start()
                .call(DownloaderCallRequest::TrieNodes {
                    root_hash: state_root,
                    paths,
                })
                .await
            {
                Ok(DownloaderCallResponse::TrieNodes(nodes)) => {
                    self.record_peer_success(peer_id).await;
                    self.mark_peer_as_free(peer_id).await;
                    Some(nodes)
                }
                _ => {
                    self.record_peer_failure(peer_id).await;
                    self.mark_peer_as_free(peer_id).await;
                    None
                }
            };
        }
        None
    }

    /// Requests storage trie nodes given the root of the state trie where they are contained and
    /// a hashmap mapping the path to the account in the state trie (aka hashed address) to the paths to the nodes in its storage trie (can be full or partial)
    /// Returns the nodes or None if:
    /// - There are no available peers (the node just started up or was rejected by all other nodes)
    /// - No peer returned a valid response in the given time and retry limits
    pub async fn request_storage_trienodes(
        &self,
        state_root: H256,
        paths: BTreeMap<H256, Vec<Nibbles>>,
    ) -> Option<Vec<Node>> {
        for _ in 0..REQUEST_RETRY_ATTEMPTS {
            let available_downloader = loop {
                if let Some(downloader) = self.get_best_available_downloader().await {
                    break downloader;
                }
            };

            let paths = paths
                .iter()
                .map(|(acc_path, paths)| {
                    [
                        vec![Bytes::from(acc_path.0.to_vec())],
                        paths
                            .iter()
                            .map(|path| Bytes::from(path.encode_compact()))
                            .collect(),
                    ]
                    .concat()
                })
                .collect();

            let peer_id = available_downloader.peer_id();
            match available_downloader
                .start()
                .call(DownloaderCallRequest::TrieNodes {
                    root_hash: state_root,
                    paths,
                })
                .await
            {
                Ok(DownloaderCallResponse::TrieNodes(nodes)) => {
                    self.record_peer_success(peer_id).await;
                    self.mark_peer_as_free(peer_id).await;
                    Some(nodes)
                }
                _ => {
                    self.record_peer_failure(peer_id).await;
                    self.mark_peer_as_free(peer_id).await;
                    None
                }
            };
        }
        None
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
        block_number: u64,
    ) -> Result<Option<BlockHeader>, PeerHandlerError> {
        self.refresh_peers_availability().await;
        let Some(available_downloader) = self.get_best_available_downloader().await else {
            return Err(PeerHandlerError::NoPeers);
        };

        let peer_id = available_downloader.peer_id();
        info!(
            "Trying to update pivot to {block_number} with peer {}",
            peer_id
        );

        let (task_sender, mut task_receiver) =
            tokio::sync::mpsc::channel::<(Vec<BlockHeader>, H256, u64, u64)>(1000);

        available_downloader
            .start()
            .cast(DownloaderCastRequest::Headers {
                task_sender,
                start_block: block_number,
                chunk_limit: 1,
            })
            .await
            .map_err(|_| PeerHandlerError::BlockHeaders)?;

        let response =
            tokio::time::timeout(
                Duration::from_secs(5),
                async move { task_receiver.recv().await },
            )
            .await;

        self.mark_peer_as_free(peer_id).await;
        let Some(Ok((block_headers, _peer_id, _start_block, _chunk_limit))) = response
            .inspect_err(|_err| info!("Timeout while waiting for sync head from peer"))
            .transpose()
        else {
            warn!("The RLPxConnection closed the backend channel");
            self.record_peer_critical_failure(peer_id).await;
            return Ok(None);
        };

        if !block_headers.is_empty() {
            self.record_peer_success(peer_id).await;
            return Ok(Some(block_headers[0].clone()));
        } else {
            warn!("Peer returned empty block headers");
            self.record_peer_failure(peer_id).await;
            return Ok(None);
        }
    }

    // Creates a Downloader Actor from a random peer
    // Returns None if no peer is available
    async fn get_random_available_downloader(&self) -> Option<Downloader> {
        let peer_channels = self
            .peer_table
            .get_peer_channels(&SUPPORTED_SNAP_CAPABILITIES)
            .await;

        let mut peers_info = self.peers_info.lock().await;

        for (peer_id, _peer_channels) in &peer_channels {
            if peers_info.contains_key(peer_id) {
                continue;
            }
            peers_info.insert(*peer_id, PeerInformation::default());
            debug!("{peer_id} added as downloader");
        }

        let free_downloaders = peers_info
            .clone()
            .into_iter()
            .filter(|(_, peer_info)| peer_info.available)
            .collect::<Vec<_>>();

        if free_downloaders.is_empty() {
            // No available downloaders to offer
            return None;
        }

        let (free_peer_id, _) = free_downloaders
            .get(random::<usize>() % free_downloaders.len())
            .expect("There should be at least one free downloader");

        let Some(free_downloader_channels) = self
            .peer_table
            .get_peer_channels(&SUPPORTED_SNAP_CAPABILITIES)
            .await
            .iter()
            .find_map(|(peer_id, peer_channels)| {
                peer_id.eq(&free_peer_id).then_some(peer_channels.clone())
            })
        else {
            debug!(
                "Downloader {free_peer_id} is not a peer anymore, removing it from the downloaders list"
            );
            peers_info.remove(&free_peer_id);
            return None;
        };

        drop(peers_info);
        self.mark_peer_as_busy(*free_peer_id).await;

        // Create and spawn Downloader Actor
        let downloader = Downloader::new(*free_peer_id, free_downloader_channels);
        Some(downloader)
    }

    // Creates a Downloader Actor from the best available peer
    // Returns None if no peer is available
    async fn get_best_available_downloader(&self) -> Option<Downloader> {
        let peer_channels = self
            .peer_table
            .get_peer_channels(&SUPPORTED_SNAP_CAPABILITIES)
            .await;

        let mut peers_info = self.peers_info.lock().await;

        for (peer_id, _peer_channels) in &peer_channels {
            if peers_info.contains_key(peer_id) {
                continue;
            }
            peers_info.insert(*peer_id, PeerInformation::default());
            debug!("{peer_id} added as downloader");
        }

        let free_downloaders = peers_info
            .clone()
            .into_iter()
            .filter(|(_downloader_id, peer_info)| peer_info.available)
            .collect::<Vec<_>>();

        if free_downloaders.is_empty() {
            // No available downloaders to offer
            return None;
        }

        let (free_peer_id, peer_info) = free_downloaders
            .iter()
            .max_by_key(|(_, peer_info)| peer_info.score)
            .expect("Infallible");

        if peer_info.score < MIN_PEER_SCORE_THRESHOLD {
            debug!("Best available peer doesn't meet minimun scoring, skipping it");
            return None;
        }

        let Some(free_downloader_channels) = self
            .peer_table
            .get_peer_channels(&SUPPORTED_SNAP_CAPABILITIES)
            .await
            .iter()
            .find_map(|(peer_id, peer_channels)| {
                peer_id.eq(&free_peer_id).then_some(peer_channels.clone())
            })
        else {
            debug!(
                "Downloader {free_peer_id} is not a peer anymore, removing it from the downloaders list"
            );
            peers_info.remove(&free_peer_id);
            return None;
        };

        drop(peers_info);
        self.mark_peer_as_busy(*free_peer_id).await;

        // Create and spawn Downloader Actor
        let downloader = Downloader::new(*free_peer_id, free_downloader_channels);
        Some(downloader)
    }

    /// It can happen that some peers are mistakenly marked as busy,
    /// this method is a failsafe that resets all peers to available
    /// after their are kept as busy for longer than 5 seconds.
    async fn reset_timed_out_busy_peers(&self) {
        let now = Instant::now();
        let mut last_peer_timeout_check = self.last_peer_timeout_check.lock().await;
        if now.duration_since(*last_peer_timeout_check) < Duration::from_secs(5) {
            return;
        }
        *last_peer_timeout_check = now;

        let mut peers_info = self.peers_info.lock().await;
        peers_info
            .iter_mut()
            .filter(|(_, i)| {
                !i.available
                    && Instant::now().duration_since(i.request_time.unwrap_or(Instant::now()))
                        > Duration::from_secs(2)
            })
            .for_each(|(peer_id, i)| {
                debug!("{peer_id} timed out, resetting it");
                i.available = true;
                i.request_time = None
            });
    }
}

fn format_duration(duration: Duration) -> String {
    let total_seconds = duration.as_secs();
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;

    format!("{hours:02}h {minutes:02}m {seconds:02}s")
}
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
}
