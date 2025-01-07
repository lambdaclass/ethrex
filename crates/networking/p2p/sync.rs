use ethrex_blockchain::error::ChainError;
use ethrex_core::{
    types::{AccountState, Block, BlockBody, BlockHash, BlockHeader, EMPTY_KECCACK_HASH},
    H256,
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
use tracing::{debug, info, warn};

use crate::kademlia::KademliaTable;

/// Maximum amount of times we will ask a peer for an account/storage range
/// If the max amount of retries is exceeded we will asume that the state we are requesting is old and no longer available
const MAX_RETRIES: usize = 10;
/// The minimum amount of blocks from the head that we want to full sync during a snap sync
const MIN_FULL_BLOCKS: usize = 64;

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
    pub async fn start_sync(&mut self, current_head: H256, sync_head: H256, store: Store) {
        info!("Syncing from current head {current_head} to sync_head {sync_head}");
        let start_time = Instant::now();
        match self.sync_cycle(current_head, sync_head, store).await {
            Ok(()) => {
                info!(
                    "Sync finished, time elapsed: {} secs",
                    start_time.elapsed().as_secs()
                );
                // Next sync will be full-sync
                self.sync_mode = SyncMode::Full;
            }
            Err(error) => warn!(
                "Sync failed due to {error}, time elapsed: {} secs ",
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
        let mut all_block_headers = vec![];
        let mut all_block_hashes = vec![];
        loop {
            let peer = self.peers.lock().await.get_peer_channels().await;
            debug!("Requesting Block Headers from {current_head}");
            // Request Block Headers from Peer
            if let Some(block_headers) = peer.request_block_headers(current_head).await {
                debug!("Received {} block headers", block_headers.len());
                let block_hashes = block_headers
                    .iter()
                    .map(|header| header.compute_block_hash())
                    .collect::<Vec<_>>();
                // Keep headers so we can process them later
                // Discard the first header as we already have it
                all_block_headers.extend_from_slice(&block_headers[1..]);
                all_block_hashes.extend_from_slice(&block_hashes[1..]);

                // Check if we already reached our sync head or if we need to fetch more blocks
                if !block_hashes.contains(&sync_head) {
                    // Update the request to fetch the next batch
                    current_head = *block_hashes.last().unwrap();
                } else {
                    // No more headers to request
                    break;
                }
            }
        }
        // We finished fetching all headers, now we can process them
        match self.sync_mode {
            SyncMode::Snap => {
                // snap-sync: launch tasks to fetch blocks and state in parallel
                // - Fetch each block's body and its receipt via eth p2p requests
                // - Fetch the pivot block's state via snap p2p requests
                // - Execute blocks after the pivote (like in full-sync)
                let fetch_bodies_handle = tokio::spawn(fetch_block_bodies(
                    all_block_hashes.clone(),
                    self.peers.clone(),
                ));
                let mut pivot_idx = if all_block_headers.len() > MIN_FULL_BLOCKS {
                    all_block_headers.len() - MIN_FULL_BLOCKS
                } else {
                    all_block_headers.len() - 1
                };
                let mut pivot_root = all_block_headers[pivot_idx].state_root;
                let mut pivot_number = all_block_headers[pivot_idx].number;

                let mut stale_pivot =
                    !rebuild_state_trie(pivot_root, self.peers.clone(), store.clone()).await?;
                // If the pivot became stale, set a further pivot and try again
                if stale_pivot && pivot_idx != all_block_headers.len() - 1 {
                    warn!("Stale pivot, switching to newer head");
                    pivot_idx = all_block_headers.len() - 1;
                    pivot_root = all_block_headers[pivot_idx].state_root;
                    pivot_number = all_block_headers[pivot_idx].number;
                    stale_pivot =
                        !rebuild_state_trie(pivot_root, self.peers.clone(), store.clone()).await?;
                }
                if stale_pivot {
                    warn!("Stale pivot, aborting sync");
                    return Ok(());
                }
                // Wait for all bodies to be downloaded
                let all_block_bodies = fetch_bodies_handle.await??;
                // For all blocks before the pivot: Store the bodies and fetch the receipts
                // For all blocks after the pivot: Process them fully
                let store_receipts_handle = tokio::spawn(store_receipts(
                    all_block_hashes[pivot_idx..].to_vec(),
                    self.peers.clone(),
                    store.clone(),
                ));
                for (hash, (header, body)) in all_block_hashes.into_iter().zip(
                    all_block_headers
                        .into_iter()
                        .zip(all_block_bodies.into_iter()),
                ) {
                    if header.number <= pivot_number {
                        store.set_canonical_block(header.number, hash)?;
                        store.add_block(Block::new(header, body))?;
                    } else {
                        store.set_canonical_block(header.number, hash)?;
                        store.update_latest_block_number(header.number)?;
                        ethrex_blockchain::add_block(&Block::new(header, body), &store)?;
                    }
                }
                store_receipts_handle.await??;
                self.last_snap_pivot = pivot_number;
            }
            SyncMode::Full => {
                // full-sync: Fetch all block bodies and execute them sequentially to build the state
                download_and_run_blocks(
                    all_block_hashes,
                    all_block_headers,
                    self.peers.clone(),
                    store.clone(),
                )
                .await?
            }
        }
        Ok(())
    }
}

/// Requests block bodies from peers via p2p, executes and stores them
/// Returns an error if there was a problem while executing or validating the blocks
async fn download_and_run_blocks(
    mut block_hashes: Vec<BlockHash>,
    mut block_headers: Vec<BlockHeader>,
    peers: Arc<Mutex<KademliaTable>>,
    store: Store,
) -> Result<(), ChainError> {
    loop {
        let peer = peers.lock().await.get_peer_channels().await;
        debug!("Requesting Block Bodies ");
        if let Some(block_bodies) = peer.request_block_bodies(block_hashes.clone()).await {
            let block_bodies_len = block_bodies.len();
            debug!("Received {} Block Bodies", block_bodies_len);
            // Execute and store blocks
            for body in block_bodies.into_iter() {
                // We already validated that there are no more block bodies than the ones requested
                let header = block_headers.remove(0);
                let hash = block_hashes.remove(0);
                let number = header.number;
                let block = Block::new(header, body);
                if let Err(error) = ethrex_blockchain::add_block(&block, &store) {
                    warn!("Failed to add block during FullSync: {error}");
                    return Err(error);
                }
                store.set_canonical_block(number, hash)?;
                store.update_latest_block_number(number)?;
            }
            debug!("Executed & stored {} blocks", block_bodies_len);
            // Check if we need to ask for another batch
            if block_hashes.is_empty() {
                break;
            }
        }
    }
    Ok(())
}

/// Fetches all block bodies for the given block hashes via p2p and returns them
async fn fetch_block_bodies(
    mut block_hashes: Vec<BlockHash>,
    peers: Arc<Mutex<KademliaTable>>,
) -> Result<Vec<BlockBody>, SyncError> {
    let mut all_block_bodies = Vec::new();
    loop {
        let peer = peers.lock().await.get_peer_channels().await;
        debug!("Requesting Block Headers ");
        if let Some(block_bodies) = peer.request_block_bodies(block_hashes.clone()).await {
            debug!(" Received {} Block Bodies", block_bodies.len());
            // Track which bodies we have already fetched
            block_hashes.drain(0..block_bodies.len());
            all_block_bodies.extend(block_bodies);
            // Check if we need to ask for another batch
            if block_hashes.is_empty() {
                break;
            }
        }
    }
    Ok(all_block_bodies)
}

/// Fetches all receipts for the given block hashes via p2p and stores them
async fn store_receipts(
    mut block_hashes: Vec<BlockHash>,
    peers: Arc<Mutex<KademliaTable>>,
    store: Store,
) -> Result<(), SyncError> {
    loop {
        let peer = peers.lock().await.get_peer_channels().await;
        debug!("Requesting Block Headers ");
        if let Some(receipts) = peer.request_receipts(block_hashes.clone()).await {
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
/// Returns true if all state was fetched or false if the block is too old and the state is no longer available
async fn rebuild_state_trie(
    state_root: H256,
    peers: Arc<Mutex<KademliaTable>>,
    store: Store,
) -> Result<bool, SyncError> {
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
    let mut start_account_hash = H256::zero();
    // Start from an empty state trie
    // We cannot keep an open trie here so we will track the root between lookups
    let mut current_state_root = *EMPTY_TRIE_HASH;
    // Fetch Account Ranges
    // If we reached the maximum amount of retries then it means the state we are requesting is probably old and no longer available
    for _ in 0..MAX_RETRIES {
        let peer = peers.clone().lock().await.get_peer_channels().await;
        debug!("Requesting Account Range for state root {state_root}, starting hash: {start_account_hash}");
        if let Some((account_hashes, accounts, should_continue)) = peer
            .request_account_range(state_root, start_account_hash)
            .await
        {
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
        }
    }
    // Send empty batch to signal that no more batches are incoming
    storage_sender.send(vec![]).await?;
    storage_fetcher_handle.await??;
    let sync_complete = if current_state_root == state_root {
        debug!("Completed state sync for state root {state_root}");
        true
    } else {
        // Perform state healing to fix any potential inconsistency in the rebuilt tries
        // As we are not fetching different chunks of the same trie this step is not necessary
        heal_state_trie(bytecode_sender.clone(), state_root, store, peers).await?
    };
    // Send empty batch to signal that no more batches are incoming
    bytecode_sender.send(vec![]).await?;
    bytecode_fetcher_handle.await??;
    Ok(sync_complete)
}

/// Waits for incoming code hashes from the receiver channel endpoint, queues them, and fetches and stores their bytecodes in batches
async fn bytecode_fetcher(
    mut receiver: Receiver<Vec<H256>>,
    peers: Arc<Mutex<KademliaTable>>,
    store: Store,
) -> Result<(), SyncError> {
    const BATCH_SIZE: usize = 200;
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
    peers: Arc<Mutex<KademliaTable>>,
    store: Store,
) -> Result<Vec<H256>, StoreError> {
    loop {
        let peer = peers.lock().await.get_peer_channels().await;
        if let Some(bytecodes) = peer.request_bytecodes(batch.clone()).await {
            debug!("Received {} bytecodes", bytecodes.len());
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
async fn storage_fetcher(
    mut receiver: Receiver<Vec<(H256, H256)>>,
    peers: Arc<Mutex<KademliaTable>>,
    store: Store,
    state_root: H256,
) -> Result<(), StoreError> {
    const BATCH_SIZE: usize = 100;
    // Pending list of storages to fetch
    let mut pending_storage: Vec<(H256, H256)> = vec![];
    // TODO: Also add a queue for storages that were incompletely fecthed,
    // but for the first iteration we will asume not fully fetched -> fetch again
    let mut incoming = true;
    while incoming {
        // Fetch incoming requests
        match receiver.recv().await {
            Some(account_hashes_and_roots) if !account_hashes_and_roots.is_empty() => {
                pending_storage.extend(account_hashes_and_roots);
            }
            // Disconnect / Empty message signaling no more bytecodes to sync
            _ => incoming = false,
        }
        // If we have enough pending bytecodes to fill a batch
        // or if we have no more incoming batches, spawn a fetch process
        while pending_storage.len() >= BATCH_SIZE || !incoming && !pending_storage.is_empty() {
            let next_batch = pending_storage
                .drain(..BATCH_SIZE.min(pending_storage.len()))
                .collect::<Vec<_>>();
            let remaining =
                fetch_storage_batch(next_batch, state_root, peers.clone(), store.clone()).await?;
            // Add unfeched bytecodes back to the queue
            pending_storage.extend(remaining);
        }
    }
    Ok(())
}

/// Receives a batch of account hashes with their storage roots, fetches their respective storage ranges via p2p and returns a list of the code hashes that couldn't be fetched in the request (if applicable)
async fn fetch_storage_batch(
    mut batch: Vec<(H256, H256)>,
    state_root: H256,
    peers: Arc<Mutex<KademliaTable>>,
    store: Store,
) -> Result<Vec<(H256, H256)>, StoreError> {
    for _ in 0..MAX_RETRIES {
        let peer = peers.lock().await.get_peer_channels().await;
        let (batch_hahses, batch_roots) = batch.clone().into_iter().unzip();
        if let Some((mut keys, mut values, incomplete)) = peer
            .request_storage_ranges(state_root, batch_roots, batch_hahses, H256::zero())
            .await
        {
            debug!("Received {} storage ranges", keys.len());
            let mut _last_range;
            // Hold on to the last batch (if incomplete)
            if incomplete {
                // An incomplete range cannot be empty
                _last_range = (keys.pop().unwrap(), values.pop().unwrap());
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
            // TODO: if the last range is incomplete add it to the incomplete batches queue
            // For now we will fetch the full range again
            // Return remaining code hashes in the batch if we couldn't fetch all of them
            return Ok(batch);
        }
    }
    // This is a corner case where we fetched an account range for a block but the chain has moved on and the block
    // was dropped by the peer's snapshot. We will keep the fetcher alive to avoid errors and stop fetching as from the next account
    Ok(vec![])
}

/// Heals the trie given its state_root by fetching any missing nodes in it via p2p
async fn heal_state_trie(
    bytecode_sender: Sender<Vec<H256>>,
    state_root: H256,
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
    // Begin by requesting the root node
    let mut paths = vec![Nibbles::default()];
    // Count the number of request retries so we don't get stuck requesting old state
    let mut retry_count = 0;
    while !paths.is_empty() && retry_count < MAX_RETRIES {
        let peer = peers.lock().await.get_peer_channels().await;
        if let Some(nodes) = peer
            .request_state_trienodes(state_root, paths.clone())
            .await
        {
            // Reset retry counter for next request
            retry_count = 0;
            let mut hahsed_addresses = vec![];
            let mut code_hashes = vec![];
            // For each fetched node:
            // - Add its children to the queue (if we don't have them already)
            // - If it is a leaf, request its bytecode & storage
            // - Add it to the trie's state
            for node in nodes {
                let path = paths.remove(0);
                // We cannot keep the trie state open
                let mut trie = store.open_state_trie(*EMPTY_TRIE_HASH);
                let trie_state = trie.state_mut();
                paths.extend(node_missing_children(&node, &path, trie_state)?);
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
                }
                let hash = node.compute_hash();
                trie_state.write_node(node, hash)?;
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
    // Send empty batch to signal that no more batches are incoming
    storage_sender.send(vec![]).await?;
    storage_healer_handler.await??;
    Ok(retry_count < MAX_RETRIES)
}

/// Waits for incoming hashed addresses from the receiver channel endpoint and queues the associated root nodes for state retrieval
/// Also retrieves their children nodes until we have the full storage trie stored
async fn storage_healer(
    state_root: H256,
    mut receiver: Receiver<Vec<H256>>,
    peers: Arc<Mutex<KademliaTable>>,
    store: Store,
) -> Result<(), SyncError> {
    const BATCH_SIZE: usize = 200;
    // Pending list of bytecodes to fetch
    let mut pending_storages: Vec<(H256, Nibbles)> = vec![];
    let mut incoming = true;
    while incoming {
        // Fetch incoming requests
        match receiver.recv().await {
            Some(account_paths) if !account_paths.is_empty() => {
                // Add the root paths of each account trie to the queue
                pending_storages.extend(
                    account_paths
                        .into_iter()
                        .map(|acc_path| (acc_path, Nibbles::default())),
                );
            }
            // Disconnect / Empty message signaling no more bytecodes to sync
            _ => incoming = false,
        }
        // If we have enough pending storages to fill a batch
        // or if we have no more incoming batches, spawn a fetch process
        while pending_storages.len() >= BATCH_SIZE || !incoming && !pending_storages.is_empty() {
            let mut next_batch: BTreeMap<H256, Vec<Nibbles>> = BTreeMap::new();
            // Group pending storages by account path
            // We do this here instead of keeping them sorted so we don't prioritize further nodes from the first tries
            for (account, path) in pending_storages.drain(..BATCH_SIZE.min(pending_storages.len()))
            {
                next_batch.entry(account).or_default().push(path);
            }
            let return_batch =
                heal_storage_batch(state_root, next_batch, peers.clone(), store.clone()).await?;
            for (acc_path, paths) in return_batch {
                for path in paths {
                    pending_storages.push((acc_path, path));
                }
            }
        }
    }
    Ok(())
}

/// Receives a set of storage trie paths (grouped by their corresponding account's state trie path),
/// fetches their respective nodes, stores them, and returns their children paths and the paths that couldn't be fetched so they can be returned to the queue
async fn heal_storage_batch(
    state_root: H256,
    mut batch: BTreeMap<H256, Vec<Nibbles>>,
    peers: Arc<Mutex<KademliaTable>>,
    store: Store,
) -> Result<BTreeMap<H256, Vec<Nibbles>>, SyncError> {
    for _ in 0..MAX_RETRIES {
        let peer = peers.lock().await.get_peer_channels().await;
        if let Some(mut nodes) = peer
            .request_storage_trienodes(state_root, batch.clone())
            .await
        {
            debug!("Received {} nodes", nodes.len());
            // Process the nodes for each account path
            for (acc_path, paths) in batch.iter_mut() {
                let mut trie = store.open_storage_trie(*acc_path, *EMPTY_TRIE_HASH);
                let trie_state = trie.state_mut();
                // Get the corresponding nodes
                for node in nodes.drain(..paths.len().min(nodes.len())) {
                    let path = paths.remove(0);
                    // Add children to batch
                    let children = node_missing_children(&node, &path, trie_state)?;
                    paths.extend(children);
                    // Add node to the state
                    let hash = node.compute_hash();
                    trie_state.write_node(node, hash)?;
                }
                // Cut the loop if we ran out of nodes
                if nodes.is_empty() {
                    break;
                }
            }
            // Return remaining and added paths to be added to the queue
            return Ok(batch);
        }
    }
    // This is a corner case where we fetched an account range for a block but the chain has moved on and the block
    // was dropped by the peer's snapshot. We will keep the fetcher alive to avoid errors and stop fetching as from the next account
    Ok(BTreeMap::new())
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
}
