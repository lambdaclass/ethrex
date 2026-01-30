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
        snap::{
            AccountRange, AccountRangeUnit, ByteCodes, GetAccountRange, GetByteCodes,
            GetStorageRanges, GetTrieNodes, StorageRanges, TrieNodes,
        },
    },
    snap::encodable_to_proof,
    sync::{AccountStorageRoots, block_is_stale},
    utils::{
        AccountsWithStorage, dump_storages_to_file,
        get_account_storages_snapshot_file,
    },
};
use bytes::Bytes;
use ethrex_common::{
    BigEndianHash, H256, U256,
    types::{AccountState, BlockBody, BlockHeader, validate_block_body},
};
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};
use ethrex_storage::Store;
use ethrex_trie::Nibbles;
use ethrex_trie::{Node, verify_range};
use spawned_concurrency::tasks::GenServerHandle;
use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    io::ErrorKind,
    mem::size_of,
    path::{Path, PathBuf},
    sync::atomic::Ordering,
    time::{Duration, SystemTime},
};
use tempfile::TempDir;
use tokio::{
    fs::File,
    io::{AsyncWriteExt, BufWriter},
    sync::mpsc,
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
pub const RANGE_FILE_CHUNK_SIZE: usize = 1024 * 1024 * 64; // 64MB
pub const SNAP_LIMIT: usize = 128;

// Bucket-based download architecture constants
// Split address space by first byte of account hash (256 buckets)
pub const BUCKET_COUNT: usize = 256;
// Channel buffer size for bucket writers (accounts buffered before write)
pub const BUCKET_CHANNEL_CAPACITY: usize = 1000;
// Retry logic constants
pub const MAX_RETRIES_PER_RANGE: u32 = 10;
pub const RETRY_BACKOFF_BASE_MS: u64 = 100;
pub const RETRY_BACKOFF_MAX_MS: u64 = 10_000;

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
    pub peer_table: PeerTable,
    pub initiator: GenServerHandle<RLPxInitiator>,
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

// Bucket-based download architecture types

/// Sets up 256 bucket writer tasks with lock-free channel communication
///
/// Each bucket writer:
/// - Receives accounts via a dedicated channel
/// - Writes RLP-encoded accounts to an append-only file
/// - Sends completion notification with file path when channel closes
///
/// Returns:
/// - HashMap of bucket_id -> sender channels
/// - Receiver for completion notifications (bucket_id, file_path)
/// - TempDir handle (caller must keep alive until all writes complete)
async fn setup_bucket_writers() -> Result<
    (
        HashMap<u8, mpsc::Sender<Vec<AccountRangeUnit>>>,
        mpsc::Receiver<(u8, PathBuf)>,
        TempDir,
    ),
    PeerHandlerError,
> {
    // Create temp directory for bucket files
    let temp_dir = TempDir::new().map_err(|e| {
        PeerHandlerError::UnrecoverableError(format!("Failed to create temp directory: {}", e))
    })?;

    let base_path = temp_dir.path().to_path_buf();

    let mut bucket_channels = HashMap::new();
    let (completion_tx, completion_rx) = mpsc::channel(BUCKET_COUNT);

    // Spawn 256 bucket writer tasks
    for bucket_id in 0..BUCKET_COUNT {
        let bucket_id_u8 = bucket_id as u8;
        let (tx, mut rx) = mpsc::channel::<Vec<AccountRangeUnit>>(BUCKET_CHANNEL_CAPACITY);
        bucket_channels.insert(bucket_id_u8, tx);

        let bucket_path = base_path.join(format!("bucket_{:02x}.rlp", bucket_id_u8));
        let completion_tx = completion_tx.clone();

        // Spawn dedicated writer task for this bucket
        tokio::spawn(async move {
            let result: Result<(), std::io::Error> = async {
                let file = File::create(&bucket_path).await?;
                let mut writer = BufWriter::new(file);

                // Receive and write accounts until channel closes
                while let Some(accounts) = rx.recv().await {
                    for account in accounts {
                        let encoded = account.encode_to_vec();
                        writer.write_all(&encoded).await?;
                    }
                }

                // Flush remaining data
                writer.flush().await?;
                Ok(())
            }
            .await;

            match result {
                Ok(()) => {
                    debug!("Bucket {} writer completed successfully", bucket_id_u8);
                    // Send completion notification
                    let _ = completion_tx.send((bucket_id_u8, bucket_path)).await;
                }
                Err(e) => {
                    error!("Bucket {} writer failed: {}", bucket_id_u8, e);
                }
            }
        });
    }

    Ok((bucket_channels, completion_rx, temp_dir))
}

/// Downloads an account range and fans out accounts to bucket channels (verify-then-fanout pattern)
///
/// Algorithm:
/// 1. Request account range from peer (start_hash to H256::MAX, let peer decide how much to return)
/// 2. Verify FULL peer response with Merkle proof (validates peer's contiguous range)
/// 3. Fan out verified accounts to bucket channels based on first byte of hash
/// 4. Return next starting hash if more data available, None if range complete
///
/// Key insight: Verification happens on peer's range BEFORE bucketing, so proof structure is valid
///
/// Returns:
/// - Ok(Some(next_hash)) if should continue from next_hash
/// - Ok(None) if range is complete (no more accounts)
/// - Err(_) on failure (caller should retry)
async fn download_range_and_fanout(
    peer_handler: &mut PeerHandler,
    start_hash: H256,
    state_root: H256,
    bucket_channels: &HashMap<u8, mpsc::Sender<Vec<AccountRangeUnit>>>,
) -> Result<Option<H256>, PeerHandlerError> {
    // Get best available peer
    let Some((peer_id, mut connection)) = peer_handler
        .peer_table
        .get_best_peer(&SUPPORTED_ETH_CAPABILITIES)
        .await?
    else {
        return Err(PeerHandlerError::NoResponseFromPeer);
    };

    // Request account range (no end limit - let peer decide)
    let request_id = rand::random();
    let request = RLPxMessage::GetAccountRange(GetAccountRange {
        id: request_id,
        root_hash: state_root,
        starting_hash: start_hash,
        limit_hash: HASH_MAX, // No limit, peer returns what it can
        response_bytes: MAX_RESPONSE_BYTES,
    });

    let response = PeerHandler::make_request(
        &mut peer_handler.peer_table,
        peer_id,
        &mut connection,
        request,
        PEER_REPLY_TIMEOUT,
    )
    .await;

    let Ok(RLPxMessage::AccountRange(AccountRange {
        id: _,
        accounts,
        proof,
    })) = response
    else {
        peer_handler.peer_table.record_failure(&peer_id).await?;
        return Err(PeerHandlerError::UnexpectedResponseFromPeer(peer_id));
    };

    if accounts.is_empty() {
        peer_handler.peer_table.record_failure(&peer_id).await?;
        return Err(PeerHandlerError::EmptyResponseFromPeer(peer_id));
    }

    // Verify FULL response (proof validates peer's contiguous range)
    let proof = encodable_to_proof(&proof);
    let (account_hashes, account_states): (Vec<_>, Vec<_>) =
        accounts.iter().map(|unit| (unit.hash, unit.account)).unzip();
    let encoded_accounts = account_states
        .iter()
        .map(|acc| acc.encode_to_vec())
        .collect::<Vec<_>>();

    let should_continue = match verify_range(
        state_root,
        &start_hash,
        &account_hashes,
        &encoded_accounts,
        &proof,
    ) {
        Ok(should_continue) => should_continue,
        Err(_) => {
            // Record failure on invalid proof
            peer_handler.peer_table.record_failure(&peer_id).await.ok();
            return Err(PeerHandlerError::UnrecoverableError(
                "Invalid account range proof".to_string(),
            ));
        }
    };

    // Record success
    peer_handler.peer_table.record_success(&peer_id).await?;

    // Fan out verified accounts to bucket channels (NO LOCKS!)
    let mut accounts_by_bucket: HashMap<u8, Vec<AccountRangeUnit>> = HashMap::new();
    for account in accounts.iter() {
        let bucket_id = account.hash.0[0]; // First byte determines bucket
        accounts_by_bucket
            .entry(bucket_id)
            .or_default()
            .push(account.clone());
    }

    // Send to each bucket's channel
    for (bucket_id, bucket_accounts) in accounts_by_bucket {
        if let Some(sender) = bucket_channels.get(&bucket_id) {
            sender
                .send(bucket_accounts)
                .await
                .map_err(|_| {
                    PeerHandlerError::UnrecoverableError(format!(
                        "Bucket {} channel closed unexpectedly",
                        bucket_id
                    ))
                })?;
        }
    }

    // Return next starting point if more data available
    if should_continue {
        let last_hash = account_hashes
            .last()
            .ok_or(PeerHandlerError::AccountHashes)?;
        let next_start_u256 = U256::from_big_endian(&last_hash.0) + 1;
        let next_start = H256::from_uint(&next_start_u256);
        Ok(Some(next_start))
    } else {
        Ok(None) // Range complete
    }
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

    async fn make_request(
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


    /// Downloads all accounts using bucket-based architecture with verify-then-fanout pattern
    ///
    /// Replaces the complex chunking approach (800 dynamic chunks) with a simpler design:
    /// - 256 fixed buckets by first byte of account hash
    /// - Verify-then-fanout: verify peer response, then distribute to buckets
    /// - Lock-free channels: dedicated writer task per bucket
    /// - Sequential download: simpler coordination than parallel chunks
    ///
    /// Returns paths to 256 bucket files containing RLP-encoded accounts
    pub async fn download_accounts_bucketed(
        &mut self,
        state_root: H256,
    ) -> Result<Vec<PathBuf>, PeerHandlerError> {
        info!("[SNAP] Phase 1/2: Starting bucket-based account download");
        METRICS
            .current_step
            .set(CurrentStepValue::RequestingAccountRanges);

        // Setup bucket writers (256 dedicated tasks with channels)
        let (bucket_channels, mut completion_rx, _temp_dir) = setup_bucket_writers().await?;

        *METRICS.account_tries_download_start_time.lock().await = Some(SystemTime::now());

        // Download entire address space using verify-then-fanout pattern with exponential backoff
        let mut current_start = H256::zero();
        let mut bytes_downloaded = 0u64;
        let mut last_log = SystemTime::now();
        let mut consecutive_failures = 0u32;
        let mut current_backoff_ms = RETRY_BACKOFF_BASE_MS;

        loop {
            // Download range and fan out to buckets
            match download_range_and_fanout(self, current_start, state_root, &bucket_channels).await
            {
                Ok(Some(next_start)) => {
                    // Success - reset retry state
                    consecutive_failures = 0;
                    current_backoff_ms = RETRY_BACKOFF_BASE_MS;
                    current_start = next_start;

                    // Update metrics and logging
                    bytes_downloaded += MAX_RESPONSE_BYTES; // Approximate
                    if last_log.elapsed().unwrap_or_default() >= Duration::from_secs(5) {
                        let gb = bytes_downloaded as f64 / 1_000_000_000.0;
                        info!("[SNAP] Phase 1/2: Downloaded {:.2} GB of accounts", gb);
                        METRICS
                            .downloaded_account_tries
                            .store(bytes_downloaded / size_of::<AccountState>() as u64, Ordering::Relaxed);
                        last_log = SystemTime::now();
                    }
                }
                Ok(None) => {
                    // Download complete
                    info!("[SNAP] Phase 1/2: Account download complete");
                    break;
                }
                Err(e) => {
                    consecutive_failures += 1;

                    // Check retry limit
                    if consecutive_failures > MAX_RETRIES_PER_RANGE {
                        error!(
                            "[SNAP] Failed after {} retries: {}",
                            MAX_RETRIES_PER_RANGE, e
                        );
                        return Err(e);
                    }

                    // Log with backoff info
                    warn!(
                        "[SNAP] Download error (attempt {}): {}, retrying in {}ms",
                        consecutive_failures, e, current_backoff_ms
                    );

                    // Exponential backoff
                    tokio::time::sleep(Duration::from_millis(current_backoff_ms)).await;
                    current_backoff_ms = (current_backoff_ms * 2).min(RETRY_BACKOFF_MAX_MS);

                    // Switch peer every 3 failures to avoid getting stuck on bad peer
                    if consecutive_failures % 3 == 0 {
                        debug!("[SNAP] Switching to different peer after 3 failures");
                        // get_best_peer in next iteration will select different peer
                    }

                    // Don't update current_start - retry same range
                    continue;
                }
            }
        }

        // Close all bucket channels to signal writers to finish
        drop(bucket_channels);

        // Collect bucket file paths from completion notifications
        let mut bucket_files = Vec::with_capacity(BUCKET_COUNT);
        for _ in 0..BUCKET_COUNT {
            let Some((bucket_id, path)) = completion_rx.recv().await else {
                return Err(PeerHandlerError::UnrecoverableError(
                    "Bucket writer channel closed unexpectedly".to_string(),
                ));
            };
            debug!("Bucket {} completed: {:?}", bucket_id, path);
            bucket_files.push(path);
        }

        *METRICS.account_tries_download_end_time.lock().await = Some(SystemTime::now());
        info!("[SNAP] Phase 1/2: All {} buckets downloaded", BUCKET_COUNT);

        Ok(bucket_files)
    }


    /// Requests bytecodes for the given code hashes
    /// Returns the bytecodes or None if:
    /// - There are no available peers (the node just started up or was rejected by all other nodes)
    /// - No peer returned a valid response in the given time and retry limits
    pub async fn request_bytecodes(
        &mut self,
        all_bytecode_hashes: &[H256],
    ) -> Result<Option<Vec<Bytes>>, PeerHandlerError> {
        METRICS
            .current_step
            .set(CurrentStepValue::RequestingBytecodes);
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

        debug!("Starting to download bytecodes from peers");

        METRICS
            .bytecodes_to_download
            .fetch_add(all_bytecode_hashes.len() as u64, Ordering::Relaxed);

        let mut completed_tasks = 0;

        let mut logged_no_free_peers_count = 0;

        loop {
            if let Ok(result) = task_receiver.try_recv() {
                let TaskResult {
                    start_index,
                    bytecodes,
                    peer_id,
                    remaining_start,
                    remaining_end,
                } = result;

                debug!(
                    "Downloaded {} bytecodes from peer {peer_id} (current count: {downloaded_count})",
                    bytecodes.len(),
                );

                if remaining_start < remaining_end {
                    tasks_queue_not_started.push_back((remaining_start, remaining_end));
                } else {
                    completed_tasks += 1;
                }
                if bytecodes.is_empty() {
                    self.peer_table.record_failure(&peer_id).await?;
                    continue;
                }

                downloaded_count += bytecodes.len() as u64;

                self.peer_table.record_success(&peer_id).await?;
                for (i, bytecode) in bytecodes.into_iter().enumerate() {
                    all_bytecodes[start_index + i] = bytecode;
                }
            }

            let Some((peer_id, mut connection)) = self
                .peer_table
                .get_best_peer(&SUPPORTED_ETH_CAPABILITIES)
                .await
                .inspect_err(|err| warn!(%err, "Error requesting a peer for bytecodes"))
                .unwrap_or(None)
            else {
                // Log ~ once every 10 seconds
                if logged_no_free_peers_count == 0 {
                    trace!("We are missing peers in request_bytecodes");
                    logged_no_free_peers_count = 1000;
                }
                logged_no_free_peers_count -= 1;
                // Sleep a bit to avoid busy polling
                tokio::time::sleep(Duration::from_millis(10)).await;
                continue;
            };

            let Some((chunk_start, chunk_end)) = tasks_queue_not_started.pop_front() else {
                if completed_tasks >= chunk_count {
                    debug!("All bytecodes downloaded successfully");
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

            let mut peer_table = self.peer_table.clone();

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
                if let Ok(RLPxMessage::ByteCodes(ByteCodes { id: _, codes })) =
                    PeerHandler::make_request(
                        &mut peer_table,
                        peer_id,
                        &mut connection,
                        request,
                        PEER_REPLY_TIMEOUT,
                    )
                    .await
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
                        .take_while(|(b, hash)| ethrex_common::utils::keccak(b) == *hash)
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
        debug!(
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
        &mut self,
        account_storage_roots: &mut AccountStorageRoots,
        account_storages_snapshots_dir: &Path,
        mut chunk_index: u64,
        pivot_header: &mut BlockHeader,
        store: Store,
    ) -> Result<u64, PeerHandlerError> {
        METRICS
            .current_step
            .set(CurrentStepValue::RequestingStorageRanges);
        debug!("Starting request_storage_ranges function");
        // 1) split the range in chunks of same length
        let mut accounts_by_root_hash: BTreeMap<_, Vec<_>> = BTreeMap::new();
        for (account, (maybe_root_hash, _)) in &account_storage_roots.accounts_with_storage_root {
            match maybe_root_hash {
                Some(root) => {
                    accounts_by_root_hash
                        .entry(*root)
                        .or_default()
                        .push(*account);
                }
                None => {
                    let root = store
                        .get_account_state_by_acc_hash(pivot_header.hash(), *account)
                        .expect("Failed to get account in state trie")
                        .expect("Could not find account that should have been downloaded or healed")
                        .storage_root;
                    accounts_by_root_hash
                        .entry(root)
                        .or_default()
                        .push(*account);
                }
            }
        }
        let mut accounts_by_root_hash = Vec::from_iter(accounts_by_root_hash);
        // TODO: Turn this into a stable sort for binary search.
        accounts_by_root_hash.sort_unstable_by_key(|(_, accounts)| !accounts.len());
        let chunk_size = 300;
        let chunk_count = (accounts_by_root_hash.len() / chunk_size) + 1;

        // list of tasks to be executed
        // Types are (start_index, end_index, starting_hash)
        // NOTE: end_index is NOT inclusive

        let mut tasks_queue_not_started = VecDeque::<StorageTask>::new();
        for i in 0..chunk_count {
            let chunk_start = chunk_size * i;
            let chunk_end = (chunk_start + chunk_size).min(accounts_by_root_hash.len());
            tasks_queue_not_started.push_back(StorageTask {
                start_index: chunk_start,
                end_index: chunk_end,
                start_hash: H256::zero(),
                end_hash: None,
            });
        }

        // channel to send the tasks to the peers
        let (task_sender, mut task_receiver) =
            tokio::sync::mpsc::channel::<StorageTaskResult>(1000);

        // channel to send the result of dumping storages
        let mut disk_joinset: tokio::task::JoinSet<Result<(), DumpError>> =
            tokio::task::JoinSet::new();

        let mut task_count = tasks_queue_not_started.len();
        let mut completed_tasks = 0;

        // TODO: in a refactor, delete this replace with a structure that can handle removes
        let mut accounts_done: HashMap<H256, Vec<(H256, H256)>> = HashMap::new();
        // Maps storage root to vector of hashed addresses matching that root and
        // vector of hashed storage keys and storage values.
        let mut current_account_storages: BTreeMap<H256, AccountsWithStorage> = BTreeMap::new();

        let mut logged_no_free_peers_count = 0;

        debug!("Starting request_storage_ranges loop");
        loop {
            if current_account_storages
                .values()
                .map(|accounts| 32 * accounts.accounts.len() + 64 * accounts.storages.len())
                .sum::<usize>()
                > RANGE_FILE_CHUNK_SIZE
            {
                let current_account_storages = std::mem::take(&mut current_account_storages);
                let snapshot = current_account_storages.into_values().collect::<Vec<_>>();

                if !std::fs::exists(account_storages_snapshots_dir)
                    .map_err(|_| PeerHandlerError::NoStorageSnapshotsDir)?
                {
                    std::fs::create_dir_all(account_storages_snapshots_dir)
                        .map_err(|_| PeerHandlerError::CreateStorageSnapshotsDir)?;
                }
                let account_storages_snapshots_dir_cloned =
                    account_storages_snapshots_dir.to_path_buf();
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
                        &account_storages_snapshots_dir_cloned,
                        chunk_index,
                    );
                    dump_storages_to_file(&path, snapshot)
                });

                chunk_index += 1;
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

                for (_, accounts) in accounts_by_root_hash[start_index..remaining_start].iter() {
                    for account in accounts {
                        if !accounts_done.contains_key(account) {
                            let (_, old_intervals) = account_storage_roots
                                .accounts_with_storage_root
                                .get_mut(account)
                                .ok_or(PeerHandlerError::UnrecoverableError("Tried to get the old download intervals for an account but did not find them".to_owned()))?;

                            if old_intervals.is_empty() {
                                accounts_done.insert(*account, vec![]);
                            }
                        }
                    }
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

                            let acc_hash = accounts_by_root_hash[remaining_start].1[0];
                            let (_, old_intervals) = account_storage_roots
                                .accounts_with_storage_root
                                .get_mut(&acc_hash).ok_or(PeerHandlerError::UnrecoverableError("Tried to get the old download intervals for an account but did not find them".to_owned()))?;
                            for (old_start, end) in old_intervals {
                                if end == &hash_end {
                                    *old_start = hash_start;
                                }
                            }
                            account_storage_roots
                                .healed_accounts
                                .extend(accounts_by_root_hash[start_index].1.iter().copied());
                        } else {
                            let mut acc_hash: H256 = H256::zero();
                            // This search could potentially be expensive, but it's something that should happen very
                            // infrequently (only when we encounter an account we think it's big but it's not). In
                            // normal cases the vec we are iterating over just has one element (the big account).
                            for account in accounts_by_root_hash[remaining_start].1.iter() {
                                if let Some((_, old_intervals)) = account_storage_roots
                                    .accounts_with_storage_root
                                    .get(account)
                                {
                                    if !old_intervals.is_empty() {
                                        acc_hash = *account;
                                    }
                                } else {
                                    continue;
                                }
                            }
                            if acc_hash.is_zero() {
                                panic!("Should have found the account hash");
                            }
                            let (_, old_intervals) = account_storage_roots
                                .accounts_with_storage_root
                                .get_mut(&acc_hash)
                                .ok_or(PeerHandlerError::UnrecoverableError("Tried to get the old download intervals for an account but did not find them".to_owned()))?;
                            old_intervals.remove(
                                old_intervals
                                    .iter()
                                    .position(|(_old_start, end)| end == &hash_end)
                                    .ok_or(PeerHandlerError::UnrecoverableError(
                                        "Could not find an old interval that we were tracking"
                                            .to_owned(),
                                    ))?,
                            );
                            if old_intervals.is_empty() {
                                for account in accounts_by_root_hash[remaining_start].1.iter() {
                                    accounts_done.insert(*account, vec![]);
                                    account_storage_roots.healed_accounts.insert(*account);
                                }
                            }
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

                        // Big accounts need to be marked for storage healing unconditionally
                        for account in accounts_by_root_hash[remaining_start].1.iter() {
                            account_storage_roots.healed_accounts.insert(*account);
                        }

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

                        let maybe_old_intervals = account_storage_roots
                            .accounts_with_storage_root
                            .get(&accounts_by_root_hash[remaining_start].1[0]);

                        if let Some((_, old_intervals)) = maybe_old_intervals {
                            if !old_intervals.is_empty() {
                                for (start_hash, end_hash) in old_intervals {
                                    let task = StorageTask {
                                        start_index: remaining_start,
                                        end_index: remaining_start + 1,
                                        start_hash: *start_hash,
                                        end_hash: Some(*end_hash),
                                    };

                                    tasks_queue_not_started.push_back(task);
                                    task_count += 1;
                                }
                            } else {
                                // TODO: DRY
                                account_storage_roots.accounts_with_storage_root.insert(
                                    accounts_by_root_hash[remaining_start].1[0],
                                    (None, vec![]),
                                );
                                let (_, intervals) = account_storage_roots
                                    .accounts_with_storage_root
                                    .get_mut(&accounts_by_root_hash[remaining_start].1[0])
                                    .ok_or(PeerHandlerError::UnrecoverableError("Tried to get the old download intervals for an account but did not find them".to_owned()))?;

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

                                    let task = StorageTask {
                                        start_index: remaining_start,
                                        end_index: remaining_start + 1,
                                        start_hash,
                                        end_hash: Some(end_hash),
                                    };

                                    intervals.push((start_hash, end_hash));

                                    tasks_queue_not_started.push_back(task);
                                    task_count += 1;
                                }
                                debug!("Split big storage account into {chunk_count} chunks.");
                            }
                        } else {
                            account_storage_roots.accounts_with_storage_root.insert(
                                accounts_by_root_hash[remaining_start].1[0],
                                (None, vec![]),
                            );
                            let (_, intervals) = account_storage_roots
                                .accounts_with_storage_root
                                .get_mut(&accounts_by_root_hash[remaining_start].1[0])
                                .ok_or(PeerHandlerError::UnrecoverableError("Trie to get the old download intervals for an account but did not find them".to_owned()))?;

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

                                let task = StorageTask {
                                    start_index: remaining_start,
                                    end_index: remaining_start + 1,
                                    start_hash,
                                    end_hash: Some(end_hash),
                                };

                                intervals.push((start_hash, end_hash));

                                tasks_queue_not_started.push_back(task);
                                task_count += 1;
                            }
                            debug!("Split big storage account into {chunk_count} chunks.");
                        }
                    }
                }

                if account_storages.is_empty() {
                    self.peer_table.record_failure(&peer_id).await?;
                    continue;
                }
                if let Some(hash_end) = hash_end {
                    // This is a big storage account, and the range might be empty
                    if account_storages[0].len() == 1 && account_storages[0][0].0 > hash_end {
                        continue;
                    }
                }

                self.peer_table.record_success(&peer_id).await?;

                let n_storages = account_storages.len();
                let n_slots = account_storages
                    .iter()
                    .map(|storage| storage.len())
                    .sum::<usize>();

                // These take into account we downloaded the same thing for different accounts
                let effective_slots: usize = account_storages
                    .iter()
                    .enumerate()
                    .map(|(i, storages)| {
                        accounts_by_root_hash[start_index + i].1.len() * storages.len()
                    })
                    .sum();

                METRICS
                    .storage_leaves_downloaded
                    .inc_by(effective_slots as u64);

                debug!("Downloaded {n_storages} storages ({n_slots} slots) from peer {peer_id}");
                debug!(
                    "Total tasks: {task_count}, completed tasks: {completed_tasks}, queued tasks: {}",
                    tasks_queue_not_started.len()
                );
                // THEN: update insert to read with the correct structure and reuse
                // tries, only changing the prefix for insertion.
                if account_storages.len() == 1 {
                    let (root_hash, accounts) = &accounts_by_root_hash[start_index];
                    // We downloaded a big storage account
                    current_account_storages
                        .entry(*root_hash)
                        .or_insert_with(|| AccountsWithStorage {
                            accounts: accounts.clone(),
                            storages: Vec::new(),
                        })
                        .storages
                        .extend(account_storages.remove(0));
                } else {
                    for (i, storages) in account_storages.into_iter().enumerate() {
                        let (root_hash, accounts) = &accounts_by_root_hash[start_index + i];
                        current_account_storages.insert(
                            *root_hash,
                            AccountsWithStorage {
                                accounts: accounts.clone(),
                                storages,
                            },
                        );
                    }
                }
            }

            if block_is_stale(pivot_header) {
                info!("request_storage_ranges became stale, breaking");
                break;
            }

            let Some((peer_id, connection)) = self
                .peer_table
                .get_best_peer(&SUPPORTED_ETH_CAPABILITIES)
                .await
                .inspect_err(|err| warn!(%err, "Error requesting a peer for storage ranges"))
                .unwrap_or(None)
            else {
                // Log ~ once every 10 seconds
                if logged_no_free_peers_count == 0 {
                    trace!("We are missing peers in request_storage_ranges");
                    logged_no_free_peers_count = 1000;
                }
                logged_no_free_peers_count -= 1;
                // Sleep a bit to avoid busy polling
                tokio::time::sleep(Duration::from_millis(10)).await;
                continue;
            };

            let Some(task) = tasks_queue_not_started.pop_front() else {
                if completed_tasks >= task_count {
                    break;
                }
                continue;
            };

            let tx = task_sender.clone();

            // FIXME: this unzip is probably pointless and takes up unnecessary memory.
            let (chunk_account_hashes, chunk_storage_roots): (Vec<_>, Vec<_>) =
                accounts_by_root_hash[task.start_index..task.end_index]
                    .iter()
                    .map(|(root, storages)| (storages[0], *root))
                    .unzip();

            if task_count - completed_tasks < 30 {
                debug!(
                    "Assigning task: {task:?}, account_hash: {}, storage_root: {}",
                    chunk_account_hashes.first().unwrap_or(&H256::zero()),
                    chunk_storage_roots.first().unwrap_or(&H256::zero()),
                );
            }
            let peer_table = self.peer_table.clone();

            tokio::spawn(PeerHandler::request_storage_ranges_worker(
                task,
                peer_id,
                connection,
                peer_table,
                pivot_header.state_root,
                chunk_account_hashes,
                chunk_storage_roots,
                tx,
            ));
        }

        {
            let snapshot = current_account_storages.into_values().collect::<Vec<_>>();

            if !std::fs::exists(account_storages_snapshots_dir)
                .map_err(|_| PeerHandlerError::NoStorageSnapshotsDir)?
            {
                std::fs::create_dir_all(account_storages_snapshots_dir)
                    .map_err(|_| PeerHandlerError::CreateStorageSnapshotsDir)?;
            }
            let path =
                get_account_storages_snapshot_file(account_storages_snapshots_dir, chunk_index);
            dump_storages_to_file(&path, snapshot)
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

        for (account_done, intervals) in accounts_done {
            if intervals.is_empty() {
                account_storage_roots
                    .accounts_with_storage_root
                    .remove(&account_done);
            }
        }

        // Dropping the task sender so that the recv returns None
        drop(task_sender);

        Ok(chunk_index + 1)
    }

    #[allow(clippy::too_many_arguments)]
    async fn request_storage_ranges_worker(
        task: StorageTask,
        peer_id: H256,
        mut connection: PeerConnection,
        mut peer_table: PeerTable,
        state_root: H256,
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
            peer_id,
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
        let Ok(RLPxMessage::StorageRanges(StorageRanges {
            id: _,
            slots,
            proof,
        })) = PeerHandler::make_request(
            &mut peer_table,
            peer_id,
            &mut connection,
            request,
            PEER_REPLY_TIMEOUT,
        )
        .await
        else {
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
            peer_id,
            remaining_start,
            remaining_end,
            remaining_hash_range: (remaining_start_hash, task.end_hash),
        };
        tx.send(task_result).await.ok();
        Ok::<(), PeerHandlerError>(())
    }

    pub async fn request_state_trienodes(
        peer_id: H256,
        mut connection: PeerConnection,
        mut peer_table: PeerTable,
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
        let nodes = match PeerHandler::make_request(
            &mut peer_table,
            peer_id,
            &mut connection,
            request,
            PEER_REPLY_TIMEOUT,
        )
        .await
        {
            Ok(RLPxMessage::TrieNodes(trie_nodes)) => trie_nodes
                .nodes
                .iter()
                .map(|node| Node::decode(node))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| {
                    RequestStateTrieNodesError::RequestError(PeerConnectionError::RLPDecodeError(e))
                }),
            Ok(other_msg) => Err(RequestStateTrieNodesError::RequestError(
                PeerConnectionError::UnexpectedResponse(
                    "TrieNodes".to_string(),
                    other_msg.to_string(),
                ),
            )),
            Err(other_err) => Err(RequestStateTrieNodesError::RequestError(other_err)),
        }?;

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
        peer_id: H256,
        mut connection: PeerConnection,
        mut peer_table: PeerTable,
        get_trie_nodes: GetTrieNodes,
    ) -> Result<TrieNodes, RequestStorageTrieNodes> {
        // Keep track of peers we requested from so we can penalize unresponsive peers when we get a response
        // This is so we avoid penalizing peers due to requesting stale data
        let id = get_trie_nodes.id;
        let request = RLPxMessage::GetTrieNodes(get_trie_nodes);
        match PeerHandler::make_request(
            &mut peer_table,
            peer_id,
            &mut connection,
            request,
            PEER_REPLY_TIMEOUT,
        )
        .await
        {
            Ok(RLPxMessage::TrieNodes(trie_nodes)) => Ok(trie_nodes),
            Ok(other_msg) => Err(RequestStorageTrieNodes::RequestError(
                id,
                PeerConnectionError::UnexpectedResponse(
                    "TrieNodes".to_string(),
                    other_msg.to_string(),
                ),
            )),
            Err(e) => Err(RequestStorageTrieNodes::RequestError(id, e)),
        }
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

pub struct DumpError {
    pub path: PathBuf,
    pub contents: Vec<u8>,
    pub error: ErrorKind,
}

impl core::fmt::Debug for DumpError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DumpError")
            .field("path", &self.path)
            .field("contents_len", &self.contents.len())
            .field("error", &self.error)
            .finish()
    }
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
    #[error("Encountered an unexpected error. This is a bug {0}")]
    UnrecoverableError(String),
    #[error("Error in Peer Table: {0}")]
    PeerTableError(#[from] PeerTableError),
}

#[derive(Debug, Clone)]
pub struct RequestMetadata {
    pub hash: H256,
    pub path: Nibbles,
    /// What node is the parent of this node
    pub parent_path: Nibbles,
}

#[derive(Debug, thiserror::Error)]
pub enum RequestStateTrieNodesError {
    #[error("Send request error")]
    RequestError(PeerConnectionError),
    #[error("Invalid data")]
    InvalidData,
    #[error("Invalid Hash")]
    InvalidHash,
}

#[derive(Debug, thiserror::Error)]
pub enum RequestStorageTrieNodes {
    #[error("Send request error")]
    RequestError(u64, PeerConnectionError),
}
