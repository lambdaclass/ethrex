mod code_collector;
pub mod healing_cache;
#[cfg(test)]
mod healing_bench;
#[cfg(test)]
mod healing_tests;
mod state_healing;
mod storage_healing;

use crate::peer_handler::{BlockRequestOrder, PeerHandlerError, SNAP_LIMIT};
use crate::peer_table::PeerTableError;
use crate::rlpx::p2p::SUPPORTED_ETH_CAPABILITIES;
use crate::sync::code_collector::CodeHashCollector;
use crate::utils::{
    current_unix_time, delete_leaves_folder, get_account_state_snapshots_dir,
    get_account_storages_snapshots_dir, get_code_hashes_snapshots_dir,
};
use crate::{
    metrics::METRICS,
    peer_handler::PeerHandler,
    snap_sync_progress::SNAP_PROGRESS,
};
use ethrex_blockchain::{BatchBlockProcessingFailure, Blockchain, error::ChainError};
use ethrex_common::U256;
use ethrex_common::types::Code;
use ethrex_common::{
    H256,
    constants::{EMPTY_KECCACK_HASH, EMPTY_TRIE_HASH},
    types::{AccountState, Block, BlockBody, BlockHeader},
};
use ethrex_rlp::{decode::RLPDecode, error::RLPDecodeError};
use ethrex_storage::{Store, SnapSyncTrie, SnapSyncCheckpoint, SnapSyncPhase, error::StoreError};
use ethrex_trie::TrieError;
use ethrex_trie::trie_sorted::TrieGenerationError;
use rayon::iter::{ParallelBridge, ParallelIterator};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::{sync::mpsc::error::SendError, time::Instant};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

/// The minimum amount of blocks from the head that we want to full sync during a snap sync
const MIN_FULL_BLOCKS: u64 = 10_000;
/// Amount of blocks to execute in a single batch during FullSync
const EXECUTE_BATCH_SIZE_DEFAULT: usize = 1024;
/// Amount of seconds between blocks
const SECONDS_PER_BLOCK: u64 = 12;

/// Bytecodes to download per batch - increased for faster downloads
const BYTECODE_CHUNK_SIZE: usize = 25_000;

/// We assume this amount of slots are missing a block to adjust our timestamp
/// based update pivot algorithm. This is also used to try to find "safe" blocks in the chain
/// that are unlikely to be re-orged.
const MISSING_SLOTS_PERCENTAGE: f64 = 0.8;

/// Maximum attempts before giving up on header downloads during syncing
const MAX_HEADER_FETCH_ATTEMPTS: u64 = 100;

/// Maximum age in seconds for a checkpoint to be considered valid for resume.
/// If a checkpoint is older than this, snap sync will start fresh.
const CHECKPOINT_MAX_AGE_SECS: u64 = 30 * 60; // 30 minutes

/// Default storage flush threshold (slots) when memory info unavailable (~160MB)
const DEFAULT_FLUSH_THRESHOLD: usize = 1_000_000;
/// Minimum storage flush threshold (slots) - ~80MB
const MIN_FLUSH_THRESHOLD: usize = 500_000;
/// Maximum storage flush threshold (slots) - ~1.6GB
const MAX_FLUSH_THRESHOLD: usize = 10_000_000;
/// Approximate memory bytes per storage slot in trie
const BYTES_PER_STORAGE_SLOT: usize = 160;
/// Percentage of available memory to use for storage tries (10%)
const MEMORY_USAGE_PERCENT: usize = 10;

/// Get available memory in bytes from /proc/meminfo (Linux only)
/// Returns None on non-Linux systems or if reading fails
#[cfg(target_os = "linux")]
fn get_available_memory_bytes() -> Option<usize> {
    let contents = std::fs::read_to_string("/proc/meminfo").ok()?;
    for line in contents.lines() {
        if line.starts_with("MemAvailable:") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                // Value is in kB
                let kb: usize = parts[1].parse().ok()?;
                return Some(kb * 1024);
            }
        }
    }
    None
}

#[cfg(not(target_os = "linux"))]
fn get_available_memory_bytes() -> Option<usize> {
    None
}

/// Calculate storage flush threshold based on available memory.
/// Uses MEMORY_USAGE_PERCENT of available memory, bounded between MIN and MAX thresholds.
fn calculate_flush_threshold() -> usize {
    if let Some(available_bytes) = get_available_memory_bytes() {
        let target_bytes = available_bytes * MEMORY_USAGE_PERCENT / 100;
        let threshold = target_bytes / BYTES_PER_STORAGE_SLOT;
        let clamped = threshold.clamp(MIN_FLUSH_THRESHOLD, MAX_FLUSH_THRESHOLD);
        debug!(
            "[SNAP SYNC] Dynamic flush threshold: {} slots (~{}MB) based on {}MB available",
            clamped,
            clamped * BYTES_PER_STORAGE_SLOT / 1024 / 1024,
            available_bytes / 1024 / 1024
        );
        clamped
    } else {
        debug!(
            "[SNAP SYNC] Using default flush threshold: {} slots (~{}MB)",
            DEFAULT_FLUSH_THRESHOLD,
            DEFAULT_FLUSH_THRESHOLD * BYTES_PER_STORAGE_SLOT / 1024 / 1024
        );
        DEFAULT_FLUSH_THRESHOLD
    }
}

#[cfg(feature = "sync-test")]
lazy_static::lazy_static! {
    static ref EXECUTE_BATCH_SIZE: usize = std::env::var("EXECUTE_BATCH_SIZE").map(|var| var.parse().expect("Execute batch size environmental variable is not a number")).unwrap_or(EXECUTE_BATCH_SIZE_DEFAULT);
}
#[cfg(not(feature = "sync-test"))]
lazy_static::lazy_static! {
    static ref EXECUTE_BATCH_SIZE: usize = EXECUTE_BATCH_SIZE_DEFAULT;
}

#[derive(Debug, PartialEq, Clone, Default)]
pub enum SyncMode {
    #[default]
    Full,
    Snap,
}

/// Manager in charge the sync process
#[derive(Debug)]
pub struct Syncer {
    /// This is also held by the SyncManager allowing it to track the latest syncmode, without modifying it
    /// No outside process should modify this value, only being modified by the sync cycle
    snap_enabled: Arc<AtomicBool>,
    peers: PeerHandler,
    // Used for cancelling long-living tasks upon shutdown
    cancel_token: CancellationToken,
    blockchain: Arc<Blockchain>,
    /// This string indicates a folder where the snap algorithm will store temporary files that are
    /// used during the syncing process
    datadir: PathBuf,
}

impl Syncer {
    pub fn new(
        peers: PeerHandler,
        snap_enabled: Arc<AtomicBool>,
        cancel_token: CancellationToken,
        blockchain: Arc<Blockchain>,
        datadir: PathBuf,
    ) -> Self {
        Self {
            snap_enabled,
            peers,
            cancel_token,
            blockchain,
            datadir,
        }
    }

    /// Starts a sync cycle, updating the state with all blocks between the current head and the sync head
    /// Will perform either full or snap sync depending on the manager's `snap_mode`
    /// In full mode, all blocks will be fetched via p2p eth requests and executed to rebuild the state
    /// In snap mode, blocks and receipts will be fetched and stored in parallel while the state is fetched via p2p snap requests
    /// After the sync cycle is complete, the sync mode will be set to full
    /// If the sync fails, no error will be returned but a warning will be emitted
    /// [WARNING] Sync is done optimistically, so headers and bodies may be stored even if their data has not been fully synced if the sync is aborted halfway
    /// [WARNING] Sync is currenlty simplified and will not download bodies + receipts previous to the pivot during snap sync
    pub async fn start_sync(&mut self, sync_head: H256, store: Store) {
        let start_time = Instant::now();
        match self.sync_cycle(sync_head, store).await {
            Ok(()) => {
                info!(
                    time_elapsed_s = start_time.elapsed().as_secs(),
                    %sync_head,
                    "Sync cycle finished successfully",
                );
            }

            // If the error is irrecoverable, we exit ethrex
            Err(error) => {
                match error.is_recoverable() {
                    false => {
                        // We exit the node, as we can't recover this error
                        error!(
                            time_elapsed_s = start_time.elapsed().as_secs(),
                            %sync_head,
                            %error, "Sync cycle failed, exiting as the error is irrecoverable",
                        );
                        std::process::exit(2);
                    }
                    true => {
                        // We do nothing, as the error is recoverable
                        error!(
                            time_elapsed_s = start_time.elapsed().as_secs(),
                            %sync_head,
                            %error, "Sync cycle failed, retrying",
                        );
                    }
                }
            }
        }
    }

    /// Performs the sync cycle described in `start_sync`, returns an error if the sync fails at any given step and aborts all active processes
    async fn sync_cycle(&mut self, sync_head: H256, store: Store) -> Result<(), SyncError> {
        // Take picture of the current sync mode, we will update the original value when we need to
        if self.snap_enabled.load(Ordering::Relaxed) {
            METRICS.enable().await;
            let sync_cycle_result = self.sync_cycle_snap(sync_head, store).await;
            METRICS.disable().await;
            sync_cycle_result
        } else {
            self.sync_cycle_full(sync_head, store).await
        }
    }

    /// Performs the sync cycle described in `start_sync`, returns an error if the sync fails at any given step and aborts all active processes
    async fn sync_cycle_snap(&mut self, sync_head: H256, store: Store) -> Result<(), SyncError> {
        // Check for existing checkpoint first
        let existing_checkpoint = store.load_snap_sync_checkpoint().await?;
        if let Some(checkpoint) = &existing_checkpoint {
            if checkpoint.phase != SnapSyncPhase::NotStarted
                && checkpoint.phase != SnapSyncPhase::Completed
            {
                if checkpoint.is_stale(CHECKPOINT_MAX_AGE_SECS) {
                    info!(
                        "Found stale checkpoint (phase: {}, age: {}s). Starting fresh snap sync.",
                        checkpoint.phase,
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_secs())
                            .unwrap_or(0)
                            .saturating_sub(checkpoint.checkpoint_timestamp)
                    );
                } else {
                    info!(
                        "Found valid checkpoint at phase: {}, pivot block: {}, resuming snap sync",
                        checkpoint.phase, checkpoint.pivot_block_number
                    );
                }
            }
        }

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
        let pending_block = match store.get_pending_block(sync_head).await {
            Ok(res) => res,
            Err(e) => return Err(e.into()),
        };

        let mut attempts = 0;

        // Save initial checkpoint for header download phase
        let checkpoint = existing_checkpoint
            .filter(|cp| !cp.is_stale(CHECKPOINT_MAX_AGE_SECS) && cp.phase != SnapSyncPhase::NotStarted)
            .unwrap_or_else(|| SnapSyncCheckpoint::new(SnapSyncPhase::HeaderDownload));
        store.save_snap_sync_checkpoint(&checkpoint).await?;

        // We validate that we have the folders that are being used empty, as we currently assume
        // they are. If they are not empty we empty the folder
        // Note: Only delete if starting fresh (phase is HeaderDownload or NotStarted)
        if checkpoint.phase == SnapSyncPhase::HeaderDownload || checkpoint.phase == SnapSyncPhase::NotStarted {
            delete_leaves_folder(&self.datadir);
        }

        // Log phase entry for header download
        SNAP_PROGRESS.enter_phase(SnapSyncPhase::HeaderDownload as u8).await;
        SNAP_PROGRESS.headers.start(0).await; // Total unknown initially
        let header_download_start = Instant::now();
        let mut last_header_log = Instant::now();

        info!(
            "[SNAP SYNC] Phase 1/{}: Starting header download from block #{}",
            crate::snap_sync_progress::TOTAL_PHASES,
            current_head_number
        );

        loop {
            debug!("Requesting Block Headers from {current_head}");

            let Some(mut block_headers) = self
                .peers
                .request_block_headers(current_head_number, sync_head)
                .await?
            else {
                if attempts > MAX_HEADER_FETCH_ATTEMPTS {
                    warn!("Sync failed to find target block header, aborting");
                    return Ok(());
                }
                attempts += 1;
                tokio::time::sleep(Duration::from_millis(1.1_f64.powf(attempts as f64) as u64))
                    .await;
                continue;
            };

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
                self.snap_enabled.store(false, Ordering::Relaxed);
                return self.sync_cycle_full(sync_head, store.clone()).await;
            }

            // Discard the first header as we already have it
            if block_headers.len() > 1 {
                let block_headers_iter = block_headers.into_iter().skip(1);

                block_sync_state
                    .process_incoming_headers(block_headers_iter)
                    .await?;
            }

            // Log progress periodically (every 2 seconds)
            if last_header_log.elapsed() >= Duration::from_secs(2) {
                let headers_downloaded = block_sync_state.block_hashes.len() as u64;
                let elapsed = header_download_start.elapsed();
                let rate = if elapsed.as_secs_f64() > 0.0 {
                    headers_downloaded as f64 / elapsed.as_secs_f64()
                } else {
                    0.0
                };

                SNAP_PROGRESS.headers.set_current(headers_downloaded);

                info!(
                    "[SNAP SYNC] Phase 1/{}: Headers: {} downloaded | Block #{} | Rate: {}/s | Elapsed: {}",
                    crate::snap_sync_progress::TOTAL_PHASES,
                    crate::snap_sync_progress::format_count(headers_downloaded),
                    current_head_number,
                    crate::snap_sync_progress::format_count(rate as u64),
                    crate::snap_sync_progress::format_duration(elapsed)
                );
                last_header_log = Instant::now();
            }

            if sync_head_found {
                break;
            };
        }

        // Log header download completion
        let total_headers = block_sync_state.block_hashes.len() as u64;
        let elapsed = header_download_start.elapsed();
        let rate = if elapsed.as_secs_f64() > 0.0 {
            total_headers as f64 / elapsed.as_secs_f64()
        } else {
            0.0
        };
        SNAP_PROGRESS.headers.set_current(total_headers);
        info!(
            "[SNAP SYNC] Phase 1/{} complete: Header Download | Downloaded: {} | Rate: {}/s | Duration: {}",
            crate::snap_sync_progress::TOTAL_PHASES,
            crate::snap_sync_progress::format_count(total_headers),
            crate::snap_sync_progress::format_count(rate as u64),
            crate::snap_sync_progress::format_duration(elapsed)
        );

        self.snap_sync(&store, &mut block_sync_state, checkpoint).await?;

        store.clear_snap_state().await?;
        self.snap_enabled.store(false, Ordering::Relaxed);

        Ok(())
    }

    /// Performs the sync cycle described in `start_sync`.
    ///
    /// # Returns
    ///
    /// Returns an error if the sync fails at any given step and aborts all active processes
    async fn sync_cycle_full(
        &mut self,
        mut sync_head: H256,
        store: Store,
    ) -> Result<(), SyncError> {
        info!("Syncing to sync_head {:?}", sync_head);

        // Check if the sync_head is a pending block, if so, gather all pending blocks belonging to its chain
        let mut pending_blocks = vec![];
        while let Some(block) = store.get_pending_block(sync_head).await? {
            if store.is_canonical_sync(block.hash())? {
                // Ignore canonical blocks still in pending
                break;
            }
            sync_head = block.header.parent_hash;
            pending_blocks.insert(0, block);
        }

        // Request all block headers between the sync head and our local chain
        // We will begin from the sync head so that we download the latest state first, ensuring we follow the correct chain
        // This step is not parallelized
        let mut start_block_number;
        let mut end_block_number = 0;
        let mut headers = vec![];
        let mut single_batch = true;

        let mut attempts = 0;

        // Request and store all block headers from the advertised sync head
        loop {
            let Some(mut block_headers) = self
                .peers
                .request_block_headers_from_hash(sync_head, BlockRequestOrder::NewToOld)
                .await?
            else {
                if attempts > MAX_HEADER_FETCH_ATTEMPTS {
                    warn!("Sync failed to find target block header, aborting");
                    return Ok(());
                }
                attempts += 1;
                tokio::time::sleep(Duration::from_millis(1.1_f64.powf(attempts as f64) as u64))
                    .await;
                continue;
            };

            let first_header = block_headers.first().ok_or(SyncError::NoBlocks)?;
            let last_header = block_headers.last().ok_or(SyncError::NoBlocks)?;

            info!(
                "Received {} block headers| First Number: {} Last Number: {}",
                block_headers.len(),
                first_header.number,
                last_header.number,
            );
            end_block_number = end_block_number.max(first_header.number);
            start_block_number = last_header.number;

            sync_head = last_header.parent_hash;
            if store.is_canonical_sync(sync_head)? || sync_head.is_zero() {
                // Incoming chain merged with current chain
                // Filter out already canonical blocks from batch
                let mut first_canon_block = block_headers.len();
                for (index, header) in block_headers.iter().enumerate() {
                    if store.is_canonical_sync(header.hash())? {
                        first_canon_block = index;
                        break;
                    }
                }
                block_headers.drain(first_canon_block..block_headers.len());
                if let Some(last_header) = block_headers.last() {
                    start_block_number = last_header.number;
                }
                // If the fullsync consists of a single batch of headers we can just keep them in memory instead of writing them to Store
                if single_batch {
                    headers = block_headers.into_iter().rev().collect();
                } else {
                    store.add_fullsync_batch(block_headers).await?;
                }
                break;
            }
            store.add_fullsync_batch(block_headers).await?;
            single_batch = false;
        }
        end_block_number += 1;
        start_block_number = start_block_number.max(1);

        // Download block bodies and execute full blocks in batches with pipelining
        // We prefetch the next batch's bodies while executing the current batch
        let batch_starts: Vec<u64> = (start_block_number..end_block_number)
            .step_by(*EXECUTE_BATCH_SIZE)
            .collect();

        // Handle for prefetched bodies (None initially)
        let mut prefetch_handle: Option<
            tokio::task::JoinHandle<Result<Vec<BlockBody>, PeerHandlerError>>,
        > = None;
        let mut prefetched_headers: Option<Vec<BlockHeader>> = None;

        for (batch_idx, &start) in batch_starts.iter().enumerate() {
            let batch_size = EXECUTE_BATCH_SIZE.min((end_block_number - start) as usize);
            let final_batch = batch_idx == batch_starts.len() - 1;

            // Get current batch headers
            let current_headers = if let Some(prefetched) = prefetched_headers.take() {
                prefetched
            } else if !single_batch {
                store
                    .read_fullsync_batch(start, batch_size as u64)
                    .await?
                    .into_iter()
                    .map(|opt| opt.ok_or(SyncError::MissingFullsyncBatch))
                    .collect::<Result<Vec<_>, SyncError>>()?
            } else {
                std::mem::take(&mut headers)
            };

            // Get current batch bodies (either from prefetch or download now)
            let current_bodies = if let Some(handle) = prefetch_handle.take() {
                handle.await.map_err(|_| SyncError::BodiesNotFound)??
            } else {
                self.peers
                    .request_block_bodies_parallel(&current_headers)
                    .await?
            };

            debug!("Obtained: {} block bodies in parallel", current_bodies.len());

            // Start prefetching next batch if there is one
            if !final_batch {
                let next_start = batch_starts[batch_idx + 1];
                let next_batch_size =
                    EXECUTE_BATCH_SIZE.min((end_block_number - next_start) as usize);

                // Load next batch headers
                let next_headers = if !single_batch {
                    store
                        .read_fullsync_batch(next_start, next_batch_size as u64)
                        .await?
                        .into_iter()
                        .map(|opt| opt.ok_or(SyncError::MissingFullsyncBatch))
                        .collect::<Result<Vec<_>, SyncError>>()?
                } else {
                    Vec::new() // Single batch means we won't need more
                };

                if !next_headers.is_empty() {
                    let mut peers_clone = self.peers.clone();
                    let headers_for_prefetch = next_headers.clone();
                    prefetch_handle = Some(tokio::spawn(async move {
                        peers_clone
                            .request_block_bodies_parallel(&headers_for_prefetch)
                            .await
                    }));
                    prefetched_headers = Some(next_headers);
                }
            }

            // Build and execute blocks
            let blocks: Vec<Block> = current_headers
                .into_iter()
                .zip(current_bodies)
                .map(|(header, body)| Block { header, body })
                .collect();

            if !blocks.is_empty() {
                info!(
                    "Executing {} blocks for full sync. First block hash: {:#?} Last block hash: {:#?}",
                    blocks.len(),
                    blocks.first().ok_or(SyncError::NoBlocks)?.hash(),
                    blocks.last().ok_or(SyncError::NoBlocks)?.hash()
                );
                self.add_blocks_in_batch(blocks, final_batch, store.clone())
                    .await?;
            }
        }

        // Execute pending blocks
        if !pending_blocks.is_empty() {
            info!(
                "Executing {} blocks for full sync. First block hash: {:#?} Last block hash: {:#?}",
                pending_blocks.len(),
                pending_blocks.first().ok_or(SyncError::NoBlocks)?.hash(),
                pending_blocks.last().ok_or(SyncError::NoBlocks)?.hash()
            );
            self.add_blocks_in_batch(pending_blocks, true, store.clone())
                .await?;
        }

        store.clear_fullsync_headers().await?;
        Ok(())
    }

    async fn add_blocks_in_batch(
        &self,
        blocks: Vec<Block>,
        final_batch: bool,
        store: Store,
    ) -> Result<(), SyncError> {
        let execution_start = Instant::now();
        // Copy some values for later
        let blocks_len = blocks.len();
        let numbers_and_hashes = blocks
            .iter()
            .map(|b| (b.header.number, b.hash()))
            .collect::<Vec<_>>();
        let (last_block_number, last_block_hash) = numbers_and_hashes
            .last()
            .cloned()
            .ok_or(SyncError::InvalidRangeReceived)?;
        let (first_block_number, first_block_hash) = numbers_and_hashes
            .first()
            .cloned()
            .ok_or(SyncError::InvalidRangeReceived)?;

        let blocks_hashes = blocks.iter().map(|block| block.hash()).collect::<Vec<_>>();
        // Run the batch
        if let Err((err, batch_failure)) = Syncer::add_blocks(
            self.blockchain.clone(),
            blocks,
            final_batch,
            self.cancel_token.clone(),
        )
        .await
        {
            if let Some(batch_failure) = batch_failure {
                warn!("Failed to add block during FullSync: {err}");
                // Since running the batch failed we set the failing block and its descendants
                // with having an invalid ancestor on the following cases.
                if let ChainError::InvalidBlock(_) = err {
                    let mut block_hashes_with_invalid_ancestor: Vec<H256> = vec![];
                    if let Some(index) = blocks_hashes
                        .iter()
                        .position(|x| x == &batch_failure.failed_block_hash)
                    {
                        block_hashes_with_invalid_ancestor = blocks_hashes[index..].to_vec();
                    }

                    for hash in block_hashes_with_invalid_ancestor {
                        store
                            .set_latest_valid_ancestor(hash, batch_failure.last_valid_hash)
                            .await?;
                    }
                }
            }
            return Err(err.into());
        }

        store
            .forkchoice_update(
                numbers_and_hashes,
                last_block_number,
                last_block_hash,
                None,
                None,
            )
            .await?;

        let execution_time: f64 = execution_start.elapsed().as_millis() as f64 / 1000.0;
        let blocks_per_second = blocks_len as f64 / execution_time;

        info!(
            "[SYNCING] Executed & stored {} blocks in {:.3} seconds.\n\
        Started at block with hash {} (number {}).\n\
        Finished at block with hash {} (number {}).\n\
        Blocks per second: {:.3}",
            blocks_len,
            execution_time,
            first_block_hash,
            first_block_number,
            last_block_hash,
            last_block_number,
            blocks_per_second
        );
        Ok(())
    }

    /// Executes the given blocks and stores them
    /// If sync_head_found is true, they will be executed one by one
    /// If sync_head_found is false, they will be executed in a single batch
    async fn add_blocks(
        blockchain: Arc<Blockchain>,
        blocks: Vec<Block>,
        sync_head_found: bool,
        cancel_token: CancellationToken,
    ) -> Result<(), (ChainError, Option<BatchBlockProcessingFailure>)> {
        // If we found the sync head, run the blocks sequentially to store all the blocks's state
        if sync_head_found {
            tokio::task::spawn_blocking(move || {
                let mut last_valid_hash = H256::default();
                for block in blocks {
                    let block_hash = block.hash();
                    blockchain.add_block_pipeline(block).map_err(|e| {
                        (
                            e,
                            Some(BatchBlockProcessingFailure {
                                last_valid_hash,
                                failed_block_hash: block_hash,
                            }),
                        )
                    })?;
                    last_valid_hash = block_hash;
                }
                Ok(())
            })
            .await
            .map_err(|e| (ChainError::Custom(e.to_string()), None))?
        } else {
            blockchain.add_blocks_in_batch(blocks, cancel_token).await
        }
    }
}

/// Fetches all block bodies for the given block headers via p2p and stores them
async fn store_block_bodies(
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

/// Persisted State during the Block Sync phase for SnapSync
#[derive(Clone)]
pub struct SnapBlockSyncState {
    block_hashes: Vec<H256>,
    store: Store,
}

impl SnapBlockSyncState {
    fn new(store: Store) -> Self {
        Self {
            block_hashes: Vec::new(),
            store,
        }
    }

    /// Obtain the current head from where to start or resume block sync
    async fn get_current_head(&self) -> Result<H256, SyncError> {
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
    async fn process_incoming_headers(
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

impl Syncer {
    async fn snap_sync(
        &mut self,
        store: &Store,
        block_sync_state: &mut SnapBlockSyncState,
        mut checkpoint: SnapSyncCheckpoint,
    ) -> Result<(), SyncError> {
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
                &mut self.peers,
                block_sync_state,
            )
            .await?;
        }
        debug!(
            "Selected block {} as pivot for snap sync",
            pivot_header.number
        );

        // Initialize the progress tracker
        SNAP_PROGRESS.start_sync(pivot_header.number).await;

        // Update checkpoint with pivot info
        checkpoint.pivot_block_number = pivot_header.number;
        checkpoint.pivot_block_hash = pivot_header.compute_block_hash();
        checkpoint.pivot_state_root = pivot_header.state_root;

        let account_state_snapshots_dir = get_account_state_snapshots_dir(&self.datadir);
        let account_storages_snapshots_dir = get_account_storages_snapshots_dir(&self.datadir);

        let code_hashes_snapshot_dir = get_code_hashes_snapshots_dir(&self.datadir);
        std::fs::create_dir_all(&code_hashes_snapshot_dir).map_err(|_| SyncError::CorruptPath)?;

        // Create collector to store code hashes in files
        let mut code_hash_collector: CodeHashCollector =
            CodeHashCollector::new(code_hashes_snapshot_dir.clone());

        let mut storage_accounts = AccountStorageRoots::default();
        // Create SnapSyncTrie for direct ethrex_db state insertion
        let mut snap_trie = store.create_snap_sync_trie();

        // Helper to determine if we should skip a phase based on checkpoint
        let should_skip_phase = |phase: SnapSyncPhase, checkpoint_phase: SnapSyncPhase| -> bool {
            (checkpoint_phase as u8) > (phase as u8)
        };

        // Track if we're resuming with persisted state (skipped storage insertion)
        // If true, state is already in the store and we should use persisted state root
        let resuming_with_persisted_state = should_skip_phase(SnapSyncPhase::StorageInsertion, checkpoint.phase);

        if !std::env::var("SKIP_START_SNAP_SYNC").is_ok_and(|var| !var.is_empty()) {
            // Phase: Account Download
            if !should_skip_phase(SnapSyncPhase::AccountDownload, checkpoint.phase) {
                checkpoint.phase = SnapSyncPhase::AccountDownload;
                checkpoint.touch();
                store.save_snap_sync_checkpoint(&checkpoint).await?;

                // Log phase entry
                SNAP_PROGRESS.enter_phase(SnapSyncPhase::AccountDownload as u8).await;
                SNAP_PROGRESS.accounts.start(0).await; // Total unknown until download completes

                // We start by downloading all of the leafs of the trie of accounts
                // The function request_account_range writes the leafs into files in
                // account_state_snapshots_dir
                self.peers
                    .request_account_range(
                        H256::zero(),
                        H256::repeat_byte(0xff),
                        account_state_snapshots_dir.as_ref(),
                        &mut pivot_header,
                        block_sync_state,
                    )
                    .await?;
                SNAP_PROGRESS.complete_phase(SnapSyncPhase::AccountDownload as u8).await;
            } else {
                info!("[SNAP SYNC] Skipping account download phase (already completed in checkpoint)");
            }

            // Phase: Account Insertion
            #[allow(unused_variables)]
            let (computed_state_root, accounts_with_storage) = if !should_skip_phase(SnapSyncPhase::AccountInsertion, checkpoint.phase) {
                checkpoint.phase = SnapSyncPhase::AccountInsertion;
                checkpoint.touch();
                store.save_snap_sync_checkpoint(&checkpoint).await?;

                // Log phase entry
                SNAP_PROGRESS.enter_phase(SnapSyncPhase::AccountInsertion as u8).await;
                SNAP_PROGRESS.accounts.start(0).await;

                *METRICS.account_tries_insert_start_time.lock().await = Some(SystemTime::now());
                // We read the account leafs from the files in account_state_snapshots_dir, write it into
                // ethrex_db's SnapSyncTrie for high-performance state storage

                let result = insert_accounts_with_checkpoint(
                    store.clone(),
                    &mut storage_accounts,
                    &account_state_snapshots_dir,
                    &self.datadir,
                    &mut code_hash_collector,
                    &mut snap_trie,
                    &mut checkpoint,
                )
                .await?;

                // Update progress with final count
                SNAP_PROGRESS.accounts.set_current(snap_trie.account_count() as u64);
                SNAP_PROGRESS.complete_phase(SnapSyncPhase::AccountInsertion as u8).await;

                info!(
                    "[SNAP SYNC] Total storage accounts to process: {}",
                    storage_accounts.accounts_with_storage_root.len()
                );
                *METRICS.account_tries_insert_end_time.lock().await = Some(SystemTime::now());

                // Save incremental state trie checkpoint
                store.save_incremental_state_trie(pivot_header.number, pivot_header.compute_block_hash())?;
                checkpoint.touch();
                store.save_snap_sync_checkpoint(&checkpoint).await?;

                result
            } else {
                info!("[SNAP SYNC] Skipping account insertion phase (already completed in checkpoint)");
                // When resuming, we need default values; the accounts were already inserted
                (H256::zero(), BTreeSet::new())
            };

            // Phase: Storage Download
            if !should_skip_phase(SnapSyncPhase::StorageDownload, checkpoint.phase) {
                checkpoint.phase = SnapSyncPhase::StorageDownload;
                checkpoint.touch();
                store.save_snap_sync_checkpoint(&checkpoint).await?;

                // Log phase entry
                SNAP_PROGRESS.enter_phase(SnapSyncPhase::StorageDownload as u8).await;
                SNAP_PROGRESS.storage.start(0).await;

                *METRICS.storage_tries_download_start_time.lock().await = Some(SystemTime::now());
                // Note: State trie healing is skipped for ethrex_db backend because:
                // 1. SnapSyncTrie builds its own merkle structure from inserted accounts
                // 2. The accounts are not accessible via open_direct_state_trie() until set_snap_sync_trie() is called
                // 3. The storage_accounts data was already populated during account insertion
                // So we proceed directly to storage downloads without healing.
                let mut chunk_index = 0_u64;
                let mut storage_range_request_attempts = 0;
                loop {
                    while block_is_stale(&pivot_header) {
                        pivot_header = update_pivot(
                            pivot_header.number,
                            pivot_header.timestamp,
                            &mut self.peers,
                            block_sync_state,
                        )
                        .await?;
                    }

                    debug!(
                        "Started request_storage_ranges with {} accounts with storage root unchanged",
                        storage_accounts.accounts_with_storage_root.len()
                    );
                    storage_range_request_attempts += 1;
                    if storage_range_request_attempts < 5 {
                        chunk_index = self
                            .peers
                            .request_storage_ranges(
                                &mut storage_accounts,
                                account_storages_snapshots_dir.as_ref(),
                                chunk_index,
                                &mut pivot_header,
                                store.clone(),
                            )
                            .await
                            .map_err(SyncError::PeerHandler)?;
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

                        warn!("Storage could not be downloaded after multiple attempts. Marking for healing.
                            This could impact snap sync time (healing may take a while).");

                        storage_accounts.accounts_with_storage_root.clear();
                    }

                    debug!(
                        "Ended request_storage_ranges with {} accounts remaining, {} big/healed accounts",
                        storage_accounts.accounts_with_storage_root.len(),
                        storage_accounts.healed_accounts.len(),
                    );
                    if !block_is_stale(&pivot_header) {
                        break;
                    }
                    info!("We stopped because of staleness, restarting loop");
                }
                SNAP_PROGRESS.complete_phase(SnapSyncPhase::StorageDownload as u8).await;
                *METRICS.storage_tries_download_end_time.lock().await = Some(SystemTime::now());
            } else {
                info!("[SNAP SYNC] Skipping storage download phase (already completed in checkpoint)");
            }

            // Phase: Storage Insertion
            if !should_skip_phase(SnapSyncPhase::StorageInsertion, checkpoint.phase) {
                checkpoint.phase = SnapSyncPhase::StorageInsertion;
                checkpoint.touch();
                store.save_snap_sync_checkpoint(&checkpoint).await?;

                // Log phase entry
                SNAP_PROGRESS.enter_phase(SnapSyncPhase::StorageInsertion as u8).await;
                SNAP_PROGRESS.storage.start(0).await;

                *METRICS.storage_tries_insert_start_time.lock().await = Some(SystemTime::now());
                METRICS
                    .current_step
                    .set(crate::metrics::CurrentStepValue::InsertingStorageRanges);
                let account_storages_snapshots_dir = get_account_storages_snapshots_dir(&self.datadir);

                insert_storages_with_checkpoint(
                    store.clone(),
                    accounts_with_storage,
                    &account_storages_snapshots_dir,
                    &self.datadir,
                    &mut snap_trie,
                    &mut checkpoint,
                )
                .await?;

                *METRICS.storage_tries_insert_end_time.lock().await = Some(SystemTime::now());

                // Save incremental state trie checkpoint after storage insertion
                store.save_incremental_state_trie(pivot_header.number, pivot_header.compute_block_hash())?;
                checkpoint.touch();
                store.save_snap_sync_checkpoint(&checkpoint).await?;

                SNAP_PROGRESS.complete_phase(SnapSyncPhase::StorageInsertion as u8).await;
            } else {
                info!("[SNAP SYNC] Skipping storage insertion phase (already completed in checkpoint)");
            }
        }

        // Phase: Healing (State and Storage)
        if !should_skip_phase(SnapSyncPhase::StateHealing, checkpoint.phase) {
            checkpoint.phase = SnapSyncPhase::StateHealing;
            checkpoint.touch();
            store.save_snap_sync_checkpoint(&checkpoint).await?;

            // Log phase entry
            SNAP_PROGRESS.enter_phase(SnapSyncPhase::StateHealing as u8).await;
            SNAP_PROGRESS.healing.start(0).await;

            *METRICS.heal_start_time.lock().await = Some(SystemTime::now());

            // Get the state root to verify against
            // If resuming with persisted state, use the already-computed root from the store
            // Otherwise compute from the snap_trie we just built
            let computed_root = if resuming_with_persisted_state {
                info!("[SNAP SYNC] Using persisted state root (resuming from checkpoint)");
                store.get_persisted_state_root()
            } else {
                // Calculate staleness timestamp for healing
                let staleness_timestamp = calculate_staleness_timestamp(pivot_header.timestamp);
                let mut global_slots_healed = 0u64;

                // Heal storage tries for accounts that failed during initial download
                if !storage_accounts.healed_accounts.is_empty() {
                    info!(
                        "[SNAP SYNC] Phase 6/{}: Healing storage for {} accounts",
                        crate::snap_sync_progress::TOTAL_PHASES,
                        storage_accounts.healed_accounts.len()
                    );

                    let healed = storage_healing::heal_storage_trie_snap(
                        pivot_header.state_root,
                        &storage_accounts,
                        &mut self.peers,
                        &mut snap_trie,
                        staleness_timestamp,
                        &mut global_slots_healed,
                    ).await?;

                    if !healed {
                        // Pivot became stale during healing
                        return Err(SyncError::StorageHealingFailed);
                    }
                }

                // Compute initial state root from snap_trie
                let mut computed_root = snap_trie.compute_state_root();

                // If state root doesn't match, try account healing
                if computed_root != pivot_header.state_root {
                    warn!(
                        "[SNAP SYNC] State root mismatch detected. Expected: {:?}, Got: {:?}. Attempting account healing...",
                        pivot_header.state_root, computed_root
                    );

                    // Try to heal by re-downloading accounts
                    let healed = storage_healing::heal_accounts_snap(
                        pivot_header.state_root,
                        &mut self.peers,
                        &mut snap_trie,
                        staleness_timestamp,
                    ).await?;

                    if !healed {
                        // Pivot became stale during healing
                        return Err(SyncError::StorageHealingFailed);
                    }

                    // Recompute state root after account healing
                    computed_root = snap_trie.compute_state_root();
                }

                computed_root
            };

            // Verify state root
            info!("[SNAP SYNC] Phase 6/{}: Verifying state root...", crate::snap_sync_progress::TOTAL_PHASES);
            if computed_root != pivot_header.state_root {
                warn!(
                    "[SNAP SYNC] State root mismatch after healing! Expected: {:?}, Got: {:?}",
                    pivot_header.state_root, computed_root
                );
                return Err(SyncError::StateRootMismatch {
                    expected: pivot_header.state_root,
                    computed: computed_root,
                });
            }
            info!("[SNAP SYNC] State root verified successfully: {:?}", computed_root);

            *METRICS.heal_end_time.lock().await = Some(SystemTime::now());
            SNAP_PROGRESS.complete_phase(SnapSyncPhase::StateHealing as u8).await;
        } else {
            info!("[SNAP SYNC] Skipping healing phase (already completed in checkpoint)");
        }

        // Integrate the SnapSyncTrie into the state manager (only if we built a new trie)
        if !resuming_with_persisted_state {
            store.set_snap_sync_trie(snap_trie);
            let pivot_hash = pivot_header.compute_block_hash();
            store.persist_snap_sync_state(pivot_header.number, pivot_hash)?;
        }

        store.generate_flatkeyvalue()?;

        // Validate state and storage roots (run in all builds, not just debug)
        validate_state_root(store.clone(), pivot_header.state_root).await?;
        validate_storage_root(store.clone(), pivot_header.state_root).await?;

        // Phase: Bytecode Download
        if !should_skip_phase(SnapSyncPhase::BytecodeDownload, checkpoint.phase) {
            checkpoint.phase = SnapSyncPhase::BytecodeDownload;
            checkpoint.touch();
            store.save_snap_sync_checkpoint(&checkpoint).await?;

            // Finish code hash collection
            code_hash_collector.finish().await?;

            // Log phase entry
            SNAP_PROGRESS.enter_phase(SnapSyncPhase::BytecodeDownload as u8).await;

            *METRICS.bytecode_download_start_time.lock().await = Some(SystemTime::now());

            let code_hashes_dir = get_code_hashes_snapshots_dir(&self.datadir);

            // Collect all file paths first
            let file_paths: Vec<PathBuf> = std::fs::read_dir(&code_hashes_dir)
                .map_err(|_| SyncError::CodeHashesSnapshotsDirNotFound)?
                .filter_map(|entry| entry.ok().map(|e| e.path()))
                .collect();

            // Process files sequentially to avoid loading all into memory at once
            let mut seen_code_hashes = HashSet::new();
            let mut all_code_hashes = Vec::new();
            for path in file_paths {
                let snapshot_contents = std::fs::read(&path)
                    .map_err(|err| SyncError::SnapshotReadError(path.clone(), err))?;
                let code_hashes: Vec<H256> = RLPDecode::decode(&snapshot_contents)
                    .map_err(|_| SyncError::CodeHashesSnapshotDecodeError(path))?;

                // Drop file contents immediately
                drop(snapshot_contents);

                for hash in code_hashes {
                    if seen_code_hashes.insert(hash) {
                        all_code_hashes.push(hash);
                    }
                }
            }

            // Skip already downloaded bytecodes based on checkpoint
            let bytecodes_to_skip = checkpoint.bytecodes_downloaded;
            let remaining_code_hashes: Vec<H256> = all_code_hashes
                .into_iter()
                .skip(bytecodes_to_skip)
                .collect();

            let total_bytecodes = remaining_code_hashes.len() + bytecodes_to_skip;
            SNAP_PROGRESS.bytecodes.start(total_bytecodes as u64).await;
            SNAP_PROGRESS.bytecodes.set_current(bytecodes_to_skip as u64);

            info!(
                "[SNAP SYNC] Phase 8/{}: Downloading {} unique bytecodes{}",
                crate::snap_sync_progress::TOTAL_PHASES,
                remaining_code_hashes.len(),
                if bytecodes_to_skip > 0 { format!(" (skipped {} from checkpoint)", bytecodes_to_skip) } else { String::new() }
            );

            // Download bytecodes in chunks
            let mut bytecodes_downloaded = bytecodes_to_skip;
            for chunk in remaining_code_hashes.chunks(BYTECODE_CHUNK_SIZE) {
                let bytecodes = self
                    .peers
                    .request_bytecodes(chunk)
                    .await
                    .map_err(SyncError::PeerHandler)?
                    .ok_or(SyncError::BytecodesNotFound)?;

                store
                    .write_account_code_batch(
                        chunk
                            .iter()
                            .copied()
                            .zip(bytecodes)
                            // SAFETY: hash already checked by the download worker
                            .map(|(hash, code)| (hash, Code::from_bytecode_unchecked(code, hash)))
                            .collect(),
                    )
                    .await?;

                // Update checkpoint and progress after each chunk
                bytecodes_downloaded += chunk.len();
                checkpoint.bytecodes_downloaded = bytecodes_downloaded;
                checkpoint.touch();
                store.save_snap_sync_checkpoint(&checkpoint).await?;

                // Log progress with ETA
                SNAP_PROGRESS.bytecodes.set_current(bytecodes_downloaded as u64);
                SNAP_PROGRESS.log_bytecodes_progress(bytecodes_downloaded as u64, total_bytecodes as u64).await;
            }

            std::fs::remove_dir_all(code_hashes_dir)
                .map_err(|_| SyncError::CodeHashesSnapshotsDirNotFound)?;

            *METRICS.bytecode_download_end_time.lock().await = Some(SystemTime::now());
            SNAP_PROGRESS.complete_phase(SnapSyncPhase::BytecodeDownload as u8).await;

            // Validate bytecodes (run in all builds, not just debug)
            validate_bytecodes(store.clone(), pivot_header.state_root)?;
        } else {
            info!("[SNAP SYNC] Skipping bytecode download phase (already completed in checkpoint)");
            // Still need to finish code hash collector
            code_hash_collector.finish().await?;
        }

        store_block_bodies(
            vec![pivot_header.clone()],
            self.peers.clone(),
            store.clone(),
        )
        .await?;

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

        // Mark snap sync as completed
        checkpoint.phase = SnapSyncPhase::Completed;
        checkpoint.touch();
        store.save_snap_sync_checkpoint(&checkpoint).await?;

        // Log final summary
        SNAP_PROGRESS.complete_sync().await;

        Ok(())
    }
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
#[derive(Debug, Default)]
#[allow(clippy::type_complexity)]
/// We store for optimization the accounts that need to heal storage
pub struct AccountStorageRoots {
    /// The accounts that have not been healed are guaranteed to have the original storage root
    /// we can read this storage root
    pub accounts_with_storage_root: BTreeMap<H256, (Option<H256>, Vec<(H256, H256)>)>,
    /// If an account has been healed, it may return to a previous state, so we just store the account
    /// in a hashset
    pub healed_accounts: HashSet<H256>,
}

#[derive(thiserror::Error, Debug)]
pub enum SyncError {
    #[error(transparent)]
    Chain(#[from] ChainError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("{0}")]
    Send(String),
    #[error(transparent)]
    Trie(#[from] TrieError),
    #[error(transparent)]
    Rlp(#[from] RLPDecodeError),
    #[error(transparent)]
    JoinHandle(#[from] tokio::task::JoinError),
    #[error("Missing data from DB")]
    CorruptDB,
    #[error("No bodies were found for the given headers")]
    BodiesNotFound,
    #[error("Failed to fetch latest canonical block, unable to sync")]
    NoLatestCanonical,
    #[error("Range received is invalid")]
    InvalidRangeReceived,
    #[error("Failed to fetch block number for head {0}")]
    BlockNumber(H256),
    #[error("No blocks found")]
    NoBlocks,
    #[error("Failed to read snapshot from {0:?} with error {1:?}")]
    SnapshotReadError(PathBuf, std::io::Error),
    #[error("Failed to RLP decode account_state_snapshot from {0:?}")]
    SnapshotDecodeError(PathBuf),
    #[error("Failed to RLP decode code_hashes_snapshot from {0:?}")]
    CodeHashesSnapshotDecodeError(PathBuf),
    #[error("Failed to get account state for block {0:?} and account hash {1:?}")]
    AccountState(H256, H256),
    #[error("Failed to fetch bytecodes from peers")]
    BytecodesNotFound,
    #[error("Failed to get account state snapshots directory")]
    AccountStateSnapshotsDirNotFound,
    #[error("Failed to get account storages snapshots directory")]
    AccountStoragesSnapshotsDirNotFound,
    #[error("Failed to get code hashes snapshots directory")]
    CodeHashesSnapshotsDirNotFound,
    #[error("Got different state roots for account hash: {0:?}, expected: {1:?}, computed: {2:?}")]
    DifferentStateRoots(H256, H256, H256),
    #[error("Failed to get block headers")]
    NoBlockHeaders,
    #[error("Peer handler error: {0}")]
    PeerHandler(#[from] PeerHandlerError),
    #[error("Corrupt Path")]
    CorruptPath,
    #[error("Sorted Trie Generation Error: {0}")]
    TrieGenerationError(#[from] TrieGenerationError),
    #[error("Failed to get account temp db directory: {0}")]
    AccountTempDBDirNotFound(String),
    #[error("Failed to get storage temp db directory: {0}")]
    StorageTempDBDirNotFound(String),
    #[error("RocksDB Error: {0}")]
    RocksDBError(String),
    #[error("Bytecode file error")]
    BytecodeFileError,
    #[error("Error in Peer Table: {0}")]
    PeerTableError(#[from] PeerTableError),
    #[error("Missing fullsync batch")]
    MissingFullsyncBatch,
    #[error("Storage healing failed")]
    StorageHealingFailed,
    #[error("State root mismatch: expected {expected}, got {computed}")]
    StateRootMismatch { expected: H256, computed: H256 },
    #[error("State validation failed: {0}")]
    StateValidationFailed(String),
}

impl SyncError {
    pub fn is_recoverable(&self) -> bool {
        match self {
            SyncError::SnapshotReadError(_, _)
            | SyncError::SnapshotDecodeError(_)
            | SyncError::CodeHashesSnapshotDecodeError(_)
            | SyncError::AccountState(_, _)
            | SyncError::BytecodesNotFound
            | SyncError::AccountStateSnapshotsDirNotFound
            | SyncError::AccountStoragesSnapshotsDirNotFound
            | SyncError::CodeHashesSnapshotsDirNotFound
            | SyncError::DifferentStateRoots(_, _, _)
            | SyncError::NoBlockHeaders
            | SyncError::PeerHandler(_)
            | SyncError::CorruptPath
            | SyncError::TrieGenerationError(_)
            | SyncError::AccountTempDBDirNotFound(_)
            | SyncError::StorageTempDBDirNotFound(_)
            | SyncError::RocksDBError(_)
            | SyncError::BytecodeFileError
            | SyncError::NoLatestCanonical
            | SyncError::PeerTableError(_)
            | SyncError::MissingFullsyncBatch
            | SyncError::StorageHealingFailed
            | SyncError::StateRootMismatch { .. }
            | SyncError::StateValidationFailed(_) => false,
            SyncError::Chain(_)
            | SyncError::Store(_)
            | SyncError::Send(_)
            | SyncError::Trie(_)
            | SyncError::Rlp(_)
            | SyncError::JoinHandle(_)
            | SyncError::CorruptDB
            | SyncError::BodiesNotFound
            | SyncError::InvalidRangeReceived
            | SyncError::BlockNumber(_)
            | SyncError::NoBlocks => true,
        }
    }
}

impl<T> From<SendError<T>> for SyncError {
    fn from(value: SendError<T>) -> Self {
        Self::Send(value.to_string())
    }
}

pub async fn validate_state_root(store: Store, state_root: H256) -> Result<(), SyncError> {
    let validated = tokio::task::spawn_blocking(move || {
        store
            .open_locked_state_trie(state_root)
            .map_err(|e| SyncError::StateValidationFailed(format!("couldn't open trie: {e}")))?
            .validate()
            .map_err(|e| SyncError::StateValidationFailed(format!("state tree validation failed: {e}")))
    })
    .await
    .map_err(|e| SyncError::StateValidationFailed(format!("thread spawn failed: {e}")))?;

    validated
}

pub async fn validate_storage_root(store: Store, state_root: H256) -> Result<(), SyncError> {
    tokio::task::spawn_blocking(move || {
        store
            .iter_accounts(state_root)
            .map_err(|e| SyncError::StateValidationFailed(format!("couldn't iterate accounts: {e}")))?
            .par_bridge()
            .try_for_each(|(hashed_address, account_state)| {
                let store_clone = store.clone();
                store_clone
                    .open_locked_storage_trie(
                        hashed_address,
                        state_root,
                        account_state.storage_root,
                    )
                    .map_err(|e| SyncError::StateValidationFailed(format!("couldn't open storage trie: {e}")))?
                    .validate()
                    .map_err(|e| SyncError::StateValidationFailed(format!("storage root validation failed: {e}")))
            })
    })
    .await
    .map_err(|e| SyncError::StateValidationFailed(format!("thread spawn failed: {e}")))?
}

pub fn validate_bytecodes(store: Store, state_root: H256) -> Result<(), SyncError> {
    let mut missing_hashes = Vec::new();
    for (account_hash, account_state) in store
        .iter_accounts(state_root)
        .map_err(|e| SyncError::StateValidationFailed(format!("couldn't iterate accounts: {e}")))?
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
            missing_hashes.push(account_state.code_hash);
        }
    }
    if !missing_hashes.is_empty() {
        return Err(SyncError::StateValidationFailed(format!(
            "missing {} bytecodes",
            missing_hashes.len()
        )));
    }
    Ok(())
}

/// Checkpoint-aware version of insert_accounts.
/// Tracks progress and saves checkpoint after each file processed.
async fn insert_accounts_with_checkpoint(
    store: Store,
    storage_accounts: &mut AccountStorageRoots,
    account_state_snapshots_dir: &Path,
    _: &Path,
    code_hash_collector: &mut CodeHashCollector,
    snap_trie: &mut SnapSyncTrie,
    checkpoint: &mut SnapSyncCheckpoint,
) -> Result<(H256, BTreeSet<H256>), SyncError> {
    // Collect all file paths first
    let mut file_paths: Vec<PathBuf> = std::fs::read_dir(account_state_snapshots_dir)
        .map_err(|_| SyncError::AccountStateSnapshotsDirNotFound)?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .collect();

    // Sort paths to ensure consistent ordering for checkpoint resume
    file_paths.sort();

    let file_count = file_paths.len();
    let files_to_skip = checkpoint.account_files_processed;
    info!(
        "[SNAP SYNC] Phase 3/{}: Inserting accounts from {} files{}",
        crate::snap_sync_progress::TOTAL_PHASES,
        file_count,
        if files_to_skip > 0 { format!(" (skipping {} from checkpoint)", files_to_skip) } else { String::new() }
    );

    // Process files sequentially to avoid loading all into memory at once
    let mut files_processed = 0usize;
    for snapshot_path in file_paths {
        // Skip already processed files based on checkpoint
        if files_processed < files_to_skip {
            files_processed += 1;
            continue;
        }

        // Read and decode one file at a time
        let snapshot_contents = std::fs::read(&snapshot_path)
            .map_err(|err| SyncError::SnapshotReadError(snapshot_path.clone(), err))?;
        let account_states_snapshot: Vec<(H256, AccountState)> =
            RLPDecode::decode(&snapshot_contents)
                .map_err(|_| SyncError::SnapshotDecodeError(snapshot_path.clone()))?;

        // Drop the raw file contents immediately to free memory
        drop(snapshot_contents);

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

        // Insert accounts into ethrex_db's SnapSyncTrie using batch API for better performance
        snap_trie.insert_accounts_batch(
            account_states_snapshot.into_iter().map(|(hash, account)| {
                (hash, account.nonce, account.balance, account.storage_root, account.code_hash)
            })
        );

        // Update checkpoint after each file
        files_processed += 1;
        checkpoint.account_files_processed = files_processed;
        checkpoint.touch();
        store.save_snap_sync_checkpoint(checkpoint).await?;
    }

    std::fs::remove_dir_all(account_state_snapshots_dir)
        .map_err(|_| SyncError::AccountStoragesSnapshotsDirNotFound)?;

    let computed_state_root = snap_trie.compute_state_root();
    Ok((computed_state_root, BTreeSet::new()))
}

/// Checkpoint-aware version of insert_storages.
/// Tracks progress and saves checkpoint after each file processed.
async fn insert_storages_with_checkpoint(
    store: Store,
    _: BTreeSet<H256>,
    account_storages_snapshots_dir: &Path,
    _: &Path,
    snap_trie: &mut SnapSyncTrie,
    checkpoint: &mut SnapSyncCheckpoint,
) -> Result<(), SyncError> {
    use crate::utils::AccountsWithStorage;

    // Collect all file paths first
    let mut file_paths: Vec<PathBuf> = std::fs::read_dir(account_storages_snapshots_dir)
        .map_err(|_| SyncError::AccountStoragesSnapshotsDirNotFound)?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .collect();

    // Sort paths to ensure consistent ordering for checkpoint resume
    file_paths.sort();

    let file_count = file_paths.len();
    let files_to_skip = checkpoint.storage_files_processed;
    info!(
        "[SNAP SYNC] Phase 5/{}: Inserting storage from {} files{}",
        crate::snap_sync_progress::TOTAL_PHASES,
        file_count,
        if files_to_skip > 0 { format!(" (skipping {} from checkpoint)", files_to_skip) } else { String::new() }
    );

    // Process files sequentially to avoid loading all into memory at once
    // This is critical for memory efficiency with large state
    let mut total_storage_count = 0usize;
    let mut files_processed = 0usize;
    let storage_insert_start = std::time::Instant::now();

    // Flush storage tries periodically to keep memory bounded
    // Threshold is calculated dynamically based on available memory
    let storage_flush_threshold = calculate_flush_threshold();
    let mut slots_since_flush = 0usize;
    let mut total_flushes = 0usize;

    for snapshot_path in file_paths {
        // Skip already processed files based on checkpoint
        if files_processed < files_to_skip {
            files_processed += 1;
            continue;
        }

        // Read and decode one file at a time
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

        // Drop the raw file contents immediately to free memory
        drop(snapshot_contents);

        let mut storage_count = 0usize;
        for account_storages in account_storages_snapshot {
            let slot_count = account_storages.storages.len();
            // Batch insert storage slots for each account
            for account_hash in account_storages.accounts {
                snap_trie.insert_storage_batch(
                    account_hash,
                    account_storages.storages.iter().map(|(k, v)| (*k, *v))
                );
                storage_count += slot_count;
            }
        }
        total_storage_count += storage_count;
        slots_since_flush += storage_count;

        // Flush storage tries periodically to free memory
        // This computes storage roots and clears the in-memory tries
        if slots_since_flush >= storage_flush_threshold {
            let flushed = snap_trie.flush_storage_tries();
            total_flushes += 1;
            debug!(
                "[SNAP SYNC] Flushed {} storage tries (flush #{}, {} slots since last)",
                flushed, total_flushes, slots_since_flush
            );
            slots_since_flush = 0;
        }

        // Update checkpoint after each file
        files_processed += 1;
        checkpoint.storage_files_processed = files_processed;
        checkpoint.touch();
        store.save_snap_sync_checkpoint(checkpoint).await?;

        // Log progress periodically
        let elapsed = storage_insert_start.elapsed();
        let rate = if elapsed.as_secs_f64() > 0.0 {
            total_storage_count as f64 / elapsed.as_secs_f64()
        } else {
            0.0
        };
        info!(
            "[SNAP SYNC] Phase 5/{}: Inserted {} slots from file {}/{} | Total: {} | Rate: {}/s",
            crate::snap_sync_progress::TOTAL_PHASES,
            crate::snap_sync_progress::format_count(storage_count as u64),
            files_processed,
            file_count,
            crate::snap_sync_progress::format_count(total_storage_count as u64),
            crate::snap_sync_progress::format_count(rate as u64)
        );
    }

    // Final flush for any remaining storage tries
    if snap_trie.storage_trie_count() > 0 {
        let flushed = snap_trie.flush_storage_tries();
        total_flushes += 1;
        debug!(
            "[SNAP SYNC] Final flush: {} storage tries (total flushes: {})",
            flushed, total_flushes
        );
    }

    info!(
        "[SNAP SYNC] Phase 5/{} complete: Inserted {} total storage slots ({} flushes)",
        crate::snap_sync_progress::TOTAL_PHASES,
        crate::snap_sync_progress::format_count(total_storage_count as u64),
        total_flushes
    );

    std::fs::remove_dir_all(account_storages_snapshots_dir)
        .map_err(|_| SyncError::AccountStoragesSnapshotsDirNotFound)?;

    Ok(())
}
