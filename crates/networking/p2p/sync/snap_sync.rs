//! Snap sync implementation
//!
//! This module contains the logic for snap synchronization mode where state is
//! fetched via snap p2p requests while blocks and receipts are fetched in parallel.

use std::collections::{BTreeSet, HashMap};
use std::path::Path;
#[cfg(feature = "rocksdb")]
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, SystemTime};

use ethrex_blockchain::Blockchain;
use ethrex_common::types::{AccountState, BlockHeader};
use ethrex_common::{
    H256,
    constants::{EMPTY_KECCAK_HASH, EMPTY_TRIE_HASH},
};
use ethrex_rlp::decode::RLPDecode;
use ethrex_storage::Store;
#[cfg(feature = "rocksdb")]
use ethrex_trie::Trie;
use rayon::iter::{ParallelBridge, ParallelIterator};
use tracing::{debug, error, info, warn};

use crate::metrics::{CurrentStepValue, METRICS};
use crate::peer_handler::PeerHandler;
use crate::peer_table::PeerTableServerProtocol as _;
use crate::rlpx::p2p::SUPPORTED_ETH_CAPABILITIES;
use crate::snap::{
    async_fs,
    constants::{
        MAX_HEADER_FETCH_ATTEMPTS, MIN_FULL_BLOCKS, MISSING_SLOTS_PERCENTAGE, SECONDS_PER_BLOCK,
        SNAP_LIMIT,
    },
    request_account_range, request_storage_ranges,
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
    /// When false, header batches are stored without advancing the
    /// header-download checkpoint. The pivot-extension path runs concurrently
    /// with the background backfill that owns the checkpoint; if both wrote
    /// it, a restart could resume past headers the backfill never downloaded.
    checkpoint_enabled: bool,
}

impl SnapBlockSyncState {
    pub fn new(store: Store) -> Self {
        Self {
            block_hashes: Vec::new(),
            store,
            checkpoint_enabled: true,
        }
    }

    /// State for the pivot-extension segment while a background header
    /// backfill owns the checkpoint (see `checkpoint_enabled`).
    pub fn new_without_checkpoint(store: Store) -> Self {
        Self {
            block_hashes: Vec::new(),
            store,
            checkpoint_enabled: false,
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
        if self.checkpoint_enabled {
            self.store
                .set_header_download_checkpoint(
                    *block_hashes.last().ok_or(SyncError::InvalidRangeReceived)?,
                )
                .await?;
        }
        self.block_hashes.extend_from_slice(&block_hashes);
        self.store.add_block_headers(block_headers_vec).await?;
        Ok(())
    }
}

/// Downloads headers from `current_head` up to `sync_head` in the background
/// while the state phases run against a provisional pivot. Owns the
/// header-download checkpoint. Returns the ordered hashes (oldest first,
/// ending at the sync head), or `None` if peers never served the target
/// header — the cycle should then end and wait for a newer sync head.
async fn header_backfill(
    mut peers: PeerHandler,
    store: Store,
    mut current_head: H256,
    mut current_head_number: u64,
    sync_head: H256,
    pending_block: Option<ethrex_common::types::Block>,
    diagnostics: Arc<tokio::sync::RwLock<super::SyncDiagnostics>>,
) -> Result<Option<Vec<H256>>, SyncError> {
    let mut block_sync_state = SnapBlockSyncState::new(store);
    let mut attempts = 0;

    loop {
        let _ = peers.peer_table.prune_table();
        debug!("Backfill requesting block headers from {current_head}");

        let Some(mut block_headers) = peers
            .request_block_headers(current_head_number, sync_head)
            .await?
        else {
            if attempts >= MAX_HEADER_FETCH_ATTEMPTS {
                warn!("Header backfill failed to find the target header after {attempts} attempts");
                return Ok(None);
            }
            attempts += 1;
            tokio::time::sleep(Duration::from_secs(2)).await;
            continue;
        };
        attempts = 0;

        let Some((first_block_hash, first_block_parent_hash)) = block_headers
            .first()
            .map(|header| (header.hash(), header.parent_hash))
        else {
            continue;
        };
        let Some(last_block_hash) = block_headers.last().map(|header| header.hash()) else {
            continue;
        };
        // Same side-chain fallback as the serial path (TODO #2126).
        if first_block_hash == last_block_hash
            && first_block_hash == current_head
            && current_head != sync_head
        {
            warn!("Backfill failed to find target block header, going back to the previous parent");
            current_head = first_block_parent_hash;
            continue;
        }

        if let Some(ref block) = pending_block
            && block.header.parent_hash == last_block_hash
        {
            block_headers.push(block.header.clone());
        }

        let mut sync_head_found = false;
        if let Some(index) = block_headers
            .iter()
            .position(|header| header.hash() == sync_head)
        {
            sync_head_found = true;
            block_headers.drain(index + 1..);
        }

        current_head = block_headers.last().map(|h| h.hash()).unwrap_or(sync_head);
        current_head_number = block_headers.last().map(|h| h.number).unwrap_or_default();

        // Discard the first header as we already have it
        if block_headers.len() > 1 {
            block_sync_state
                .process_incoming_headers(block_headers.into_iter().skip(1))
                .await?;
        }

        diagnostics.write().await.phase_progress.insert(
            "headers_downloaded".to_string(),
            block_sync_state.block_hashes.len() as u64,
        );

        if sync_head_found {
            return Ok(Some(block_sync_state.block_hashes));
        }
    }
}

/// Performs snap sync cycle - fetches state via snap protocol while downloading blocks in parallel
#[allow(clippy::too_many_arguments)]
pub async fn sync_cycle_snap(
    peers: &mut PeerHandler,
    blockchain: Arc<Blockchain>,
    snap_enabled: &std::sync::atomic::AtomicBool,
    sync_head: H256,
    store: Store,
    datadir: &Path,
    diagnostics: &Arc<tokio::sync::RwLock<super::SyncDiagnostics>>,
    probed_head: Option<BlockHeader>,
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
    {
        let mut diag = diagnostics.write().await;
        diag.current_phase = "headers".to_string();
        diag.sync_mode = "snap".to_string();
    }
    debug!(
        "Syncing from current head {:?} to sync_head {:?}",
        current_head, sync_head
    );
    let pending_block = match store.get_pending_block(sync_head).await {
        Ok(res) => res,
        Err(e) => return Err(e.into()),
    };

    // Parallel path: with the sync-head header already probed, the state
    // phases start immediately against it as a provisional pivot while the
    // header chain backfills in a background task. The two header segments
    // stay separate (backfill: current head -> sync head, owning the
    // checkpoint; pivot extensions: sync head -> final pivot, no checkpoint)
    // and are concatenated before the forkchoice update, which requires one
    // contiguous ordered chain ending at the pivot.
    if let Some(pivot0) = probed_head.filter(|h| h.hash() == sync_head) {
        info!(
            pivot = pivot0.number,
            "Starting state download against the probed sync head while headers backfill in the background"
        );
        // The provisional pivot's header must be readable from the store for
        // body fetch and pivot bookkeeping before the backfill reaches it.
        store.add_block_headers(vec![pivot0.clone()]).await?;
        let backfill = tokio::spawn(header_backfill(
            peers.clone(),
            store.clone(),
            current_head,
            current_head_number,
            sync_head,
            pending_block,
            diagnostics.clone(),
        ));
        let mut extension_state = SnapBlockSyncState::new_without_checkpoint(store.clone());
        let completed = snap_sync(
            peers,
            &store,
            &mut extension_state,
            datadir,
            diagnostics,
            Some(pivot0),
            Some(backfill),
        )
        .await?;
        if !completed {
            // Header chain incomplete: keep snap mode and the checkpoint so
            // the next forkchoice update retries with a newer head.
            return Ok(());
        }

        store.clear_snap_state().await?;
        snap_enabled.store(false, Ordering::Relaxed);
        return Ok(());
    }

    let mut attempts = 0;

    loop {
        // Prune dead/unresponsive peers periodically to allow replacements to be promoted
        let _ = peers.peer_table.prune_table();

        debug!("Requesting Block Headers from {current_head}");

        let Some(mut block_headers) = peers
            .request_block_headers(current_head_number, sync_head)
            .await?
        else {
            if attempts >= MAX_HEADER_FETCH_ATTEMPTS {
                warn!(
                    "Sync failed to find target block header after {attempts} attempts, aborting to wait for a newer sync head"
                );
                return Ok(());
            }
            attempts += 1;
            debug!(
                "Failed to fetch headers for sync head (attempt {attempts}/{MAX_HEADER_FETCH_ATTEMPTS}), retrying in 2s"
            );
            tokio::time::sleep(Duration::from_secs(2)).await;
            continue;
        };
        // Reset failure counter on success so it tracks consecutive failures
        attempts = 0;

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
        // Or the head is very close to 0. A pre-check in `sync.rs::sync_cycle`
        // also gates on `< MIN_FULL_BLOCKS`; keep both — this one stays as a
        // safety net for callers that enter `sync_cycle_snap` directly.
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

        // Update diagnostics with header progress
        {
            let mut diag = diagnostics.write().await;
            diag.phase_progress.insert(
                "headers_downloaded".to_string(),
                block_sync_state.block_hashes.len() as u64,
            );
        }

        if sync_head_found {
            break;
        };
    }

    snap_sync(
        peers,
        &store,
        &mut block_sync_state,
        datadir,
        diagnostics,
        None,
        None,
    )
    .await?;

    store.clear_snap_state().await?;
    snap_enabled.store(false, Ordering::Relaxed);

    Ok(())
}

/// Main snap sync logic - downloads state via snap protocol
///
/// With `initial_pivot`/`backfill` set (the parallel header path), the pivot
/// comes from the caller and the header chain is joined from the backfill
/// task before the forkchoice update.
#[allow(clippy::too_many_arguments)]
pub async fn snap_sync(
    peers: &mut PeerHandler,
    store: &Store,
    block_sync_state: &mut SnapBlockSyncState,
    datadir: &Path,
    diagnostics: &Arc<tokio::sync::RwLock<super::SyncDiagnostics>>,
    initial_pivot: Option<BlockHeader>,
    mut backfill: Option<tokio::task::JoinHandle<Result<Option<Vec<H256>>, SyncError>>>,
) -> Result<bool, SyncError> {
    // snap-sync: launch tasks to fetch blocks and state in parallel
    // - Fetch each block's body and its receipt via eth p2p requests
    // - Fetch the pivot block's state via snap p2p requests
    // - Execute blocks after the pivot (like in full-sync)
    let mut pivot_header = match initial_pivot {
        Some(header) => header,
        None => {
            let pivot_hash = block_sync_state
                .block_hashes
                .last()
                .ok_or(SyncError::NoBlockHeaders)?;
            store
                .get_block_header_by_hash(*pivot_hash)?
                .ok_or(SyncError::CorruptDB)?
        }
    };

    while block_is_stale(&pivot_header) {
        pivot_header = update_pivot(
            pivot_header.number,
            pivot_header.timestamp,
            peers,
            block_sync_state,
            diagnostics,
        )
        .await?;
    }
    debug!(
        "Selected block {} as pivot for snap sync",
        pivot_header.number
    );
    {
        let mut diag = diagnostics.write().await;
        diag.pivot_block_number = Some(pivot_header.number);
        diag.pivot_timestamp = Some(pivot_header.timestamp);
        let pivot_age = current_unix_time().saturating_sub(pivot_header.timestamp);
        diag.pivot_age_seconds = Some(pivot_age);
        diag.staleness_threshold_seconds = (SNAP_LIMIT as u64) * SECONDS_PER_BLOCK;
        diag.sync_mode = "snap".to_string();
        METRICS
            .pivot_timestamp
            .store(pivot_header.timestamp, std::sync::atomic::Ordering::Relaxed);
    }

    let state_root = pivot_header.state_root;
    let account_state_snapshots_dir = get_account_state_snapshots_dir(datadir);
    let account_storages_snapshots_dir = get_account_storages_snapshots_dir(datadir);

    let code_hashes_snapshot_dir = get_code_hashes_snapshots_dir(datadir);
    async_fs::ensure_dir_exists(&code_hashes_snapshot_dir).await?;

    // Create collector to store code hashes in files. Bytecodes are
    // content-addressed and pivot-independent, so a background fetcher
    // downloads them concurrently with the remaining phases, consuming each
    // code-hash file as the collector finishes writing it.
    let (code_file_tx, code_file_rx) = tokio::sync::mpsc::unbounded_channel();
    let mut code_hash_collector: CodeHashCollector =
        CodeHashCollector::new(code_hashes_snapshot_dir.clone(), code_file_tx);
    let bytecode_fetcher =
        super::bytecode_fetcher::spawn_bytecode_fetcher(peers.clone(), store.clone(), code_file_rx);

    let mut storage_accounts = AccountStorageRoots::default();
    if !std::env::var("SKIP_START_SNAP_SYNC").is_ok_and(|var| !var.is_empty()) {
        // We start by downloading all of the leafs of the trie of accounts
        // The function request_account_range writes the leafs into files in
        // account_state_snapshots_dir

        diagnostics.write().await.current_phase = "account_ranges".to_string();
        // Storage ranges download concurrently with the account phase: every
        // account leaf carries its storage root, so discovered accounts feed
        // a background wave runner instead of waiting for the built and
        // healed state trie. Wave leftovers rejoin the post-build loop.
        let (storage_feed_tx, storage_feed_rx) = tokio::sync::mpsc::unbounded_channel();
        let (pivot_watch_tx, pivot_watch_rx) = tokio::sync::watch::channel(pivot_header.clone());
        let storage_waves = tokio::spawn(super::storage_feed::run_storage_waves(
            peers.clone(),
            store.clone(),
            account_storages_snapshots_dir.clone(),
            storage_feed_rx,
            pivot_watch_rx,
        ));
        let storage_hooks = crate::snap::StorageDiscoveryHooks {
            feed: storage_feed_tx,
            pivot_watch: pivot_watch_tx,
        };
        // Ingest each finished snapshot file into the temporary RocksDB
        // while the range download is still running, instead of ingesting
        // the whole batch only after the download completes. The sorted trie
        // build still runs once, in `insert_accounts`, after the download
        // and the ingest task both finish.
        #[cfg(feature = "rocksdb")]
        let (account_ingest_tx, account_ingest_handle) = {
            let (tx, handle) = super::account_ingestor::spawn_account_snapshot_ingestor(datadir);
            (Some(tx), handle)
        };
        #[cfg(not(feature = "rocksdb"))]
        let account_ingest_tx = None;
        request_account_range(
            peers,
            H256::zero(),
            H256::repeat_byte(0xff),
            account_state_snapshots_dir.as_ref(),
            &mut pivot_header,
            block_sync_state,
            diagnostics,
            Some(&storage_hooks),
            account_ingest_tx,
        )
        .await?;
        debug!("Finished downloading account ranges from peers");

        {
            let mut diag = diagnostics.write().await;
            diag.current_phase = "account_insertion".to_string();
            diag.phase_progress.insert(
                "account_ranges_downloaded".to_string(),
                METRICS
                    .downloaded_account_tries
                    .load(std::sync::atomic::Ordering::Relaxed),
            );
        }
        *METRICS.account_tries_insert_start_time.lock().await = Some(SystemTime::now());
        METRICS
            .current_step
            .set(CurrentStepValue::InsertingAccountRanges);
        // We read the account leafs from the files in account_state_snapshots_dir, write it into
        // the trie to compute the nodes and stores the accounts with storages for later use

        #[cfg(feature = "rocksdb")]
        let (computed_state_root, accounts_with_storage) = insert_accounts(
            store.clone(),
            &mut storage_accounts,
            &account_state_snapshots_dir,
            datadir,
            &mut code_hash_collector,
            account_ingest_handle,
        )
        .await?;
        // Variable `accounts_with_storage` unused if not in rocksdb
        #[cfg(not(feature = "rocksdb"))]
        #[allow(unused_variables)]
        let (computed_state_root, accounts_with_storage) = insert_accounts(
            store.clone(),
            &mut storage_accounts,
            &account_state_snapshots_dir,
            datadir,
            &mut code_hash_collector,
        )
        .await?;
        debug!(
            "Finished inserting account ranges, total storage accounts: {}",
            storage_accounts.accounts_with_storage_root.len()
        );
        *METRICS.account_tries_insert_end_time.lock().await = Some(SystemTime::now());

        debug!("Original state root: {state_root:?}");
        debug!("Computed state root after request_account_ranges: {computed_state_root:?}");

        diagnostics.write().await.current_phase = "storage_ranges".to_string();
        *METRICS.storage_tries_download_start_time.lock().await = Some(SystemTime::now());
        // The wave runner drains once the feed closes (request_account_range
        // returning dropped the sender); give it a fresh pivot to finish with,
        // then reconcile: wave-completed accounts must not be re-downloaded,
        // wave leftovers replace their stale bookkeeping here.
        while block_is_stale(&pivot_header) {
            pivot_header = update_pivot(
                pivot_header.number,
                pivot_header.timestamp,
                peers,
                block_sync_state,
                diagnostics,
            )
            .await?;
        }
        storage_hooks.pivot_watch.send_replace(pivot_header.clone());
        drop(storage_hooks);
        let wave_outcome = storage_waves.await??;
        storage_accounts
            .accounts_with_storage_root
            .retain(|account, _| !wave_outcome.done.contains(account));
        storage_accounts
            .accounts_with_storage_root
            .extend(wave_outcome.carry.accounts_with_storage_root);
        storage_accounts
            .healed_accounts
            .extend(wave_outcome.carry.healed_accounts);
        debug!(
            wave_done = wave_outcome.done.len(),
            remaining = storage_accounts.accounts_with_storage_root.len(),
            "Reconciled storage waves with the post-build loop"
        );
        // We start downloading the storage leafs. To do so, we need to be sure that the storage root
        // is correct. To do so, we always heal the state trie before requesting storage rates
        let mut chunk_index = wave_outcome.chunk_index;
        let mut state_leafs_healed = 0_u64;
        let mut storage_range_request_attempts = 0;
        loop {
            while block_is_stale(&pivot_header) {
                pivot_header = update_pivot(
                    pivot_header.number,
                    pivot_header.timestamp,
                    peers,
                    block_sync_state,
                    diagnostics,
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

            debug!(
                "Started request_storage_ranges with {} accounts with storage root unchanged",
                storage_accounts.accounts_with_storage_root.len()
            );
            storage_range_request_attempts += 1;
            if storage_range_request_attempts < 5 {
                METRICS
                    .current_step
                    .set(CurrentStepValue::RequestingStorageRanges);
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
                    "Storage could not be downloaded after multiple attempts. Marking for healing. This could impact snap sync time (healing may take a while)."
                );

                storage_accounts.accounts_with_storage_root.clear();
            }

            debug!(
                "Ended request_storage_ranges with {} accounts with storage root unchanged and not downloaded yet and with {} big/healed accounts",
                storage_accounts.accounts_with_storage_root.len(),
                // These accounts are marked as heals if they're a big account. This is
                // because we don't know if the storage root is still valid
                storage_accounts.healed_accounts.len(),
            );
            if !block_is_stale(&pivot_header) {
                break;
            }
            debug!("Pivot became stale during storage download, restarting loop");
        }
        debug!("Finished request_storage_ranges");
        *METRICS.storage_tries_download_end_time.lock().await = Some(SystemTime::now());

        diagnostics.write().await.current_phase = "storage_insertion".to_string();
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

        debug!("Finished storing storage tries");
    }

    diagnostics.write().await.current_phase = "healing".to_string();
    *METRICS.heal_start_time.lock().await = Some(SystemTime::now());
    debug!("Starting healing process");
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
                diagnostics,
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

    debug!("Finished healing");

    // Finish code hash collection: flushes the final file and closes the
    // file channel, letting the bytecode fetcher drain its last batch.
    code_hash_collector.finish().await?;

    diagnostics.write().await.current_phase = "bytecodes".to_string();
    METRICS
        .current_step
        .set(CurrentStepValue::RequestingBytecodes);
    debug!("Waiting for streamed bytecode download to finish");
    bytecode_fetcher.await??;

    let code_hashes_dir = get_code_hashes_snapshots_dir(datadir);
    async_fs::remove_dir_all(&code_hashes_dir).await?;

    *METRICS.bytecode_download_end_time.lock().await = Some(SystemTime::now());

    debug_assert!(validate_bytecodes(store.clone(), pivot_header.state_root));

    store_block_bodies(vec![pivot_header.clone()], peers.clone(), store.clone()).await?;

    let block = store
        .get_block_by_hash(pivot_header.hash())
        .await?
        .ok_or(SyncError::CorruptDB)?;

    store.add_block(block).await?;

    // With a background backfill, the canonical chain is its segment plus
    // the pivot extensions accumulated here; both are ordered and contiguous
    // by construction (backfill ends at the original sync head, extensions
    // start right after it).
    let full_chain_hashes = match backfill.take() {
        Some(handle) => match handle.await?? {
            Some(mut hashes) => {
                hashes.extend(block_sync_state.block_hashes.iter().copied());
                hashes
            }
            None => {
                warn!(
                    "Header backfill could not find the sync head; ending the cycle to wait for a newer head"
                );
                return Ok(false);
            }
        },
        None => std::mem::take(&mut block_sync_state.block_hashes),
    };

    let numbers_and_hashes = full_chain_hashes
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
    Ok(true)
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
    diagnostics: &Arc<tokio::sync::RwLock<super::SyncDiagnostics>>,
) -> Result<BlockHeader, SyncError> {
    /// Maximum number of full peer rotations before giving up. With rotation,
    /// each pass tries every eligible peer once; the budget scales naturally
    /// with network size. Between rotations we back off exponentially.
    const MAX_ROTATIONS: u64 = 5;
    const INITIAL_RETRY_DELAY: Duration = Duration::from_secs(1);
    const MAX_RETRY_DELAY: Duration = Duration::from_secs(30);

    // We multiply the estimation by 0.9 in order to account for missing slots (~9% in tesnets)
    let new_pivot_block_number = block_number
        + ((current_unix_time().saturating_sub(block_timestamp) / SECONDS_PER_BLOCK) as f64
            * MISSING_SLOTS_PERCENTAGE) as u64;
    debug!(
        "Current pivot is stale (number: {}, timestamp: {}). New pivot number: {}",
        block_number, block_timestamp, new_pivot_block_number
    );

    let mut rotation_count: u64 = 0;
    // Track peers that already failed this rotation so we try every eligible
    // peer once before retrying any. When the rotation is exhausted, clear
    // and start a new one.
    let mut excluded_peers: Vec<H256> = Vec::new();

    loop {
        if rotation_count >= MAX_ROTATIONS {
            #[cfg(feature = "metrics")]
            ethrex_metrics::sync::METRICS_SYNC.inc_pivot_update("max_failures");
            diagnostics
                .write()
                .await
                .push_pivot_change(super::PivotChangeEvent {
                    timestamp: current_unix_time(),
                    old_pivot_number: block_number,
                    new_pivot_number: new_pivot_block_number,
                    outcome: "max_failures".to_string(),
                    failure_reason: Some(format!("Exhausted {MAX_ROTATIONS} full rotations")),
                });
            return Err(SyncError::PeerHandler(
                crate::peer_handler::PeerHandlerError::BlockHeaders,
            ));
        }

        // Exponential backoff: doubles each rotation, capped at MAX_RETRY_DELAY
        if rotation_count > 0 {
            let delay = INITIAL_RETRY_DELAY.saturating_mul(1 << rotation_count.min(4));
            let delay = delay.min(MAX_RETRY_DELAY);
            debug!(
                "update_pivot: backing off for {}s (rotation={rotation_count})",
                delay.as_secs()
            );
            tokio::time::sleep(delay).await;
        }

        // One permit per attempt: consumed by `get_block_header` below.
        let Some((peer_id, mut connection, permit)) = peers
            .peer_table
            .get_best_peer_excluding(SUPPORTED_ETH_CAPABILITIES.to_vec(), excluded_peers.clone())
            .await?
        else {
            // Distinguish "rotation exhausted" from "no peers currently eligible
            // (all at capacity)". Read-only probe — does not bump `requests`.
            let any_eligible = peers
                .peer_table
                .has_eligible_peer(SUPPORTED_ETH_CAPABILITIES.to_vec())
                .await?;

            if !any_eligible {
                debug!("update_pivot: no eligible peers available, waiting");
                #[cfg(feature = "metrics")]
                ethrex_metrics::sync::METRICS_SYNC.inc_pivot_update("no_peers");
                tokio::time::sleep(Duration::from_secs(1)).await;
            } else if excluded_peers.is_empty() {
                // Peers exist but none match — shouldn't happen in practice
                debug!("update_pivot: peers exist but none selectable, retrying");
                tokio::time::sleep(Duration::from_secs(1)).await;
            } else {
                // All non-excluded peers were already tried — rotation done
                debug!(
                    "update_pivot: rotation {rotation_count} complete ({} peers tried), starting next",
                    excluded_peers.len()
                );
                excluded_peers.clear();
                rotation_count = rotation_count.saturating_add(1);
            }
            continue;
        };

        let peer_score = peers.peer_table.get_score(peer_id).await?;
        let diag = peers.read_peer_diagnostics().await;
        let eligible_count = diag.iter().filter(|p| p.eligible).count();
        let total_count = diag.len();
        debug!(
            eligible_peers = eligible_count,
            total_peers = total_count,
            selected_peer = %peer_id,
            peer_score = peer_score,
            excluded_count = excluded_peers.len(),
            rotation = rotation_count,
            "update_pivot: attempting with peer"
        );
        debug!(
            "Trying to update pivot to {new_pivot_block_number} with peer {peer_id} (score: {peer_score})"
        );

        // One attempt per peer per rotation. A peer that fails is excluded for
        // this rotation and will be retried (with backoff) in the next one.
        let outcome = peers
            .get_block_header(&mut connection, permit, new_pivot_block_number)
            .await;

        match outcome {
            Ok(Some(pivot)) => {
                peers.peer_table.record_success(peer_id)?;
                #[cfg(feature = "metrics")]
                ethrex_metrics::sync::METRICS_SYNC.inc_pivot_update("success");
                info!("Snap sync pivot updated to block {}", pivot.number);

                {
                    let mut diag = diagnostics.write().await;
                    diag.push_pivot_change(super::PivotChangeEvent {
                        timestamp: current_unix_time(),
                        old_pivot_number: block_number,
                        new_pivot_number: pivot.number,
                        outcome: "success".to_string(),
                        failure_reason: None,
                    });
                    diag.pivot_block_number = Some(pivot.number);
                    diag.pivot_timestamp = Some(pivot.timestamp);
                    let pivot_age = current_unix_time().saturating_sub(pivot.timestamp);
                    diag.pivot_age_seconds = Some(pivot_age);
                    METRICS
                        .pivot_timestamp
                        .store(pivot.timestamp, std::sync::atomic::Ordering::Relaxed);
                }
                let block_headers = peers
                    .request_block_headers(block_number + 1, pivot.hash())
                    .await?
                    .ok_or(SyncError::NoBlockHeaders)?;
                block_sync_state
                    .process_incoming_headers(block_headers.into_iter())
                    .await?;
                *METRICS.sync_head_hash.lock().await = pivot.hash();
                return Ok(pivot);
            }
            Ok(None) => {
                peers.peer_table.record_failure(peer_id)?;
                let peer_score = peers.peer_table.get_score(peer_id).await?;
                debug!(
                    "update_pivot: peer {peer_id} returned None (score: {peer_score}), excluding for this rotation"
                );
                #[cfg(feature = "metrics")]
                ethrex_metrics::sync::METRICS_SYNC.inc_pivot_update("peer_none");
                excluded_peers.push(peer_id);
            }
            Err(e) if e.is_recoverable() => {
                peers.peer_table.record_failure(peer_id)?;
                debug!("update_pivot: peer {peer_id} failed with {e}, excluding for this rotation");
                #[cfg(feature = "metrics")]
                ethrex_metrics::sync::METRICS_SYNC.inc_pivot_update("peer_error");
                excluded_peers.push(peer_id);
            }
            Err(e) => {
                // Non-recoverable error (e.g., dead peer table actor,
                // storage full) — surface it.
                return Err(SyncError::PeerHandler(e));
            }
        }
    }
}

pub fn block_is_stale(block_header: &BlockHeader) -> bool {
    let threshold = calculate_staleness_timestamp(block_header.timestamp);
    let now = current_unix_time();
    let is_stale = threshold < now;
    if is_stale {
        let pivot_age = now.saturating_sub(block_header.timestamp);
        let staleness_limit = (SNAP_LIMIT as u64) * SECONDS_PER_BLOCK;
        debug!(
            pivot_number = block_header.number,
            pivot_timestamp = block_header.timestamp,
            pivot_age_seconds = pivot_age,
            staleness_threshold_seconds = staleness_limit,
            "Pivot block detected as stale"
        );
    }
    is_stale
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
        info!("Successfully validated tree, {state_root} found");
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
        if account_state.code_hash != *EMPTY_KECCAK_HASH
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
type StorageRoots = (H256, Vec<(ethrex_trie::Nibbles, Vec<u8>)>);

#[cfg(not(feature = "rocksdb"))]
fn compute_storage_roots(
    store: Store,
    account_hash: H256,
    key_value_pairs: &[(H256, U256)],
) -> Result<StorageRoots, SyncError> {
    use ethrex_trie::{Nibbles, Node};

    let storage_trie = store.open_direct_storage_trie(account_hash, *EMPTY_TRIE_HASH)?;
    let trie_hash = match storage_trie.db().get(Nibbles::default())? {
        Some(noderlp) => Node::decode(&noderlp)?
            .compute_hash(&ethrex_crypto::NativeCrypto)
            .finalize(&ethrex_crypto::NativeCrypto),
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

    let (_, changes) = storage_trie.collect_changes_since_last_hash(&ethrex_crypto::NativeCrypto);

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
    let snapshot_files = async_fs::read_dir_paths(account_state_snapshots_dir).await?;
    for snapshot_path in snapshot_files {
        debug!("Reading account file from {snapshot_path:?}");
        let snapshot_contents = async_fs::read_file(&snapshot_path).await?;
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
                (state.code_hash != *EMPTY_KECCAK_HASH).then_some(state.code_hash)
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
                let current_state_root = trie.hash(&ethrex_crypto::NativeCrypto)?;
                Ok(current_state_root)
            })
            .await?;

        computed_state_root = current_state_root?;
    }
    async_fs::remove_dir_all(account_state_snapshots_dir).await?;
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
    use crate::utils::AccountsWithStorage;
    use rayon::iter::IntoParallelIterator;

    let snapshot_files = async_fs::read_dir_paths(account_storages_snapshots_dir).await?;
    for snapshot_path in snapshot_files {
        info!("Reading account storage file from {snapshot_path:?}");

        let snapshot_contents = async_fs::read_file(&snapshot_path).await?;

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

    async_fs::remove_dir_all(account_storages_snapshots_dir).await?;

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
    ingest_handle: super::account_ingestor::AccountIngestHandle,
) -> Result<(H256, BTreeSet<H256>), SyncError> {
    use crate::sync::account_ingestor;
    use crate::utils::get_rocksdb_temp_accounts_dir;
    use ethrex_trie::trie_sorted::trie_from_sorted_accounts_wrap;
    use ethrex_trie::{Nibbles, TrieDB, TrieError};

    /// How many account leaves to accumulate before flushing a
    /// FlatKeyValue batch through the trie backend.
    const FKV_EMISSION_BATCH_SIZE: usize = 100_000;

    let trie = store.open_direct_state_trie(*EMPTY_TRIE_HASH)?;
    // The ingest task has been moving snapshot files into the temp DB since
    // the download started; the download's return closed the channel, so the
    // join yields the DB with every received file already ingested. If the
    // task died early, reopen the DB: every chunk it never ingested is still
    // in the snapshot dir (sends are best-effort and `move_files` only
    // removes ingested files), so the sweep below retries them and a
    // persistent ingest error resurfaces from there.
    let db = match ingest_handle.await? {
        Ok(db) => db,
        Err(err) => {
            warn!(
                "Account snapshot ingest task failed ({err}); re-ingesting the remaining snapshot files"
            );
            account_ingestor::open_temp_accounts_db(datadir)?
        }
    };
    // Ingest whatever is left in the snapshot dir: chunks whose send failed
    // because the ingest task had died, or leftovers from a previous run.
    account_ingestor::ingest_remaining_snapshot_files(&db, account_state_snapshots_dir).await?;
    // Single pass: the trie build consumes the sorted leaves while the same
    // decode feeds code-hash collection and the storage-root map, instead of
    // a dedicated full iteration of the temp DB for the code hashes.
    //
    // The same pass also mirrors each leaf into ACCOUNT_FLATKEYVALUE: the
    // (nibble path, RLP value) pairs are byte-identical to the rows the
    // post-sync FlatKeyValue generator would re-derive by walking the finished
    // trie. The rows stay inert — the FKV read gate ("last_written") is
    // untouched, and the coverage marker written below only records that they
    // exist so a later phase can reconcile and promote them.
    let trie_db = trie.db();
    let mut fkv_buffer: Vec<(Nibbles, Vec<u8>)> = Vec::with_capacity(FKV_EMISSION_BATCH_SIZE);
    // `inspect` can't propagate errors, so the first flush failure is parked
    // here and returned once the trie build finishes.
    let mut fkv_flush_error: Option<TrieError> = None;
    let mut fkv_last_emitted: Option<H256> = None;
    let iter = db.full_iterator(rocksdb::IteratorMode::Start);
    let compute_state_root = trie_from_sorted_accounts_wrap(
        trie_db,
        &mut iter
            .map(|k| k.expect("We shouldn't have a rocksdb error here")) // TODO: remove unwrap
            .inspect(|(k, v)| {
                METRICS
                    .account_tries_inserted
                    .fetch_add(1, Ordering::Relaxed);
                let account_state = AccountState::decode(v).expect("We should have accounts here");
                if account_state.code_hash != *EMPTY_KECCAK_HASH {
                    code_hash_collector.add(account_state.code_hash);
                    code_hash_collector.flush_if_needed_sync();
                }
                if account_state.storage_root != *EMPTY_TRIE_HASH {
                    storage_accounts.accounts_with_storage_root.insert(
                        H256::from_slice(k),
                        (Some(account_state.storage_root), Vec::new()),
                    );
                }
                if fkv_flush_error.is_none() {
                    fkv_buffer.push((Nibbles::from_bytes(k), v.to_vec()));
                    fkv_last_emitted = Some(H256::from_slice(k));
                    if fkv_buffer.len() >= FKV_EMISSION_BATCH_SIZE
                        && let Err(err) = trie_db.put_batch(std::mem::take(&mut fkv_buffer))
                    {
                        fkv_flush_error = Some(err);
                    }
                }
            })
            .map(|(k, v)| (H256::from_slice(&k), v.to_vec())),
    )
    .map_err(SyncError::TrieGenerationError)?;

    if let Some(err) = fkv_flush_error {
        return Err(err.into());
    }
    if !fkv_buffer.is_empty() {
        trie_db.put_batch(fkv_buffer)?;
    }
    if let Some(account_hash) = fkv_last_emitted {
        store.set_fkv_prebuilt_accounts_marker(
            Nibbles::from_bytes(account_hash.as_bytes()).as_ref(),
        )?;
    }

    drop(db); // close db before removing directory

    async_fs::remove_dir_all(account_state_snapshots_dir).await?;
    async_fs::remove_dir_all(&get_rocksdb_temp_accounts_dir(datadir)).await?;

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
    let file_paths: Vec<PathBuf> = async_fs::read_dir_paths(account_storages_snapshots_dir).await?;
    // Move SST files into the temp DB instead of copying them. The snapshot dir
    // and the temp DB live under the same datadir, so rename succeeds and we
    // avoid keeping two on-disk copies of the leaf data during ingest.
    let mut ingest_opts = rocksdb::IngestExternalFileOptions::default();
    ingest_opts.set_move_files(true);
    db.ingest_external_file_opts(&ingest_opts, file_paths)
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

    async_fs::remove_dir_all(account_storages_snapshots_dir).await?;
    async_fs::remove_dir_all(&get_rocksdb_temp_storage_dir(datadir)).await?;

    Ok(())
}
