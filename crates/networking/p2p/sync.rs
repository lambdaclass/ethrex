use ethrex_blockchain::error::ChainError;
use ethrex_core::{
    types::{AccountState, Block, BlockHash, EMPTY_KECCACK_HASH},
    BigEndianHash, H256, U256, U512,
};
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode, error::RLPDecodeError};
use ethrex_storage::{error::StoreError, Store};
use ethrex_trie::{Nibbles, Node, TrieError, TrieState, EMPTY_TRIE_HASH};
use std::{collections::BTreeMap, sync::Arc};
use tokio::{
    sync::{
        mpsc::{self, error::SendError, Receiver, Sender},
        Mutex,
    },
    time::Instant,
};
use tracing::{info, warn};

use crate::rlpx::p2p::Capability;
use crate::{kademlia::KademliaTable, peer_channels::BlockRequestOrder};

/// Maximum amount of times we will ask a peer for an account/storage range
/// If the max amount of retries is exceeded we will asume that the state we are requesting is old and no longer available
const MAX_RETRIES: usize = 5;
/// The minimum amount of blocks from the head that we want to full sync during a snap sync
const MIN_FULL_BLOCKS: usize = 64;
/// Max size of a bach to stat a fetch request in queues
const BATCH_SIZE: usize = 300;

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
    peers: Arc<Mutex<KademliaTable>>,
    /// The last block number used as a pivot for snap-sync
    /// Syncing beyond this pivot should re-enable snap-sync (as we will not have that state stored)
    /// TODO: Reorgs
    last_snap_pivot: u64,
}

impl SyncManager {
    pub fn new(peers: Arc<Mutex<KademliaTable>>, sync_mode: SyncMode) -> Self {
        Self {
            sync_mode,
            peers,
            last_snap_pivot: 0,
        }
    }

    /// Creates a dummy SyncManager for tests where syncing is not needed
    /// This should only be used in tests as it won't be able to connect to the p2p network
    pub fn dummy() -> Self {
        let dummy_peer_table = Arc::new(Mutex::new(KademliaTable::new(Default::default())));
        Self {
            sync_mode: SyncMode::Full,
            peers: dummy_peer_table,
            last_snap_pivot: 0,
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
                let n = store.get_block_header_by_hash(last_header).unwrap().unwrap().number;
                info!("Found checkpoint at {last_header}, number: {n}");
                // Set latest downloaded header as current head for header fetching
                current_head = last_header;
            }
        }
        let mut retry_count = 0;
        while retry_count <= MAX_RETRIES {
            let peer = self
                .peers
                .lock()
                .await
                .get_peer_channels(Capability::Eth)
                .await;
            info!("Requesting Block Headers from {current_head}");
            // Request Block Headers from Peer
            if let Some(mut block_headers) = peer
                .request_block_headers(current_head, BlockRequestOrder::OldToNew)
                .await
            {
                retry_count = 0;
                info!(
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
                    // Update snap state
                    store.set_header_download_checkpoint(current_head)?;
                }
                // Discard the first header as we already have it
                block_hashes.remove(0);
                block_headers.remove(0);
                // Store headers and save hashes for full block retrieval
                all_block_hashes.extend_from_slice(&block_hashes[..]);
                store.add_block_headers(block_hashes, block_headers.clone())?;

                if sync_head_found {
                    let sync_header = block_headers
                        .iter()
                        .find(|h| h.compute_block_hash() == sync_head)
                        .unwrap();
                    info!("Found sync head at block: {}", sync_header.number);

                    // No more headers to request
                    break;
                }
            } else {
                retry_count += 1;
            }
            if retry_count > MAX_RETRIES {
                warn!("Sync failed to find target block header, aborting");
                return Ok(())
            }
        }
        // We finished fetching all headers, now we can process them
        match self.sync_mode {
            SyncMode::Snap => {
                // snap-sync: launch tasks to fetch blocks and state in parallel
                // - Fetch each block's body and its receipt via eth p2p requests
                // - Fetch the pivot block's state via snap p2p requests
                // - Execute blocks after the pivot (like in full-sync)
                let pivot_idx = if all_block_hashes.len() > MIN_FULL_BLOCKS {
                    all_block_hashes.len() - MIN_FULL_BLOCKS
                } else {
                    all_block_hashes.len() - 1
                };
                let pivot_header = store
                    .get_block_header_by_hash(all_block_hashes[pivot_idx])?
                    .ok_or(SyncError::CorruptDB)?;
                info!(
                    "Selected block {} as pivot for snap sync",
                    pivot_header.number
                );
                let store_bodies_handle = tokio::spawn(store_block_bodies(
                    all_block_hashes[pivot_idx..].to_vec(),
                    self.peers.clone(),
                    store.clone(),
                ));
                let stale_pivot =
                    !rebuild_state_trie(pivot_header.state_root, self.peers.clone(), store.clone())
                        .await?;
                if stale_pivot {
                    warn!("Stale pivot, aborting sync");
                    return Ok(());
                }
                // Wait for all bodies to be downloaded
                store_bodies_handle.await??;
                // For all blocks before the pivot: Store the bodies and fetch the receipts
                // For all blocks after the pivot: Process them fully
                for hash in &all_block_hashes[pivot_idx..] {
                    let block = store
                        .get_block_by_hash(*hash)?
                        .ok_or(SyncError::CorruptDB)?;
                    store.set_canonical_block(block.header.number, *hash)?;
                    store.update_latest_block_number(block.header.number)?;
                    ethrex_blockchain::add_block(&block, &store)?;
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
    peers: Arc<Mutex<KademliaTable>>,
    store: Store,
) -> Result<(), SyncError> {
    loop {
        let peer = peers.lock().await.get_peer_channels(Capability::Eth).await;
        info!("Requesting Block Bodies ");
        if let Some(block_bodies) = peer.request_block_bodies(block_hashes.clone()).await {
            let block_bodies_len = block_bodies.len();
            info!("Received {} Block Bodies", block_bodies_len);
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
    peers: Arc<Mutex<KademliaTable>>,
    store: Store,
) -> Result<(), SyncError> {
    loop {
        let peer = peers.lock().await.get_peer_channels(Capability::Eth).await;
        info!("Requesting Block Bodies ");
        if let Some(block_bodies) = peer.request_block_bodies(block_hashes.clone()).await {
            info!(" Received {} Block Bodies", block_bodies.len());
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
    peers: Arc<Mutex<KademliaTable>>,
    store: Store,
) -> Result<(), SyncError> {
    loop {
        let peer = peers.lock().await.get_peer_channels(Capability::Eth).await;
        info!("Requesting Receipts");
        if let Some(receipts) = peer.request_receipts(block_hashes.clone()).await {
            info!(" Received {} Receipts", receipts.len());
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

/// Rebuilds a Block's state trie by requesting snap state from peers, also performs state healing (TODO)
/// Receives an optional checkpoint in case there was a previous snap sync process that became stale, in which
/// case it will continue from the checkpoint and then apply healing to fix inconsistencies with the older state
/// Returns true if all state was fetched or false if the block is too old and the state is no longer available
async fn rebuild_state_trie(
    state_root: H256,
    peers: Arc<Mutex<KademliaTable>>,
    store: Store,
) -> Result<bool, SyncError> {
    info!("Rebuilding State Trie");
    // Spawn storage & bytecode fetchers
    let (bytecode_sender, bytecode_receiver) = mpsc::channel::<Vec<H256>>(500);
    let (storage_sender, storage_receiver) = mpsc::channel::<Vec<(H256, H256)>>(500);
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
    ));
    // Resume download from checkpoint if available or start from an empty trie
    // We cannot keep an open trie here so we will track the root between lookups
    let mut current_state_root = store
        .get_state_trie_root_checkpoint()?
        .unwrap_or(*EMPTY_TRIE_HASH);
    let mut start_account_hash = store.get_state_trie_key_checkpoint()?.unwrap_or_default();
    info!("Starting/Resuming state trie download from key {start_account_hash}");
    // Fetch Account Ranges
    // If we reached the maximum amount of retries then it means the state we are requesting is probably old and no longer available
    let mut retry_count = 0;
    let mut progress_timer = Instant::now();
    let initial_timestamp = Instant::now();
    let initial_account_hash = start_account_hash.into_uint();
    const PROGRESS_OUTPUT_TIMER: std::time::Duration = std::time::Duration::from_secs(30);
    while retry_count <= MAX_RETRIES {
        // Show Progress stats (this task is not vital so we can detach it)
        if Instant::now().duration_since(progress_timer) >= PROGRESS_OUTPUT_TIMER {
            progress_timer = Instant::now();
            tokio::spawn(show_progress(
                start_account_hash,
                initial_account_hash,
                initial_timestamp,
            ));
        }
        let peer = peers
            .clone()
            .lock()
            .await
            .get_peer_channels(Capability::Snap)
            .await;
        info!("Requesting Account Range for state root {state_root}, starting hash: {start_account_hash}");
        if let Some((account_hashes, accounts, should_continue)) = peer
            .request_account_range(state_root, start_account_hash)
            .await
        {
            info!("Received {} account ranges", accounts.len());
            // Reset retry counter
            retry_count = 0;
            // Update starting hash for next batch
            if should_continue {
                start_account_hash = *account_hashes.last().unwrap();
            }
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
            // Update trie
            let mut trie = store.open_state_trie(current_state_root);
            for (account_hash, account) in account_hashes.iter().zip(accounts.iter()) {
                trie.insert(account_hash.0.to_vec(), account.encode_to_vec())?;
            }
            current_state_root = trie.hash()?;

            if !should_continue {
                // All accounts fetched!
                break;
            }
        } else {
            retry_count += 1;
        }
    }
    if retry_count > MAX_RETRIES {
        // Store current checkpoint
        store.set_state_trie_root_checkpoint(current_state_root)?;
        store.set_state_trie_key_checkpoint(start_account_hash)?;
    }
    info!("Account Trie Fetching ended, signaling storage fetcher process");
    // Send empty batch to signal that no more batches are incoming
    storage_sender.send(vec![]).await?;
    let pending_storage_accounts = storage_fetcher_handle.await??;
    let pending_storages = !pending_storage_accounts.is_empty();
    // Next cycle may have different storage roots for these accounts so we will leave them to healing
    if pending_storages {
        let mut stored_pending_storages = store
            .get_pending_storage_heal_accounts()?
            .unwrap_or_default();
        stored_pending_storages.extend(pending_storage_accounts);
        info!(
            "Current pending storage accounts: {}",
            stored_pending_storages.len()
        );
        store.set_pending_storage_heal_accounts(stored_pending_storages)?;
    }
    if retry_count > MAX_RETRIES || pending_storages {
        // Skip healing and return stale status
        return Ok(false);
    }
    // Perform state healing to fix inconsistencies with older state
    info!("Healing");
    let res = heal_state_trie(
        bytecode_sender.clone(),
        state_root,
        current_state_root,
        store.clone(),
        peers.clone(),
    )
    .await?;
    // Send empty batch to signal that no more batches are incoming
    info!("Account Trie fully rebuilt, signaling bytecode fetcher process");
    bytecode_sender.send(vec![]).await?;
    bytecode_fetcher_handle.await??;
    Ok(res)
}

/// Waits for incoming code hashes from the receiver channel endpoint, queues them, and fetches and stores their bytecodes in batches
async fn bytecode_fetcher(
    mut receiver: Receiver<Vec<H256>>,
    peers: Arc<Mutex<KademliaTable>>,
    store: Store,
) -> Result<(), SyncError> {
    let mut pending_bytecodes: Vec<H256> = vec![];
    let mut incoming = true;
    while incoming {
        // Fetch incoming requests
        match receiver.recv().await {
            Some(code_hashes) if !code_hashes.is_empty() => {
                pending_bytecodes.extend(code_hashes);
                info!(
                    "Received incoming bytecode request, current batch: {}/{BATCH_SIZE}",
                    pending_bytecodes.len()
                )
            }
            // Disconnect / Empty message signaling no more bytecodes to sync
            _ => {
                info!("Final bytecode batch");
                incoming = false
            }
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
    peers: Arc<Mutex<KademliaTable>>,
    store: Store,
) -> Result<Vec<H256>, StoreError> {
    loop {
        let peer = peers.lock().await.get_peer_channels(Capability::Snap).await;
        if let Some(bytecodes) = peer.request_bytecodes(batch.clone()).await {
            info!("Received {} bytecodes", bytecodes.len());
            // Store the bytecodes
            for code in bytecodes.into_iter() {
                store.add_account_code(batch.remove(0), code)?;
            }
            // Return remaining code hashes in the batch if we couldn't fetch all of them
            return Ok(batch);
        }
    }
}

/// Waits for incoming account hashes & storage roots from the receiver channel endpoint, queues them, and fetches and stores their bytecodes in batches
/// This function will remain active until either an empty vec is sent to the receiver or the pivot becomes stale
/// In the last case, the fetcher will return the account hashes of the accounts in the queue
async fn storage_fetcher(
    mut receiver: Receiver<Vec<(H256, H256)>>,
    peers: Arc<Mutex<KademliaTable>>,
    store: Store,
    state_root: H256,
) -> Result<Vec<H256>, SyncError> {
    // Pending list of storages to fetch
    let mut pending_storage: Vec<(H256, H256)> = vec![];
    // The pivot may become stale while the fetcher is active, we will still keep the process
    // alive until the end signal so we don't lose queued messages
    let mut stale = false;
    let mut incoming = true;
    while incoming {
        // Fetch incoming requests
        let awaiting_batch = Instant::now();
        match receiver.recv().await {
            Some(account_hashes_and_roots) if !account_hashes_and_roots.is_empty() => {
                info!(
                    "Spent {} secs waiting for incoming batch",
                    awaiting_batch.elapsed().as_secs()
                );
                pending_storage.extend(account_hashes_and_roots);
                info!(
                    "Received incoming storage range request, current batch: {}/{BATCH_SIZE}",
                    pending_storage.len()
                );
                info!("Number of messages in receiver: {}", receiver.len());
            }
            // Disconnect / Empty message signaling no more bytecodes to sync
            _ => {
                info!("Final storage batch");
                incoming = false
            }
        }
        info!("Processing current batches");
        // If we have enough pending bytecodes to fill a batch
        // or if we have no more incoming batches, spawn a fetch process
        // If the pivot became stale don't process anything and just save incoming requests
        if stale {
            info!("Storage fetcher detected stale pivot");
        }
        while !stale
            && (pending_storage.len() >= BATCH_SIZE || !incoming && !pending_storage.is_empty())
        {
            // We will be spawning multiple tasks and then collecting their results
            // This uses a loop inside the main loop as the result from these tasks may lead to more values in queue
            let mut storage_tasks = tokio::task::JoinSet::new();
            let mut task_num = 1;
            while !stale
                && (pending_storage.len() >= BATCH_SIZE || !incoming && !pending_storage.is_empty())
            {
                let next_batch = pending_storage
                    .drain(..BATCH_SIZE.min(pending_storage.len()))
                    .collect::<Vec<_>>();
                info!("Spawning storage fetcher number {task_num}");
                storage_tasks.spawn(fetch_storage_batch(
                    next_batch.clone(),
                    state_root,
                    peers.clone(),
                    store.clone(),
                ));
                task_num +=1;
            }
            // Add unfetched accounts to queue and handle stale signal
            let mut ret_num = 1;
            for res in storage_tasks.join_all().await {
                let (remaining, is_stale) = res?;
                info!("Task {}/{} returned {} elements to the queue", ret_num, task_num, remaining.len());
                pending_storage.extend(remaining);
                stale |= is_stale;
                ret_num +=1;
            }
        }
    }
    info!(
        "Concluding storage fetcher, {} storages left in queue to be healed later",
        pending_storage.len()
    );
    Ok(pending_storage.into_iter().map(|(acc, _)| acc).collect())
}

/// Receives a batch of account hashes with their storage roots, fetches their respective storage ranges via p2p and returns a list of the code hashes that couldn't be fetched in the request (if applicable)
/// Also returns a boolean indicating if the pivot became stale during the request
async fn fetch_storage_batch(
    mut batch: Vec<(H256, H256)>,
    state_root: H256,
    peers: Arc<Mutex<KademliaTable>>,
    store: Store,
) -> Result<(Vec<(H256, H256)>, bool), SyncError> {
    info!(
        "Requesting storage ranges for addresses {}..{}",
        batch.first().unwrap().0,
        batch.last().unwrap().0
    );
    for _ in 0..MAX_RETRIES {
        let peer = peers.lock().await.get_peer_channels(Capability::Snap).await;
        let (batch_hahses, batch_roots) = batch.clone().into_iter().unzip();
        if let Some((mut keys, mut values, incomplete)) = peer
            .request_storage_ranges(state_root, batch_roots, batch_hahses, H256::zero())
            .await
        {
            info!("Received {} storage ranges", keys.len(),);
            // Handle incomplete ranges
            if incomplete {
                // An incomplete range cannot be empty
                let (last_keys, last_values) = (keys.pop().unwrap(), values.pop().unwrap());
                // If only one incomplete range is returned then it must belong to a trie that is too big to fit into one request
                // We will handle this large trie separately
                if keys.is_empty() {
                    info!("Large storage trie encountered, handling separately");
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
                    .await? {
                        // Pivot became stale
                        info!("[DEBUG] Pivot became stale during large trie fetch, scheduling for healing");
                        // Add trie back to the queue and return stale pivot status
                        batch.push((account_hash, storage_root));
                        return Ok((batch, true))
                    }
                }
                // The incomplete range is not the first, we cannot asume it is a large trie, so lets add it back to the queue
            }
            // Store the storage ranges & rebuild the storage trie for each account
            for (keys, values) in keys.into_iter().zip(values.into_iter()) {
                let (account_hash, storage_root) = batch.remove(0);
                let mut trie = store.open_storage_trie(account_hash, *EMPTY_TRIE_HASH);
                for (key, value) in keys.into_iter().zip(values.into_iter()) {
                    trie.insert(key.0.to_vec(), value.encode_to_vec())?;
                }
                if trie.hash()? != storage_root {
                    warn!("State sync failed for storage root {storage_root}");
                }
            }
            // Return remaining code hashes in the batch if we couldn't fetch all of them
            return Ok((batch, false));
        }
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
    peers: Arc<Mutex<KademliaTable>>,
    store: Store,
) -> Result<bool, SyncError> {
    // First process the initial range
    // Keep hold of the last key as this will be the first key of the next range
    let mut next_key = *keys.last().unwrap();
    let mut current_root = {
        let mut trie = store.open_storage_trie(account_hash, *EMPTY_TRIE_HASH);
        for (key, value) in keys.into_iter().zip(values.into_iter()) {
            trie.insert(key.0.to_vec(), value.encode_to_vec())?;
        }
        // Compute current root so we can extend this trie later
        trie.hash()?
    };
    let mut should_continue = true;
    // Fetch the remaining range
    let mut retry_count = 0;
    while should_continue {
        while retry_count <= MAX_RETRIES {
            info!("Fetching large storage trie, current key: {}", next_key);
            let peer = peers.lock().await.get_peer_channels(Capability::Snap).await;
            if let Some((keys, values, incomplete)) = peer
                .request_storage_range(state_root, storage_root, account_hash, next_key)
                .await
            {
                next_key = *keys.last().unwrap();
                should_continue = incomplete;
                let mut trie = store.open_storage_trie(account_hash, current_root);
                for (key, value) in keys.into_iter().zip(values.into_iter()) {
                    trie.insert(key.0.to_vec(), value.encode_to_vec())?;
                }
                // Compute current root so we can extend this trie later
                current_root = trie.hash()?;
                break;
            } else {
                retry_count += 1;
            }
        }
    }
    if current_root != storage_root && retry_count <= MAX_RETRIES {
        warn!("State sync failed for storage root {storage_root}");
    }
    Ok(retry_count > MAX_RETRIES)
}

/// Heals the trie given its state_root by fetching any missing nodes in it via p2p
/// Doesn't store nodes, only leaf values to avoid inconsistent tries on restarts
async fn heal_state_trie(
    bytecode_sender: Sender<Vec<H256>>,
    state_root: H256,
    mut current_root: H256,
    store: Store,
    peers: Arc<Mutex<KademliaTable>>,
) -> Result<bool, SyncError> {
    // Spawn a storage healer for this blocks's storage
    let (storage_sender, storage_receiver) = mpsc::channel::<Vec<H256>>(500);
    let storage_healer_handler = tokio::spawn(storage_healer(
        state_root,
        storage_receiver,
        peers.clone(),
        store.clone(),
    ));
    // Check if we have pending storages to heal from a previous cycle
    if let Some(pending) = store.get_pending_storage_heal_accounts()? {
        info!(
            "Retrieved {} pending storage healing requests",
            pending.len()
        );
        storage_sender.send(pending).await?;
    }
    // Begin by requesting the root node
    let mut paths = vec![Nibbles::default()];
    // Count the number of request retries so we don't get stuck requesting old state
    let mut retry_count = 0;
    while !paths.is_empty() && retry_count < MAX_RETRIES {
        info!("Next paths to fetch: {paths:?}");
        let peer = peers.lock().await.get_peer_channels(Capability::Snap).await;
        if let Some(nodes) = peer
            .request_state_trienodes(state_root, paths.clone())
            .await
        {
            info!("Received {} state nodes", nodes.len());
            // Reset retry counter for next request
            retry_count = 0;
            let mut hahsed_addresses = vec![];
            let mut code_hashes = vec![];
            // For each fetched node:
            // - Add its children to the queue (if we don't have them already)
            // - If it is a leaf, request its bytecode & storage
            // - If it is a leaf, add its path & value to the trie
            for node in nodes {
                // We cannot keep the trie state open
                let mut trie = store.open_state_trie(current_root);
                let path = paths.remove(0);
                paths.extend(node_missing_children(&node, &path, trie.state())?);
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
                        hahsed_addresses.push(account_hash);
                    }
                    if account.code_hash != *EMPTY_KECCACK_HASH
                        && store.get_account_code(account.code_hash)?.is_none()
                    {
                        code_hashes.push(account.code_hash);
                    }
                    // Write values to trie
                    trie.insert(account_hash.0.to_vec(), account.encode_to_vec())?;
                    // Update current root
                    current_root = trie.hash()?;
                }
            }
            // Send storage & bytecode requests
            if !hahsed_addresses.is_empty() {
                storage_sender.send(hahsed_addresses).await?;
            }
            if !code_hashes.is_empty() {
                bytecode_sender.send(code_hashes).await?;
            }
        } else {
            retry_count += 1;
        }
    }
    info!("State Healing stopped, signaling storage healer");
    // Send empty batch to signal that no more batches are incoming
    storage_sender.send(vec![]).await?;
    let pending_storage_heal_accounts = storage_healer_handler.await??;
    // Update pending list
    // If a storage trie was left mid-healing we will heal it again
    let storage_healing_succesful = pending_storage_heal_accounts.is_empty();
    if !storage_healing_succesful {
        info!(
            "{} storages with pending healing",
            pending_storage_heal_accounts.len()
        );
        store.set_pending_storage_heal_accounts(pending_storage_heal_accounts)?;
    }
    Ok(retry_count < MAX_RETRIES && storage_healing_succesful)
}

/// Waits for incoming hashed addresses from the receiver channel endpoint and queues the associated root nodes for state retrieval
/// Also retrieves their children nodes until we have the full storage trie stored
/// If the state becomes stale while fetching, returns its current queued account hashes
async fn storage_healer(
    state_root: H256,
    mut receiver: Receiver<Vec<H256>>,
    peers: Arc<Mutex<KademliaTable>>,
    store: Store,
) -> Result<Vec<H256>, SyncError> {
    // Pending list of storages to fetch
    // Each entry is made up of AccountHash -> (CurrentRoot, Paths)
    let mut pending_storages: BTreeMap<H256, (H256, Vec<Nibbles>)> = BTreeMap::new();
    //let mut pending_storages: Vec<(H256, Nibbles)> = vec![];
    // The pivot may become stale while the fetcher is active, we will still keep the process
    // alive until the end signal so we don't lose queued messages
    let mut stale = false;
    let mut incoming = true;
    while incoming {
        // Fetch incoming requests
        match receiver.recv().await {
            Some(account_paths) if !account_paths.is_empty() => {
                // Add the root paths of each account trie to the queue
                pending_storages.extend(
                    account_paths
                        .into_iter()
                        .map(|acc_path| (acc_path, (*EMPTY_TRIE_HASH, vec![Nibbles::default()]))),
                );
                info!(
                    "Received incoming storage heal request, current batch: {}/{BATCH_SIZE}",
                    pending_storages.len()
                );
                info!("Number of messages in receiver: {}", receiver.len());
            }
            // Disconnect / Empty message signaling no more bytecodes to sync
            _ => incoming = false,
        }
        // If we have enough pending storages to fill a batch
        // or if we have no more incoming batches, spawn a fetch process
        // If the pivot became stale don't process anything and just save incoming requests
        while !stale && !pending_storages.is_empty() {
            let mut next_batch: BTreeMap<H256, (H256, Vec<Nibbles>)> = BTreeMap::new();
            // Fill batch
            let mut batch_size = 0;
            while batch_size < BATCH_SIZE {
                let (key, val) = pending_storages.pop_first().unwrap();
                batch_size += val.1.len();
                next_batch.insert(key, val);
            }
            info!("Sending storage heal batch of size {batch_size}");
            let (return_batch, is_stale) = heal_storage_batch(
                state_root,
                next_batch.clone(),
                peers.clone(),
                store.clone(),
            )
            .await?;
            info!("Returned storage heal batch of size {}", return_batch.iter().map(|b| b.1.1.len()).sum::<usize>());
            pending_storages.extend(return_batch.into_iter());
            stale |= is_stale;
        }
    }
    Ok(pending_storages.into_iter().map(|(h, _)| h).collect())
}

/// Receives a set of storage trie paths (grouped by their corresponding account's state trie path),
/// fetches their respective nodes, stores their values, and returns their children paths and the paths that couldn't be fetched so they can be returned to the queue
/// Also returns a boolean indicating if the pivot became stale during the request
async fn heal_storage_batch(
    state_root: H256,
    mut batch: BTreeMap<H256, (H256, Vec<Nibbles>)>,
    peers: Arc<Mutex<KademliaTable>>,
    store: Store,
) -> Result<(BTreeMap<H256, (H256, Vec<Nibbles>)>, bool), SyncError> {
    for _ in 0..MAX_RETRIES {
        let peer = peers.lock().await.get_peer_channels(Capability::Snap).await;
        let req_batch = batch.iter().map(|(k, v)| (*k, v.1.clone())).collect();
        if let Some(mut nodes) = peer.request_storage_trienodes(state_root, req_batch).await {
            info!("Received {} storage nodes", nodes.len());
            // Process the nodes for each account path
            for (acc_path, (root, paths)) in batch.iter_mut() {
                let mut trie = store.open_storage_trie(*acc_path, *root);
                // Get the corresponding nodes
                for node in nodes.drain(..paths.len().min(nodes.len())) {
                    let path = paths.remove(0);
                    // Add children to batch
                    let children = node_missing_children(&node, &path, trie.state())?;
                    paths.extend(children);
                    // If it is a leaf node, insert values into the trie
                    if let Node::Leaf(leaf) = node {
                        let path = &path.concat(leaf.partial.clone()).to_bytes();
                        if path.len() != 32 {
                            // Something went wrong
                            return Err(SyncError::CorruptPath);
                        }
                        trie.insert(path.to_vec(), leaf.value.encode_to_vec())?;
                    }
                }
                // Update current root
                *root = trie.hash()?;
                // Cut the loop if we ran out of nodes
                if nodes.is_empty() {
                    break;
                }
            }
            // Return remaining and added paths to be added to the queue
            return Ok((batch, false));
        }
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

async fn show_progress(
    current_account_hash: H256,
    initial_account_hash: U256,
    start_time: Instant,
) {
    // Calculate current progress percentage
    // Add 1 here to avoid dividing by zero, the change should be inperceptible
    let completion_rate: U512 =
        U512::from(current_account_hash.into_uint() + 1) * 100 / U512::from(U256::MAX);
    // Make a simple time to finish estimation based on current progress
    // The estimation relies on account hashes being (close to) evenly distributed
    let synced_account_hashes = current_account_hash.into_uint() - initial_account_hash;
    let remaining_account_hashes = U256::MAX - current_account_hash.into_uint();
    // Time to finish = Time since start / synced_account_hashes * remaining_account_hashes
    let time_to_finish_secs = U512::from(Instant::now().duration_since(start_time).as_secs())
        * U512::from(remaining_account_hashes)
        / U512::from(synced_account_hashes);
    info!(
        "Downloading state trie, completion rate: {}%, estimated time to finish: {}",
        completion_rate,
        seconds_to_readable(time_to_finish_secs)
    )
}

fn seconds_to_readable(seconds: U512) -> String {
    let (days, rest) = seconds.div_mod(U512::from(60 * 60 * 24));
    let (hours, rest) = rest.div_mod(U512::from(60 * 60));
    let (minutes, seconds) = rest.div_mod(U512::from(60));
    if days > U512::zero() {
        return format!("Over {days} days");
    }
    format!("{hours}h{minutes}m{seconds}s")
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
