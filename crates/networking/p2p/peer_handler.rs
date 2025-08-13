use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
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
use ethrex_trie::{Node, verify_range};
use futures::stream::select_all;
use rand::{random, seq::SliceRandom};
use spawned_concurrency::tasks::{GenServer, GenServerHandle};
use tokio::sync::Mutex;

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
    snap_sync::downloader::{self, Downloader, DownloaderRequest},
    utils::{
        get_account_state_snapshot_file, get_account_state_snapshots_dir,
        get_account_storages_snapshot_file, get_account_storages_snapshots_dir,
    },
};
use tracing::{debug, error, info, trace, warn};
pub const PEER_REPLY_TIMEOUT: Duration = Duration::from_secs(15);
pub const PEER_SELECT_RETRY_ATTEMPTS: usize = 3;
pub const REQUEST_RETRY_ATTEMPTS: usize = 5;
pub const MAX_RESPONSE_BYTES: u64 = 512 * 1024;
pub const HASH_MAX: H256 = H256([0xFF; 32]);

pub const SNAP_LIMIT: usize = 20;

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
    pub peer_scores: Arc<Mutex<HashMap<H256, i64>>>,
}

pub enum BlockRequestOrder {
    OldToNew,
    NewToOld,
}

async fn ask_peer_head_number(
    peer_id: H256,
    peer_channel: &mut PeerChannels,
    sync_head: H256,
    retries: i32,
) -> Result<u64, String> {
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
        .map_err(|e| format!("Failed to send message to peer {peer_id}: {e}"))?;

    debug!("(Retry {retries}) Requesting sync head {sync_head} to peer {peer_id}");

    match tokio::time::timeout(Duration::from_millis(100), async move {
        peer_channel.receiver.lock().await.recv().await
    })
    .await
    {
        Ok(Some(RLPxMessage::BlockHeaders(BlockHeaders { id, block_headers }))) => {
            if id == request_id && !block_headers.is_empty() {
                let sync_head_number = block_headers.last().unwrap().number;
                trace!(
                    "Sync Log 12: Received sync head block headers from peer {peer_id}, sync head number {sync_head_number}"
                );
                Ok(sync_head_number)
            } else {
                Err(format!("Received unexpected response from peer {peer_id}"))
            }
        }
        Ok(None) => Err(format!("Error receiving message from peer {peer_id}")),
        Ok(_other_msgs) => Err("Received unexpected message from peer {peer_id}".into()),
        Err(_err) => Err(format!(
            "Timeout while waiting for sync head from {peer_id}"
        )),
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

    /// Helper method to record a succesful peer response as well as record previous failed responses from other peers
    /// We make this distinction for snap requests as the data we request might have become stale
    /// So we cannot know whether a peer returning an empty response is a failure until another peer returns the requested data
    async fn record_snap_peer_success(&self, succesful_peer_id: H256, mut peer_ids: HashSet<H256>) {
        // Reward succesful peer
        self.record_peer_success(succesful_peer_id).await;
        // Penalize previous peers that returned empty/invalid responses
        peer_ids.remove(&succesful_peer_id);
        for peer_id in peer_ids {
            info!(
                "[SYNCING] Penalizing peer {peer_id} as it failed to return data cornfirmed as non-stale"
            );
            self.record_peer_failure(peer_id).await;
        }
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
        scores: &mut HashMap<H256, i64>,
    ) -> Option<(H256, PeerChannels)> {
        let (mut free_peer_id, mut free_peer_channel) = self
            .peer_table
            .get_peer_channels(capabilities)
            .await
            .first()
            .unwrap()
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

        Some((free_peer_id, free_peer_channel.clone()))
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

        let mut ret = Vec::<BlockHeader>::new();

        let peers_table = self
            .peer_table
            .get_peer_channels(&SUPPORTED_ETH_CAPABILITIES)
            .await;

        let mut sync_head_number = 0_u64;

        let sync_head_number_retrieval_start = SystemTime::now();

        info!("Retrieving sync head block number from peers");

        let mut retries = 1;

        while sync_head_number == 0 {
            for (peer_id, mut peer_channel) in peers_table.clone() {
                match ask_peer_head_number(peer_id, &mut peer_channel, sync_head, retries).await {
                    Ok(number) => {
                        sync_head_number = number;
                    }
                    Err(err) => {
                        trace!(
                            "Sync Log 13: Failed to retrieve sync head block number from peer {peer_id}: {err}"
                        );
                    }
                }
            }

            retries += 1;
        }

        let sync_head_number_retrieval_elapsed = sync_head_number_retrieval_start
            .elapsed()
            .expect("Failed to get elapsed time");

        info!("Sync head block number retrieved");

        *METRICS.time_to_retrieve_sync_head_block.lock().await =
            Some(sync_head_number_retrieval_elapsed);
        *METRICS.sync_head_block.lock().await = sync_head_number;
        *METRICS.headers_to_download.lock().await = sync_head_number + 1;
        *METRICS.sync_head_hash.lock().await = sync_head;

        let block_count = sync_head_number + 1 - start;
        let chunk_count = if block_count < 800_u64 { 1 } else { 800_u64 };

        // 2) partition the amount of headers in `K` tasks
        let chunk_limit = block_count / chunk_count as u64;

        // list of tasks to be executed
        let mut tasks_queue_not_started = VecDeque::<(u64, u64)>::new();

        for i in 0..(chunk_count as u64) {
            tasks_queue_not_started.push_back((i * chunk_limit + start, chunk_limit));
        }

        // Push the reminder
        if block_count % chunk_count as u64 != 0 {
            tasks_queue_not_started.push_back((
                chunk_count as u64 * chunk_limit + start,
                block_count % chunk_count as u64,
            ));
        }

        let mut downloaded_count = 0_u64;

        // channel to send the tasks to the peers
        let (task_sender, mut task_receiver) =
            tokio::sync::mpsc::channel::<(Vec<BlockHeader>, H256, PeerChannels, u64, u64)>(1000);

        let mut current_show = 0;
        let mut downloaders: BTreeMap<H256, bool> = BTreeMap::from_iter(
            peers_table
                .iter()
                .map(|(peer_id, _peer_data)| (*peer_id, true)),
        );

        // 3) create tasks that will request a chunk of headers from a peer

        info!("Starting to download block headers from peers");

        *METRICS.headers_download_start_time.lock().await = Some(SystemTime::now());

        let mut last_metrics_update = SystemTime::now();

        loop {
            let new_last_metrics_update = last_metrics_update.elapsed().unwrap();

            if new_last_metrics_update >= Duration::from_secs(1) {
                *METRICS.header_downloads_tasks_queued.lock().await =
                    tasks_queue_not_started.len() as u64;

                *METRICS.total_header_downloaders.lock().await = downloaders.len() as u64;
            }

            if let Ok((headers, peer_id, _peer_channel, startblock, previous_chunk_limit)) =
                task_receiver.try_recv()
            {
                if headers.is_empty() {
                    trace!("Failed to download chunk from peer {peer_id}");

                    downloaders.entry(peer_id).and_modify(|downloader_is_free| {
                        *downloader_is_free = true; // mark the downloader as free
                    });

                    debug!("Downloader {peer_id} freed");

                    // reinsert the task to the queue
                    tasks_queue_not_started.push_back((startblock, previous_chunk_limit));

                    continue; // Retry with the next peer
                }

                downloaded_count += headers.len() as u64;

                if new_last_metrics_update >= Duration::from_secs(1) {
                    *METRICS.downloaded_headers.lock().await = downloaded_count;
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

                downloaders.entry(peer_id).and_modify(|downloader_is_free| {
                    *downloader_is_free = true; // mark the downloader as free
                });
                debug!("Downloader {peer_id} freed");
            }

            let peer_channels = self
                .peer_table
                .get_peer_channels(&SUPPORTED_ETH_CAPABILITIES)
                .await;

            for (peer_id, _peer_channels) in &peer_channels {
                if downloaders.contains_key(peer_id) {
                    // Peer is already in the downloaders list, skip it
                    continue;
                }

                downloaders.insert(*peer_id, true);

                debug!("{peer_id} added as downloader");
            }

            let free_downloaders = downloaders
                .clone()
                .into_iter()
                .filter(|(_downloader_id, downloader_is_free)| *downloader_is_free)
                .collect::<Vec<_>>();

            if new_last_metrics_update >= Duration::from_secs(1) {
                *METRICS.free_header_downloaders.lock().await = free_downloaders.len() as u64;
            }

            if free_downloaders.is_empty() {
                continue;
            }

            let Some(free_peer_id) = free_downloaders
                .get(random::<usize>() % free_downloaders.len())
                .map(|(peer_id, _)| *peer_id)
            else {
                debug!("(2) No free downloaders available, waiting for a peer to finish, retrying");
                continue;
            };

            let Some(mut free_downloader_channels) =
                peer_channels.iter().find_map(|(peer_id, peer_channels)| {
                    peer_id.eq(&free_peer_id).then_some(peer_channels.clone())
                })
            else {
                // The free downloader is not a peer of us anymore.
                debug!(
                    "Downloader {free_peer_id} is not a peer anymore, removing it from the downloaders list"
                );
                downloaders.remove(&free_peer_id);
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

            downloaders
                .entry(free_peer_id)
                .and_modify(|downloader_is_free| {
                    *downloader_is_free = false; // mark the downloader as busy
                });

            debug!("Downloader {free_peer_id} is now busy");

            // run download_chunk_from_peer in a different Tokio task
            let _download_result = tokio::spawn(async move {
                trace!(
                    "Sync Log 5: Requesting block headers from peer {free_peer_id}, chunk_limit: {chunk_limit}"
                );
                debug!(
                    "Requesting block headers from peer {free_peer_id}, chunk_limit: {chunk_limit}"
                );

                let headers = Self::download_chunk_from_peer(
                    free_peer_id,
                    &mut free_downloader_channels,
                    startblock,
                    chunk_limit,
                )
                .await
                .inspect_err(|err| {
                    trace!("Sync Log 6: {free_peer_id} failed to download chunk: {err}")
                })
                .unwrap_or_default();

                tx.send((
                    headers,
                    free_peer_id,
                    free_downloader_channels,
                    startblock,
                    chunk_limit,
                ))
                .await
                .unwrap();
            });

            // 4) assign the tasks to the peers
            //     4.1) launch a tokio task with the chunk and a peer ready (giving the channels)

            // TODO!!! spawn a task to download the chunk, calling `download_chunk_from_peer`

            if new_last_metrics_update >= Duration::from_secs(1) {
                last_metrics_update = SystemTime::now();
            }
        }

        *METRICS.header_downloads_tasks_queued.lock().await = tasks_queue_not_started.len() as u64;
        *METRICS.free_header_downloaders.lock().await = downloaders.len() as u64;
        *METRICS.total_header_downloaders.lock().await = downloaders.len() as u64;
        *METRICS.downloaded_headers.lock().await = downloaded_count;

        let elapsed = start_time.elapsed().expect("Failed to get elapsed time");

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

        // 4) assign the tasks to the peers
        //     4.1) launch a tokio task with the chunk and a peer ready (giving the channels)
        //     4.2) mark the peer as busy
        //     4.3) wait for the response and handle it

        // 5) loop until all the chunks are received (retry to get the chunks that failed)

        ret.sort_by(|x, y| x.number.cmp(&y.number));
        // info!("Last header downloaded: {:?} ?? ", ret.last().unwrap());
        Some(ret)
        // std::process::exit(0);
    }

    /// given a peer id, a chunk start and a chunk limit, requests the block headers from the peer
    ///
    /// If it fails, returns an error message.
    async fn download_chunk_from_peer(
        peer_id: H256,
        peer_channel: &mut PeerChannels,
        startblock: u64,
        chunk_limit: u64,
    ) -> Result<Vec<BlockHeader>, String> {
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
            .map_err(|e| format!("Failed to send message to peer {peer_id}: {e}"))?;

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
        .map_err(|_| "Failed to receive block headers")?
        .ok_or("Block no received".to_owned())?;

        if are_block_headers_chained(&block_headers, &BlockRequestOrder::OldToNew) {
            Ok(block_headers)
        } else {
            warn!("[SYNCING] Received invalid headers from peer: {peer_id}");
            Err("Invalid block headers".into())
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
    pub async fn request_account_range(&self, state_root: H256, start: H256, limit: H256) {
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
            .expect("we just inserted some elements");
        last_task.1 = limit;

        // 2) request the chunks from peers
        let peers_table = self
            .peer_table
            .get_peer_channels(&SUPPORTED_SNAP_CAPABILITIES)
            .await;

        let mut downloaded_count = 0_u64;
        let mut all_account_hashes = Vec::new();
        let mut all_accounts_state = Vec::new();

        // channel to send the tasks to the peers
        let (task_sender, mut task_receiver) =
            tokio::sync::mpsc::channel::<(Vec<AccountRangeUnit>, H256, Option<(H256, H256)>)>(1000);

        // channel to send the result of dumping accounts
        let (dump_account_result_sender, mut dump_account_result_receiver) =
            tokio::sync::mpsc::channel::<Result<(), AccountDumpError>>(1000);

        let mut downloaders: BTreeMap<H256, bool> = BTreeMap::from_iter(
            peers_table
                .iter()
                .map(|(peer_id, _peer_data)| (*peer_id, true)),
        );

        info!("Starting to download account ranges from peers");

        *METRICS.account_tries_download_start_time.lock().await = Some(SystemTime::now());

        let mut last_metrics_update = SystemTime::now();
        let mut completed_tasks = 0;
        let mut scores = self.peer_scores.lock().await;
        let mut chunk_file = 0;

        // TODO: handle this error
        let account_state_snapshots_dir = get_account_state_snapshots_dir()
            .expect("Failed to get account_state_snapshots directory");
        loop {
            if all_accounts_state.len() * size_of::<AccountState>() >= 1024 * 1024 * 1024 * 8 {
                let current_account_hashes = std::mem::take(&mut all_account_hashes);
                let current_account_states = std::mem::take(&mut all_accounts_state);

                let account_state_chunk = current_account_hashes
                    .into_iter()
                    .zip(current_account_states)
                    .collect::<Vec<(H256, AccountState)>>()
                    .encode_to_vec();

                if !std::fs::exists(&account_state_snapshots_dir).expect("Failed") {
                    std::fs::create_dir_all(&account_state_snapshots_dir)
                        .expect("Failed to create accounts_state_snapshot dir");
                }

                let account_state_snapshots_dir_cloned = account_state_snapshots_dir.clone();
                let dump_account_result_sender_cloned = dump_account_result_sender.clone();
                tokio::task::spawn(async move {
                    let path = get_account_state_snapshot_file(
                        account_state_snapshots_dir_cloned,
                        chunk_file,
                    );
                    // TODO: check the error type and handle it properly
                    let result =
                        std::fs::write(path.clone(), account_state_chunk.clone()).map_err(|_| {
                            AccountDumpError {
                                path,
                                contents: account_state_chunk,
                            }
                        });
                    dump_account_result_sender_cloned.send(result).await;
                })
                .await
                .unwrap();

                chunk_file += 1;
            }

            let new_last_metrics_update = last_metrics_update.elapsed().unwrap();

            if new_last_metrics_update >= Duration::from_secs(1) {
                *METRICS.accounts_downloads_tasks_queued.lock().await =
                    tasks_queue_not_started.len() as u64;
                *METRICS.total_accounts_downloaders.lock().await = downloaders.len() as u64;
                *METRICS.downloaded_account_tries.lock().await = downloaded_count;
            }

            if let Ok((accounts, peer_id, chunk_start_end)) = task_receiver.try_recv() {
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
                    let peer_score = scores.entry(peer_id).or_default();
                    *peer_score -= 1;
                    continue;
                }
                let peer_score = scores.entry(peer_id).or_default();
                *peer_score += 1;

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

                downloaders.entry(peer_id).and_modify(|downloader_is_free| {
                    *downloader_is_free = true;
                });
            }

            // Check if any dump account task finished
            // TODO: consider tracking in-flight (dump) tasks
            if let Ok(dump_account_result) = dump_account_result_receiver.try_recv() {
                if let Err(dump_account_data) = dump_account_result {
                    // If the dumping failed, retry it
                    let dump_account_result_sender_cloned = dump_account_result_sender.clone();
                    tokio::task::spawn(async move {
                        let AccountDumpError { path, contents } = dump_account_data;
                        // Dump the account data
                        // TODO: check the error type and handle it properly
                        let result = std::fs::write(path.clone(), contents.clone())
                            .map_err(|_| AccountDumpError { path, contents });
                        // Send the result through the channel
                        dump_account_result_sender_cloned.send(result).await;
                    })
                    .await
                    .unwrap();
                }
            }

            // TODO: move higher?
            let Some((chunk_start, chunk_end)) = tasks_queue_not_started.pop_front() else {
                if completed_tasks >= chunk_count {
                    info!("All account ranges downloaded successfully");
                    break;
                }
                continue;
            };

            let peer_channels = self
                .peer_table
                .get_peer_channels(&SUPPORTED_SNAP_CAPABILITIES)
                .await;

            for (peer_id, _peer_channels) in &peer_channels {
                if downloaders.contains_key(peer_id) {
                    continue;
                }
                downloaders.insert(*peer_id, true);
                debug!("{peer_id} added as downloader");
            }

            let Some(mut downloader) = self.get_available_downloader(&mut downloaders).await else {
                debug!("No available downloader found, retrying");
                continue;
            };

            downloader
                .cast(DownloaderRequest::AccountRange {
                    task_sender: task_sender.clone(),
                    root_hash: state_root,
                    starting_hash: chunk_start,
                    limit_hash: chunk_end,
                })
                .await
                .unwrap();

            if new_last_metrics_update >= Duration::from_secs(1) {
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

            if !std::fs::exists(&account_state_snapshots_dir).expect("Failed") {
                std::fs::create_dir_all(&account_state_snapshots_dir)
                    .expect("Failed to create accounts_state_snapshot dir");
            }

            tokio::task::spawn(async move {
                let path = get_account_state_snapshot_file(account_state_snapshots_dir, chunk_file);
                std::fs::write(path, account_state_chunk).unwrap_or_else(|_| {
                    panic!("Failed to write account_state_snapshot chunk {chunk_file}")
                });
            })
            .await
            .unwrap();
        }

        *METRICS.accounts_downloads_tasks_queued.lock().await =
            tasks_queue_not_started.len() as u64;
        *METRICS.total_accounts_downloaders.lock().await = downloaders.len() as u64;
        *METRICS.downloaded_account_tries.lock().await = downloaded_count;
        *METRICS.free_accounts_downloaders.lock().await = downloaders.len() as u64;
        *METRICS.account_tries_download_end_time.lock().await = Some(SystemTime::now());
    }

    /// Requests bytecodes for the given code hashes
    /// Returns the bytecodes or None if:
    /// - There are no available peers (the node just started up or was rejected by all other nodes)
    /// - No peer returned a valid response in the given time and retry limits
    pub async fn request_bytecodes(&self, all_bytecode_hashes: &[H256]) -> Option<Vec<Bytes>> {
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
            .expect("we just inserted some elements");
        last_task.1 = all_bytecode_hashes.len();

        // 2) request the chunks from peers
        let peers_table = self
            .peer_table
            .get_peer_channels(&SUPPORTED_SNAP_CAPABILITIES)
            .await;

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

        let mut downloaders: BTreeMap<H256, bool> = BTreeMap::from_iter(
            peers_table
                .iter()
                .map(|(peer_id, _peer_data)| (*peer_id, true)),
        );

        info!("Starting to download bytecodes from peers");

        *METRICS.bytecodes_to_download.lock().await = all_bytecode_hashes.len() as u64;
        *METRICS.bytecode_download_start_time.lock().await = Some(SystemTime::now());

        let mut last_metrics_update = SystemTime::now();
        let mut completed_tasks = 0;
        let mut scores = self.peer_scores.lock().await;

        loop {
            let new_last_metrics_update = last_metrics_update.elapsed().unwrap();

            if new_last_metrics_update >= Duration::from_secs(1) {
                *METRICS.bytecode_downloads_tasks_queued.lock().await =
                    tasks_queue_not_started.len() as u64;
                *METRICS.total_bytecode_downloaders.lock().await = downloaders.len() as u64;
                *METRICS.downloaded_bytecodes.lock().await = downloaded_count;
            }

            if let Ok(result) = task_receiver.try_recv() {
                let TaskResult {
                    start_index,
                    bytecodes,
                    peer_id,
                    remaining_start,
                    remaining_end,
                } = result;
                if remaining_start < remaining_end {
                    tasks_queue_not_started.push_back((remaining_start, remaining_end));
                } else {
                    completed_tasks += 1;
                }
                if bytecodes.is_empty() {
                    let peer_score = scores.entry(peer_id).or_default();
                    *peer_score -= 1;
                    continue;
                }

                downloaded_count += bytecodes.len() as u64;

                let peer_score = scores.entry(peer_id).or_default();
                *peer_score += 1;

                debug!(
                    "Downloaded {} bytecodes from peer {peer_id} (current count: {downloaded_count})",
                    bytecodes.len(),
                );
                for (i, bytecode) in bytecodes.into_iter().enumerate() {
                    all_bytecodes[start_index + i] = bytecode;
                }

                downloaders.entry(peer_id).and_modify(|downloader_is_free| {
                    *downloader_is_free = true;
                });
            }

            let peer_channels = self
                .peer_table
                .get_peer_channels(&SUPPORTED_SNAP_CAPABILITIES)
                .await;

            for (peer_id, _peer_channels) in &peer_channels {
                if downloaders.contains_key(peer_id) {
                    continue;
                }
                downloaders.insert(*peer_id, true);
                debug!("{peer_id} added as downloader");
            }

            let free_downloaders = downloaders
                .clone()
                .into_iter()
                .filter(|(_downloader_id, downloader_is_free)| *downloader_is_free)
                .collect::<Vec<_>>();

            if new_last_metrics_update >= Duration::from_secs(1) {
                *METRICS.free_bytecode_downloaders.lock().await = free_downloaders.len() as u64;
            }

            if free_downloaders.is_empty() {
                continue;
            }

            let (mut free_peer_id, _) = free_downloaders[0];

            for (peer_id, _) in free_downloaders.iter() {
                let peer_id_score = scores.get(&peer_id).unwrap_or(&0);
                let max_peer_id_score = scores.get(&free_peer_id).unwrap_or(&0);
                if peer_id_score >= max_peer_id_score {
                    free_peer_id = *peer_id;
                }
            }

            // let peer_id_score = scores.get(&free_peer_id).unwrap_or(&0);

            // let mut score_values : Vec<i64> = Vec::from_iter(scores.values().cloned());
            // score_values.sort();

            // let middle_value = score_values.get(score_values.len() / 2).unwrap_or(&0);

            // if (*peer_id_score < 0) && (*peer_id_score < *middle_value) {
            //     continue;
            // }

            let Some(free_downloader_channels) =
                peer_channels.iter().find_map(|(peer_id, peer_channels)| {
                    peer_id.eq(&free_peer_id).then_some(peer_channels.clone())
                })
            else {
                debug!(
                    "Downloader {free_peer_id} is not a peer anymore, removing it from the downloaders list"
                );
                downloaders.remove(&free_peer_id);
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
            downloaders
                .entry(free_peer_id)
                .and_modify(|downloader_is_free| {
                    *downloader_is_free = false;
                });
            debug!("Downloader {free_peer_id} is now busy");

            let hashes_to_request: Vec<_> = all_bytecode_hashes
                .iter()
                .skip(chunk_start)
                .take((chunk_end - chunk_start).min(MAX_BYTECODES_REQUEST_SIZE))
                .copied()
                .collect();

            let mut free_downloader_channels_clone = free_downloader_channels.clone();
            tokio::spawn(async move {
                let empty_task_result = TaskResult {
                    start_index: chunk_start,
                    bytecodes: vec![],
                    peer_id: free_peer_id,
                    remaining_start: chunk_start,
                    remaining_end: chunk_end,
                };
                debug!(
                    "Requesting bytecode from peer {free_peer_id}, chunk: {chunk_start:?} - {chunk_end:?}"
                );
                let request_id = rand::random();
                let request = RLPxMessage::GetByteCodes(GetByteCodes {
                    id: request_id,
                    hashes: hashes_to_request.clone(),
                    bytes: MAX_RESPONSE_BYTES,
                });
                let mut receiver = free_downloader_channels_clone.receiver.lock().await;
                if let Err(err) = (&mut free_downloader_channels_clone.connection)
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
                    let validated_codes: Vec<Bytes> = tokio::task::spawn_blocking(move || {
                        codes
                            .into_iter()
                            .zip(hashes_to_request)
                            .take_while(|(b, hash)| keccak_hash::keccak(b) == *hash)
                            .map(|(b, _hash)| b)
                            .collect()
                    })
                    .await
                    .unwrap();
                    let result = TaskResult {
                        start_index: chunk_start,
                        remaining_start: chunk_start + validated_codes.len(),
                        bytecodes: validated_codes,
                        peer_id: free_peer_id,
                        remaining_end: chunk_end,
                    };
                    tx.send(result).await.ok();
                } else {
                    tracing::error!("Failed to get bytecode");
                    tx.send(empty_task_result).await.ok();
                }
            });

            if new_last_metrics_update >= Duration::from_secs(1) {
                last_metrics_update = SystemTime::now();
            }
        }

        *METRICS.bytecode_downloads_tasks_queued.lock().await =
            tasks_queue_not_started.len() as u64;
        *METRICS.total_bytecode_downloaders.lock().await = downloaders.len() as u64;
        *METRICS.downloaded_bytecodes.lock().await = downloaded_count;
        *METRICS.free_bytecode_downloaders.lock().await = downloaders.len() as u64;

        info!(
            "Finished downloading bytecodes, total bytecodes: {}",
            all_bytecode_hashes.len()
        );

        Some(all_bytecodes)
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
        mut chunk_index: u64,
    ) -> u64 {
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

        // 2) request the chunks from peers
        let peers_table = self
            .peer_table
            .get_peer_channels(&SUPPORTED_SNAP_CAPABILITIES)
            .await;

        let mut downloaded_count = 0_u64;
        let mut all_account_storages = vec![vec![]; account_storage_roots.len()];

        struct TaskResult {
            start_index: usize,
            account_storages: Vec<Vec<(H256, U256)>>,
            peer_id: H256,
            remaining_start: usize,
            remaining_end: usize,
            remaining_hash_range: (H256, Option<H256>),
        }

        // channel to send the tasks to the peers
        let (task_sender, mut task_receiver) = tokio::sync::mpsc::channel::<TaskResult>(1000);

        let mut downloaders: BTreeMap<H256, bool> = BTreeMap::from_iter(
            peers_table
                .iter()
                .map(|(peer_id, _peer_data)| (*peer_id, true)),
        );

        info!("Starting to download storage ranges from peers");
        *METRICS.storage_tries_to_download.lock().await = account_storage_roots.len() as u64;
        METRICS
            .storage_tries_download_start_time
            .lock()
            .await
            .replace(SystemTime::now());

        let mut last_metrics_update = SystemTime::now();
        let mut task_count = tasks_queue_not_started.len();
        let mut completed_tasks = 0;

        let mut scores = self.peer_scores.lock().await;

        let account_storages_snapshots_dir = get_account_storages_snapshots_dir().unwrap();
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

                if !std::fs::exists(&account_storages_snapshots_dir).expect("Failed") {
                    std::fs::create_dir_all(&account_storages_snapshots_dir)
                        .expect("Failed to create accounts_state_snapshot dir");
                }
                let account_storages_snapshots_dir_cloned = account_storages_snapshots_dir.clone();
                tokio::task::spawn(async move {
                    let path = get_account_storages_snapshot_file(
                        account_storages_snapshots_dir_cloned,
                        chunk_index,
                    );
                    std::fs::write(path, snapshot).unwrap_or_else(|_| {
                        panic!("Failed to write account_storages_snapshot chunk {chunk_index}")
                    });
                })
                .await
                .expect("");

                chunk_index += 1;
            }

            let new_last_metrics_update = last_metrics_update.elapsed().unwrap();

            if new_last_metrics_update >= Duration::from_secs(1) {
                *METRICS.storages_downloads_tasks_queued.lock().await =
                    tasks_queue_not_started.len() as u64;
                *METRICS.total_storages_downloaders.lock().await = downloaders.len() as u64;
                *METRICS.downloaded_storage_tries.lock().await = downloaded_count;
            }

            if let Ok(result) = task_receiver.try_recv() {
                let TaskResult {
                    start_index,
                    mut account_storages,
                    peer_id,
                    remaining_start,
                    remaining_end,
                    remaining_hash_range: (hash_start, hash_end),
                } = result;
                completed_tasks += 1;

                downloaders.entry(peer_id).and_modify(|downloader_is_free| {
                    *downloader_is_free = true;
                });

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

                        let slot_count = account_storages.last().map(|v| v.len()).unwrap().max(1);
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
                                start_hash: start_hash,
                                end_hash: Some(end_hash),
                            };
                            tasks_queue_not_started.push_back(task);
                            task_count += 1;
                        }
                        debug!("Split big storage account into {chunk_count} chunks.");
                    }
                }

                if account_storages.is_empty() {
                    let peer_score = scores.entry(peer_id).or_default();
                    *peer_score -= 1;
                    continue;
                }
                if let Some(hash_end) = hash_end {
                    // This is a big storage account, and the range might be empty
                    if account_storages[0].len() == 1 && account_storages[0][0].0 > hash_end {
                        continue;
                    }
                }

                let peer_score = scores.entry(peer_id).or_default();
                if *peer_score < 10 {
                    *peer_score += 1;
                }

                downloaded_count += account_storages.len() as u64;
                // If we didn't finish downloading the account, don't count it
                if !hash_start.is_zero() {
                    downloaded_count -= 1;
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

            let peer_channels = self
                .peer_table
                .get_peer_channels(&SUPPORTED_SNAP_CAPABILITIES)
                .await;

            for (peer_id, _peer_channels) in &peer_channels {
                if downloaders.contains_key(peer_id) {
                    continue;
                }
                downloaders.insert(*peer_id, true);
                debug!("{peer_id} added as downloader");
            }

            let free_downloaders = downloaders
                .clone()
                .into_iter()
                .filter(|(_downloader_id, downloader_is_free)| *downloader_is_free)
                .collect::<Vec<_>>();

            if new_last_metrics_update >= Duration::from_secs(1) {
                *METRICS.free_storages_downloaders.lock().await = free_downloaders.len() as u64;
            }

            if free_downloaders.is_empty() {
                continue;
            }

            let (mut free_peer_id, _) = free_downloaders[0];

            for (peer_id, _) in free_downloaders.iter() {
                let peer_id_score = scores.get(&peer_id).unwrap_or(&0);
                let max_peer_id_score = scores.get(&free_peer_id).unwrap_or(&0);
                if peer_id_score >= max_peer_id_score {
                    free_peer_id = *peer_id;
                }
            }

            let Some(free_downloader_channels) =
                peer_channels.iter().find_map(|(peer_id, peer_channels)| {
                    peer_id.eq(&free_peer_id).then_some(peer_channels.clone())
                })
            else {
                debug!(
                    "Downloader {free_peer_id} is not a peer anymore, removing it from the downloaders list"
                );
                downloaders.remove(&free_peer_id);
                continue;
            };

            let Some(task) = tasks_queue_not_started.pop_front() else {
                if completed_tasks >= task_count {
                    info!("All account storages downloaded successfully");
                    break;
                }
                continue;
            };

            let tx = task_sender.clone();
            downloaders
                .entry(free_peer_id)
                .and_modify(|downloader_is_free| {
                    *downloader_is_free = false;
                });
            debug!("Downloader {free_peer_id} is now busy");

            let mut free_downloader_channels_clone = free_downloader_channels.clone();

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

            tokio::spawn(async move {
                let start = task.start_index;
                let end = task.end_index;
                let start_hash = task.start_hash;

                let empty_task_result = TaskResult {
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
                if let Err(err) = (&mut free_downloader_channels_clone.connection)
                    .cast(CastMessage::BackendMessage(request))
                    .await
                {
                    error!("Failed to send message to peer: {err:?}");
                    tx.send(empty_task_result).await.ok();
                    return;
                }
                let request_result = tokio::time::timeout(Duration::from_secs(2), async move {
                    loop {
                        match receiver.recv().await {
                            Some(RLPxMessage::StorageRanges(StorageRanges {
                                id,
                                slots,
                                proof,
                            })) if id == request_id => return Some((slots, proof)),
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
                    return;
                };
                if slots.is_empty() && proof.is_empty() {
                    tx.send(empty_task_result).await.ok();
                    tracing::debug!("Received empty account range");
                    return;
                }
                // Check we got some data and no more than the requested amount
                if slots.len() > chunk_storage_roots.len() || slots.is_empty() {
                    tx.send(empty_task_result).await.ok();
                    return;
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
                        tx.send(empty_task_result).await.ok();
                        return;
                    }
                    let encoded_values = next_account_slots
                        .iter()
                        .map(|slot| slot.data.encode_to_vec())
                        .collect::<Vec<_>>();
                    let hashed_keys: Vec<_> =
                        next_account_slots.iter().map(|slot| slot.hash).collect();

                    let storage_root = storage_roots.next().unwrap();

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
                            return;
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
                        tx.send(empty_task_result).await.ok();
                        return;
                    }

                    account_storages.push(
                        next_account_slots
                            .iter()
                            .map(|slot| (slot.hash, slot.data))
                            .collect(),
                    );
                }
                let (remaining_start, remaining_end, remaining_start_hash) = if should_continue {
                    let (last_hash, _) = account_storages.last().unwrap().last().unwrap();
                    let next_hash_u256 =
                        U256::from_big_endian(&last_hash.0).saturating_add(1.into());
                    let next_hash = H256::from_uint(&next_hash_u256);
                    (start + account_storages.len() - 1, end, next_hash)
                } else {
                    (start + account_storages.len(), end, H256::zero())
                };
                let task_result = TaskResult {
                    start_index: start,
                    account_storages,
                    peer_id: free_peer_id,
                    remaining_start,
                    remaining_end,
                    remaining_hash_range: (remaining_start_hash, task.end_hash),
                };
                tx.send(task_result).await.ok();
            });

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
            all_account_storages = vec![vec![]; account_storage_roots.len()];

            let snapshot = current_account_hashes
                .into_iter()
                .zip(current_account_storages)
                .collect::<Vec<_>>()
                .encode_to_vec();

            if !std::fs::exists(&account_storages_snapshots_dir).expect("Failed") {
                std::fs::create_dir_all(&account_storages_snapshots_dir)
                    .expect("Failed to create accounts_state_snapshot dir");
            }
            let account_storages_snapshots_dir_cloned = account_storages_snapshots_dir.clone();
            tokio::task::spawn(async move {
                let path = get_account_storages_snapshot_file(
                    account_storages_snapshots_dir_cloned,
                    chunk_index,
                );
                std::fs::write(path, snapshot).unwrap_or_else(|_| {
                    panic!("Failed to write account_storages_snapshot chunk {chunk_index}")
                });
            })
            .await
            .expect("");
        }

        *METRICS.storages_downloads_tasks_queued.lock().await =
            tasks_queue_not_started.len() as u64;
        *METRICS.total_storages_downloaders.lock().await = downloaders.len() as u64;
        *METRICS.downloaded_storage_tries.lock().await = downloaded_count;
        *METRICS.free_storages_downloaders.lock().await = downloaders.len() as u64;
        METRICS
            .storage_tries_download_end_time
            .lock()
            .await
            .replace(SystemTime::now());

        let total_slots = all_account_storages.iter().map(|s| s.len()).sum::<usize>();
        info!("Finished downloading account ranges, total storage slots: {total_slots}");

        chunk_index + 1
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
        let expected_nodes = paths.len();
        // Keep track of peers we requested from so we can penalize unresponsive peers when we get a response
        // This is so we avoid penalizing peers due to requesting stale data
        let mut peer_ids = HashSet::new();
        for _ in 0..REQUEST_RETRY_ATTEMPTS {
            let request_id = rand::random();
            let request = RLPxMessage::GetTrieNodes(GetTrieNodes {
                id: request_id,
                root_hash: state_root,
                // [acc_path, acc_path,...] -> [[acc_path], [acc_path]]
                paths: paths
                    .iter()
                    .map(|vec| vec![Bytes::from(vec.encode_compact())])
                    .collect(),
                bytes: MAX_RESPONSE_BYTES,
            });
            let (peer_id, mut peer_channel) = self
                .get_peer_channel_with_retry(&SUPPORTED_SNAP_CAPABILITIES)
                .await?;
            peer_ids.insert(peer_id);
            let mut receiver = peer_channel.receiver.lock().await;
            if let Err(err) = peer_channel
                .connection
                .cast(CastMessage::BackendMessage(request))
                .await
            {
                debug!("Failed to send message to peer: {err:?}");
                continue;
            }
            if let Some(nodes) = tokio::time::timeout(PEER_REPLY_TIMEOUT, async move {
                loop {
                    match receiver.recv().await {
                        Some(RLPxMessage::TrieNodes(TrieNodes { id, nodes }))
                            if id == request_id =>
                        {
                            return Some(nodes);
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
            .and_then(|nodes| {
                (!nodes.is_empty() && nodes.len() <= expected_nodes)
                    .then(|| {
                        nodes
                            .iter()
                            .map(|node| Node::decode_raw(node))
                            .collect::<Result<Vec<_>, _>>()
                            .ok()
                    })
                    .flatten()
            }) {
                self.record_snap_peer_success(peer_id, peer_ids).await;
                return Some(nodes);
            }
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
        // Keep track of peers we requested from so we can penalize unresponsive peers when we get a response
        // This is so we avoid penalizing peers due to requesting stale data
        let mut peer_ids = HashSet::new();
        for _ in 0..REQUEST_RETRY_ATTEMPTS {
            let request_id = rand::random();
            let expected_nodes = paths.iter().fold(0, |acc, item| acc + item.1.len());
            let request = RLPxMessage::GetTrieNodes(GetTrieNodes {
                id: request_id,
                root_hash: state_root,
                // {acc_path: [path, path, ...]} -> [[acc_path, path, path, ...]]
                paths: paths
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
                    .collect(),
                bytes: MAX_RESPONSE_BYTES,
            });
            let (peer_id, mut peer_channel) = self
                .get_peer_channel_with_retry(&SUPPORTED_SNAP_CAPABILITIES)
                .await?;
            peer_ids.insert(peer_id);
            let mut receiver = peer_channel.receiver.lock().await;
            if let Err(err) = peer_channel
                .connection
                .cast(CastMessage::BackendMessage(request))
                .await
            {
                debug!("Failed to send message to peer: {err:?}");
                continue;
            }
            if let Some(nodes) = tokio::time::timeout(PEER_REPLY_TIMEOUT, async move {
                loop {
                    match receiver.recv().await {
                        Some(RLPxMessage::TrieNodes(TrieNodes { id, nodes }))
                            if id == request_id =>
                        {
                            return Some(nodes);
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
            .and_then(|nodes| {
                (!nodes.is_empty() && nodes.len() <= expected_nodes)
                    .then(|| {
                        nodes
                            .iter()
                            .map(|node| Node::decode_raw(node))
                            .collect::<Result<Vec<_>, _>>()
                            .ok()
                    })
                    .flatten()
            }) {
                self.record_snap_peer_success(peer_id, peer_ids).await;
                return Some(nodes);
            }
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
        peer_channel: &mut PeerChannels,
        block_number: u64,
    ) -> Option<BlockHeader> {
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
        peer_channel
            .connection
            .cast(CastMessage::BackendMessage(request.clone()))
            .await
            .map_err(|e| format!("Failed to send message to peer. Error: {e}"))
            .inspect_err(|err| error!(err))
            .expect("############### Error peer_channel connection");
        let response = tokio::time::timeout(Duration::from_secs(5), async move {
            let response = receiver.recv().await;
            if response.is_none() {
                error!("############### Error Message");
            };
            response.unwrap()
        })
        .await;

        match response {
            Ok(RLPxMessage::BlockHeaders(BlockHeaders { id, block_headers })) => {
                if id == request_id && !block_headers.is_empty() {
                    return Some(block_headers.last().expect("############### Error").clone());
                }
            }
            Ok(_other_msgs) => {
                info!("Received unexpected message from peer");
            }
            Err(_err) => {
                info!("Timeout while waiting for sync head from peer");
            }
        }

        None
    }

    // Creates a Downloader Actor from the best available peer
    // Returns None if no peer is available
    async fn get_available_downloader(
        &self,
        downloaders: &mut BTreeMap<H256, bool>,
    ) -> Option<GenServerHandle<Downloader>> {
        // TODO: check if downloaders can be instantiated here instead of reciving it as a parameter
        let free_downloaders = downloaders
            .clone()
            .into_iter()
            .filter(|(_downloader_id, downloader_is_free)| *downloader_is_free)
            .collect::<Vec<_>>();

        // TODO: move metric elsewhere
        // if new_last_metrics_update >= Duration::from_secs(1) {
        //     *METRICS.free_accounts_downloaders.lock().await = free_downloaders.len() as u64;
        // }

        if free_downloaders.is_empty() {
            // No available downloaders to offer
            return None;
        }

        let (mut free_peer_id, _) = free_downloaders[0];
        let peer_scores = self.peer_scores.lock().await; // WARNING: lock elsewhere may cause deadlock!!
        for (peer_id, _) in free_downloaders.iter() {
            let peer_id_score = peer_scores.get(&peer_id).unwrap_or(&0);
            let max_peer_id_score = peer_scores.get(&free_peer_id).unwrap_or(&0);
            if peer_id_score >= max_peer_id_score {
                free_peer_id = *peer_id;
            }
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
            downloaders.remove(&free_peer_id);
            return None;
        };

        // Create and spawn Downloader Actor
        let downloader = Downloader::new(free_peer_id, free_downloader_channels).start();
        Some(downloader)
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

struct AccountDumpError {
    pub path: String,
    pub contents: Vec<u8>,
}
