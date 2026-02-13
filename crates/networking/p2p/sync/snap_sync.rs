//! Snap sync implementation
//!
//! This module contains the logic for snap synchronization mode where state is
//! fetched via snap p2p requests while blocks and receipts are fetched in parallel.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;
#[cfg(feature = "rocksdb")]
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, SystemTime};

use ethrex_blockchain::Blockchain;
use ethrex_common::types::{AccountState, BlockHeader, Code};
use ethrex_common::{
    H256,
    constants::{EMPTY_KECCACK_HASH, EMPTY_TRIE_HASH},
};
use ethrex_rlp::decode::RLPDecode;
use ethrex_storage::Store;
#[cfg(feature = "rocksdb")]
use ethrex_trie::Trie;
use rayon::iter::{ParallelBridge, ParallelIterator};
use tracing::{debug, error, info, warn};

use crate::metrics::{CurrentStepValue, METRICS};
use crate::peer_handler::PeerHandler;
use crate::rlpx::p2p::SUPPORTED_ETH_CAPABILITIES;
use crate::snap::{
    constants::{
        BYTECODE_CHUNK_SIZE, MAX_HEADER_FETCH_ATTEMPTS, MIN_FULL_BLOCKS, MISSING_SLOTS_PERCENTAGE,
        SECONDS_PER_BLOCK, SNAP_LIMIT,
    },
    request_account_range, request_bytecodes, request_storage_ranges,
};
use crate::sync::code_collector::CodeHashCollector;
use crate::sync::healing::{heal_state_trie_wrap, heal_storage_trie};
use crate::utils::{
    current_unix_time, get_account_state_snapshots_dir, get_account_storages_snapshots_dir,
    get_code_hashes_snapshots_dir,
};

use super::{AccountStorageRoots, SyncError};

#[cfg(not(feature = "rocksdb"))]
use ethrex_common::U256;
#[cfg(not(feature = "rocksdb"))]
use ethrex_rlp::encode::RLPEncode;

/// Persisted State during the Block Sync phase for SnapSync
#[derive(Clone)]
pub struct SnapBlockSyncState {
    pub block_hashes: Vec<H256>,
    store: Store,
}

impl SnapBlockSyncState {
    pub fn new(store: Store) -> Self {
        Self {
            block_hashes: Vec::new(),
            store,
        }
    }

    /// Obtain the current head from where to start or resume block sync
    pub async fn get_current_head(&self) -> Result<H256, SyncError> {
        if let Some(head) = self.store.get_header_download_checkpoint().await? {
            Ok(head)
        } else {
            self.store
                .get_latest_canonical_block_hash()
                .await?
                .ok_or(SyncError::NoLatestCanonical)
        }
    }

    /// Stores incoming headers to the Store and saves their hashes
    pub async fn process_incoming_headers(
        &mut self,
        block_headers: impl Iterator<Item = BlockHeader>,
    ) -> Result<(), SyncError> {
        let mut block_headers_vec = Vec::with_capacity(block_headers.size_hint().1.unwrap_or(0));
        let mut block_hashes = Vec::with_capacity(block_headers.size_hint().1.unwrap_or(0));
        for header in block_headers {
            block_hashes.push(header.hash());
            block_headers_vec.push(header);
        }
        self.store
            .set_header_download_checkpoint(
                *block_hashes.last().ok_or(SyncError::InvalidRangeReceived)?,
            )
            .await?;
        self.block_hashes.extend_from_slice(&block_hashes);
        self.store.add_block_headers(block_headers_vec).await?;
        Ok(())
    }
}

/// Performs snap sync cycle - fetches state via snap protocol while downloading blocks in parallel
pub async fn sync_cycle_snap(
    peers: &mut PeerHandler,
    blockchain: Arc<Blockchain>,
    snap_enabled: &std::sync::atomic::AtomicBool,
    sync_head: H256,
    store: Store,
    datadir: &Path,
) -> Result<(), SyncError> {
    // Request all block headers between the current head and the sync head
    // We will begin from the current head so that we download the earliest state first
    // This step is not parallelized
    let mut block_sync_state = SnapBlockSyncState::new(store.clone());
    // Check if we have some blocks downloaded from a previous sync attempt
    // This applies only to snap sync—full sync always starts fetching headers
    // from the canonical block, which updates as new block headers are fetched.
    let mut current_head = block_sync_state.get_current_head().await?;
    let mut current_head_number = store
        .get_block_number(current_head)
        .await?
        .ok_or(SyncError::BlockNumber(current_head))?;
    info!(
        "Syncing from current head {:?} to sync_head {:?}",
        current_head, sync_head
    );
    let pending_block = match store.get_pending_block(sync_head).await {
        Ok(res) => res,
        Err(e) => return Err(e.into()),
    };

    let mut attempts = 0;

    loop {
        debug!("Requesting Block Headers from {current_head}");

        let Some(mut block_headers) = peers
            .request_block_headers(current_head_number, sync_head)
            .await?
        else {
            if attempts > MAX_HEADER_FETCH_ATTEMPTS {
                warn!("Sync failed to find target block header, aborting");
                return Ok(());
            }
            attempts += 1;
            tokio::time::sleep(Duration::from_millis(1.1_f64.powf(attempts as f64) as u64)).await;
            continue;
        };

        debug!("Sync Log 1: In snap sync");
        debug!(
            "Sync Log 2: State block hashes len {}",
            block_sync_state.block_hashes.len()
        );

        let (first_block_hash, first_block_number, first_block_parent_hash) =
            match block_headers.first() {
                Some(header) => (header.hash(), header.number, header.parent_hash),
                None => continue,
            };
        let (last_block_hash, last_block_number) = match block_headers.last() {
            Some(header) => (header.hash(), header.number),
            None => continue,
        };
        // TODO(#2126): This is just a temporary solution to avoid a bug where the sync would get stuck
        // on a loop when the target head is not found, i.e. on a reorg with a side-chain.
        if first_block_hash == last_block_hash
            && first_block_hash == current_head
            && current_head != sync_head
        {
            // There is no path to the sync head this goes back until it find a common ancerstor
            warn!("Sync failed to find target block header, going back to the previous parent");
            current_head = first_block_parent_hash;
            continue;
        }

        debug!(
            "Received {} block headers| First Number: {} Last Number: {}",
            block_headers.len(),
            first_block_number,
            last_block_number
        );

        // If we have a pending block from new_payload request
        // attach it to the end if it matches the parent_hash of the latest received header
        if let Some(ref block) = pending_block
            && block.header.parent_hash == last_block_hash
        {
            block_headers.push(block.header.clone());
        }

        // Filter out everything after the sync_head
        let mut sync_head_found = false;
        if let Some(index) = block_headers
            .iter()
            .position(|header| header.hash() == sync_head)
        {
            sync_head_found = true;
            block_headers.drain(index + 1..);
        }

        // Update current fetch head
        current_head = last_block_hash;
        current_head_number = last_block_number;

        // If the sync head is not 0 we search to fullsync
        let head_found = sync_head_found && store.get_latest_block_number().await? > 0;
        // Or the head is very close to 0
        let head_close_to_0 = last_block_number < MIN_FULL_BLOCKS;

        if head_found || head_close_to_0 {
            // Too few blocks for a snap sync, switching to full sync
            info!("Sync head is found, switching to FullSync");
            snap_enabled.store(false, Ordering::Relaxed);
            return super::full::sync_cycle_full(
                peers,
                blockchain,
                tokio_util::sync::CancellationToken::new(),
                sync_head,
                store.clone(),
            )
            .await;
        }

        // Discard the first header as we already have it
        if block_headers.len() > 1 {
            let block_headers_iter = block_headers.into_iter().skip(1);

            block_sync_state
                .process_incoming_headers(block_headers_iter)
                .await?;
        }

        if sync_head_found {
            break;
        };
    }

    snap_sync(peers, &store, &mut block_sync_state, datadir).await?;

    store.clear_snap_state().await?;
    snap_enabled.store(false, Ordering::Relaxed);

    Ok(())
}

/// Main snap sync logic - downloads state via snap protocol
pub async fn snap_sync(
    peers: &mut PeerHandler,
    store: &Store,
    block_sync_state: &mut SnapBlockSyncState,
    datadir: &Path,
) -> Result<(), SyncError> {
    let capture_dir = std::env::var("ETHREX_SNAP_PROFILE_CAPTURE_DIR")
        .ok()
        .filter(|s| !s.is_empty())
        .map(std::path::PathBuf::from);

    // snap-sync: launch tasks to fetch blocks and state in parallel
    // - Fetch each block's body and its receipt via eth p2p requests
    // - Fetch the pivot block's state via snap p2p requests
    // - Execute blocks after the pivot (like in full-sync)
    let pivot_hash = block_sync_state
        .block_hashes
        .last()
        .ok_or(SyncError::NoBlockHeaders)?;
    let mut pivot_header = store
        .get_block_header_by_hash(*pivot_hash)?
        .ok_or(SyncError::CorruptDB)?;

    while block_is_stale(&pivot_header) {
        pivot_header = update_pivot(
            pivot_header.number,
            pivot_header.timestamp,
            peers,
            block_sync_state,
        )
        .await?;
    }
    debug!(
        "Selected block {} as pivot for snap sync",
        pivot_header.number
    );

    let state_root = pivot_header.state_root;
    let account_state_snapshots_dir = get_account_state_snapshots_dir(datadir);
    let account_storages_snapshots_dir = get_account_storages_snapshots_dir(datadir);

    let code_hashes_snapshot_dir = get_code_hashes_snapshots_dir(datadir);
    std::fs::create_dir_all(&code_hashes_snapshot_dir).map_err(|e| {
        SyncError::FileSystem(format!(
            "Failed to create {code_hashes_snapshot_dir:?}: {e}"
        ))
    })?;

    // Create collector to store code hashes in files
    let mut code_hash_collector: CodeHashCollector =
        CodeHashCollector::new(code_hashes_snapshot_dir.clone());

    let mut storage_accounts = AccountStorageRoots::default();
    if !std::env::var("SKIP_START_SNAP_SYNC").is_ok_and(|var| !var.is_empty()) {
        // We start by downloading all of the leafs of the trie of accounts
        // The function request_account_range writes the leafs into files in
        // account_state_snapshots_dir

        info!("Starting to download account ranges from peers");
        request_account_range(
            peers,
            H256::zero(),
            H256::repeat_byte(0xff),
            account_state_snapshots_dir.as_ref(),
            &mut pivot_header,
            block_sync_state,
        )
        .await?;
        info!("Finish downloading account ranges from peers");

        *METRICS.account_tries_insert_start_time.lock().await = Some(SystemTime::now());
        METRICS
            .current_step
            .set(CurrentStepValue::InsertingAccountRanges);
        // We read the account leafs from the files in account_state_snapshots_dir, write it into
        // the trie to compute the nodes and stores the accounts with storages for later use

        // Variable `accounts_with_storage` unused if not in rocksdb
        #[allow(unused_variables)]
        let (computed_state_root, accounts_with_storage) = insert_accounts(
            store.clone(),
            &mut storage_accounts,
            &account_state_snapshots_dir,
            datadir,
            &mut code_hash_collector,
        )
        .await?;
        info!(
            "Finished inserting account ranges, total storage accounts: {}",
            storage_accounts.accounts_with_storage_root.len()
        );
        *METRICS.account_tries_insert_end_time.lock().await = Some(SystemTime::now());

        info!("Original state root: {state_root:?}");
        info!("Computed state root after request_account_rages: {computed_state_root:?}");

        *METRICS.storage_tries_download_start_time.lock().await = Some(SystemTime::now());
        // We start downloading the storage leafs. To do so, we need to be sure that the storage root
        // is correct. To do so, we always heal the state trie before requesting storage rates
        let mut chunk_index = 0_u64;
        let mut state_leafs_healed = 0_u64;
        let mut storage_range_request_attempts = 0;
        loop {
            while block_is_stale(&pivot_header) {
                pivot_header = update_pivot(
                    pivot_header.number,
                    pivot_header.timestamp,
                    peers,
                    block_sync_state,
                )
                .await?;
            }
            // heal_state_trie_wrap returns false if we ran out of time before fully healing the trie
            // We just need to update the pivot and start again
            if !heal_state_trie_wrap(
                pivot_header.state_root,
                store.clone(),
                peers,
                calculate_staleness_timestamp(pivot_header.timestamp),
                &mut state_leafs_healed,
                &mut storage_accounts,
                &mut code_hash_collector,
            )
            .await?
            {
                continue;
            };

            info!(
                "Started request_storage_ranges with {} accounts with storage root unchanged",
                storage_accounts.accounts_with_storage_root.len()
            );
            storage_range_request_attempts += 1;
            if storage_range_request_attempts < 5 {
                chunk_index = request_storage_ranges(
                    peers,
                    &mut storage_accounts,
                    account_storages_snapshots_dir.as_ref(),
                    chunk_index,
                    &mut pivot_header,
                    store.clone(),
                )
                .await?;
            } else {
                for (acc_hash, (maybe_root, old_intervals)) in
                    storage_accounts.accounts_with_storage_root.iter()
                {
                    // When we fall into this case what happened is there are certain accounts for which
                    // the storage root went back to a previous value we already had, and thus could not download
                    // their storage leaves because we were using an old value for their storage root.
                    // The fallback is to ensure we mark it for storage healing.
                    storage_accounts.healed_accounts.insert(*acc_hash);
                    debug!(
                        "We couldn't download these accounts on request_storage_ranges. Falling back to storage healing for it.
                        Account hash: {:x?}, {:x?}. Number of intervals {}",
                        acc_hash,
                        maybe_root,
                        old_intervals.len()
                    );
                }

                warn!(
                    "Storage could not be downloaded after multiple attempts. Marking for healing.
                    This could impact snap sync time (healing may take a while)."
                );

                storage_accounts.accounts_with_storage_root.clear();
            }

            info!(
                "Ended request_storage_ranges with {} accounts with storage root unchanged and not downloaded yet and with {} big/healed accounts",
                storage_accounts.accounts_with_storage_root.len(),
                // These accounts are marked as heals if they're a big account. This is
                // because we don't know if the storage root is still valid
                storage_accounts.healed_accounts.len(),
            );
            if !block_is_stale(&pivot_header) {
                break;
            }
            info!("We stopped because of staleness, restarting loop");
        }
        info!("Finished request_storage_ranges");
        *METRICS.storage_tries_download_end_time.lock().await = Some(SystemTime::now());

        *METRICS.storage_tries_insert_start_time.lock().await = Some(SystemTime::now());
        METRICS
            .current_step
            .set(CurrentStepValue::InsertingStorageRanges);
        let account_storages_snapshots_dir = get_account_storages_snapshots_dir(datadir);

        insert_storages(
            store.clone(),
            accounts_with_storage,
            &account_storages_snapshots_dir,
            datadir,
        )
        .await?;

        *METRICS.storage_tries_insert_end_time.lock().await = Some(SystemTime::now());

        info!("Finished storing storage tries");

        // Capture dataset for offline profiling if requested
        if let Some(ref capture_dir) = capture_dir {
            capture_snap_profile_dataset(
                capture_dir,
                &account_state_snapshots_dir,
                &account_storages_snapshots_dir,
                &pivot_header,
                computed_state_root,
            )?;
        }

        // Clean up snapshot directories (after capture has copied them, if active)
        std::fs::remove_dir_all(&account_state_snapshots_dir)
            .map_err(|_| SyncError::AccountStateSnapshotsDirNotFound)?;
        std::fs::remove_dir_all(&account_storages_snapshots_dir)
            .map_err(|_| SyncError::AccountStoragesSnapshotsDirNotFound)?;
    }

    *METRICS.heal_start_time.lock().await = Some(SystemTime::now());
    info!("Starting Healing Process");
    let mut global_state_leafs_healed: u64 = 0;
    let mut global_storage_leafs_healed: u64 = 0;
    let mut healing_done = false;
    while !healing_done {
        // This if is an edge case for the skip snap sync scenario
        if block_is_stale(&pivot_header) {
            pivot_header = update_pivot(
                pivot_header.number,
                pivot_header.timestamp,
                peers,
                block_sync_state,
            )
            .await?;
        }
        healing_done = heal_state_trie_wrap(
            pivot_header.state_root,
            store.clone(),
            peers,
            calculate_staleness_timestamp(pivot_header.timestamp),
            &mut global_state_leafs_healed,
            &mut storage_accounts,
            &mut code_hash_collector,
        )
        .await?;
        if !healing_done {
            continue;
        }
        healing_done = heal_storage_trie(
            pivot_header.state_root,
            &storage_accounts,
            peers,
            store.clone(),
            HashMap::new(),
            calculate_staleness_timestamp(pivot_header.timestamp),
            &mut global_storage_leafs_healed,
        )
        .await?;
    }
    *METRICS.heal_end_time.lock().await = Some(SystemTime::now());

    store.generate_flatkeyvalue()?;

    debug_assert!(validate_state_root(store.clone(), pivot_header.state_root).await);
    debug_assert!(validate_storage_root(store.clone(), pivot_header.state_root).await);

    info!("Finished healing");

    // Finish code hash collection
    code_hash_collector.finish().await?;

    *METRICS.bytecode_download_start_time.lock().await = Some(SystemTime::now());

    let code_hashes_dir = get_code_hashes_snapshots_dir(datadir);
    let mut seen_code_hashes = HashSet::new();
    let mut code_hashes_to_download = Vec::new();

    info!("Starting download code hashes from peers");
    for entry in std::fs::read_dir(&code_hashes_dir)
        .map_err(|_| SyncError::CodeHashesSnapshotsDirNotFound)?
    {
        let entry =
            entry.map_err(|e| SyncError::FileSystem(format!("Failed to read dir entry: {e}")))?;
        let snapshot_contents = std::fs::read(entry.path())
            .map_err(|err| SyncError::SnapshotReadError(entry.path(), err))?;
        let code_hashes: Vec<H256> = RLPDecode::decode(&snapshot_contents)
            .map_err(|_| SyncError::CodeHashesSnapshotDecodeError(entry.path()))?;

        for hash in code_hashes {
            // If we haven't seen the code hash yet, add it to the list of hashes to download
            if seen_code_hashes.insert(hash) {
                code_hashes_to_download.push(hash);

                if code_hashes_to_download.len() >= BYTECODE_CHUNK_SIZE {
                    info!(
                        "Starting bytecode download of {} hashes",
                        code_hashes_to_download.len()
                    );
                    let bytecodes = request_bytecodes(peers, &code_hashes_to_download)
                        .await?
                        .ok_or(SyncError::BytecodesNotFound)?;

                    store
                        .write_account_code_batch(
                            code_hashes_to_download
                                .drain(..)
                                .zip(bytecodes)
                                // SAFETY: hash already checked by the download worker
                                .map(|(hash, code)| {
                                    (hash, Code::from_bytecode_unchecked(code, hash))
                                })
                                .collect(),
                        )
                        .await?;
                }
            }
        }
    }

    // Download remaining bytecodes if any
    if !code_hashes_to_download.is_empty() {
        let bytecodes = request_bytecodes(peers, &code_hashes_to_download)
            .await?
            .ok_or(SyncError::BytecodesNotFound)?;
        store
            .write_account_code_batch(
                code_hashes_to_download
                    .drain(..)
                    .zip(bytecodes)
                    // SAFETY: hash already checked by the download worker
                    .map(|(hash, code)| (hash, Code::from_bytecode_unchecked(code, hash)))
                    .collect(),
            )
            .await?;
    }

    std::fs::remove_dir_all(code_hashes_dir)
        .map_err(|_| SyncError::CodeHashesSnapshotsDirNotFound)?;

    *METRICS.bytecode_download_end_time.lock().await = Some(SystemTime::now());

    debug_assert!(validate_bytecodes(store.clone(), pivot_header.state_root));

    store_block_bodies(vec![pivot_header.clone()], peers.clone(), store.clone()).await?;

    let block = store
        .get_block_by_hash(pivot_header.hash())
        .await?
        .ok_or(SyncError::CorruptDB)?;

    store.add_block(block).await?;

    let numbers_and_hashes = block_sync_state
        .block_hashes
        .iter()
        .rev()
        .enumerate()
        .map(|(i, hash)| (pivot_header.number - i as u64, *hash))
        .collect::<Vec<_>>();

    store
        .forkchoice_update(
            numbers_and_hashes,
            pivot_header.number,
            pivot_header.hash(),
            None,
            None,
        )
        .await?;
    Ok(())
}

/// Fetches all block bodies for the given block headers via p2p and stores them
pub async fn store_block_bodies(
    mut block_headers: Vec<BlockHeader>,
    mut peers: PeerHandler,
    store: Store,
) -> Result<(), SyncError> {
    loop {
        debug!("Requesting Block Bodies ");
        if let Some(block_bodies) = peers.request_block_bodies(&block_headers).await? {
            debug!(" Received {} Block Bodies", block_bodies.len());
            // Track which bodies we have already fetched
            let current_block_headers = block_headers.drain(..block_bodies.len());
            // Add bodies to storage
            for (hash, body) in current_block_headers
                .map(|h| h.hash())
                .zip(block_bodies.into_iter())
            {
                store.add_block_body(hash, body).await?;
            }

            // Check if we need to ask for another batch
            if block_headers.is_empty() {
                break;
            }
        }
    }
    Ok(())
}

pub async fn update_pivot(
    block_number: u64,
    block_timestamp: u64,
    peers: &mut PeerHandler,
    block_sync_state: &mut SnapBlockSyncState,
) -> Result<BlockHeader, SyncError> {
    // We multiply the estimation by 0.9 in order to account for missing slots (~9% in tesnets)
    let new_pivot_block_number = block_number
        + ((current_unix_time().saturating_sub(block_timestamp) / SECONDS_PER_BLOCK) as f64
            * MISSING_SLOTS_PERCENTAGE) as u64;
    debug!(
        "Current pivot is stale (number: {}, timestamp: {}). New pivot number: {}",
        block_number, block_timestamp, new_pivot_block_number
    );
    loop {
        let Some((peer_id, mut connection)) = peers
            .peer_table
            .get_best_peer(&SUPPORTED_ETH_CAPABILITIES)
            .await?
        else {
            // When we come here, we may be waiting for requests to timeout.
            // Because we're waiting for a timeout, we sleep so the rest of the code
            // can get to them
            debug!("We tried to get peers during update_pivot, but we found no free peers");
            tokio::time::sleep(Duration::from_secs(1)).await;
            continue;
        };

        let peer_score = peers.peer_table.get_score(&peer_id).await?;
        info!(
            "Trying to update pivot to {new_pivot_block_number} with peer {peer_id} (score: {peer_score})"
        );
        let Some(pivot) = peers
            .get_block_header(peer_id, &mut connection, new_pivot_block_number)
            .await
            .map_err(SyncError::PeerHandler)?
        else {
            // Penalize peer
            peers.peer_table.record_failure(&peer_id).await?;
            let peer_score = peers.peer_table.get_score(&peer_id).await?;
            warn!(
                "Received None pivot from peer {peer_id} (score after penalizing: {peer_score}). Retrying"
            );
            continue;
        };

        // Reward peer
        peers.peer_table.record_success(&peer_id).await?;
        info!("Succesfully updated pivot");
        let block_headers = peers
            .request_block_headers(block_number + 1, pivot.hash())
            .await?
            .ok_or(SyncError::NoBlockHeaders)?;
        block_sync_state
            .process_incoming_headers(block_headers.into_iter())
            .await?;
        *METRICS.sync_head_hash.lock().await = pivot.hash();
        return Ok(pivot.clone());
    }
}

pub fn block_is_stale(block_header: &BlockHeader) -> bool {
    calculate_staleness_timestamp(block_header.timestamp) < current_unix_time()
}

pub fn calculate_staleness_timestamp(timestamp: u64) -> u64 {
    timestamp + (SNAP_LIMIT as u64 * 12)
}

pub async fn validate_state_root(store: Store, state_root: H256) -> bool {
    info!("Starting validate_state_root");
    let validated = tokio::task::spawn_blocking(move || {
        store
            .open_locked_state_trie(state_root)
            .expect("couldn't open trie")
            .validate()
    })
    .await
    .expect("We should be able to create threads");

    if validated.is_ok() {
        info!("Succesfully validated tree, {state_root} found");
    } else {
        error!("We have failed the validation of the state tree");
        std::process::exit(1);
    }
    validated.is_ok()
}

pub async fn validate_storage_root(store: Store, state_root: H256) -> bool {
    info!("Starting validate_storage_root");
    let is_valid = tokio::task::spawn_blocking(move || {
        store
            .iter_accounts(state_root)
            .expect("couldn't iterate accounts")
            .par_bridge()
            .try_for_each(|(hashed_address, account_state)| {
                let store_clone = store.clone();
                store_clone
                    .open_locked_storage_trie(
                        hashed_address,
                        state_root,
                        account_state.storage_root,
                    )
                    .expect("couldn't open storage trie")
                    .validate()
            })
    })
    .await
    .expect("We should be able to create threads");
    info!("Finished validate_storage_root");
    if is_valid.is_err() {
        std::process::exit(1);
    }
    is_valid.is_ok()
}

pub fn validate_bytecodes(store: Store, state_root: H256) -> bool {
    info!("Starting validate_bytecodes");
    let mut is_valid = true;
    for (account_hash, account_state) in store
        .iter_accounts(state_root)
        .expect("we couldn't iterate over accounts")
    {
        if account_state.code_hash != *EMPTY_KECCACK_HASH
            && !store
                .get_account_code(account_state.code_hash)
                .is_ok_and(|code| code.is_some())
        {
            error!(
                "Missing code hash {:x} for account {:x}",
                account_state.code_hash, account_hash
            );
            is_valid = false
        }
    }
    if !is_valid {
        std::process::exit(1);
    }
    is_valid
}

// ============================================================================
// Account and Storage Insertion (non-rocksdb)
// ============================================================================

#[cfg(not(feature = "rocksdb"))]
pub(crate) type StorageRoots = (H256, Vec<(ethrex_trie::Nibbles, Vec<u8>)>);

#[cfg(not(feature = "rocksdb"))]
pub(crate) fn compute_storage_roots(
    store: Store,
    account_hash: H256,
    key_value_pairs: &[(H256, U256)],
) -> Result<StorageRoots, SyncError> {
    use ethrex_trie::{Nibbles, Node};

    let storage_trie = store.open_direct_storage_trie(account_hash, *EMPTY_TRIE_HASH)?;
    let trie_hash = match storage_trie.db().get(Nibbles::default())? {
        Some(noderlp) => Node::decode(&noderlp)?.compute_hash().finalize(),
        None => *EMPTY_TRIE_HASH,
    };
    let mut storage_trie = store.open_direct_storage_trie(account_hash, trie_hash)?;

    for (hashed_key, value) in key_value_pairs {
        if let Err(err) = storage_trie.insert(hashed_key.0.to_vec(), value.encode_to_vec()) {
            warn!(
                "Failed to insert hashed key {hashed_key:?} in account hash: {account_hash:?}, err={err:?}"
            );
        };
        METRICS.storage_leaves_inserted.inc();
    }

    let (_, changes) = storage_trie.collect_changes_since_last_hash();

    Ok((account_hash, changes))
}

#[cfg(not(feature = "rocksdb"))]
async fn insert_accounts(
    store: Store,
    storage_accounts: &mut AccountStorageRoots,
    account_state_snapshots_dir: &Path,
    _: &Path,
    code_hash_collector: &mut CodeHashCollector,
) -> Result<(H256, BTreeSet<H256>), SyncError> {
    let mut computed_state_root = *EMPTY_TRIE_HASH;
    for entry in std::fs::read_dir(account_state_snapshots_dir)
        .map_err(|_| SyncError::AccountStateSnapshotsDirNotFound)?
    {
        let entry = entry
            .map_err(|err| SyncError::SnapshotReadError(account_state_snapshots_dir.into(), err))?;
        info!("Reading account file from entry {entry:?}");
        let snapshot_path = entry.path();
        let snapshot_contents = std::fs::read(&snapshot_path)
            .map_err(|err| SyncError::SnapshotReadError(snapshot_path.clone(), err))?;
        let account_states_snapshot: Vec<(H256, AccountState)> =
            RLPDecode::decode(&snapshot_contents)
                .map_err(|_| SyncError::SnapshotDecodeError(snapshot_path.clone()))?;

        storage_accounts.accounts_with_storage_root.extend(
            account_states_snapshot.iter().filter_map(|(hash, state)| {
                (state.storage_root != *EMPTY_TRIE_HASH)
                    .then_some((*hash, (Some(state.storage_root), Vec::new())))
            }),
        );

        // Collect valid code hashes from current account snapshot
        let code_hashes_from_snapshot: Vec<H256> = account_states_snapshot
            .iter()
            .filter_map(|(_, state)| {
                (state.code_hash != *EMPTY_KECCACK_HASH).then_some(state.code_hash)
            })
            .collect();

        code_hash_collector.extend(code_hashes_from_snapshot);
        code_hash_collector.flush_if_needed().await?;

        info!("Inserting accounts into the state trie");

        let store_clone = store.clone();
        let current_state_root: Result<H256, SyncError> =
            tokio::task::spawn_blocking(move || -> Result<H256, SyncError> {
                let mut trie = store_clone.open_direct_state_trie(computed_state_root)?;

                for (account_hash, account) in account_states_snapshot {
                    trie.insert(account_hash.0.to_vec(), account.encode_to_vec())?;
                }
                info!("Comitting to disk");
                let current_state_root = trie.hash()?;
                Ok(current_state_root)
            })
            .await?;

        computed_state_root = current_state_root?;
    }
    info!("computed_state_root {computed_state_root}");
    Ok((computed_state_root, BTreeSet::new()))
}

#[cfg(not(feature = "rocksdb"))]
async fn insert_storages(
    store: Store,
    _: BTreeSet<H256>,
    account_storages_snapshots_dir: &Path,
    _: &Path,
) -> Result<(), SyncError> {
    use rayon::iter::IntoParallelIterator;

    for entry in std::fs::read_dir(account_storages_snapshots_dir)
        .map_err(|_| SyncError::AccountStoragesSnapshotsDirNotFound)?
    {
        use crate::utils::AccountsWithStorage;

        let entry = entry.map_err(|err| {
            SyncError::SnapshotReadError(account_storages_snapshots_dir.into(), err)
        })?;
        info!("Reading account storage file from entry {entry:?}");

        let snapshot_path = entry.path();

        let snapshot_contents = std::fs::read(&snapshot_path)
            .map_err(|err| SyncError::SnapshotReadError(snapshot_path.clone(), err))?;

        #[expect(clippy::type_complexity)]
        let account_storages_snapshot: Vec<AccountsWithStorage> =
            RLPDecode::decode(&snapshot_contents)
                .map(|all_accounts: Vec<(Vec<H256>, Vec<(H256, U256)>)>| {
                    all_accounts
                        .into_iter()
                        .map(|(accounts, storages)| AccountsWithStorage { accounts, storages })
                        .collect()
                })
                .map_err(|_| SyncError::SnapshotDecodeError(snapshot_path.clone()))?;

        let store_clone = store.clone();
        info!("Starting compute of account_storages_snapshot");
        let storage_trie_node_changes = tokio::task::spawn_blocking(move || {
            let store: Store = store_clone;

            account_storages_snapshot
                .into_par_iter()
                .flat_map(|account_storages| {
                    let storages: Arc<[_]> = account_storages.storages.into();
                    account_storages
                        .accounts
                        .into_par_iter()
                        // FIXME: we probably want to make storages an Arc
                        .map(move |account| (account, storages.clone()))
                })
                .map(|(account, storages)| compute_storage_roots(store.clone(), account, &storages))
                .collect::<Result<Vec<_>, SyncError>>()
        })
        .await??;
        info!("Writing to db");

        store
            .write_storage_trie_nodes_batch(storage_trie_node_changes)
            .await?;
    }

    Ok(())
}

// ============================================================================
// Dataset capture for offline profiling
// ============================================================================

fn capture_snap_profile_dataset(
    capture_dir: &Path,
    account_state_snapshots_dir: &Path,
    account_storages_snapshots_dir: &Path,
    pivot_header: &BlockHeader,
    computed_state_root: H256,
) -> Result<(), SyncError> {
    use crate::sync::profile::{DatasetPaths, PivotInfo, SnapProfileManifest};

    std::fs::create_dir_all(capture_dir)
        .map_err(|e| SyncError::FileSystem(format!("Failed to create capture dir: {e}")))?;

    copy_dir_recursive(
        account_state_snapshots_dir,
        &capture_dir.join("account_state_snapshots"),
    )?;
    copy_dir_recursive(
        account_storages_snapshots_dir,
        &capture_dir.join("account_storages_snapshots"),
    )?;

    let manifest = SnapProfileManifest {
        version: 1,
        chain_id: 1,
        rocksdb_enabled: cfg!(feature = "rocksdb"),
        pivot: PivotInfo {
            number: pivot_header.number,
            hash: pivot_header.hash(),
            state_root: pivot_header.state_root,
            timestamp: pivot_header.timestamp,
        },
        post_accounts_insert_state_root: computed_state_root,
        paths: DatasetPaths {
            account_state_snapshots_dir: "account_state_snapshots".to_string(),
            account_storages_snapshots_dir: "account_storages_snapshots".to_string(),
        },
    };

    let manifest_json = serde_json::to_string_pretty(&manifest)
        .map_err(|e| SyncError::FileSystem(format!("Failed to serialize manifest: {e}")))?;
    std::fs::write(capture_dir.join("manifest.json"), manifest_json)
        .map_err(|e| SyncError::FileSystem(format!("Failed to write manifest: {e}")))?;

    info!("Snap profile dataset captured to {capture_dir:?}");
    Ok(())
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), SyncError> {
    std::fs::create_dir_all(dst)
        .map_err(|e| SyncError::FileSystem(format!("Failed to create {dst:?}: {e}")))?;
    for entry in std::fs::read_dir(src)
        .map_err(|e| SyncError::FileSystem(format!("Failed to read {src:?}: {e}")))?
    {
        let entry =
            entry.map_err(|e| SyncError::FileSystem(format!("Failed to read entry: {e}")))?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path).map_err(|e| {
                SyncError::FileSystem(format!("Failed to copy {src_path:?} to {dst_path:?}: {e}"))
            })?;
        }
    }
    Ok(())
}

// ============================================================================
// Account and Storage Insertion (rocksdb)
// ============================================================================

#[cfg(feature = "rocksdb")]
async fn insert_accounts(
    store: Store,
    storage_accounts: &mut AccountStorageRoots,
    account_state_snapshots_dir: &Path,
    datadir: &Path,
    code_hash_collector: &mut CodeHashCollector,
) -> Result<(H256, BTreeSet<H256>), SyncError> {
    use crate::utils::get_rocksdb_temp_accounts_dir;
    use ethrex_trie::trie_sorted::trie_from_sorted_accounts_wrap;

    let trie = store.open_direct_state_trie(*EMPTY_TRIE_HASH)?;
    let mut db_options = rocksdb::Options::default();
    db_options.create_if_missing(true);
    let db = rocksdb::DB::open(&db_options, get_rocksdb_temp_accounts_dir(datadir))
        .map_err(|e| SyncError::AccountTempDBDirNotFound(e.to_string()))?;
    let file_paths: Vec<PathBuf> = std::fs::read_dir(account_state_snapshots_dir)
        .map_err(|_| SyncError::AccountStateSnapshotsDirNotFound)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| SyncError::AccountStateSnapshotsDirNotFound)?
        .into_iter()
        .map(|res| res.path())
        .collect();
    db.ingest_external_file(file_paths)
        .map_err(|err| SyncError::RocksDBError(err.into_string()))?;
    let iter = db.full_iterator(rocksdb::IteratorMode::Start);
    for account in iter {
        let account = account.map_err(|err| SyncError::RocksDBError(err.into_string()))?;
        let account_state = AccountState::decode(&account.1).map_err(SyncError::Rlp)?;
        if account_state.code_hash != *EMPTY_KECCACK_HASH {
            code_hash_collector.add(account_state.code_hash);
            code_hash_collector.flush_if_needed().await?;
        }
    }

    let iter = db.full_iterator(rocksdb::IteratorMode::Start);
    let compute_state_root = trie_from_sorted_accounts_wrap(
        trie.db(),
        &mut iter
            .map(|k| k.expect("We shouldn't have a rocksdb error here")) // TODO: remove unwrap
            .inspect(|(k, v)| {
                METRICS
                    .account_tries_inserted
                    .fetch_add(1, Ordering::Relaxed);
                let account_state = AccountState::decode(v).expect("We should have accounts here");
                if account_state.storage_root != *EMPTY_TRIE_HASH {
                    storage_accounts.accounts_with_storage_root.insert(
                        H256::from_slice(k),
                        (Some(account_state.storage_root), Vec::new()),
                    );
                }
            })
            .map(|(k, v)| (H256::from_slice(&k), v.to_vec())),
    )
    .map_err(SyncError::TrieGenerationError)?;

    drop(db); // close db before removing directory

    // Snapshot dir cleanup is handled by the caller (conditional on capture mode).
    std::fs::remove_dir_all(get_rocksdb_temp_accounts_dir(datadir))
        .map_err(|e| SyncError::AccountTempDBDirNotFound(e.to_string()))?;

    let accounts_with_storage =
        BTreeSet::from_iter(storage_accounts.accounts_with_storage_root.keys().copied());
    Ok((compute_state_root, accounts_with_storage))
}

#[cfg(feature = "rocksdb")]
async fn insert_storages(
    store: Store,
    accounts_with_storage: BTreeSet<H256>,
    account_storages_snapshots_dir: &Path,
    datadir: &Path,
) -> Result<(), SyncError> {
    use crate::utils::get_rocksdb_temp_storage_dir;
    use crossbeam::channel::{bounded, unbounded};
    use ethrex_trie::{
        Nibbles, Node, ThreadPool,
        trie_sorted::{BUFFER_COUNT, SIZE_TO_WRITE_DB, trie_from_sorted_accounts},
    };
    use std::thread::scope;

    struct RocksDBIterator<'a> {
        iter: rocksdb::DBRawIterator<'a>,
        limit: H256,
    }

    impl<'a> Iterator for RocksDBIterator<'a> {
        type Item = (H256, Vec<u8>);

        fn next(&mut self) -> Option<Self::Item> {
            if !self.iter.valid() {
                return None;
            }
            let return_value = {
                let key = self.iter.key();
                let value = self.iter.value();
                match (key, value) {
                    (Some(key), Some(value)) => {
                        let hash = H256::from_slice(&key[0..32]);
                        let key = H256::from_slice(&key[32..]);
                        let value = value.to_vec();
                        if hash != self.limit {
                            None
                        } else {
                            Some((key, value))
                        }
                    }
                    _ => None,
                }
            };
            self.iter.next();
            return_value
        }
    }

    let mut db_options = rocksdb::Options::default();
    db_options.create_if_missing(true);
    let db = rocksdb::DB::open(&db_options, get_rocksdb_temp_storage_dir(datadir))
        .map_err(|err: rocksdb::Error| SyncError::RocksDBError(err.into_string()))?;
    let file_paths: Vec<PathBuf> = std::fs::read_dir(account_storages_snapshots_dir)
        .map_err(|_| SyncError::AccountStoragesSnapshotsDirNotFound)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| SyncError::AccountStoragesSnapshotsDirNotFound)?
        .into_iter()
        .map(|res| res.path())
        .collect();
    db.ingest_external_file(file_paths)
        .map_err(|err| SyncError::RocksDBError(err.into_string()))?;
    let snapshot = db.snapshot();

    let account_with_storage_and_tries = accounts_with_storage
        .into_iter()
        .map(|account_hash| {
            (
                account_hash,
                store
                    .open_direct_storage_trie(account_hash, *EMPTY_TRIE_HASH)
                    .expect("Should be able to open trie"),
            )
        })
        .collect::<Vec<(H256, Trie)>>();

    let (sender, receiver) = unbounded::<()>();
    let mut counter = 0;
    let thread_count = std::thread::available_parallelism()
        .map(|num| num.into())
        .unwrap_or(8);

    let (buffer_sender, buffer_receiver) = bounded::<Vec<(Nibbles, Node)>>(BUFFER_COUNT as usize);
    for _ in 0..BUFFER_COUNT {
        let _ = buffer_sender.send(Vec::with_capacity(SIZE_TO_WRITE_DB as usize));
    }

    scope(|scope| {
        let pool: Arc<ThreadPool<'_>> = Arc::new(ThreadPool::new(thread_count, scope));
        for (account_hash, trie) in account_with_storage_and_tries.iter() {
            let sender = sender.clone();
            let buffer_sender = buffer_sender.clone();
            let buffer_receiver = buffer_receiver.clone();
            if counter >= thread_count - 1 {
                let _ = receiver.recv();
                counter -= 1;
            }
            counter += 1;
            let pool_clone = pool.clone();
            let mut iter = snapshot.raw_iterator();
            let task = Box::new(move || {
                let mut buffer: [u8; 64] = [0_u8; 64];
                buffer[..32].copy_from_slice(&account_hash.0);
                iter.seek(buffer);
                let iter = RocksDBIterator {
                    iter,
                    limit: *account_hash,
                };

                let _ = trie_from_sorted_accounts(
                    trie.db(),
                    &mut iter.inspect(|_| METRICS.storage_leaves_inserted.inc()),
                    pool_clone,
                    buffer_sender,
                    buffer_receiver,
                )
                .inspect_err(|err: &ethrex_trie::trie_sorted::TrieGenerationError| {
                    error!(
                        "we found an error while inserting the storage trie for the account {account_hash:x}, err {err}"
                    );
                })
                .map_err(SyncError::TrieGenerationError);
                let _ = sender.send(());
            });
            pool.execute(task);
        }
    });

    // close db before removing directory
    drop(snapshot);
    drop(db);

    // Snapshot dir cleanup is handled by the caller (conditional on capture mode).
    std::fs::remove_dir_all(get_rocksdb_temp_storage_dir(datadir))
        .map_err(|e| SyncError::StorageTempDBDirNotFound(e.to_string()))?;

    Ok(())
}
