mod code_collector;
mod state_healing;
mod storage_healing;

use crate::peer_handler::{BlockRequestOrder, PeerHandlerError, SNAP_LIMIT};
use crate::peer_table::PeerTableError;
use crate::rlpx::p2p::SUPPORTED_ETH_CAPABILITIES;
use crate::sync::code_collector::CodeHashCollector;
use crate::sync::state_healing::heal_state_trie_wrap;
use crate::sync::storage_healing::heal_storage_trie;
use crate::utils::{
    current_unix_time, delete_leaves_folder, get_account_state_snapshots_dir,
    get_account_storages_snapshots_dir, get_code_hashes_snapshots_dir,
};
use crate::{
    metrics::{CurrentStepValue, METRICS},
    peer_handler::{MAX_BLOCK_BODIES_TO_REQUEST, PeerHandler},
};
use ethrex_blockchain::{BatchBlockProcessingFailure, Blockchain, error::ChainError};
#[cfg(not(feature = "rocksdb"))]
use ethrex_common::U256;
use ethrex_common::types::Code;
use ethrex_common::{
    H256,
    constants::{EMPTY_KECCACK_HASH, EMPTY_TRIE_HASH},
    types::{AccountState, Block, BlockHeader},
};
use ethrex_rlp::{decode::RLPDecode, error::RLPDecodeError};
use ethrex_storage::{Store, error::StoreError};
#[cfg(feature = "rocksdb")]
use ethrex_trie::Trie;
use ethrex_trie::TrieError;
use ethrex_trie::trie_sorted::TrieGenerationError;
use rayon::iter::{ParallelBridge, ParallelIterator};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use std::{
    cmp::min,
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
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

/// Bytecodes to downloader per batch
const BYTECODE_CHUNK_SIZE: usize = 50_000;

/// We assume this amount of slots are missing a block to adjust our timestamp
/// based update pivot algorithm. This is also used to try to find "safe" blocks in the chain
/// that are unlikely to be re-orged.
const MISSING_SLOTS_PERCENTAGE: f64 = 0.8;

/// Maximum attempts before giving up on header downloads during syncing
const MAX_HEADER_FETCH_ATTEMPTS: u64 = 100;

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

        // We validate that we have the folders that are being used empty, as we currently assume
        // they are. If they are not empty we empty the folder
        delete_leaves_folder(&self.datadir);

        info!("Starting to download block headers from peers");

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

            if sync_head_found {
                break;
            };
        }

        info!("All block headers downloaded successfully");

        self.snap_sync(&store, &mut block_sync_state).await?;

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
            debug!("Sync Log 9: Received {} block headers", block_headers.len());

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

        // Download block bodies and execute full blocks in batches
        for start in (start_block_number..end_block_number).step_by(*EXECUTE_BATCH_SIZE) {
            let batch_size = EXECUTE_BATCH_SIZE.min((end_block_number - start) as usize);
            let final_batch = end_block_number == start + batch_size as u64;
            // Retrieve batch from DB
            if !single_batch {
                headers = store
                    .read_fullsync_batch(start, batch_size as u64)
                    .await?
                    .into_iter()
                    .map(|opt| opt.ok_or(SyncError::MissingFullsyncBatch))
                    .collect::<Result<Vec<_>, SyncError>>()?;
            }
            let mut blocks = Vec::new();
            // Request block bodies
            // Download block bodies
            while !headers.is_empty() {
                let header_batch = &headers[..min(MAX_BLOCK_BODIES_TO_REQUEST, headers.len())];
                let bodies = self
                    .peers
                    .request_block_bodies(header_batch)
                    .await?
                    .ok_or(SyncError::BodiesNotFound)?;
                debug!("Obtained: {} block bodies", bodies.len());
                let block_batch = headers
                    .drain(..bodies.len())
                    .zip(bodies)
                    .map(|(header, body)| Block { header, body });
                blocks.extend(block_batch);
            }
            if !blocks.is_empty() {
                // Execute blocks
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

        let state_root = pivot_header.state_root;
        let account_state_snapshots_dir = get_account_state_snapshots_dir(&self.datadir);
        let account_storages_snapshots_dir = get_account_storages_snapshots_dir(&self.datadir);

        let code_hashes_snapshot_dir = get_code_hashes_snapshots_dir(&self.datadir);
        std::fs::create_dir_all(&code_hashes_snapshot_dir).map_err(|_| SyncError::CorruptPath)?;

        // Create collector to store code hashes in files
        let mut code_hash_collector: CodeHashCollector =
            CodeHashCollector::new(code_hashes_snapshot_dir.clone());

        let mut storage_accounts = AccountStorageRoots::default();
        if !std::env::var("SKIP_START_SNAP_SYNC").is_ok_and(|var| !var.is_empty()) {
            // We start by downloading all of the leafs of the trie of accounts
            // The function request_account_range writes the leafs into files in
            // account_state_snapshots_dir

            info!("Starting to download account ranges from peers");
            self.peers
                .request_account_range(
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
                &self.datadir,
                &mut code_hash_collector,
            )
            .await?;
            info!(
                "Finished inserting account ranges, total storage accounts: {}",
                storage_accounts.accounts_with_storage_root.len()
            );
            *METRICS.account_tries_insert_end_time.lock().await = Some(SystemTime::now());

            debug!("Original state root: {state_root:?}");
            debug!("Computed state root after request_account_rages: {computed_state_root:?}");

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
                        &mut self.peers,
                        block_sync_state,
                    )
                    .await?;
                }
                // heal_state_trie_wrap returns false if we ran out of time before fully healing the trie
                // We just need to update the pivot and start again
                if !heal_state_trie_wrap(
                    pivot_header.state_root,
                    store.clone(),
                    &self.peers,
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
                    "Ended request_storage_ranges with {} accounts with storage root unchanged and not downloaded yet and with {} big/healed accounts",
                    storage_accounts.accounts_with_storage_root.len(),
                    // These accounts are marked as heals if they're a big account. This is
                    // because we don't know if the storage root is still valid
                    storage_accounts.healed_accounts.len(),
                );
                if !block_is_stale(&pivot_header) {
                    break;
                }
                debug!("We stopped because of staleness, restarting loop");
            }
            info!("Finished request_storage_ranges");
            *METRICS.storage_tries_download_end_time.lock().await = Some(SystemTime::now());

            *METRICS.storage_tries_insert_start_time.lock().await = Some(SystemTime::now());
            METRICS
                .current_step
                .set(crate::metrics::CurrentStepValue::InsertingStorageRanges);
            let account_storages_snapshots_dir = get_account_storages_snapshots_dir(&self.datadir);

            insert_storages(
                store.clone(),
                accounts_with_storage,
                &account_storages_snapshots_dir,
                &self.datadir,
            )
            .await?;

            *METRICS.storage_tries_insert_end_time.lock().await = Some(SystemTime::now());

            info!("Finished storing storage tries");
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
                    &mut self.peers,
                    block_sync_state,
                )
                .await?;
            }
            healing_done = heal_state_trie_wrap(
                pivot_header.state_root,
                store.clone(),
                &self.peers,
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
                &mut self.peers,
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

        let code_hashes_dir = get_code_hashes_snapshots_dir(&self.datadir);
        let mut seen_code_hashes = HashSet::new();
        let mut code_hashes_to_download = Vec::new();

        info!("Starting download bytecodes from peers");
        for entry in std::fs::read_dir(&code_hashes_dir)
            .map_err(|_| SyncError::CodeHashesSnapshotsDirNotFound)?
        {
            let entry = entry.map_err(|_| SyncError::CorruptPath)?;
            let snapshot_contents = std::fs::read(entry.path())
                .map_err(|err| SyncError::SnapshotReadError(entry.path(), err))?;
            let code_hashes: Vec<H256> = RLPDecode::decode(&snapshot_contents)
                .map_err(|_| SyncError::CodeHashesSnapshotDecodeError(entry.path()))?;

            for hash in code_hashes {
                // If we haven't seen the code hash yet, add it to the list of hashes to download
                if seen_code_hashes.insert(hash) {
                    code_hashes_to_download.push(hash);

                    if code_hashes_to_download.len() >= BYTECODE_CHUNK_SIZE {
                        debug!(
                            "Starting bytecode download of {} hashes",
                            code_hashes_to_download.len()
                        );
                        let bytecodes = self
                            .peers
                            .request_bytecodes(&code_hashes_to_download)
                            .await
                            .map_err(SyncError::PeerHandler)?
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
            let bytecodes = self
                .peers
                .request_bytecodes(&code_hashes_to_download)
                .await
                .map_err(SyncError::PeerHandler)?
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

        info!("Finished download bytecodes from peers");

        std::fs::remove_dir_all(code_hashes_dir)
            .map_err(|_| SyncError::CodeHashesSnapshotsDirNotFound)?;

        *METRICS.bytecode_download_end_time.lock().await = Some(SystemTime::now());

        debug_assert!(validate_bytecodes(store.clone(), pivot_header.state_root));

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
        Ok(())
    }
}

#[cfg(not(feature = "rocksdb"))]
use ethrex_rlp::encode::RLPEncode;

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
        debug!(
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
        debug!("Succesfully updated pivot");
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
            | SyncError::MissingFullsyncBatch => false,
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
    std::fs::remove_dir_all(account_state_snapshots_dir)
        .map_err(|_| SyncError::AccountStoragesSnapshotsDirNotFound)?;
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

    std::fs::remove_dir_all(account_storages_snapshots_dir)
        .map_err(|_| SyncError::AccountStoragesSnapshotsDirNotFound)?;

    Ok(())
}

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

    std::fs::remove_dir_all(account_state_snapshots_dir)
        .map_err(|_| SyncError::AccountStateSnapshotsDirNotFound)?;
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
                .inspect_err(|err: &TrieGenerationError| {
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

    std::fs::remove_dir_all(account_storages_snapshots_dir)
        .map_err(|_| SyncError::AccountStoragesSnapshotsDirNotFound)?;
    std::fs::remove_dir_all(get_rocksdb_temp_storage_dir(datadir))
        .map_err(|e| SyncError::StorageTempDBDirNotFound(e.to_string()))?;

    Ok(())
}
