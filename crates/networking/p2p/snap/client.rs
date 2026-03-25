//! Snap sync client - functions for requesting snap protocol data from peers
//!
//! This module contains all the client-side snap protocol request functions.

use crate::rlpx::message::Message as RLPxMessage;
use crate::snap::mpt_stubs::{Nibbles, Node, verify_range};
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
use ethrex_crypto::NativeCrypto;
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};
use ethrex_storage::Store;
use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    path::Path,
    sync::atomic::Ordering,
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
    peers: &mut PeerHandler,
    start: H256,
    limit: H256,
    account_state_snapshots_dir: &Path,
    pivot_header: &mut BlockHeader,
    block_sync_state: &mut SnapBlockSyncState,
) -> Result<(), SnapError> {
    METRICS
        .current_step
        .set(CurrentStepValue::RequestingAccountRanges);
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
    let mut all_account_hashes = Vec::new();
    let mut all_accounts_state = Vec::new();

    // channel to send the tasks to the peers
    let (task_sender, mut task_receiver) =
        tokio::sync::mpsc::channel::<(Vec<AccountRangeUnit>, H256, Option<(H256, H256)>)>(1000);

    info!("Starting to download account ranges from peers");

    *METRICS.account_tries_download_start_time.lock().await = Some(SystemTime::now());

    let mut completed_tasks = 0;
    let mut chunk_file = 0;
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

            let account_state_snapshots_dir_cloned = account_state_snapshots_dir.to_path_buf();
            write_set.spawn(async move {
                let path = get_account_state_snapshot_file(
                    &account_state_snapshots_dir_cloned,
                    chunk_file,
                );
                // TODO: check the error type and handle it properly
                dump_accounts_to_file(&path, account_state_chunk)
            });

            chunk_file += 1;
        }

        if last_update
            .elapsed()
            .expect("Time shouldn't be in the past")
            >= Duration::from_secs(1)
        {
            METRICS
                .downloaded_account_tries
                .store(downloaded_count, Ordering::Relaxed);
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
                "Downloaded {} accounts from peer {} (current count: {downloaded_count})",
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
                trace!("We are missing peers in request_account_range");
                logged_no_free_peers_count = 1000;
            }
            logged_no_free_peers_count -= 1;
            // Sleep a bit to avoid busy polling
            tokio::time::sleep(Duration::from_millis(10)).await;
            continue;
        };

        let Some((chunk_start, chunk_end)) = tasks_queue_not_started.pop_front() else {
            if completed_tasks >= chunk_count {
                info!("All account ranges downloaded successfully");
                break;
            }
            continue;
        };

        let tx = task_sender.clone();

        if block_is_stale(pivot_header) {
            info!("request_account_range became stale, updating pivot");
            *pivot_header = update_pivot(
                pivot_header.number,
                pivot_header.timestamp,
                peers,
                block_sync_state,
            )
            .await
            .expect("Should be able to update pivot")
        }

        let peer_table = peers.peer_table.clone();

        tokio::spawn(request_account_range_worker(
            peer_id,
            connection,
            peer_table,
            chunk_start,
            chunk_end,
            pivot_header.state_root,
            tx,
        ));
    }

    write_set
        .join_all()
        .await
        .into_iter()
        .collect::<Result<Vec<()>, DumpError>>()
        .map_err(SnapError::from)?;

    // TODO: This is repeated code, consider refactoring
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

        let path = get_account_state_snapshot_file(account_state_snapshots_dir, chunk_file);
        dump_accounts_to_file(&path, account_state_chunk)
            .inspect_err(|err| {
                error!(
                    "We had an error dumping the last accounts to disk {}",
                    err.error
                )
            })
            .map_err(|_| {
                SnapError::SnapshotDir(format!(
                    "Failed to write state snapshot chunk {}",
                    chunk_file
                ))
            })?;
    }

    METRICS
        .downloaded_account_tries
        .store(downloaded_count, Ordering::Relaxed);
    *METRICS.account_tries_download_end_time.lock().await = Some(SystemTime::now());

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
    _peers: &mut PeerHandler,
    _account_storage_roots: &mut AccountStorageRoots,
    _account_storages_snapshots_dir: &Path,
    chunk_index: u64,
    _pivot_header: &mut BlockHeader,
    _store: Store,
) -> Result<u64, SnapError> {
    // snap sync not supported on binary trie branch
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
        if node.compute_hash(&NativeCrypto).finalize(&NativeCrypto) != paths[index].hash {
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
