use crate::rlpx::initiator::RLPxInitiator;
use crate::{
    metrics::{CurrentStepValue, METRICS},
    peer_table::{PeerData, PeerTable, PeerTableError},
    rlpx::{
        connection::server::PeerConnection,
        error::PeerConnectionError,
        eth::blocks::{
            BLOCK_HEADER_LIMIT, BlockBodies, BlockHeaders, GetBlockBodies, GetBlockHeaders,
            HashOrNumber,
        },
        message::Message as RLPxMessage,
        p2p::{Capability, SUPPORTED_ETH_CAPABILITIES},
    },
};
use ethrex_common::{
    H256,
    types::{BlockBody, BlockHeader, validate_block_body},
};
use spawned_concurrency::tasks::GenServerHandle;
use std::{
    collections::{HashSet, VecDeque},
    sync::atomic::Ordering,
    time::{Duration, SystemTime},
};
use tracing::{debug, error, trace, warn};

// Re-export constants from snap::constants for backward compatibility
pub use crate::snap::constants::{
    HASH_MAX, MAX_BLOCK_BODIES_TO_REQUEST, MAX_HEADER_CHUNK, MAX_RESPONSE_BYTES,
    PEER_REPLY_TIMEOUT, PEER_SELECT_RETRY_ATTEMPTS, RANGE_FILE_CHUNK_SIZE, REQUEST_RETRY_ATTEMPTS,
    SNAP_LIMIT,
};

// Re-export snap client types for backward compatibility
pub use crate::snap::{DumpError, RequestMetadata, RequestStorageTrieNodesError, SnapError};

/// An abstraction over the [Kademlia] containing logic to make requests to peers
#[derive(Debug, Clone)]
pub struct PeerHandler {
    pub peer_table: PeerTable,
    pub initiator: GenServerHandle<RLPxInitiator>,
}

pub enum BlockRequestOrder {
    OldToNew,
    NewToOld,
}

async fn ask_peer_head_number(
    peer_id: H256,
    connection: &mut PeerConnection,
    peer_table: &mut PeerTable,
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

    debug!("(Retry {retries}) Requesting sync head {sync_head:?} to peer {peer_id}");

    match PeerHandler::make_request(peer_table, peer_id, connection, request, PEER_REPLY_TIMEOUT)
        .await
    {
        Ok(RLPxMessage::BlockHeaders(BlockHeaders {
            id: _,
            block_headers,
        })) => {
            if !block_headers.is_empty() {
                let sync_head_number = block_headers
                    .last()
                    .ok_or(PeerHandlerError::BlockHeaders)?
                    .number;
                trace!(
                    "Sync Log 12: Received sync head block headers from peer {peer_id}, sync head number {sync_head_number}"
                );
                Ok(sync_head_number)
            } else {
                Err(PeerHandlerError::EmptyResponseFromPeer(peer_id))
            }
        }
        Ok(_other_msgs) => Err(PeerHandlerError::UnexpectedResponseFromPeer(peer_id)),
        Err(PeerConnectionError::Timeout) => {
            Err(PeerHandlerError::ReceiveMessageFromPeerTimeout(peer_id))
        }
        Err(_other_err) => Err(PeerHandlerError::ReceiveMessageFromPeer(peer_id)),
    }
}

impl PeerHandler {
    pub fn new(peer_table: PeerTable, initiator: GenServerHandle<RLPxInitiator>) -> PeerHandler {
        Self {
            peer_table,
            initiator,
        }
    }

    pub(crate) async fn make_request(
        // TODO: We should receive the PeerHandler (or self) instead, but since it is not yet spawnified it cannot be shared
        // Fix this to avoid passing the PeerTable as a parameter
        peer_table: &mut PeerTable,
        peer_id: H256,
        connection: &mut PeerConnection,
        message: RLPxMessage,
        timeout: Duration,
    ) -> Result<RLPxMessage, PeerConnectionError> {
        peer_table.inc_requests(peer_id).await?;
        let result = connection.outgoing_request(message, timeout).await;
        peer_table.dec_requests(peer_id).await?;
        result
    }

    /// Returns a random node id and the channel ends to an active peer connection that supports the given capability
    /// It doesn't guarantee that the selected peer is not currently busy
    async fn get_random_peer(
        &mut self,
        capabilities: &[Capability],
    ) -> Result<Option<(H256, PeerConnection)>, PeerHandlerError> {
        return Ok(self.peer_table.get_random_peer(capabilities).await?);
    }

    /// Requests block headers from any suitable peer, starting from the `start` block hash towards either older or newer blocks depending on the order
    /// Returns the block headers or None if:
    /// - There are no available peers (the node just started up or was rejected by all other nodes)
    /// - No peer returned a valid response in the given time and retry limits
    pub async fn request_block_headers(
        &mut self,
        start: u64,
        sync_head: H256,
    ) -> Result<Option<Vec<BlockHeader>>, PeerHandlerError> {
        let start_time = SystemTime::now();
        METRICS
            .current_step
            .set(CurrentStepValue::DownloadingHeaders);

        let mut ret = Vec::<BlockHeader>::new();

        let mut sync_head_number = 0_u64;

        let sync_head_number_retrieval_start = SystemTime::now();

        debug!("Retrieving sync head block number from peers");

        let mut retries = 1;

        while sync_head_number == 0 {
            if retries > 10 {
                // sync_head might be invalid
                return Ok(None);
            }
            let peer_connection = self
                .peer_table
                .get_peer_connections(&SUPPORTED_ETH_CAPABILITIES)
                .await?;

            for (peer_id, mut connection) in peer_connection {
                match ask_peer_head_number(
                    peer_id,
                    &mut connection,
                    &mut self.peer_table,
                    sync_head,
                    retries,
                )
                .await
                {
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
        METRICS
            .sync_head_block
            .store(sync_head_number, Ordering::Relaxed);
        sync_head_number = sync_head_number.min(start + MAX_HEADER_CHUNK);

        let sync_head_number_retrieval_elapsed = sync_head_number_retrieval_start
            .elapsed()
            .unwrap_or_default();

        debug!("Sync head block number retrieved");

        *METRICS.time_to_retrieve_sync_head_block.lock().await =
            Some(sync_head_number_retrieval_elapsed);
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
        if !block_count.is_multiple_of(chunk_count) {
            tasks_queue_not_started
                .push_back((chunk_count * chunk_limit + start, block_count % chunk_count));
        }

        let mut downloaded_count = 0_u64;

        // channel to send the tasks to the peers
        let (task_sender, mut task_receiver) =
            tokio::sync::mpsc::channel::<(Vec<BlockHeader>, H256, PeerConnection, u64, u64)>(1000);

        let mut current_show = 0;

        // 3) create tasks that will request a chunk of headers from a peer

        debug!("Starting to download block headers from peers");

        *METRICS.headers_download_start_time.lock().await = Some(SystemTime::now());

        let mut logged_no_free_peers_count = 0;

        loop {
            if let Ok((headers, peer_id, _connection, startblock, previous_chunk_limit)) =
                task_receiver.try_recv()
            {
                trace!("We received a download chunk from peer");
                if headers.is_empty() {
                    self.peer_table.record_failure(&peer_id).await?;

                    debug!("Failed to download chunk from peer. Downloader {peer_id} freed");

                    // reinsert the task to the queue
                    tasks_queue_not_started.push_back((startblock, previous_chunk_limit));

                    continue; // Retry with the next peer
                }

                downloaded_count += headers.len() as u64;

                METRICS.downloaded_headers.inc_by(headers.len() as u64);

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

                self.peer_table.record_success(&peer_id).await?;
                debug!("Downloader {peer_id} freed");
            }
            let Some((peer_id, mut connection)) = self
                .peer_table
                .get_best_peer(&SUPPORTED_ETH_CAPABILITIES)
                .await?
            else {
                // Log ~ once every 10 seconds
                if logged_no_free_peers_count == 0 {
                    trace!("We are missing peers in request_block_headers");
                    logged_no_free_peers_count = 1000;
                }
                logged_no_free_peers_count -= 1;
                // Sleep a bit to avoid busy polling
                tokio::time::sleep(Duration::from_millis(10)).await;
                continue;
            };

            let Some((startblock, chunk_limit)) = tasks_queue_not_started.pop_front() else {
                if downloaded_count >= block_count {
                    debug!("All headers downloaded successfully");
                    break;
                }

                let batch_show = downloaded_count / 10_000;

                if current_show < batch_show {
                    current_show += 1;
                }

                continue;
            };
            let tx = task_sender.clone();
            debug!("Downloader {peer_id} is now busy");
            let mut peer_table = self.peer_table.clone();

            // run download_chunk_from_peer in a different Tokio task
            tokio::spawn(async move {
                trace!(
                    "Sync Log 5: Requesting block headers from peer {peer_id}, chunk_limit: {chunk_limit}"
                );
                let headers = Self::download_chunk_from_peer(
                    peer_id,
                    &mut connection,
                    &mut peer_table,
                    startblock,
                    chunk_limit,
                )
                .await
                .inspect_err(|err| trace!("Sync Log 6: {peer_id} failed to download chunk: {err}"))
                .unwrap_or_default();

                tx.send((headers, peer_id, connection, startblock, chunk_limit))
                    .await
                    .inspect_err(|err| {
                        error!("Failed to send headers result through channel. Error: {err}")
                    })
            });
        }

        let elapsed = start_time.elapsed().unwrap_or_default();

        debug!(
            "Downloaded all headers ({}) in {} seconds",
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
                    debug!("All downloaded headers are unique");
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
        Ok(Some(ret))
    }

    /// Requests block headers from any suitable peer, starting from the `start` block hash towards either older or newer blocks depending on the order
    /// - No peer returned a valid response in the given time and retry limits
    ///   Since request_block_headers brought problems in cases of reorg seen in this pr https://github.com/lambdaclass/ethrex/pull/4028, we have this other function to request block headers only for full sync.
    pub async fn request_block_headers_from_hash(
        &mut self,
        start: H256,
        order: BlockRequestOrder,
    ) -> Result<Option<Vec<BlockHeader>>, PeerHandlerError> {
        let request_id = rand::random();
        let request = RLPxMessage::GetBlockHeaders(GetBlockHeaders {
            id: request_id,
            startblock: start.into(),
            limit: BLOCK_HEADER_LIMIT,
            skip: 0,
            reverse: matches!(order, BlockRequestOrder::NewToOld),
        });
        match self.get_random_peer(&SUPPORTED_ETH_CAPABILITIES).await? {
            None => Ok(None),
            Some((peer_id, mut connection)) => {
                if let Ok(RLPxMessage::BlockHeaders(BlockHeaders {
                    id: _,
                    block_headers,
                })) = PeerHandler::make_request(
                    &mut self.peer_table,
                    peer_id,
                    &mut connection,
                    request,
                    PEER_REPLY_TIMEOUT,
                )
                .await
                {
                    if !block_headers.is_empty()
                        && are_block_headers_chained(&block_headers, &order)
                    {
                        return Ok(Some(block_headers));
                    } else {
                        warn!(
                            "[SYNCING] Received empty/invalid headers from peer, penalizing peer {peer_id}"
                        );
                        return Ok(None);
                    }
                }
                // Timeouted
                warn!(
                    "[SYNCING] Didn't receive block headers from peer, penalizing peer {peer_id}..."
                );
                Ok(None)
            }
        }
    }

    /// Given a peer id, a chunk start and a chunk limit, requests the block headers from the peer
    ///
    /// If it fails, returns an error message.
    async fn download_chunk_from_peer(
        peer_id: H256,
        connection: &mut PeerConnection,
        peer_table: &mut PeerTable,
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
        if let Ok(RLPxMessage::BlockHeaders(BlockHeaders {
            id: _,
            block_headers,
        })) =
            PeerHandler::make_request(peer_table, peer_id, connection, request, PEER_REPLY_TIMEOUT)
                .await
        {
            if are_block_headers_chained(&block_headers, &BlockRequestOrder::OldToNew) {
                Ok(block_headers)
            } else {
                warn!("[SYNCING] Received invalid headers from peer: {peer_id}");
                Err(PeerHandlerError::InvalidHeaders)
            }
        } else {
            Err(PeerHandlerError::BlockHeaders)
        }
    }

    /// Internal method to request block bodies from any suitable peer given their block hashes
    /// Returns the block bodies or None if:
    /// - There are no available peers (the node just started up or was rejected by all other nodes)
    /// - The requested peer did not return a valid response in the given time limit
    async fn request_block_bodies_inner(
        &mut self,
        block_hashes: &[H256],
    ) -> Result<Option<(Vec<BlockBody>, H256)>, PeerHandlerError> {
        let block_hashes_len = block_hashes.len();
        let request_id = rand::random();
        let request = RLPxMessage::GetBlockBodies(GetBlockBodies {
            id: request_id,
            block_hashes: block_hashes.to_vec(),
        });
        match self.get_random_peer(&SUPPORTED_ETH_CAPABILITIES).await? {
            None => Ok(None),
            Some((peer_id, mut connection)) => {
                if let Ok(RLPxMessage::BlockBodies(BlockBodies {
                    id: _,
                    block_bodies,
                })) = PeerHandler::make_request(
                    &mut self.peer_table,
                    peer_id,
                    &mut connection,
                    request,
                    PEER_REPLY_TIMEOUT,
                )
                .await
                {
                    // Check that the response is not empty and does not contain more bodies than the ones requested
                    if !block_bodies.is_empty() && block_bodies.len() <= block_hashes_len {
                        self.peer_table.record_success(&peer_id).await?;
                        return Ok(Some((block_bodies, peer_id)));
                    }
                }
                warn!(
                    "[SYNCING] Didn't receive block bodies from peer, penalizing peer {peer_id}..."
                );
                self.peer_table.record_failure(&peer_id).await?;
                Ok(None)
            }
        }
    }

    /// Requests block bodies from any suitable peer given their block headers and validates them
    /// Returns the requested block bodies or None if:
    /// - There are no available peers (the node just started up or was rejected by all other nodes)
    /// - No peer returned a valid response in the given time and retry limits
    /// - The block bodies are invalid given the block headers
    pub async fn request_block_bodies(
        &mut self,
        block_headers: &[BlockHeader],
    ) -> Result<Option<Vec<BlockBody>>, PeerHandlerError> {
        let block_hashes: Vec<H256> = block_headers.iter().map(|h| h.hash()).collect();

        for _ in 0..REQUEST_RETRY_ATTEMPTS {
            let Some((block_bodies, peer_id)) =
                self.request_block_bodies_inner(&block_hashes).await?
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
                    self.peer_table.record_critical_failure(&peer_id).await?;
                    break;
                }
                res.push(body);
            }
            // Retry on validation failure
            if validation_success {
                return Ok(Some(res));
            }
        }
        Ok(None)
    }
    /// Returns the PeerData for each connected Peer
    pub async fn read_connected_peers(&mut self) -> Vec<PeerData> {
        self.peer_table
            .get_peers_data()
            .await
            // Proper error handling
            .unwrap_or(Vec::new())
    }

    pub async fn count_total_peers(&mut self) -> Result<usize, PeerHandlerError> {
        Ok(self.peer_table.peer_count().await?)
    }

    pub async fn get_block_header(
        &mut self,
        peer_id: H256,
        connection: &mut PeerConnection,
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
        debug!("get_block_header: requesting header with number {block_number}");
        match PeerHandler::make_request(
            &mut self.peer_table,
            peer_id,
            connection,
            request,
            PEER_REPLY_TIMEOUT,
        )
        .await
        {
            Ok(RLPxMessage::BlockHeaders(BlockHeaders {
                id: _,
                block_headers,
            })) => {
                if !block_headers.is_empty() {
                    return Ok(Some(
                        block_headers
                            .last()
                            .ok_or(PeerHandlerError::BlockHeaders)?
                            .clone(),
                    ));
                }
            }
            Ok(_other_msgs) => {
                debug!("Received unexpected message from peer");
            }
            Err(PeerConnectionError::Timeout) => {
                debug!("Timeout while waiting for sync head from peer");
            }
            // TODO: we need to check, this seems a scenario where the peer channel does teardown
            // after we sent the backend message
            Err(_) => {
                warn!("The RLPxConnection closed the backend channel");
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

#[derive(thiserror::Error, Debug)]
pub enum PeerHandlerError {
    #[error("Failed to send message to peer: {0}")]
    SendMessageToPeer(String),
    #[error("Failed to receive block headers")]
    BlockHeaders,
    #[error("Received unexpected response from peer {0}")]
    UnexpectedResponseFromPeer(H256),
    #[error("Received an empty response from peer {0}")]
    EmptyResponseFromPeer(H256),
    #[error("Failed to receive message from peer {0}")]
    ReceiveMessageFromPeer(H256),
    #[error("Timeout while waiting for message from peer {0}")]
    ReceiveMessageFromPeerTimeout(H256),
    #[error("Received invalid headers")]
    InvalidHeaders,
    #[error("Storage Full")]
    StorageFull,
    #[error("No response from peer")]
    NoResponseFromPeer,
    #[error("Error in Peer Table: {0}")]
    PeerTableError(#[from] PeerTableError),
    #[error("Snap error: {0}")]
    Snap(#[from] SnapError),
}
