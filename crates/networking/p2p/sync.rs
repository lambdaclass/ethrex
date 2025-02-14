use ethrex_blockchain::error::ChainError;
use ethrex_common::{
    types::{AccountState, Block, BlockHash, EMPTY_KECCACK_HASH},
    BigEndianHash, H256, U256, U512,
};
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode, error::RLPDecodeError};
use ethrex_storage::{error::StoreError, Store, MAX_SNAPSHOT_READS, STATE_TRIE_SEGMENTS};
use ethrex_trie::{Nibbles, Node, TrieError, TrieState, EMPTY_TRIE_HASH};
use std::{array, cmp::min, collections::BTreeMap, sync::Arc};
use tokio::{
    sync::{
        mpsc::{self, error::SendError, Receiver, Sender},
        Mutex,
    },
    task::JoinSet,
    time::{sleep, Duration, Instant},
};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::{
    kademlia::KademliaTable,
    peer_handler::{BlockRequestOrder, PeerHandler, HASH_MAX},
};

/// The minimum amount of blocks from the head that we want to full sync during a snap sync
const MIN_FULL_BLOCKS: usize = 64;
/// Max size of a bach to stat a fetch request in queues
const BATCH_SIZE: usize = 300;
/// Max size of a bach to stat a fetch request in queues for nodes
const NODE_BATCH_SIZE: usize = 900;
/// Maximum amount of concurrent paralell fetches for a queue
const MAX_PARALLEL_FETCHES: usize = 10;
/// Maximum amount of messages in a channel
const MAX_CHANNEL_MESSAGES: usize = 500;
/// Maximum amount of messages to read from a channel at once
const MAX_CHANNEL_READS: usize = 200;
/// Pace at which progress is shown via info tracing
const SHOW_PROGRESS_INTERVAL_DURATION: Duration = Duration::from_secs(30);

lazy_static::lazy_static! {
    // Size of each state trie segment
    static ref STATE_TRIE_SEGMENT_SIZE: U256 = HASH_MAX.into_uint()/STATE_TRIE_SEGMENTS;
    // Starting hash of each state trie segment
    static ref STATE_TRIE_SEGMENTS_START: [H256; STATE_TRIE_SEGMENTS] = {
        array::from_fn(|i| H256::from_uint(&(*STATE_TRIE_SEGMENT_SIZE * i)))
    };
    // Ending hash of each state trie segment
    static ref STATE_TRIE_SEGMENTS_END: [H256; STATE_TRIE_SEGMENTS] = {
        array::from_fn(|i| H256::from_uint(&(*STATE_TRIE_SEGMENT_SIZE * (i+1))))
    };
}

#[derive(Debug)]
pub enum SyncMode {
    Full,
    Snap,
}

/// Manager in charge the sync process
/// Only performs full-sync but will also be in charge of snap-sync in the future
#[derive(Debug)]
pub struct SyncManager {
    sync_mode: SyncMode,
    peers: PeerHandler,
    /// The last block number used as a pivot for snap-sync
    /// Syncing beyond this pivot should re-enable snap-sync (as we will not have that state stored)
    /// TODO: Reorgs
    last_snap_pivot: u64,
    trie_rebuilder: Option<TrieRebuilder>,
    // Used for cancelling long-living tasks upon shutdown
    cancel_token: CancellationToken,
}

/// Represents the permanently ongoing background trie rebuild process
/// This process will be started whenever a state sync is initiated and will be
/// kept alive throughout sync cycles, only stopping once the tries are fully rebuilt or the node is stopped
#[derive(Debug)]
struct TrieRebuilder {
    state_trie_rebuilder: tokio::task::JoinHandle<Result<(), SyncError>>,
    storage_trie_rebuilder: tokio::task::JoinHandle<Result<(), SyncError>>,
    storage_rebuilder_sender: Sender<Vec<(H256, H256)>>,
}

impl TrieRebuilder {
    /// Returns true is the trie rebuild porcess is alive and well
    fn alive(&self) -> bool {
        !(self.state_trie_rebuilder.is_finished()
            || self.storage_trie_rebuilder.is_finished()
            || self.storage_rebuilder_sender.is_closed())
    }
    /// Waits for the rebuild process to complete and returns the resulting mismatched accounts
    async fn complete(self) -> Result<(), SyncError> {
        // Signal storage rebuilder to finish
        self.storage_rebuilder_sender.send(vec![]).await?;
        self.state_trie_rebuilder.await??;
        self.storage_trie_rebuilder.await?
    }

    /// starts the background trie rebuild process
    fn startup(cancel_token: CancellationToken, store: Store) -> Self {
        let (storage_rebuilder_sender, storage_rebuilder_receiver) =
            mpsc::channel::<Vec<(H256, H256)>>(MAX_CHANNEL_MESSAGES);
        let state_trie_rebuilder = tokio::task::spawn(rebuild_state_trie_in_backgound(
            store.clone(),
            cancel_token.clone(),
        ));
        let storage_trie_rebuilder = tokio::task::spawn(rebuild_storage_trie_in_background(
            store,
            cancel_token,
            storage_rebuilder_receiver,
        ));
        Self {
            state_trie_rebuilder,
            storage_trie_rebuilder,
            storage_rebuilder_sender,
        }
    }
}

impl SyncManager {
    pub fn new(
        peer_table: Arc<Mutex<KademliaTable>>,
        sync_mode: SyncMode,
        cancel_token: CancellationToken,
    ) -> Self {
        Self {
            sync_mode,
            peers: PeerHandler::new(peer_table),
            last_snap_pivot: 0,
            trie_rebuilder: None,
            cancel_token,
        }
    }

    /// Creates a dummy SyncManager for tests where syncing is not needed
    /// This should only be used in tests as it won't be able to connect to the p2p network
    pub fn dummy() -> Self {
        let dummy_peer_table = Arc::new(Mutex::new(KademliaTable::new(Default::default())));
        Self {
            sync_mode: SyncMode::Full,
            peers: PeerHandler::new(dummy_peer_table),
            last_snap_pivot: 0,
            trie_rebuilder: None,
            // This won't be used
            cancel_token: CancellationToken::new(),
        }
    }

    /// Starts a sync cycle, updating the state with all blocks between the current head and the sync head
    /// Will perforn either full or snap sync depending on the manager's `snap_mode`
    /// In full mode, all blocks will be fetched via p2p eth requests and executed to rebuild the state
    /// In snap mode, blocks and receipts will be fetched and stored in parallel while the state is fetched via p2p snap requests
    /// After the sync cycle is complete, the sync mode will be set to full
    /// If the sync fails, no error will be returned but a warning will be emitted
    /// [WARNING] Sync is done optimistically, so headers and bodies may be stored even if their data has not been fully synced if the sync is aborted halfway
    /// [WARNING] Sync is currenlty simplified and will not download bodies + receipts previous to the pivot during snap sync
    pub async fn start_sync(&mut self, current_head: H256, sync_head: H256, store: Store) {
        info!("Syncing from current head {current_head} to sync_head {sync_head}");
        let start_time = Instant::now();
        match self.sync_cycle(current_head, sync_head, store).await {
            Ok(()) => {
                info!(
                    "Sync cycle finished, time elapsed: {} secs",
                    start_time.elapsed().as_secs()
                );
            }
            Err(error) => warn!(
                "Sync cycle failed due to {error}, time elapsed: {} secs ",
                start_time.elapsed().as_secs()
            ),
        }
    }

    /// Performs the sync cycle described in `start_sync`, returns an error if the sync fails at any given step and aborts all active processes
    async fn sync_cycle(
        &mut self,
        mut current_head: H256,
        sync_head: H256,
        store: Store,
    ) -> Result<(), SyncError> {
        // Request all block headers between the current head and the sync head
        // We will begin from the current head so that we download the earliest state first
        // This step is not parallelized
        let mut all_block_hashes = vec![];
        // Check if we have some blocks downloaded from a previous sync attempt
        if matches!(self.sync_mode, SyncMode::Snap) {
            if let Some(last_header) = store.get_header_download_checkpoint()? {
                // Set latest downloaded header as current head for header fetching
                current_head = last_header;
            }
        }
        loop {
            debug!("Requesting Block Headers from {current_head}");
            // Request Block Headers from Peer
            match self
                .peers
                .request_block_headers(current_head, BlockRequestOrder::OldToNew)
                .await
            {
                Some(mut block_headers) => {
                    debug!(
                        "Received {} block headers| Last Number: {}",
                        block_headers.len(),
                        block_headers.last().as_ref().unwrap().number
                    );
                    let mut block_hashes = block_headers
                        .iter()
                        .map(|header| header.compute_block_hash())
                        .collect::<Vec<_>>();
                    // Check if we already found the sync head
                    let sync_head_found = block_hashes.contains(&sync_head);
                    // Update current fetch head if needed
                    if !sync_head_found {
                        current_head = *block_hashes.last().unwrap();
                    }
                    if matches!(self.sync_mode, SyncMode::Snap) {
                        if !sync_head_found {
                            // Update snap state
                            store.set_header_download_checkpoint(current_head)?;
                        } else {
                            // If the sync head is less than 64 blocks away from our current head switch to full-sync
                            let last_header_number = block_headers.last().unwrap().number;
                            let latest_block_number = store.get_latest_block_number()?;
                            if last_header_number.saturating_sub(latest_block_number)
                                < MIN_FULL_BLOCKS as u64
                            {
                                // Too few blocks for a snap sync, switching to full sync
                                store.clear_snap_state()?;
                                self.sync_mode = SyncMode::Full
                            }
                        }
                    }
                    // Discard the first header as we already have it
                    block_hashes.remove(0);
                    block_headers.remove(0);
                    // Store headers and save hashes for full block retrieval
                    all_block_hashes.extend_from_slice(&block_hashes[..]);
                    store.add_block_headers(block_hashes, block_headers)?;

                    if sync_head_found {
                        // No more headers to request
                        break;
                    }
                }
                _ => {
                    warn!("Sync failed to find target block header, aborting");
                    return Ok(());
                }
            }
        }
        // We finished fetching all headers, now we can process them
        match self.sync_mode {
            SyncMode::Snap => {
                // snap-sync: launch tasks to fetch blocks and state in parallel
                // - Fetch each block's body and its receipt via eth p2p requests
                // - Fetch the pivot block's state via snap p2p requests
                // - Execute blocks after the pivot (like in full-sync)
                let pivot_idx = all_block_hashes.len().saturating_sub(MIN_FULL_BLOCKS);
                let pivot_header = store
                    .get_block_header_by_hash(all_block_hashes[pivot_idx])?
                    .ok_or(SyncError::CorruptDB)?;
                debug!(
                    "Selected block {} as pivot for snap sync",
                    pivot_header.number
                );
                let store_bodies_handle = tokio::spawn(store_block_bodies(
                    all_block_hashes[pivot_idx + 1..].to_vec(),
                    self.peers.clone(),
                    store.clone(),
                ));
                // Perform snap sync
                if !self
                    .snap_sync(pivot_header.state_root, store.clone())
                    .await?
                {
                    // Snap sync was not completed, abort and resume it on the next cycle
                    return Ok(());
                }
                // Wait for all bodies to be downloaded
                store_bodies_handle.await??;
                // For all blocks before the pivot: Store the bodies and fetch the receipts (TODO)
                // For all blocks after the pivot: Process them fully
                for hash in &all_block_hashes[pivot_idx + 1..] {
                    let block = store
                        .get_block_by_hash(*hash)?
                        .ok_or(SyncError::CorruptDB)?;
                    ethrex_blockchain::add_block(&block, &store)?;
                    store.set_canonical_block(block.header.number, *hash)?;
                    store.update_latest_block_number(block.header.number)?;
                }
                self.last_snap_pivot = pivot_header.number;
                // Finished a sync cycle without aborting halfway, clear current checkpoint
                store.clear_snap_state()?;
                // Next sync will be full-sync
                self.sync_mode = SyncMode::Full;
            }
            SyncMode::Full => {
                // full-sync: Fetch all block bodies and execute them sequentially to build the state
                download_and_run_blocks(all_block_hashes, self.peers.clone(), store.clone()).await?
            }
        }
        Ok(())
    }
}

/// Requests block bodies from peers via p2p, executes and stores them
/// Returns an error if there was a problem while executing or validating the blocks
async fn download_and_run_blocks(
    mut block_hashes: Vec<BlockHash>,
    peers: PeerHandler,
    store: Store,
) -> Result<(), SyncError> {
    loop {
        debug!("Requesting Block Bodies ");
        if let Some(block_bodies) = peers.request_block_bodies(block_hashes.clone()).await {
            let block_bodies_len = block_bodies.len();
            debug!("Received {} Block Bodies", block_bodies_len);
            // Execute and store blocks
            for (hash, body) in block_hashes
                .drain(..block_bodies_len)
                .zip(block_bodies.into_iter())
            {
                let header = store
                    .get_block_header_by_hash(hash)?
                    .ok_or(SyncError::CorruptDB)?;
                let number = header.number;
                let block = Block::new(header, body);
                if let Err(error) = ethrex_blockchain::add_block(&block, &store) {
                    warn!("Failed to add block during FullSync: {error}");
                    return Err(error.into());
                }
                store.set_canonical_block(number, hash)?;
                store.update_latest_block_number(number)?;
            }
            info!("Executed & stored {} blocks", block_bodies_len);
            // Check if we need to ask for another batch
            if block_hashes.is_empty() {
                break;
            }
        }
    }
    Ok(())
}

/// Fetches all block bodies for the given block hashes via p2p and stores them
async fn store_block_bodies(
    mut block_hashes: Vec<BlockHash>,
    peers: PeerHandler,
    store: Store,
) -> Result<(), SyncError> {
    loop {
        debug!("Requesting Block Bodies ");
        if let Some(block_bodies) = peers.request_block_bodies(block_hashes.clone()).await {
            debug!(" Received {} Block Bodies", block_bodies.len());
            // Track which bodies we have already fetched
            let current_block_hashes = block_hashes.drain(..block_bodies.len());
            // Add bodies to storage
            for (hash, body) in current_block_hashes.zip(block_bodies.into_iter()) {
                store.add_block_body(hash, body)?;
            }

            // Check if we need to ask for another batch
            if block_hashes.is_empty() {
                break;
            }
        }
    }
    Ok(())
}

/// Fetches all receipts for the given block hashes via p2p and stores them
// TODO: remove allow when used again
#[allow(unused)]
async fn store_receipts(
    mut block_hashes: Vec<BlockHash>,
    peers: PeerHandler,
    store: Store,
) -> Result<(), SyncError> {
    loop {
        debug!("Requesting Receipts ");
        if let Some(receipts) = peers.request_receipts(block_hashes.clone()).await {
            debug!(" Received {} Receipts", receipts.len());
            // Track which blocks we have already fetched receipts for
            for (block_hash, receipts) in block_hashes.drain(0..receipts.len()).zip(receipts) {
                store.add_receipts(block_hash, receipts)?;
            }
            // Check if we need to ask for another batch
            if block_hashes.is_empty() {
                break;
            }
        }
    }
    Ok(())
}

impl SyncManager {
    // Downloads the latest state trie and all associated storage tries & bytecodes from peers
    // Rebuilds the state trie and all storage tries based on the downloaded data
    // Performs state healing in order to fix all inconsistencies with the downloaded state
    // Returns the success status, if it is true, then the state is fully consistent and
    // new blocks can be executed on top of it, if false then the state is still inconsistent and
    // snap sync must be resumed on the next sync cycle
    async fn snap_sync(&mut self, state_root: H256, store: Store) -> Result<bool, SyncError> {
        // Begin the background trie rebuild process if it is not active yet or if it crashed
        if !self
            .trie_rebuilder
            .as_ref()
            .is_some_and(|rebuilder| rebuilder.alive())
        {
            self.trie_rebuilder = Some(TrieRebuilder::startup(
                self.cancel_token.clone(),
                store.clone(),
            ));
        };
        // Spawn storage healer earlier so we can start healing stale storages
        let (storage_healer_sender, storage_healer_receiver) =
            mpsc::channel::<Vec<H256>>(MAX_CHANNEL_MESSAGES);
        let storage_healer_handler = tokio::spawn(storage_healer(
            state_root,
            storage_healer_receiver,
            self.peers.clone(),
            store.clone(),
        ));
        // Perform state sync if it was not already completed on a previous cycle
        // Retrieve storage data to check which snap sync phase we are in
        let key_checkpoints = store.get_state_trie_key_checkpoint()?;
        // If we have no key checkpoints or if the key checkpoints are lower than the segment boundaries we are in state sync phase
        if key_checkpoints.is_none()
            || key_checkpoints.is_some_and(|ch| {
                ch.into_iter()
                    .zip(STATE_TRIE_SEGMENTS_END.into_iter())
                    .any(|(ch, end)| ch < end)
            })
        {
            let stale_pivot = state_sync(
                state_root,
                store.clone(),
                self.peers.clone(),
                key_checkpoints,
                self.trie_rebuilder
                    .as_ref()
                    .unwrap()
                    .storage_rebuilder_sender
                    .clone(),
                storage_healer_sender.clone(),
            )
            .await?;
            if stale_pivot {
                warn!("Stale Pivot, aborting state sync");
                return Ok(false);
            }
        }
        // Wait for the trie rebuilder to finish
        info!("Waiting for the trie rebuild to finish");
        let rebuild_start = Instant::now();
        self.trie_rebuilder.take().unwrap().complete().await?;
        info!(
            "State trie rebuilt from snapshot, overtime: {}",
            rebuild_start.elapsed().as_secs()
        );
        // Clear snapshot
        store.clear_snapshot()?;

        // Perform Healing
        let state_heal_complete = heal_state_trie(
            state_root,
            store.clone(),
            self.peers.clone(),
            storage_healer_sender.clone(),
        )
        .await?;
        // Send empty batch to signal that no more batches are incoming
        storage_healer_sender.send(vec![]).await?;
        let storage_heal_complete = storage_healer_handler.await??;
        if !(state_heal_complete && storage_heal_complete) {
            warn!("Stale pivot, aborting healing");
        }
        Ok(state_heal_complete && storage_heal_complete)
    }
}

/// Downloads the leaf values of a Block's state trie by requesting snap state from peers
/// Also downloads the storage tries & bytecodes for each downloaded account
/// Receives optional checkpoints in case there was a previous snap sync process that became stale, in which
/// case it will resume it
/// Returns the pivot staleness status (true if stale, false if not)
/// If the pivot is not stale by the end of the state sync then the state sync was completed succesfuly
async fn state_sync(
    state_root: H256,
    store: Store,
    peers: PeerHandler,
    key_checkpoints: Option<[H256; STATE_TRIE_SEGMENTS]>,
    storage_trie_rebuilder_sender: Sender<Vec<(H256, H256)>>,
    storage_healer_sender: Sender<Vec<H256>>,
) -> Result<bool, SyncError> {
    // Spawn tasks to fetch each state trie segment
    let mut state_trie_tasks = tokio::task::JoinSet::new();
    // Spawn a task to show the state sync progress
    let state_sync_progress = StateSyncProgress::new(Instant::now());
    let show_progress_handle =
        tokio::task::spawn(show_state_sync_progress(state_sync_progress.clone()));
    for i in 0..STATE_TRIE_SEGMENTS {
        state_trie_tasks.spawn(state_sync_segment(
            state_root,
            peers.clone(),
            store.clone(),
            i,
            key_checkpoints.map(|chs| chs[i]),
            state_sync_progress.clone(),
            storage_trie_rebuilder_sender.clone(),
            storage_healer_sender.clone(),
        ));
    }
    show_progress_handle.await?;
    // Check for pivot staleness
    let mut stale_pivot = false;
    let mut state_trie_checkpoint = [H256::zero(); STATE_TRIE_SEGMENTS];
    for res in state_trie_tasks.join_all().await {
        let (index, is_stale, last_key) = res?;
        stale_pivot |= is_stale;
        state_trie_checkpoint[index] = last_key;
    }
    // Update state trie checkpoint
    store.set_state_trie_key_checkpoint(state_trie_checkpoint)?;
    Ok(stale_pivot)
}

/// Downloads the leaf values of the given state trie segment by requesting snap state from peers
/// Also downloads the storage tries & bytecodes for each downloaded account
/// Receives an optional checkpoint from a previous state sync to resume it
/// Returns the segment number, the pivot staleness status (true if stale, false if not), and the last downloaded key
/// If the pivot is not stale by the end of the state sync then the state sync was completed succesfuly
#[allow(clippy::too_many_arguments)]
async fn state_sync_segment(
    state_root: H256,
    peers: PeerHandler,
    store: Store,
    segment_number: usize,
    checkpoint: Option<H256>,
    state_sync_progress: StateSyncProgress,
    storage_trie_rebuilder_sender: Sender<Vec<(H256, H256)>>,
    storage_healer_sender: Sender<Vec<H256>>,
) -> Result<(usize, bool, H256), SyncError> {
    // Resume download from checkpoint if available or start from an empty trie
    let mut start_account_hash = checkpoint.unwrap_or(STATE_TRIE_SEGMENTS_START[segment_number]);
    // Write initial sync progress (this task is not vital so we can detach it)
    tokio::task::spawn(StateSyncProgress::init_segment(
        state_sync_progress.clone(),
        segment_number,
        start_account_hash,
    ));
    // Skip state sync if we are already on healing
    if start_account_hash == STATE_TRIE_SEGMENTS_END[segment_number] {
        // Update sync progress (this task is not vital so we can detach it)
        tokio::task::spawn(StateSyncProgress::end_segment(
            state_sync_progress.clone(),
            segment_number,
        ));
        return Ok((segment_number, false, start_account_hash));
    }
    // Spawn storage & bytecode fetchers
    let (bytecode_sender, bytecode_receiver) = mpsc::channel::<Vec<H256>>(MAX_CHANNEL_MESSAGES);
    let (storage_sender, storage_receiver) =
        mpsc::channel::<Vec<(H256, H256)>>(MAX_CHANNEL_MESSAGES);
    let bytecode_fetcher_handle = tokio::spawn(bytecode_fetcher(
        bytecode_receiver,
        peers.clone(),
        store.clone(),
    ));
    let storage_fetcher_handle = tokio::spawn(storage_fetcher(
        storage_receiver,
        peers.clone(),
        store.clone(),
        state_root,
        storage_trie_rebuilder_sender.clone(),
        storage_healer_sender.clone(),
    ));
    info!("Starting/Resuming state trie download of segment number {segment_number} from key {start_account_hash}");
    // Fetch Account Ranges
    // If we reached the maximum amount of retries then it means the state we are requesting is probably old and no longer available
    let mut stale = false;
    loop {
        // Update sync progress (this task is not vital so we can detach it)
        tokio::task::spawn(StateSyncProgress::update_key(
            state_sync_progress.clone(),
            segment_number,
            start_account_hash,
        ));
        debug!("[Segment {segment_number}]: Requesting Account Range for state root {state_root}, starting hash: {start_account_hash}");
        if let Some((account_hashes, accounts, should_continue)) = peers
            .request_account_range(
                state_root,
                start_account_hash,
                STATE_TRIE_SEGMENTS_END[segment_number],
            )
            .await
        {
            debug!(
                "[Segment {segment_number}]: Received {} account ranges",
                accounts.len()
            );
            // Update starting hash for next batch
            start_account_hash = *account_hashes.last().unwrap();
            // Fetch Account Storage & Bytecode
            let mut code_hashes = vec![];
            let mut account_hashes_and_storage_roots = vec![];
            for (account_hash, account) in account_hashes.iter().zip(accounts.iter()) {
                // Build the batch of code hashes to send to the bytecode fetcher
                // Ignore accounts without code / code we already have stored
                if account.code_hash != *EMPTY_KECCACK_HASH
                    && store.get_account_code(account.code_hash)?.is_none()
                {
                    code_hashes.push(account.code_hash)
                }
                // Build the batch of hashes and roots to send to the storage fetcher
                // Ignore accounts without storage and account's which storage hasn't changed from our current stored state
                if account.storage_root != *EMPTY_TRIE_HASH
                    && !store.contains_storage_node(*account_hash, account.storage_root)?
                {
                    account_hashes_and_storage_roots.push((*account_hash, account.storage_root));
                }
            }
            // Send code hash batch to the bytecode fetcher
            if !code_hashes.is_empty() {
                bytecode_sender.send(code_hashes).await?;
            }
            // Send hash and root batch to the storage fetcher
            if !account_hashes_and_storage_roots.is_empty() {
                storage_sender
                    .send(account_hashes_and_storage_roots)
                    .await?;
            }
            // Update Snapshot
            store.write_snapshot_account_batch(account_hashes, accounts)?;
            // As we are downloading the state trie in segments the `should_continue` flag will mean that there
            // are more accounts to be fetched but these accounts may belong to the next segment
            if !should_continue || start_account_hash >= STATE_TRIE_SEGMENTS_END[segment_number] {
                // All accounts fetched!
                break;
            }
        } else {
            stale = true;
            break;
        }
    }
    debug!("[Segment {segment_number}]: Account Trie Fetching ended, signaling storage & bytecode fetcher process");
    // Update sync progress (this task is not vital so we can detach it)
    tokio::task::spawn(StateSyncProgress::end_segment(
        state_sync_progress.clone(),
        segment_number,
    ));
    // Send empty batch to signal that no more batches are incoming
    storage_sender.send(vec![]).await?;
    bytecode_sender.send(vec![]).await?;
    storage_fetcher_handle.await??;
    bytecode_fetcher_handle.await??;
    if !stale {
        // State sync finished before becoming stale, update checkpoint so we skip state sync on the next cycle
        start_account_hash = STATE_TRIE_SEGMENTS_END[segment_number]
    }
    Ok((segment_number, stale, start_account_hash))
}

/// Waits for incoming code hashes from the receiver channel endpoint, queues them, and fetches and stores their bytecodes in batches
async fn bytecode_fetcher(
    mut receiver: Receiver<Vec<H256>>,
    peers: PeerHandler,
    store: Store,
) -> Result<(), SyncError> {
    let mut pending_bytecodes: Vec<H256> = vec![];
    let mut incoming = true;
    while incoming {
        // Fetch incoming requests
        match receiver.recv().await {
            Some(code_hashes) if !code_hashes.is_empty() => {
                pending_bytecodes.extend(code_hashes);
            }
            // Disconnect / Empty message signaling no more bytecodes to sync
            _ => incoming = false,
        }
        // If we have enough pending bytecodes to fill a batch
        // or if we have no more incoming batches, spawn a fetch process
        while pending_bytecodes.len() >= BATCH_SIZE || !incoming && !pending_bytecodes.is_empty() {
            let next_batch = pending_bytecodes
                .drain(..BATCH_SIZE.min(pending_bytecodes.len()))
                .collect::<Vec<_>>();
            let remaining = fetch_bytecode_batch(next_batch, peers.clone(), store.clone()).await?;
            // Add unfeched bytecodes back to the queue
            pending_bytecodes.extend(remaining);
        }
    }
    Ok(())
}

/// Receives a batch of code hahses, fetches their respective bytecodes via p2p and returns a list of the code hashes that couldn't be fetched in the request (if applicable)
async fn fetch_bytecode_batch(
    mut batch: Vec<H256>,
    peers: PeerHandler,
    store: Store,
) -> Result<Vec<H256>, StoreError> {
    if let Some(bytecodes) = peers.request_bytecodes(batch.clone()).await {
        debug!("Received {} bytecodes", bytecodes.len());
        // Store the bytecodes
        for code in bytecodes.into_iter() {
            store.add_account_code(batch.remove(0), code)?;
        }
    }
    // Return remaining code hashes in the batch if we couldn't fetch all of them
    Ok(batch)
}

/// Waits for incoming account hashes & storage roots from the receiver channel endpoint, queues them, and fetches and stores their bytecodes in batches
/// This function will remain active until either an empty vec is sent to the receiver or the pivot becomes stale
/// Upon finsih, remaining storages will be sent to the storage healer
async fn storage_fetcher(
    mut receiver: Receiver<Vec<(H256, H256)>>,
    peers: PeerHandler,
    store: Store,
    state_root: H256,
    storage_trie_rebuilder_sender: Sender<Vec<(H256, H256)>>,
    storage_healer_sender: Sender<Vec<H256>>,
) -> Result<(), SyncError> {
    // Pending list of storages to fetch
    let mut pending_storage: Vec<(H256, H256)> = vec![];
    // The pivot may become stale while the fetcher is active, we will still keep the process
    // alive until the end signal so we don't lose queued messages
    let mut stale = false;
    let mut incoming = true;
    while incoming {
        // Fetch incoming requests
        let mut msg_buffer = vec![];
        if receiver.recv_many(&mut msg_buffer, MAX_CHANNEL_READS).await != 0 {
            for account_hashes_and_roots in msg_buffer {
                if !account_hashes_and_roots.is_empty() {
                    pending_storage.extend(account_hashes_and_roots);
                } else {
                    // Empty message signaling no more bytecodes to sync
                    incoming = false
                }
            }
        } else {
            // Disconnect
            incoming = false
        }
        // If we have enough pending bytecodes to fill a batch
        // or if we have no more incoming batches, spawn a fetch process
        // If the pivot became stale don't process anything and just save incoming requests
        while !stale
            && (pending_storage.len() >= BATCH_SIZE || (!incoming && !pending_storage.is_empty()))
        {
            // We will be spawning multiple tasks and then collecting their results
            // This uses a loop inside the main loop as the result from these tasks may lead to more values in queue
            let mut storage_tasks = tokio::task::JoinSet::new();
            for _ in 0..MAX_PARALLEL_FETCHES {
                let next_batch = pending_storage
                    .drain(..BATCH_SIZE.min(pending_storage.len()))
                    .collect::<Vec<_>>();
                storage_tasks.spawn(fetch_storage_batch(
                    next_batch,
                    state_root,
                    peers.clone(),
                    store.clone(),
                    storage_trie_rebuilder_sender.clone(),
                ));
                // End loop if we don't have enough elements to fill up a batch
                if pending_storage.is_empty() || (incoming && pending_storage.len() < BATCH_SIZE) {
                    break;
                }
            }
            // Add unfetched accounts to queue and handle stale signal
            for res in storage_tasks.join_all().await {
                let (remaining, is_stale) = res?;
                pending_storage.extend(remaining);
                stale |= is_stale;
            }
        }
    }
    debug!(
        "Concluding storage fetcher, {} storages left in queue to be healed later",
        pending_storage.len()
    );
    if !pending_storage.is_empty() {
        storage_healer_sender
            .send(pending_storage.into_iter().map(|(hash, _)| hash).collect())
            .await?;
    }
    Ok(())
}

/// Receives a batch of account hashes with their storage roots, fetches their respective storage ranges via p2p and returns a list of the code hashes that couldn't be fetched in the request (if applicable)
/// Also returns a boolean indicating if the pivot became stale during the request
async fn fetch_storage_batch(
    mut batch: Vec<(H256, H256)>,
    state_root: H256,
    peers: PeerHandler,
    store: Store,
    storage_trie_rebuilder_sender: Sender<Vec<(H256, H256)>>,
) -> Result<(Vec<(H256, H256)>, bool), SyncError> {
    // A list of all completely fetched storages to send to the rebuilder
    let mut complete_storages = vec![];
    debug!(
        "Requesting storage ranges for addresses {}..{}",
        batch.first().unwrap().0,
        batch.last().unwrap().0
    );
    let (batch_hahses, batch_roots) = batch.clone().into_iter().unzip();
    if let Some((mut keys, mut values, incomplete)) = peers
        .request_storage_ranges(state_root, batch_roots, batch_hahses, H256::zero())
        .await
    {
        debug!("Received {} storage ranges", keys.len(),);
        // Handle incomplete ranges
        if incomplete {
            // An incomplete range cannot be empty
            let (last_keys, last_values) = (keys.pop().unwrap(), values.pop().unwrap());
            // If only one incomplete range is returned then it must belong to a trie that is too big to fit into one request
            // We will handle this large trie separately
            if keys.is_empty() {
                debug!("Large storage trie encountered, handling separately");
                let (account_hash, storage_root) = batch.remove(0);
                if handle_large_storage_range(
                    state_root,
                    account_hash,
                    storage_root,
                    last_keys,
                    last_values,
                    peers.clone(),
                    store.clone(),
                )
                .await?
                {
                    // Pivot became stale
                    // Add trie back to the queue and return stale pivot status
                    batch.push((account_hash, storage_root));
                    return Ok((batch, true));
                }
                // Add large storage to completed storages
                complete_storages.push((account_hash, storage_root));
            }
            // The incomplete range is not the first, we cannot asume it is a large trie, so lets add it back to the queue
        }
        // Store the storage ranges & rebuild the storage trie for each account
        for (keys, values) in keys.into_iter().zip(values.into_iter()) {
            let (account_hash, expected_root) = batch.remove(0);
            // Write storage to snapshot
            store.write_snapshot_storage_batch(account_hash, keys, values)?;
            complete_storages.push((account_hash, expected_root));
        }
        // Send complete storages to the rebuilder
        storage_trie_rebuilder_sender
            .send(complete_storages)
            .await?;
        // Return remaining code hashes in the batch if we couldn't fetch all of them
        return Ok((batch, false));
    }
    // Pivot became stale
    Ok((batch, true))
}

/// Handles the returned incomplete storage range of a large storage trie and
/// fetches the rest of the trie using single requests
/// Returns a boolean indicating is the pivot became stale during fetching
// TODO: Later on this method can be refactored to use a separate queue process
// instead of blocking the current thread for the remainder of the retrieval
async fn handle_large_storage_range(
    state_root: H256,
    account_hash: H256,
    storage_root: H256,
    keys: Vec<H256>,
    values: Vec<U256>,
    peers: PeerHandler,
    store: Store,
) -> Result<bool, SyncError> {
    // First process the initial range
    // Keep hold of the last key as this will be the first key of the next range
    let mut next_key = *keys.last().unwrap();
    store.write_snapshot_storage_batch(account_hash, keys, values)?;
    let mut should_continue = true;
    // Fetch the remaining range
    while should_continue {
        debug!("Fetching large storage trie, current key: {}", next_key);

        if let Some((keys, values, incomplete)) = peers
            .request_storage_range(state_root, storage_root, account_hash, next_key)
            .await
        {
            next_key = *keys.last().unwrap();
            should_continue = incomplete;
            store.write_snapshot_storage_batch(account_hash, keys, values)?;
        } else {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Heals the trie given its state_root by fetching any missing nodes in it via p2p
/// Returns true if healing was fully completed or false if we need to resume healing on the next sync cycle
async fn heal_state_trie(
    state_root: H256,
    store: Store,
    peers: PeerHandler,
    storage_healer_sender: Sender<Vec<H256>>,
) -> Result<bool, SyncError> {
    let mut paths = store.get_state_heal_paths()?.unwrap_or_default();
    // Spawn a bytecode fetcher for this block
    let (bytecode_sender, bytecode_receiver) = mpsc::channel::<Vec<H256>>(MAX_CHANNEL_MESSAGES);
    let bytecode_fetcher_handle = tokio::spawn(bytecode_fetcher(
        bytecode_receiver,
        peers.clone(),
        store.clone(),
    ));
    // Add the current state trie root to the pending paths
    paths.push(Nibbles::default());
    while !paths.is_empty() {
        // Spawn multiple parallel requests
        let mut state_tasks = tokio::task::JoinSet::new();
        for _ in 0..MAX_PARALLEL_FETCHES {
            // Spawn fetcher for the batch
            let batch = paths.drain(0..min(paths.len(), NODE_BATCH_SIZE)).collect();
            state_tasks.spawn(heal_state_batch(
                state_root,
                batch,
                peers.clone(),
                store.clone(),
                storage_healer_sender.clone(),
                bytecode_sender.clone(),
            ));
            // End loop if we have no more paths to fetch
            if paths.is_empty() {
                break;
            }
        }
        // Process the results of each batch
        let mut stale = false;
        for res in state_tasks.join_all().await {
            let (return_paths, is_stale) = res?;
            stale |= is_stale;
            paths.extend(return_paths);
        }
        if stale {
            break;
        }
    }
    debug!("State Healing stopped, signaling storage healer");
    // Save paths for the next cycle
    if !paths.is_empty() {
        debug!("Caching {} paths for the next cycle", paths.len());
        store.set_state_heal_paths(paths.clone())?;
    }
    // Send empty batch to signal that no more batches are incoming
    bytecode_sender.send(vec![]).await?;
    bytecode_fetcher_handle.await??;
    Ok(paths.is_empty())
}

/// Receives a set of state trie paths, fetches their respective nodes, stores them,
/// and returns their children paths and the paths that couldn't be fetched so they can be returned to the queue
/// Also returns a boolean indicating if the pivot became stale during the request
async fn heal_state_batch(
    state_root: H256,
    mut batch: Vec<Nibbles>,
    peers: PeerHandler,
    store: Store,
    storage_sender: Sender<Vec<H256>>,
    bytecode_sender: Sender<Vec<H256>>,
) -> Result<(Vec<Nibbles>, bool), SyncError> {
    if let Some(nodes) = peers
        .request_state_trienodes(state_root, batch.clone())
        .await
    {
        debug!("Received {} state nodes", nodes.len());
        let mut hashed_addresses = vec![];
        let mut code_hashes = vec![];
        // For each fetched node:
        // - Add its children to the queue (if we don't have them already)
        // - If it is a leaf, request its bytecode & storage
        // - If it is a leaf, add its path & value to the trie
        for node in nodes {
            // We cannot keep the trie state open
            let mut trie = store.open_state_trie(*EMPTY_TRIE_HASH);
            let path = batch.remove(0);
            batch.extend(node_missing_children(&node, &path, trie.state())?);
            if let Node::Leaf(node) = &node {
                // Fetch bytecode & storage
                let account = AccountState::decode(&node.value)?;
                // By now we should have the full path = account hash
                let path = &path.concat(node.partial.clone()).to_bytes();
                if path.len() != 32 {
                    // Something went wrong
                    return Err(SyncError::CorruptPath);
                }
                let account_hash = H256::from_slice(path);
                if account.storage_root != *EMPTY_TRIE_HASH
                    && !store.contains_storage_node(account_hash, account.storage_root)?
                {
                    hashed_addresses.push(account_hash);
                }
                if account.code_hash != *EMPTY_KECCACK_HASH
                    && store.get_account_code(account.code_hash)?.is_none()
                {
                    code_hashes.push(account.code_hash);
                }
            }
            // Add node to trie
            let hash = node.compute_hash();
            trie.state_mut().write_node(node, hash)?;
        }
        // Send storage & bytecode requests
        if !hashed_addresses.is_empty() {
            storage_sender.send(hashed_addresses).await?;
        }
        if !code_hashes.is_empty() {
            bytecode_sender.send(code_hashes).await?;
        }
        Ok((batch, false))
    } else {
        Ok((batch, true))
    }
}

/// Waits for incoming hashed addresses from the receiver channel endpoint and queues the associated root nodes for state retrieval
/// Also retrieves their children nodes until we have the full storage trie stored
/// If the state becomes stale while fetching, returns its current queued account hashes
// Returns true if there are no more pending storages in the queue (aka storage healing was completed)
async fn storage_healer(
    state_root: H256,
    mut receiver: Receiver<Vec<H256>>,
    peers: PeerHandler,
    store: Store,
) -> Result<bool, SyncError> {
    let mut pending_paths: BTreeMap<H256, Vec<Nibbles>> = store
        .get_storage_heal_paths()?
        .unwrap_or_default()
        .into_iter()
        .collect();
    // The pivot may become stale while the fetcher is active, we will still keep the process
    // alive until the end signal so we don't lose queued messages
    let mut stale = false;
    let mut incoming = true;
    while incoming || !pending_paths.is_empty() {
        // If we have enough pending storages to fill a batch
        // or if we have no more incoming batches, spawn a fetch process
        // If the pivot became stale don't process anything and just save incoming requests
        let mut storage_tasks = tokio::task::JoinSet::new();
        let mut task_num = 0;
        while !stale && !pending_paths.is_empty() && task_num < MAX_PARALLEL_FETCHES {
            let mut next_batch: BTreeMap<H256, Vec<Nibbles>> = BTreeMap::new();
            // Fill batch
            let mut batch_size = 0;
            while batch_size < NODE_BATCH_SIZE && !pending_paths.is_empty() {
                let (key, val) = pending_paths.pop_first().unwrap();
                batch_size += val.len();
                next_batch.insert(key, val);
            }
            storage_tasks.spawn(heal_storage_batch(
                state_root,
                next_batch.clone(),
                peers.clone(),
                store.clone(),
            ));
            task_num += 1;
        }
        // Add unfetched paths to queue and handle stale signal
        for res in storage_tasks.join_all().await {
            let (remaining, is_stale) = res?;
            pending_paths.extend(remaining);
            stale |= is_stale;
        }

        // Read incoming requests that are already awaiting on the receiver
        // Don't wait for requests unless we have no pending paths left
        if incoming && (!receiver.is_empty() || pending_paths.is_empty()) {
            // Fetch incoming requests
            let mut msg_buffer = vec![];
            if receiver.recv_many(&mut msg_buffer, MAX_CHANNEL_READS).await != 0 {
                for account_hashes in msg_buffer {
                    if !account_hashes.is_empty() {
                        pending_paths.extend(
                            account_hashes
                                .into_iter()
                                .map(|acc_path| (acc_path, vec![Nibbles::default()])),
                        );
                    } else {
                        // Empty message signaling no more bytecodes to sync
                        incoming = false
                    }
                }
            } else {
                // Disconnect
                incoming = false
            }
        }
    }
    let healing_complete = pending_paths.is_empty();
    // Store pending paths
    store.set_storage_heal_paths(pending_paths.into_iter().collect())?;
    Ok(healing_complete)
}

/// Receives a set of storage trie paths (grouped by their corresponding account's state trie path),
/// fetches their respective nodes, stores them, and returns their children paths and the paths that couldn't be fetched so they can be returned to the queue
/// Also returns a boolean indicating if the pivot became stale during the request
async fn heal_storage_batch(
    state_root: H256,
    mut batch: BTreeMap<H256, Vec<Nibbles>>,
    peers: PeerHandler,
    store: Store,
) -> Result<(BTreeMap<H256, Vec<Nibbles>>, bool), SyncError> {
    if let Some(mut nodes) = peers
        .request_storage_trienodes(state_root, batch.clone())
        .await
    {
        debug!("Received {} storage nodes", nodes.len());
        // Process the nodes for each account path
        for (acc_path, paths) in batch.iter_mut() {
            let mut trie = store.open_storage_trie(*acc_path, *EMPTY_TRIE_HASH);
            // Get the corresponding nodes
            for node in nodes.drain(..paths.len().min(nodes.len())) {
                let path = paths.remove(0);
                // Add children to batch
                let children = node_missing_children(&node, &path, trie.state())?;
                paths.extend(children);
                let hash = node.compute_hash();
                trie.state_mut().write_node(node, hash)?;
            }
            // Cut the loop if we ran out of nodes
            if nodes.is_empty() {
                break;
            }
        }
        // Return remaining and added paths to be added to the queue
        // Filter out the storages we completely fetched
        batch.retain(|_, v| !v.is_empty());
        return Ok((batch, false));
    }
    // Pivot became stale, lets inform the fetcher
    Ok((batch, true))
}

/// Returns the partial paths to the node's children if they are not already part of the trie state
fn node_missing_children(
    node: &Node,
    parent_path: &Nibbles,
    trie_state: &TrieState,
) -> Result<Vec<Nibbles>, TrieError> {
    let mut paths = Vec::new();
    match &node {
        Node::Branch(node) => {
            for (index, child) in node.choices.iter().enumerate() {
                if child.is_valid() && trie_state.get_node(child.clone())?.is_none() {
                    paths.push(parent_path.append_new(index as u8));
                }
            }
        }
        Node::Extension(node) => {
            if node.child.is_valid() && trie_state.get_node(node.child.clone())?.is_none() {
                paths.push(parent_path.concat(node.prefix.clone()));
            }
        }
        _ => {}
    }
    Ok(paths)
}

#[derive(Debug, Clone)]
pub(crate) struct SegmentStatus {
    pub current: H256,
    pub end: H256,
}

impl SegmentStatus {
    pub(crate) fn complete(&self) -> bool {
        self.current >= self.end
    }
}

/// Rebuilds the state trie by processing the accounts from the state snapshot
/// Will only stop when state sync has finished and all account have been processed or when the cancel token is cancelled
async fn rebuild_state_trie_in_backgound(
    store: Store,
    cancel_token: CancellationToken,
) -> Result<(), SyncError> {
    // Get initial status from checkpoint if available (aka node restart)
    let checkpoint = store.get_state_trie_rebuild_checkpoint()?;
    let mut rebuild_status = array::from_fn(|i| SegmentStatus {
        current: checkpoint
            .map(|(_, ch)| ch[i])
            .unwrap_or(STATE_TRIE_SEGMENTS_START[i]),
        end: STATE_TRIE_SEGMENTS_END[i],
    });
    let mut root = checkpoint.map(|(root, _)| root).unwrap_or(*EMPTY_TRIE_HASH);
    let mut current_segment = 0;
    let start_time = Instant::now();
    let initial_rebuild_status = rebuild_status.clone();
    let mut last_show_progress = Instant::now();
    while !rebuild_status.iter().all(|status| status.complete()) {
        // Show Progress stats (this task is not vital so we can detach it)
        if Instant::now().duration_since(last_show_progress) >= SHOW_PROGRESS_INTERVAL_DURATION {
            last_show_progress = Instant::now();
            tokio::spawn(show_trie_rebuild_progress(
                start_time,
                initial_rebuild_status.clone(),
                rebuild_status.clone(),
            ));
        }
        // Check for cancellation signal from the main node execution
        if cancel_token.is_cancelled() {
            return Ok(());
        }
        if !rebuild_status[current_segment].complete() {
            // Start rebuilding the current trie segment
            let (current_root, current_hash) = rebuild_state_trie_segment(
                root,
                rebuild_status[current_segment].current,
                current_segment,
                store.clone(),
                cancel_token.clone(),
            )
            .await?;
            // Update status
            root = current_root;
            rebuild_status[current_segment].current = current_hash;
        }
        // Update DB checkpoint
        let checkpoint = (root, rebuild_status.clone().map(|st| st.current));
        store.set_state_trie_rebuild_checkpoint(checkpoint)?;
        // Move on to the next segment
        current_segment = (current_segment + 1) % STATE_TRIE_SEGMENTS
    }

    Ok(())
}

/// Fetches accounts from the state snasphot starting from the `start` hash and adds them to the trie
/// Will stop when there are no more accounts within the segment bounds in the snapshot, or when the cancel token is cancelled
// Returns the current root, the last processed account hash
// If state sync is finished and there are no more snapshot accounts for the segment the account hash
// returned will be the segment bound to notify that the segment has been fully rebuilt
async fn rebuild_state_trie_segment(
    mut root: H256,
    mut start: H256,
    segment_number: usize,
    store: Store,
    cancel_token: CancellationToken,
) -> Result<(H256, H256), SyncError> {
    let mut state_trie = store.open_state_trie(root);
    loop {
        if cancel_token.is_cancelled() {
            break;
        }
        let mut batch = store.read_account_snapshot(start)?;
        // Remove out of bounds elements
        batch.retain(|(hash, _)| *hash <= STATE_TRIE_SEGMENTS_END[segment_number]);
        let unfilled_batch = batch.len() < MAX_SNAPSHOT_READS;
        // Update start
        if let Some(last) = batch.last() {
            start = next_hash(last.0);
        }
        // Process batch
        // Add accounts to the state trie
        for (hash, account) in batch.iter() {
            state_trie.insert(hash.0.to_vec(), account.encode_to_vec())?;
        }
        root = state_trie.hash()?;
        // Return if we have no more snapshot accounts to process for this segemnt
        if unfilled_batch {
            let state_sync_complete = store
                .get_state_trie_key_checkpoint()?
                .is_some_and(|ch| ch[segment_number] == STATE_TRIE_SEGMENTS_END[segment_number]);
            // Mark segment as finished if state sync is complete
            if state_sync_complete {
                start = STATE_TRIE_SEGMENTS_END[segment_number];
            }
            break;
        }
    }
    Ok((root, start))
}

/// Waits for incoming messages from the storage fetcher and rebuilds the associated storages
/// Will stop when the stop signal is received (an empty vec) and there are no more storages in queue or when the cancel token is cancelled
// Only receives fully downloaded storages, and will only emit a warning if there is a mismatch between the expected root and the rebuilt root, as this is considered a bug
async fn rebuild_storage_trie_in_background(
    store: Store,
    cancel_token: CancellationToken,
    mut receiver: Receiver<Vec<(H256, H256)>>,
) -> Result<(), SyncError> {
    // (AccountHash, ExpectedRoot)
    let mut pending_storages = store
        .get_storage_trie_rebuild_pending()?
        .unwrap_or_default();
    let mut incoming = true;
    while incoming || !pending_storages.is_empty() {
        if cancel_token.is_cancelled() {
            break;
        }
        // Read incoming batch
        if !receiver.is_empty() || pending_storages.is_empty() {
            let mut buffer = vec![];
            receiver.recv_many(&mut buffer, MAX_CHANNEL_READS).await;
            incoming = !buffer.iter().any(|batch| batch.is_empty());
            pending_storages.extend(buffer.iter().flatten());
        }

        // Spawn tasks to rebuild current storages
        let mut rebuild_tasks = JoinSet::new();
        for _ in 0..MAX_PARALLEL_FETCHES {
            if pending_storages.is_empty() {
                break;
            }
            let (account_hash, expected_root) = pending_storages.pop().unwrap();
            let store = store.clone();
            rebuild_tasks.spawn(rebuild_storage_trie(
                account_hash,
                expected_root,
                store.clone(),
            ));
        }
        for res in rebuild_tasks.join_all().await {
            res?;
        }
    }
    store.set_storage_trie_rebuild_pending(pending_storages)?;
    Ok(())
}

/// Rebuilds a storage trie by reading from the storage snapshot
/// Assumes that the storage has been fully downloaded and will only emit a warning if there is a mismatch between the expected root and the rebuilt root, as this is considered a bug
async fn rebuild_storage_trie(
    account_hash: H256,
    expected_root: H256,
    store: Store,
) -> Result<(), SyncError> {
    let mut start = H256::zero();
    let mut storage_trie = store.open_storage_trie(account_hash, *EMPTY_TRIE_HASH);
    loop {
        let batch = store.read_storage_snapshot(account_hash, start)?;
        let unfilled_batch = batch.len() < MAX_SNAPSHOT_READS;
        // Update start
        if let Some(last) = batch.last() {
            start = next_hash(last.0);
        }
        // Process batch
        // Launch storage rebuild tasks for all non-empty storages
        for (key, val) in batch {
            storage_trie.insert(key.0.to_vec(), val.encode_to_vec())?;
        }
        storage_trie.hash()?;

        // Return if we have no more snapshot accounts to process for this segemnt
        if unfilled_batch {
            break;
        }
    }
    if storage_trie.hash()? != expected_root {
        warn!("Mismatched storage root for account {account_hash}");
    }
    Ok(())
}

async fn show_trie_rebuild_progress(
    start_time: Instant,
    initial_rebuild_status: [SegmentStatus; STATE_TRIE_SEGMENTS],
    rebuild_status: [SegmentStatus; STATE_TRIE_SEGMENTS],
) {
    // Count how many hashes we already inserted in the trie and how many we inserted this cycle
    let mut accounts_processed = U256::zero();
    let mut accounts_processed_this_cycle = U256::one();
    for i in 0..STATE_TRIE_SEGMENTS {
        accounts_processed +=
            rebuild_status[i].current.into_uint() - (STATE_TRIE_SEGMENTS_START[i].into_uint());
        accounts_processed_this_cycle +=
            rebuild_status[i].current.into_uint() - initial_rebuild_status[i].current.into_uint();
    }
    // Calculate completion rate
    let completion_rate = (U512::from(accounts_processed + U256::one()) * U512::from(100))
        / U512::from(U256::max_value());
    // Time to finish = Time since start / Accounts processed this cycle * Remaining accounts
    let remaining_accounts = U256::MAX - accounts_processed;
    let time_to_finish = (U512::from(start_time.elapsed().as_secs())
        * U512::from(remaining_accounts))
        / (U512::from(accounts_processed_this_cycle));
    info!(
        "State Trie Rebuild Progress: {}%, estimated time to finish: {}",
        completion_rate,
        seconds_to_readable(time_to_finish)
    );
}

#[derive(Clone)]
struct StateSyncProgress {
    data: Arc<Mutex<StateSyncProgressData>>,
}

#[derive(Clone)]
struct StateSyncProgressData {
    cycle_start: Instant,
    initial_keys: [H256; STATE_TRIE_SEGMENTS],
    current_keys: [H256; STATE_TRIE_SEGMENTS],
    ended: [bool; STATE_TRIE_SEGMENTS],
}

impl StateSyncProgress {
    fn new(cycle_start: Instant) -> Self {
        Self {
            data: Arc::new(Mutex::new(StateSyncProgressData {
                cycle_start,
                initial_keys: Default::default(),
                current_keys: Default::default(),
                ended: Default::default(),
            })),
        }
    }

    async fn init_segment(progress: StateSyncProgress, segment_number: usize, initial_key: H256) {
        progress.data.lock().await.initial_keys[segment_number] = initial_key;
    }
    async fn update_key(progress: StateSyncProgress, segment_number: usize, current_key: H256) {
        progress.data.lock().await.current_keys[segment_number] = current_key
    }
    async fn end_segment(progress: StateSyncProgress, segment_number: usize) {
        progress.data.lock().await.ended[segment_number] = true
    }

    // Returns true if the state sync ended
    async fn show_progress(&self) -> bool {
        // Copy the current data so we don't read while it is being written
        let data = self.data.lock().await.clone();
        // Calculate the total amount of accounts synced
        let mut synced_accounts = U256::zero();
        // Calculate the total amount of accounts synced this cycle
        let mut synced_accounts_this_cycle = U256::one();
        for i in 0..STATE_TRIE_SEGMENTS {
            let segment_synced_accounts = data.current_keys[i]
                .into_uint()
                .checked_sub(STATE_TRIE_SEGMENTS_START[i].into_uint())
                .unwrap_or_default();
            let segment_completion_rate = (U512::from(segment_synced_accounts + 1) * 100)
                / U512::from(U256::MAX / STATE_TRIE_SEGMENTS);
            debug!("Segment {i} completion rate: {segment_completion_rate}%");
            synced_accounts += segment_synced_accounts;
            synced_accounts_this_cycle += data.current_keys[i]
                .into_uint()
                .checked_sub(data.initial_keys[i].into_uint())
                .unwrap_or_default();
        }
        // Calculate current progress percentage
        let completion_rate: U512 = (U512::from(synced_accounts) * 100) / U512::from(U256::MAX);
        // Make a simple time to finish estimation based on current progress
        // The estimation relies on account hashes being (close to) evenly distributed
        let remaining_accounts =
            (U512::from(U256::MAX) / 100) * (U512::from(100) - completion_rate);
        // Time to finish = Time since start / Accounts synced this cycle * Remaining accounts
        let time_to_finish_secs =
            U512::from(Instant::now().duration_since(data.cycle_start).as_secs())
                * remaining_accounts
                / U512::from(synced_accounts_this_cycle);
        info!(
            "Downloading state trie, completion rate: {}%, estimated time to finish: {}",
            completion_rate,
            seconds_to_readable(time_to_finish_secs)
        );
        data.ended.iter().all(|e| *e)
    }
}

async fn show_state_sync_progress(progress: StateSyncProgress) {
    // Rest for one interval so we don't start computing on empty progress
    sleep(SHOW_PROGRESS_INTERVAL_DURATION).await;
    let mut interval = tokio::time::interval(SHOW_PROGRESS_INTERVAL_DURATION);
    let mut complete = false;
    while !complete {
        interval.tick().await;
        complete = progress.show_progress().await
    }
}

fn seconds_to_readable(seconds: U512) -> String {
    let (days, rest) = seconds.div_mod(U512::from(60 * 60 * 24));
    let (hours, rest) = rest.div_mod(U512::from(60 * 60));
    let (minutes, seconds) = rest.div_mod(U512::from(60));
    if days > U512::zero() {
        if days > U512::from(15) {
            return "unknown".to_string();
        }
        return format!("Over {days} days");
    }
    format!("{hours}h{minutes}m{seconds}s")
}

/// Returns hash + 1
fn next_hash(hash: H256) -> H256 {
    H256::from_uint(&(hash.into_uint() + 1))
}

#[derive(thiserror::Error, Debug)]
enum SyncError {
    #[error(transparent)]
    Chain(#[from] ChainError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    SendHashes(#[from] SendError<Vec<H256>>),
    #[error(transparent)]
    SendStorage(#[from] SendError<Vec<(H256, H256)>>),
    #[error(transparent)]
    Trie(#[from] TrieError),
    #[error(transparent)]
    Rlp(#[from] RLPDecodeError),
    #[error("Corrupt path during state healing")]
    CorruptPath,
    #[error(transparent)]
    JoinHandle(#[from] tokio::task::JoinError),
    #[error("Missing data from DB")]
    CorruptDB,
}
