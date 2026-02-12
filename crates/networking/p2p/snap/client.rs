//! Snap sync client - functions for requesting snap protocol data from peers
//!
//! This module contains all the client-side snap protocol request functions.

use crate::rlpx::message::Message as RLPxMessage;
use crate::{
    metrics::{CurrentStepValue, METRICS},
    peer_handler::PeerHandler,
    peer_table::PeerTable,
    rlpx::{
        connection::server::PeerConnection,
        error::PeerConnectionError,
        p2p::SUPPORTED_SNAP_CAPABILITIES,
        snap::{
            AccountRange, AccountRangeUnit, ByteCodes, GetAccountRange, GetByteCodes,
            GetStorageRanges, GetTrieNodes, StorageRanges, TrieNodes,
        },
    },
    snap::{constants::*, encodable_to_proof, error::SnapError},
    sync::{AccountStorageRoots, SnapBlockSyncState, block_is_stale, update_pivot},
    utils::{
        AccountsWithStorage, dump_accounts_to_file, dump_storages_to_file,
        get_account_state_snapshot_file, get_account_storages_snapshot_file,
    },
};
use bytes::Bytes;
use ethrex_common::{
    BigEndianHash, H256, U256,
    types::{AccountState, BlockHeader},
};
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};
use ethrex_storage::Store;
use ethrex_trie::Nibbles;
use ethrex_trie::{Node, verify_range};
use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    path::Path,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, SystemTime},
};
use tracing::{debug, error, info, trace, warn};

// Re-export DumpError from error module
pub use super::error::DumpError;

/// Metadata for requesting trie nodes
#[derive(Debug, Clone)]
pub struct RequestMetadata {
    pub hash: H256,
    pub path: Nibbles,
    /// What node is the parent of this node
    pub parent_path: Nibbles,
}

/// Error type for storage trie node requests (includes request ID for tracking)
#[derive(Debug, thiserror::Error)]
#[error("Storage trie node request {request_id} failed: {source}")]
pub struct RequestStorageTrieNodesError {
    pub request_id: u64,
    #[source]
    pub source: SnapError,
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

/// Splits the [0, 2^256) address space into N equal, non-overlapping partitions.
fn compute_partitions(num_partitions: usize) -> Vec<(H256, H256)> {
    assert!(num_partitions > 0, "Must have at least one partition");
    if num_partitions == 1 {
        return vec![(H256::zero(), HASH_MAX)];
    }

    let partition_size = U256::MAX / num_partitions;
    let mut partitions = Vec::with_capacity(num_partitions);

    for i in 0..num_partitions {
        let start = partition_size * i;
        let end = if i == num_partitions - 1 {
            U256::MAX
        } else {
            start + partition_size - 1
        };
        partitions.push((H256::from_uint(&start), H256::from_uint(&end)));
    }

    partitions
}

/// Downloads accounts across the full address space using concurrent partitions.
///
/// Splits the address space into `MAX_ACCOUNT_PARTITIONS` equal ranges and downloads
/// each range concurrently using separate peers. All partitions write snapshot files
/// to the same directory with unique file IDs via a shared atomic counter.
pub async fn request_account_range(
    peers: &mut PeerHandler,
    account_state_snapshots_dir: &Path,
    pivot_header: &mut BlockHeader,
    block_sync_state: &Arc<tokio::sync::Mutex<SnapBlockSyncState>>,
) -> Result<(), SnapError> {
    let num_partitions = MAX_ACCOUNT_PARTITIONS;
    let partitions = compute_partitions(num_partitions);

    METRICS
        .current_step
        .set(CurrentStepValue::RequestingAccountRanges);
    *METRICS.account_tries_download_start_time.lock().await = Some(SystemTime::now());

    let pivot = Arc::new(tokio::sync::RwLock::new(pivot_header.clone()));
    let sync_state = Arc::clone(block_sync_state);
    let chunk_file_counter = Arc::new(AtomicU64::new(0));
    let downloaded_counter = Arc::new(AtomicU64::new(0));

    info!("Starting account range download with {num_partitions} concurrent partitions");

    let mut join_set = tokio::task::JoinSet::new();

    for (partition_id, (start, limit)) in partitions.into_iter().enumerate() {
        let peers_clone = peers.clone();
        let dir = account_state_snapshots_dir.to_path_buf();
        let pivot_clone = Arc::clone(&pivot);
        let sync_state_clone = Arc::clone(&sync_state);
        let counter_clone = Arc::clone(&chunk_file_counter);
        let downloaded_clone = Arc::clone(&downloaded_counter);

        join_set.spawn(async move {
            request_account_range_partition(
                peers_clone,
                start,
                limit,
                &dir,
                pivot_clone,
                sync_state_clone,
                counter_clone,
                downloaded_clone,
                partition_id,
            )
            .await
        });
    }

    // Wait for all partitions to complete
    while let Some(result) = join_set.join_next().await {
        result.map_err(|e| {
            SnapError::InternalError(format!("Account partition task panicked: {e}"))
        })??;
    }

    // Copy back the pivot header (shared via RwLock among partitions)
    *pivot_header = pivot.read().await.clone();
    // block_sync_state is shared via Arc — no copy-back needed

    METRICS
        .downloaded_account_tries
        .store(downloaded_counter.load(Ordering::Relaxed), Ordering::Relaxed);
    *METRICS.account_tries_download_end_time.lock().await = Some(SystemTime::now());

    info!("All {num_partitions} partitions completed account range download");

    Ok(())
}

/// Downloads a single partition of the account address space.
#[allow(clippy::too_many_arguments)]
async fn request_account_range_partition(
    mut peers: PeerHandler,
    start: H256,
    limit: H256,
    account_state_snapshots_dir: &Path,
    pivot_header: Arc<tokio::sync::RwLock<BlockHeader>>,
    block_sync_state: Arc<tokio::sync::Mutex<SnapBlockSyncState>>,
    chunk_file_counter: Arc<AtomicU64>,
    downloaded_counter: Arc<AtomicU64>,
    partition_id: usize,
) -> Result<(), SnapError> {
    // 1) split the range in chunks of same length
    let start_u256 = U256::from_big_endian(&start.0);
    let limit_u256 = U256::from_big_endian(&limit.0);

    let range = limit_u256 - start_u256;
    let chunk_count = U256::from(ACCOUNT_RANGE_CHUNK_COUNT)
        .min(range.max(U256::one()))
        .as_usize();
    let chunk_size = range / chunk_count;

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
        .ok_or(SnapError::NoTasks)?;
    last_task.1 = limit;

    // 2) request the chunks from peers

    let mut downloaded_count = 0_u64;
    let mut last_reported_downloaded = 0_u64;
    let mut all_account_hashes = Vec::new();
    let mut all_accounts_state = Vec::new();

    // channel to send the tasks to the peers
    let (task_sender, mut task_receiver) =
        tokio::sync::mpsc::channel::<(Vec<AccountRangeUnit>, H256, Option<(H256, H256)>)>(1000);

    info!("Partition {partition_id}: starting account range download [{start:?}..{limit:?}]");

    let mut completed_tasks = 0;
    let mut last_update: SystemTime = SystemTime::now();
    let mut write_set = tokio::task::JoinSet::new();

    let mut logged_no_free_peers_count = 0;

    loop {
        if all_accounts_state.len() * size_of::<AccountState>() >= RANGE_FILE_CHUNK_SIZE {
            let current_account_hashes = std::mem::take(&mut all_account_hashes);
            let current_account_states = std::mem::take(&mut all_accounts_state);

            let account_state_chunk = current_account_hashes
                .into_iter()
                .zip(current_account_states)
                .collect::<Vec<(H256, AccountState)>>();

            if !std::fs::exists(account_state_snapshots_dir).map_err(|_| {
                SnapError::SnapshotDir("State snapshots directory does not exist".to_string())
            })? {
                std::fs::create_dir_all(account_state_snapshots_dir).map_err(|_| {
                    SnapError::SnapshotDir("Failed to create state snapshots directory".to_string())
                })?;
            }

            let file_id = chunk_file_counter.fetch_add(1, Ordering::Relaxed);
            let account_state_snapshots_dir_cloned = account_state_snapshots_dir.to_path_buf();
            write_set.spawn(async move {
                let path = get_account_state_snapshot_file(
                    &account_state_snapshots_dir_cloned,
                    file_id,
                );
                // TODO: check the error type and handle it properly
                dump_accounts_to_file(&path, account_state_chunk)
            });
        }

        if last_update
            .elapsed()
            .expect("Time shouldn't be in the past")
            >= Duration::from_secs(1)
        {
            let delta = downloaded_count - last_reported_downloaded;
            if delta > 0 {
                downloaded_counter.fetch_add(delta, Ordering::Relaxed);
                last_reported_downloaded = downloaded_count;
            }
            METRICS
                .downloaded_account_tries
                .store(downloaded_counter.load(Ordering::Relaxed), Ordering::Relaxed);
            last_update = SystemTime::now();
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
                peers.peer_table.record_failure(&peer_id).await?;
                continue;
            }
            peers.peer_table.record_success(&peer_id).await?;

            downloaded_count += accounts.len() as u64;

            debug!(
                "Partition {partition_id}: downloaded {} accounts from peer {} (partition count: {downloaded_count})",
                accounts.len(),
                peer_id
            );
            all_account_hashes.extend(accounts.iter().map(|unit| unit.hash));
            all_accounts_state.extend(accounts.iter().map(|unit| unit.account));
        }

        let Some((peer_id, connection)) = peers
            .peer_table
            .get_best_peer(&SUPPORTED_SNAP_CAPABILITIES)
            .await
            .inspect_err(|err| warn!(%err, "Error requesting a peer for account range"))
            .unwrap_or(None)
        else {
            // Log ~ once every 10 seconds
            if logged_no_free_peers_count == 0 {
                trace!("Partition {partition_id}: waiting for peers");
                logged_no_free_peers_count = 1000;
            }
            logged_no_free_peers_count -= 1;
            // Sleep a bit to avoid busy polling
            tokio::time::sleep(Duration::from_millis(10)).await;
            continue;
        };

        let Some((chunk_start, chunk_end)) = tasks_queue_not_started.pop_front() else {
            if completed_tasks >= chunk_count {
                info!("Partition {partition_id}: all account ranges downloaded");
                break;
            }
            continue;
        };

        let tx = task_sender.clone();

        // Check pivot staleness with read lock first, then upgrade to write if needed
        let state_root = {
            let is_stale = block_is_stale(&*pivot_header.read().await);
            if is_stale {
                let mut ph = pivot_header.write().await;
                // Double-check under write lock (another partition may have updated)
                if block_is_stale(&ph) {
                    info!("Partition {partition_id}: pivot is stale, updating");
                    // update_pivot locks block_sync_state internally — don't hold it here
                    *ph = update_pivot(ph.number, ph.timestamp, &mut peers, &block_sync_state)
                        .await
                        .expect("Should be able to update pivot");
                }
                ph.state_root
            } else {
                pivot_header.read().await.state_root
            }
        };

        let peer_table = peers.peer_table.clone();

        tokio::spawn(request_account_range_worker(
            peer_id,
            connection,
            peer_table,
            chunk_start,
            chunk_end,
            state_root,
            tx,
        ));
    }

    write_set
        .join_all()
        .await
        .into_iter()
        .collect::<Result<Vec<()>, DumpError>>()
        .map_err(SnapError::from)?;

    // Flush remaining accounts to disk
    {
        let current_account_hashes = std::mem::take(&mut all_account_hashes);
        let current_account_states = std::mem::take(&mut all_accounts_state);

        let account_state_chunk = current_account_hashes
            .into_iter()
            .zip(current_account_states)
            .collect::<Vec<(H256, AccountState)>>();

        if !std::fs::exists(account_state_snapshots_dir).map_err(|_| {
            SnapError::SnapshotDir("State snapshots directory does not exist".to_string())
        })? {
            std::fs::create_dir_all(account_state_snapshots_dir).map_err(|_| {
                SnapError::SnapshotDir("Failed to create state snapshots directory".to_string())
            })?;
        }

        let file_id = chunk_file_counter.fetch_add(1, Ordering::Relaxed);
        let path = get_account_state_snapshot_file(account_state_snapshots_dir, file_id);
        dump_accounts_to_file(&path, account_state_chunk)
            .inspect_err(|err| {
                error!(
                    "Partition {partition_id}: error dumping last accounts to disk {}",
                    err.error
                )
            })
            .map_err(|_| {
                SnapError::SnapshotDir(format!(
                    "Failed to write state snapshot chunk (partition {partition_id})"
                ))
            })?;
    }

    // Report final download count
    let delta = downloaded_count - last_reported_downloaded;
    if delta > 0 {
        downloaded_counter.fetch_add(delta, Ordering::Relaxed);
    }

    Ok(())
}

/// Requests bytecodes for the given code hashes
/// Returns the bytecodes or None if:
/// - There are no available peers (the node just started up or was rejected by all other nodes)
/// - No peer returned a valid response in the given time and retry limits
pub async fn request_bytecodes(
    peers: &mut PeerHandler,
    all_bytecode_hashes: &[H256],
) -> Result<Option<Vec<Bytes>>, SnapError> {
    METRICS
        .current_step
        .set(CurrentStepValue::RequestingBytecodes);
    if all_bytecode_hashes.is_empty() {
        return Ok(Some(Vec::new()));
    }
    const MAX_BYTECODES_REQUEST_SIZE: usize = 100;
    // 1) split the range in chunks of same length
    let chunk_count = 800;
    let chunk_count = chunk_count.min(all_bytecode_hashes.len());
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
        .ok_or(SnapError::NoTasks)?;
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
                peers.peer_table.record_failure(&peer_id).await?;
                continue;
            }

            downloaded_count += bytecodes.len() as u64;

            peers.peer_table.record_success(&peer_id).await?;
            for (i, bytecode) in bytecodes.into_iter().enumerate() {
                all_bytecodes[start_index + i] = bytecode;
            }
        }

        let Some((peer_id, mut connection)) = peers
            .peer_table
            .get_best_peer(&SUPPORTED_SNAP_CAPABILITIES)
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

        let mut peer_table = peers.peer_table.clone();

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
    peers: &mut PeerHandler,
    account_storage_roots: &mut AccountStorageRoots,
    account_storages_snapshots_dir: &Path,
    mut chunk_index: u64,
    pivot_header: &mut BlockHeader,
    store: Store,
) -> Result<u64, SnapError> {
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
                    .get_account_state_by_acc_hash(pivot_header.hash(), *account)?
                    .ok_or_else(|| {
                        SnapError::InternalError(
                            "Could not find account that should have been downloaded or healed"
                                .to_string(),
                        )
                    })?
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
    let chunk_size = STORAGE_BATCH_SIZE;
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
    let (task_sender, mut task_receiver) = tokio::sync::mpsc::channel::<StorageTaskResult>(1000);

    // channel to send the result of dumping storages
    let mut disk_joinset: tokio::task::JoinSet<Result<(), DumpError>> = tokio::task::JoinSet::new();

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

            if !std::fs::exists(account_storages_snapshots_dir).map_err(|_| {
                SnapError::SnapshotDir("Storage snapshots directory does not exist".to_string())
            })? {
                std::fs::create_dir_all(account_storages_snapshots_dir).map_err(|_| {
                    SnapError::SnapshotDir(
                        "Failed to create storage snapshots directory".to_string(),
                    )
                })?;
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
                    .inspect_err(|err| error!("We found this error while dumping to file {err:?}"))
                    .map_err(SnapError::from)?;
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
                                .ok_or(SnapError::InternalError("Tried to get the old download intervals for an account but did not find them".to_owned()))?;

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

                        let acc_hash = *accounts_by_root_hash[remaining_start]
                            .1
                            .first()
                            .ok_or(SnapError::InternalError("Empty accounts vector".to_owned()))?;
                        let (_, old_intervals) = account_storage_roots
                                .accounts_with_storage_root
                                .get_mut(&acc_hash).ok_or(SnapError::InternalError("Tried to get the old download intervals for an account but did not find them".to_owned()))?;
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
                                .ok_or(SnapError::InternalError("Tried to get the old download intervals for an account but did not find them".to_owned()))?;
                        old_intervals.remove(
                            old_intervals
                                .iter()
                                .position(|(_old_start, end)| end == &hash_end)
                                .ok_or(SnapError::InternalError(
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
                        .ok_or(SnapError::NoAccountStorages)?
                        .max(1);
                    let storage_density = start_hash_u256 / slot_count;

                    let slots_per_chunk = U256::from(10000);
                    let chunk_size = storage_density
                        .checked_mul(slots_per_chunk)
                        .unwrap_or(U256::MAX);

                    let chunk_count = (missing_storage_range / chunk_size).as_usize().max(1);

                    let first_acc_hash = *accounts_by_root_hash[remaining_start]
                        .1
                        .first()
                        .ok_or(SnapError::InternalError("Empty accounts vector".to_owned()))?;

                    let maybe_old_intervals = account_storage_roots
                        .accounts_with_storage_root
                        .get(&first_acc_hash);

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
                            account_storage_roots
                                .accounts_with_storage_root
                                .insert(first_acc_hash, (None, vec![]));
                            let (_, intervals) = account_storage_roots
                                    .accounts_with_storage_root
                                    .get_mut(&first_acc_hash)
                                    .ok_or(SnapError::InternalError("Tried to get the old download intervals for an account but did not find them".to_owned()))?;

                            for i in 0..chunk_count {
                                let start_hash_u256 = start_hash_u256 + chunk_size * i;
                                let start_hash = H256::from_uint(&start_hash_u256);
                                let end_hash = if i == chunk_count - 1 {
                                    HASH_MAX
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
                        account_storage_roots
                            .accounts_with_storage_root
                            .insert(first_acc_hash, (None, vec![]));
                        let (_, intervals) = account_storage_roots
                                .accounts_with_storage_root
                                .get_mut(&first_acc_hash)
                                .ok_or(SnapError::InternalError("Tried to get the old download intervals for an account but did not find them".to_owned()))?;

                        for i in 0..chunk_count {
                            let start_hash_u256 = start_hash_u256 + chunk_size * i;
                            let start_hash = H256::from_uint(&start_hash_u256);
                            let end_hash = if i == chunk_count - 1 {
                                HASH_MAX
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

                            intervals.push((start_hash, end_hash));

                            tasks_queue_not_started.push_back(task);
                            task_count += 1;
                        }
                        debug!("Split big storage account into {chunk_count} chunks.");
                    }
                }
            }

            if account_storages.is_empty() {
                peers.peer_table.record_failure(&peer_id).await?;
                continue;
            }
            if let Some(hash_end) = hash_end {
                // This is a big storage account, and the range might be empty
                if account_storages[0].len() == 1 && account_storages[0][0].0 > hash_end {
                    continue;
                }
            }

            peers.peer_table.record_success(&peer_id).await?;

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

        let Some((peer_id, connection)) = peers
            .peer_table
            .get_best_peer(&SUPPORTED_SNAP_CAPABILITIES)
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
        let (chunk_account_hashes, chunk_storage_roots): (Vec<_>, Vec<_>) = accounts_by_root_hash
            [task.start_index..task.end_index]
            .iter()
            .map(|(root, storages)| (*storages.first().unwrap_or(&H256::zero()), *root))
            .unzip();

        if task_count - completed_tasks < 30 {
            debug!(
                "Assigning task: {task:?}, account_hash: {}, storage_root: {}",
                chunk_account_hashes.first().unwrap_or(&H256::zero()),
                chunk_storage_roots.first().unwrap_or(&H256::zero()),
            );
        }
        let peer_table = peers.peer_table.clone();

        tokio::spawn(request_storage_ranges_worker(
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

        if !std::fs::exists(account_storages_snapshots_dir).map_err(|_| {
            SnapError::SnapshotDir("Storage snapshots directory does not exist".to_string())
        })? {
            std::fs::create_dir_all(account_storages_snapshots_dir).map_err(|_| {
                SnapError::SnapshotDir("Failed to create storage snapshots directory".to_string())
            })?;
        }
        let path = get_account_storages_snapshot_file(account_storages_snapshots_dir, chunk_index);
        dump_storages_to_file(&path, snapshot).map_err(|_| {
            SnapError::SnapshotDir(format!(
                "Failed to write storage snapshot chunk {}",
                chunk_index
            ))
        })?;
    }
    disk_joinset
        .join_all()
        .await
        .into_iter()
        .map(|result| {
            result.inspect_err(|err| error!("We found this error while dumping to file {err:?}"))
        })
        .collect::<Result<Vec<()>, DumpError>>()
        .map_err(SnapError::from)?;

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

pub async fn request_state_trienodes(
    peer_id: H256,
    mut connection: PeerConnection,
    mut peer_table: PeerTable,
    state_root: H256,
    paths: Vec<RequestMetadata>,
) -> Result<Vec<Node>, SnapError> {
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
            .map_err(SnapError::from),
        Ok(other_msg) => Err(SnapError::Protocol(
            PeerConnectionError::UnexpectedResponse("TrieNodes".to_string(), other_msg.to_string()),
        )),
        Err(other_err) => Err(SnapError::Protocol(other_err)),
    }?;

    if nodes.is_empty() || nodes.len() > expected_nodes {
        return Err(SnapError::InvalidData);
    }

    for (index, node) in nodes.iter().enumerate() {
        if node.compute_hash().finalize() != paths[index].hash {
            error!(
                "A peer is sending wrong data for the state trie node {:?}",
                paths[index].path
            );
            return Err(SnapError::InvalidHash);
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
) -> Result<TrieNodes, RequestStorageTrieNodesError> {
    // Keep track of peers we requested from so we can penalize unresponsive peers when we get a response
    // This is so we avoid penalizing peers due to requesting stale data
    let request_id = get_trie_nodes.id;
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
        Ok(other_msg) => Err(RequestStorageTrieNodesError {
            request_id,
            source: SnapError::Protocol(PeerConnectionError::UnexpectedResponse(
                "TrieNodes".to_string(),
                other_msg.to_string(),
            )),
        }),
        Err(e) => Err(RequestStorageTrieNodesError {
            request_id,
            source: SnapError::Protocol(e),
        }),
    }
}

#[allow(clippy::type_complexity)]
async fn request_account_range_worker(
    peer_id: H256,
    mut connection: PeerConnection,
    mut peer_table: PeerTable,
    chunk_start: H256,
    chunk_end: H256,
    state_root: H256,
    tx: tokio::sync::mpsc::Sender<(Vec<AccountRangeUnit>, H256, Option<(H256, H256)>)>,
) -> Result<(), SnapError> {
    debug!("Requesting account range from peer {peer_id}, chunk: {chunk_start:?} - {chunk_end:?}");
    let request_id = rand::random();
    let request = RLPxMessage::GetAccountRange(GetAccountRange {
        id: request_id,
        root_hash: state_root,
        starting_hash: chunk_start,
        limit_hash: chunk_end,
        response_bytes: MAX_RESPONSE_BYTES,
    });
    if let Ok(RLPxMessage::AccountRange(AccountRange {
        id: _,
        accounts,
        proof,
    })) = PeerHandler::make_request(
        &mut peer_table,
        peer_id,
        &mut connection,
        request,
        PEER_REPLY_TIMEOUT,
    )
    .await
    {
        if accounts.is_empty() {
            tx.send((Vec::new(), peer_id, Some((chunk_start, chunk_end))))
                .await
                .ok();
            return Ok(());
        }
        // Unzip & validate response
        let proof = encodable_to_proof(&proof);
        let (account_hashes, account_states): (Vec<_>, Vec<_>) = accounts
            .clone()
            .into_iter()
            .map(|unit| (unit.hash, unit.account))
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
            tx.send((Vec::new(), peer_id, Some((chunk_start, chunk_end))))
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
                    tx.send((Vec::new(), peer_id, Some((chunk_start, chunk_end))))
                        .await
                        .ok();
                    error!("Account hashes last failed, this shouldn't happen");
                    return Err(SnapError::NoAccountHashes);
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
            peer_id,
            chunk_left,
        ))
        .await
        .ok();
    } else {
        tracing::debug!("Failed to get account range");
        tx.send((Vec::new(), peer_id, Some((chunk_start, chunk_end))))
            .await
            .ok();
    }
    Ok::<(), SnapError>(())
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
) -> Result<(), SnapError> {
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
                return Err(SnapError::NoStorageRoots);
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
                return Err(SnapError::NoAccountStorages);
            }
        };
        let (last_hash, _) = match last_account_storage.last() {
            Some(last_hash) => last_hash,
            None => {
                tx.send(empty_task_result.clone()).await.ok();
                error!("No last hash found, this shouldn't happen");
                return Err(SnapError::NoAccountStorages);
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
    Ok::<(), SnapError>(())
}
