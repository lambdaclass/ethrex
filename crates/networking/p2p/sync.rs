use ethrex_blockchain::error::ChainError;
use ethrex_common::{
    types::{AccountState, Block, BlockHash, EMPTY_KECCACK_HASH},
    BigEndianHash, H256, U256, U512,
};
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode, error::RLPDecodeError};
use ethrex_storage::{error::StoreError, Store};
use ethrex_trie::{Nibbles, Node, TrieError, TrieState, EMPTY_TRIE_HASH};
use std::{cmp::min, collections::BTreeMap, sync::Arc};
use tokio::{
    sync::{
        mpsc::{self, error::SendError, Receiver, Sender},
        Mutex,
    },
    time::Instant,
};
use tracing::{debug, error, info, warn};

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
const MAX_PARALLEL_FETCHES: usize = 5;

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
    block_hashes: Vec<BlockHash>,
}

impl SyncManager {
    pub fn new(peer_table: Arc<Mutex<KademliaTable>>, sync_mode: SyncMode) -> Self {
        Self {
            sync_mode,
            peers: PeerHandler::new(peer_table),
            last_snap_pivot: 0,
            block_hashes: vec![],
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
            block_hashes: vec![],
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
        // Check if we have some blocks downloaded from a previous attempt
        if let Some(last_hash) = self.block_hashes.last() {
            // Set latest downloaded header as current head for header fetching
            current_head = *last_hash;
            //TODO check that the last hash is > current_head
        } else if let Some(last_hash) = store.get_header_download_checkpoint()? {
            current_head = last_hash;
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
                        "Received {} block headers| First Number: {} Last Number: {}",
                        block_headers.len(),
                        block_headers.first().as_ref().unwrap().number,
                        block_headers.last().as_ref().unwrap().number
                    );
                    let mut block_hashes = block_headers
                        .iter()
                        .map(|header| header.compute_block_hash())
                        .collect::<Vec<_>>();
                    // Check if we already found the sync head
                    let sync_head_found = block_hashes.contains(&sync_head);
                    // Update current fetch head if needed
                    let last_block_hash = *block_hashes.last().unwrap();
                    if !sync_head_found {
                        debug!(
                            "Syncing head not found, updated current_head {:?}",
                            last_block_hash
                        );
                        current_head = last_block_hash;
                    }
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
                    // Discard the first header as we already have it
                    block_hashes.remove(0);
                    block_headers.remove(0);
                    // Store headers and save hashes for full block retrieval
                    self.block_hashes.extend_from_slice(&block_hashes[..]);
                    store.add_block_headers(block_hashes.clone(), block_headers)?;

                    if sync_head_found {
                        // No more headers to request
                        break;
                    } else {
                        download_and_run_blocks(
                            &mut self.block_hashes,
                            self.peers.clone(),
                            store.clone(),
                        )
                        .await?;
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
                let pivot_idx = self.block_hashes.len().saturating_sub(MIN_FULL_BLOCKS);
                let pivot_header = store
                    .get_block_header_by_hash(self.block_hashes[pivot_idx])?
                    .ok_or(SyncError::CorruptDB)?;
                debug!(
                    "Selected block {} as pivot for snap sync",
                    pivot_header.number
                );
                let store_bodies_handle = tokio::spawn(store_block_bodies(
                    self.block_hashes[pivot_idx + 1..].to_vec(),
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
                // For all blocks before the pivot: Store the bodies and fetch the receipts (TODO)
                // For all blocks after the pivot: Process them fully
                for hash in &self.block_hashes[pivot_idx + 1..] {
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
                info!("Full syncing finished request block headers, about to start fetching bodies, running and storing them");
                // full-sync: Fetch all block bodies and execute them sequentially to build the state
                let res = download_and_run_blocks(
                    &mut self.block_hashes,
                    self.peers.clone(),
                    store.clone(),
                )
                .await;
                if let Err(e) = res {
                    error!("Failed to download and run blocks, err {:?}", e);
                } else {
                    info!("Full syncing finished executing!");
                }
            }
        }
        Ok(())
    }
}

/// Requests block bodies from peers via p2p, executes and stores them
/// Returns an error if there was a problem while executing or validating the blocks
async fn download_and_run_blocks(
    block_hashes: &mut Vec<BlockHash>,
    peers: PeerHandler,
    store: Store,
) -> Result<(), SyncError> {
    // ask as much as 128 block bodies per req
    // this magic number is not part of the protocol and it is taken from geth, see:
    // https://github.com/ethereum/go-ethereum/blob/master/eth/downloader/downloader.go#L42
    let max_req_len = 64;

    let mut current_chunk_idx = 0;
    let chunks: Vec<Vec<BlockHash>> = block_hashes
        .chunks(max_req_len)
        .map(|chunk| chunk.to_vec())
        .collect();

    let mut chunk = match chunks.get(current_chunk_idx) {
        Some(res) => res.clone(),
        None => return Ok(()),
    };

    let mut current_block_hash_idx = 0;
    loop {
        debug!("Requesting Block Bodies number");
        if let Some(block_bodies) = peers.request_block_bodies(chunk.clone()).await {
            let block_bodies_len = block_bodies.len();

            let first_block_hash = chunk.first().map_or(H256::default(), |a| *a);
            let first_block_header = store
                .get_block_header_by_hash(first_block_hash)?
                .map_or(0, |h| h.number);

            debug!(
                "Received {} Block Bodies, starting from block hash {:?} with number: {}",
                block_bodies_len, first_block_hash, first_block_header
            );
            // Execute and store blocks
            let mut i = 0;
            for (hash, body) in chunk
                .drain(..block_bodies_len)
                .zip(block_bodies.into_iter())
            {
                debug!(
                    "About to add block with hash {} and number {}",
                    hash,
                    first_block_header + i
                );
                block_hashes.remove(current_block_hash_idx);
                current_block_hash_idx += 1;
                i += 1;
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
                store.set_header_download_checkpoint(hash)?;
                debug!(
                    "Executed and stored block number {} with hash {}",
                    number, hash
                );
            }
            debug!("Executed & stored {} blocks", block_bodies_len);
            if chunk.len() == 0 {
                current_chunk_idx += 1;
                chunk = match chunks.get(current_chunk_idx) {
                    Some(res) => res.clone(),
                    None => return Ok(()),
                };
            };
        }
    }
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

/// Rebuilds a Block's state trie by requesting snap state from peers, also performs state healing
/// Receives an optional checkpoint in case there was a previous snap sync process that became stale, in which
/// case it will continue from the checkpoint and then apply healing to fix inconsistencies with the older state
/// Returns true if all state was fetched or false if the block is too old and the state is no longer available
async fn rebuild_state_trie(
    state_root: H256,
    peers: PeerHandler,
    store: Store,
) -> Result<bool, SyncError> {
    // Spawn storage & bytecode fetchers
    let (bytecode_sender, bytecode_receiver) = mpsc::channel::<Vec<H256>>(500);
    let bytecode_fetcher_handle = tokio::spawn(bytecode_fetcher(
        bytecode_receiver,
        peers.clone(),
        store.clone(),
    ));
    // Resume download from checkpoint if available or start from an empty trie
    // We cannot keep an open trie here so we will track the root between lookups
    let mut current_state_root = store
        .get_state_trie_root_checkpoint()?
        .unwrap_or(*EMPTY_TRIE_HASH);
    let mut start_account_hash = store.get_state_trie_key_checkpoint()?.unwrap_or_default();
    // Skip state sync if we are already on healing
    if start_account_hash != HASH_MAX {
        let (storage_sender, storage_receiver) = mpsc::channel::<Vec<(H256, H256)>>(500);
        let storage_fetcher_handle = tokio::spawn(storage_fetcher(
            storage_receiver,
            peers.clone(),
            store.clone(),
            state_root,
        ));
        debug!("Starting/Resuming state trie download from key {start_account_hash}");
        // Fetch Account Ranges
        // If we reached the maximum amount of retries then it means the state we are requesting is probably old and no longer available
        let mut progress_timer = Instant::now();
        let initial_timestamp = Instant::now();
        let initial_account_hash = start_account_hash.into_uint();
        let mut stale = false;
        const PROGRESS_OUTPUT_TIMER: std::time::Duration = std::time::Duration::from_secs(30);
        loop {
            // Show Progress stats (this task is not vital so we can detach it)
            if Instant::now().duration_since(progress_timer) >= PROGRESS_OUTPUT_TIMER {
                progress_timer = Instant::now();
                tokio::spawn(show_progress(
                    start_account_hash,
                    initial_account_hash,
                    initial_timestamp,
                ));
            }
            debug!("Requesting Account Range for state root {state_root}, starting hash: {start_account_hash}");
            if let Some((account_hashes, accounts, should_continue)) = peers
                .request_account_range(state_root, start_account_hash)
                .await
            {
                debug!("Received {} account ranges", accounts.len());
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
                        account_hashes_and_storage_roots
                            .push((*account_hash, account.storage_root));
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
                stale = true;
                break;
            }
        }
        // Store current checkpoint
        store.set_state_trie_root_checkpoint(current_state_root)?;
        if stale {
            store.set_state_trie_key_checkpoint(start_account_hash)?;
        } else {
            // Set highest key value so we know state sync is already complete on the next cycle
            store.set_state_trie_key_checkpoint(HASH_MAX)?;
        }
        debug!("Account Trie Fetching ended, signaling storage fetcher process");
        // Send empty batch to signal that no more batches are incoming
        storage_sender.send(vec![]).await?;
        let pending_storage_accounts = storage_fetcher_handle.await??;
        let pending_storages = !pending_storage_accounts.is_empty();
        // Next cycle may have different storage roots for these accounts so we will leave them to healing
        if pending_storages {
            let mut stored_pending_storages = store.get_storage_heal_paths()?.unwrap_or_default();
            stored_pending_storages.extend(
                pending_storage_accounts
                    .iter()
                    .map(|k| (*k, vec![Nibbles::default()])),
            );
            debug!(
                "Current pending storage accounts: {}",
                stored_pending_storages.len()
            );
            store.set_storage_heal_paths(stored_pending_storages)?;
        }
        if stale || pending_storages {
            // Skip healing and return stale status
            return Ok(false);
        }
        info!("Healing Start")
    } else {
        info!("Resuming healing")
    }
    // Perform state healing to fix inconsistencies with older state
    let res = heal_state_trie(
        bytecode_sender.clone(),
        state_root,
        store.clone(),
        peers.clone(),
    )
    .await?;
    // Send empty batch to signal that no more batches are incoming
    debug!("Account Trie healing ended signaling bytecode fetcher process");
    bytecode_sender.send(vec![]).await?;
    bytecode_fetcher_handle.await??;
    Ok(res)
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
/// In the last case, the fetcher will return the account hashes of the accounts in the queue
async fn storage_fetcher(
    mut receiver: Receiver<Vec<(H256, H256)>>,
    peers: PeerHandler,
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
        let mut msg_buffer = vec![];
        if receiver.recv_many(&mut msg_buffer, 25).await != 0 {
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
                    next_batch.clone(),
                    state_root,
                    peers.clone(),
                    store.clone(),
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
    Ok(pending_storage.into_iter().map(|(acc, _)| acc).collect())
}

/// Receives a batch of account hashes with their storage roots, fetches their respective storage ranges via p2p and returns a list of the code hashes that couldn't be fetched in the request (if applicable)
/// Also returns a boolean indicating if the pivot became stale during the request
async fn fetch_storage_batch(
    mut batch: Vec<(H256, H256)>,
    state_root: H256,
    peers: PeerHandler,
    store: Store,
) -> Result<(Vec<(H256, H256)>, bool), SyncError> {
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
    while should_continue {
        debug!("Fetching large storage trie, current key: {}", next_key);

        if let Some((keys, values, incomplete)) = peers
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
        } else {
            return Ok(true);
        }
    }
    if current_root != storage_root {
        warn!("State sync failed for storage root {storage_root}");
    }
    Ok(false)
}

/// Heals the trie given its state_root by fetching any missing nodes in it via p2p
/// Doesn't store nodes, only leaf values to avoid inconsistent tries on restarts
async fn heal_state_trie(
    bytecode_sender: Sender<Vec<H256>>,
    state_root: H256,
    store: Store,
    peers: PeerHandler,
) -> Result<bool, SyncError> {
    // Check if we have pending storages to heal from a previous cycle
    let pending: BTreeMap<H256, Vec<Nibbles>> = store
        .get_storage_heal_paths()?
        .unwrap_or_default()
        .into_iter()
        .collect();
    // Spawn a storage healer for this blocks's storage
    let (storage_sender, storage_receiver) = mpsc::channel::<Vec<H256>>(500);
    let storage_healer_handler = tokio::spawn(storage_healer(
        state_root,
        pending,
        storage_receiver,
        peers.clone(),
        store.clone(),
    ));
    // Check if we have pending paths from a previous cycle
    let mut paths = store.get_state_heal_paths()?.unwrap_or_default();
    // Begin by requesting the root node
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
                storage_sender.clone(),
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
    storage_sender.send(vec![]).await?;
    let storage_heal_paths = storage_healer_handler.await??;
    // Update pending list
    // If a storage trie was left mid-healing we will heal it again
    let storage_healing_succesful = storage_heal_paths.is_empty();
    if !storage_healing_succesful {
        debug!("{} storages with pending healing", storage_heal_paths.len());
        store.set_storage_heal_paths(storage_heal_paths.into_iter().collect())?;
    }
    Ok(paths.is_empty() && storage_healing_succesful)
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
        info!("Received {} state nodes", nodes.len());
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
/// Receives the prending storages from a previous iteration
async fn storage_healer(
    state_root: H256,
    mut pending_paths: BTreeMap<H256, Vec<Nibbles>>,
    mut receiver: Receiver<Vec<H256>>,
    peers: PeerHandler,
    store: Store,
) -> Result<BTreeMap<H256, Vec<Nibbles>>, SyncError> {
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
            if receiver.recv_many(&mut msg_buffer, 25).await != 0 {
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
    Ok(pending_paths)
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

/// Shows the completion rate & estimated remaining time of the state sync phase of snap sync
/// Does not take into account healing
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
