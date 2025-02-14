//! This module contains the logic for the ongoing trie rebuild process
//! This process consists of two parallel processes: state trie rebuild & storage trie rebuild
//! State trie rebuild works on its own by processing accounts from the state snaphot as soon as they become available
//! Storage trie rebuild works passively, waiting for the storage fetcher to advertise fully downloaded storages before processing them from the storage snapshot
//! Both processes become active once a snap sync begins and only end once they finish (with state sync also being finished as a condition) or when the node is shut down (via Ctrl+C signal)
//! In the later case, this process will be resumed on the next sync cycle

use ethrex_common::{BigEndianHash, H256, U256, U512};
use ethrex_rlp::encode::RLPEncode;
use ethrex_storage::{Store, MAX_SNAPSHOT_READS, STATE_TRIE_SEGMENTS};
use ethrex_trie::EMPTY_TRIE_HASH;
use std::array;
use tokio::{
    sync::mpsc::{channel, Receiver, Sender},
    task::JoinSet,
    time::Instant,
};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::sync::seconds_to_readable;

use super::{
    SyncError, MAX_CHANNEL_MESSAGES, MAX_CHANNEL_READS, MAX_PARALLEL_FETCHES,
    SHOW_PROGRESS_INTERVAL_DURATION, STATE_TRIE_SEGMENTS_END, STATE_TRIE_SEGMENTS_START,
};

/// Represents the permanently ongoing background trie rebuild process
/// This process will be started whenever a state sync is initiated and will be
/// kept alive throughout sync cycles, only stopping once the tries are fully rebuilt or the node is stopped
#[derive(Debug)]
pub(crate) struct TrieRebuilder {
    state_trie_rebuilder: tokio::task::JoinHandle<Result<(), SyncError>>,
    storage_trie_rebuilder: tokio::task::JoinHandle<Result<(), SyncError>>,
    pub(crate) storage_rebuilder_sender: Sender<Vec<(H256, H256)>>,
}

impl TrieRebuilder {
    /// Returns true is the trie rebuild porcess is alive and well
    pub fn alive(&self) -> bool {
        !(self.state_trie_rebuilder.is_finished()
            || self.storage_trie_rebuilder.is_finished()
            || self.storage_rebuilder_sender.is_closed())
    }
    /// Waits for the rebuild process to complete and returns the resulting mismatched accounts
    pub async fn complete(self) -> Result<(), SyncError> {
        // Signal storage rebuilder to finish
        self.storage_rebuilder_sender.send(vec![]).await?;
        self.state_trie_rebuilder.await??;
        self.storage_trie_rebuilder.await?
    }

    /// starts the background trie rebuild process
    pub fn startup(cancel_token: CancellationToken, store: Store) -> Self {
        let (storage_rebuilder_sender, storage_rebuilder_receiver) =
            channel::<Vec<(H256, H256)>>(MAX_CHANNEL_MESSAGES);
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

/// Tracks the status of the state trie rebuild process for a single segment
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

/// Returns hash + 1
fn next_hash(hash: H256) -> H256 {
    H256::from_uint(&(hash.into_uint() + 1))
}

/// Shows the completion rate and estimated finish time of the state trie rebuild
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
