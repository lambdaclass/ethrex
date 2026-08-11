//! The snap/2 range download.
//!
//! From devp2p `caps/snap.md`, "Synchronization algorithm":
//!
//! > Snapshot download: ranges of state values are requested in key-order. The
//! > download starts at state root `R₀` of the initial pivot block and all
//! > responses are verified against `R₀`. As the pivot block advances, the
//! > current root is updated to `R₁`, ... `Rₙ` from the pivot. The state
//! > iteration does not restart when the pivot moves, i.e. it always advances
//! > the key until the end of state is reached. Contract storage and code is
//! > fetched concurrently with accounts.
//!
//! Fetching storage concurrently with accounts is what keeps this honest. A
//! storage range proof verifies against the account's storage root, so the
//! account leaf carrying that root has to come from the same pivot the storage
//! is requested at. snap/1 downloads every account first and every storage
//! afterwards, and closes the gap by healing the account trie back to the
//! current pivot before each storage pass — a repair snap/2 cannot make,
//! because it removes `GetTrieNodes`.
//!
//! So an account range advances only once its accounts *and* their storage are
//! in. Work still inside a pending range is dropped and re-requested when the
//! pivot moves, which is why the frontier the access-list gate reads never
//! names a leaf whose storage was verified against a different root.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use ethrex_common::{
    H256, U256,
    constants::{EMPTY_KECCAK_HASH, EMPTY_TRIE_HASH},
    types::{AccountState, BlockHeader},
};
use ethrex_storage::Store;
use tracing::{debug, info, warn};

use crate::{
    peer_handler::PeerHandler,
    peer_table::PeerTableServerProtocol as _,
    rlpx::p2p::SUPPORTED_SNAP_CAPABILITIES,
    snap::{
        async_fs,
        constants::{HASH_MAX, RANGE_FILE_CHUNK_SIZE, STORAGE_BATCH_SIZE},
    },
    sync::{
        SnapBlockSyncState, SyncDiagnostics, SyncError, block_is_stale,
        code_collector::CodeHashCollector,
        snap2::{
            DownloadCursor, FlatState, HashRange, catch_up, catch_up_exceeds_retention,
            worker::{
                AccountRangeOutcome, StorageRangeOutcome, request_account_range,
                request_storage_ranges,
            },
        },
        update_pivot,
    },
    utils::{
        AccountsWithStorage, dump_accounts_to_file, dump_storages_to_file,
        get_account_state_snapshot_file, get_account_state_snapshots_dir,
        get_account_storages_snapshot_file, get_account_storages_snapshots_dir,
    },
};

/// How long the download may go without its frontier moving before it is
/// treated as wedged rather than slow. Generous, because a thin or busy peer
/// set can go quiet for a while without anything being wrong.
///
/// This is measured against the frontier, not against request activity. A
/// download that keeps handing work to a peer which never answers usefully is
/// busy by every local measure and still getting nowhere, so "did we dispatch
/// anything" cannot tell the two apart.
const STALL_TIMEOUT: Duration = Duration::from_secs(300);

/// A contract whose storage the download still owes, and the root to verify it
/// against. The root holds only at the pivot whose response carried the account
/// leaf, so it is dropped and re-read when the pivot moves.
#[derive(Debug, Clone, Copy)]
struct StorageWork {
    root: H256,
    next_slot: H256,
}

/// One account-hash range being served, with the storage its accounts raised.
///
/// The unit of progress is one account response and the storage it raises, not
/// the whole range. `next` moves only when such a batch is complete, and that
/// is also when the cursor is told, so the two never disagree: an account below
/// `next` has been served together with its storage, and an account above it
/// has not been served at all. Anything in between would be a leaf the
/// catch-up skips as unfetched and the download never re-serves, which is a
/// silently wrong state root.
///
/// A range therefore has at most one batch outstanding. Concurrency comes from
/// running every range at once, not from pipelining within one.
struct RangeTask {
    next: H256,
    last: H256,
    accounts_done: bool,
    accounts_inflight: bool,
    /// The highest account hash of the batch in flight, once its accounts have
    /// landed. `next` moves here when the batch's storage is all in.
    served_through: Option<H256>,
    awaiting_storage: BTreeMap<H256, StorageWork>,
    storage_inflight: usize,
}

impl RangeTask {
    fn new(range: HashRange) -> Self {
        Self {
            next: range.next,
            last: range.last,
            accounts_done: false,
            accounts_inflight: false,
            served_through: None,
            awaiting_storage: BTreeMap::new(),
            storage_inflight: 0,
        }
    }

    /// Whether a batch's accounts and storage are all in, so the frontier can
    /// move to `served_through`.
    fn batch_ready(&self) -> bool {
        self.served_through.is_some()
            && self.awaiting_storage.is_empty()
            && self.storage_inflight == 0
    }

    /// Whether everything this range owns is downloaded.
    fn is_complete(&self) -> bool {
        self.accounts_done && self.served_through.is_none()
    }

    /// Whether anything is still out with a peer.
    fn is_inflight(&self) -> bool {
        self.accounts_inflight || self.storage_inflight > 0
    }
}

/// A worker's answer, tagged with the range that raised it.
enum Response {
    Accounts {
        range: RangeId,
        outcome: Box<AccountRangeOutcome>,
    },
    Storage {
        range: RangeId,
        requested: Vec<(H256, StorageWork)>,
        outcome: Box<StorageRangeOutcome>,
    },
}

/// Identifies a range for the lifetime of the download. Ranges are dropped as
/// they finish, so a position in the collection would name a different range
/// once an earlier one is gone, and a response still in flight would be folded
/// into the wrong frontier.
type RangeId = usize;

/// Buffers leaves and spills them to the sorted chunk files the flat state
/// absorbs in bulk. Writing each response straight through would turn the
/// download into one point write per leaf.
struct ChunkWriter {
    accounts: Vec<(H256, AccountState)>,
    storages: Vec<AccountsWithStorage>,
    buffered_slots: usize,
    accounts_dir: PathBuf,
    storages_dir: PathBuf,
    next_chunk: u64,
}

impl ChunkWriter {
    async fn new(accounts_dir: PathBuf, storages_dir: PathBuf) -> Result<Self, SyncError> {
        async_fs::ensure_dir_exists(&accounts_dir).await?;
        async_fs::ensure_dir_exists(&storages_dir).await?;
        Ok(Self {
            accounts: Vec::new(),
            storages: Vec::new(),
            buffered_slots: 0,
            accounts_dir,
            storages_dir,
            next_chunk: 0,
        })
    }

    fn push_storages(&mut self, account_hash: H256, slots: Vec<(H256, U256)>) {
        if slots.is_empty() {
            return;
        }
        self.buffered_slots += slots.len();
        self.storages.push(AccountsWithStorage {
            accounts: vec![account_hash],
            storages: slots,
        });
    }

    fn is_full(&self) -> bool {
        self.accounts.len() * size_of::<AccountState>() >= RANGE_FILE_CHUNK_SIZE
            || self.buffered_slots * size_of::<(H256, U256)>() >= RANGE_FILE_CHUNK_SIZE
    }

    fn flush(&mut self) -> Result<(), SyncError> {
        if self.accounts.is_empty() && self.storages.is_empty() {
            return Ok(());
        }
        if !self.accounts.is_empty() {
            let path = get_account_state_snapshot_file(&self.accounts_dir, self.next_chunk);
            dump_accounts_to_file(&path, std::mem::take(&mut self.accounts))
                .map_err(|err| SyncError::FileSystem(format!("{:?}: {:?}", err.path, err.error)))?;
        }
        if !self.storages.is_empty() {
            let path = get_account_storages_snapshot_file(&self.storages_dir, self.next_chunk);
            dump_storages_to_file(&path, std::mem::take(&mut self.storages))
                .map_err(|err| SyncError::FileSystem(format!("{:?}: {:?}", err.path, err.error)))?;
            self.buffered_slots = 0;
        }
        self.next_chunk += 1;
        Ok(())
    }
}

/// Download the whole state, keeping the flat state consistent with the pivot
/// as it moves.
///
/// Returns the pivot the flat state ends up describing. That is the header the
/// reconstruction must verify against, and the pivot is frozen from the moment
/// this returns.
#[allow(clippy::too_many_arguments)]
pub async fn download_state(
    peers: &mut PeerHandler,
    store: &Store,
    flat: &FlatState,
    cursor: &mut DownloadCursor,
    mut pivot: BlockHeader,
    block_sync_state: &mut SnapBlockSyncState,
    code_hash_collector: &mut CodeHashCollector,
    datadir: &Path,
    diagnostics: &Arc<tokio::sync::RwLock<SyncDiagnostics>>,
) -> Result<BlockHeader, SyncError> {
    let accounts_dir = get_account_state_snapshots_dir(datadir);
    let storages_dir = get_account_storages_snapshots_dir(datadir);
    let mut writer = ChunkWriter::new(accounts_dir.clone(), storages_dir.clone()).await?;

    // The frontier's last observed position and when it last moved, to tell a
    // slow download apart from a wedged one.
    let mut last_remaining = usize::MAX;
    let mut progress_at = SystemTime::now();
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Response>(1024);
    let mut tasks: BTreeMap<RangeId, RangeTask> = cursor
        .pending_account_ranges()
        .iter()
        .copied()
        .enumerate()
        .map(|(id, range)| (id, RangeTask::new(range)))
        .collect();

    while !tasks.is_empty() {
        // The pivot has aged out of what peers will serve. Roll the flat state
        // forward before any further range request, so every later response
        // lands on a frontier the access lists have already covered.
        if block_is_stale(&pivot) {
            collect_inflight(
                &mut tasks,
                peers,
                &mut rx,
                &mut writer,
                cursor,
                code_hash_collector,
                diagnostics,
            )
            .await?;
            publish(&mut writer, flat, &accounts_dir, &storages_dir).await?;

            let next = update_pivot(
                pivot.number,
                pivot.timestamp,
                peers,
                block_sync_state,
                diagnostics,
            )
            .await?;
            if catch_up_exceeds_retention(&pivot, &next) {
                // caps/snap.md: past the retention window the access lists the
                // catch-up needs are gone, and the node "must discard partial
                // state and restart synchronization".
                warn!(
                    from = pivot.number,
                    to = next.number,
                    "snap/2 catch-up gap exceeds BAL retention; discarding partial state"
                );
                return Err(SyncError::Snap2CatchUpStalled(
                    pivot.number,
                    "gap exceeds BAL retention".to_string(),
                ));
            }
            catch_up(store, peers, flat, cursor, &pivot, &next, diagnostics).await?;
            pivot = next;

            // The roots the outstanding storage would verify against belong to
            // the old pivot, and the batch they came from is abandoned: its
            // accounts sit below no frontier, so the catch-up just skipped
            // them and only re-serving at the new pivot can make them right.
            // `next` is untouched, so the work lost is one batch per range.
            for task in tasks.values_mut() {
                task.awaiting_storage.clear();
                task.served_through = None;
                task.accounts_done = false;
            }
            continue;
        }

        let mut progressed = false;
        while let Ok(response) = rx.try_recv() {
            handle_response(
                response,
                peers,
                &mut tasks,
                &mut writer,
                cursor,
                code_hash_collector,
                diagnostics,
            )
            .await?;
            progressed = true;
        }

        if writer.is_full() {
            writer.flush()?;
        }

        // A frontier only moves once the leaves behind it are readable, or an
        // access list gated on it would patch keys the flat state does not hold
        // yet.
        if tasks.values().any(RangeTask::batch_ready) {
            publish(&mut writer, flat, &accounts_dir, &storages_dir).await?;
            for task in tasks.values_mut() {
                if !task.batch_ready() {
                    continue;
                }
                let Some(served_through) = task.served_through.take() else {
                    continue;
                };
                cursor.advance_accounts(served_through);
                // Reaching the range's end finishes it. Without this the range
                // would keep requesting a span that starts past its own last
                // hash and never complete.
                task.accounts_done = served_through >= task.last;
                match next_hash(served_through) {
                    Some(next) => task.next = next,
                    // Served through the top of the whole hash space.
                    None => task.accounts_done = true,
                }
            }
            tasks.retain(|_, task| !task.is_complete());
            diagnostics
                .write()
                .await
                .phase_progress
                .insert("snap2_ranges_remaining".to_string(), tasks.len() as u64);
            continue;
        }

        if tasks.len() < last_remaining {
            last_remaining = tasks.len();
            progress_at = SystemTime::now();
        } else if progress_at
            .elapsed()
            .is_ok_and(|idle| idle >= STALL_TIMEOUT)
        {
            // The frontier has not moved for long enough that no peer is
            // serving usefully. Retrying forever is indistinguishable from a
            // slow sync and never resolves, so surface it: the caller discards
            // the partial state and the next cycle picks a fresh pivot and
            // peer set.
            return Err(SyncError::Snap2CatchUpStalled(
                pivot.number,
                format!(
                    "frontier stalled for {}s with {} ranges outstanding",
                    STALL_TIMEOUT.as_secs(),
                    tasks.len()
                ),
            ));
        }

        let scheduled = schedule(peers, &mut tasks, &pivot, &tx).await?;
        if !scheduled && !progressed {
            // Everything is out with a peer, or no peer is free.
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    publish(&mut writer, flat, &accounts_dir, &storages_dir).await?;
    async_fs::remove_dir_all(&accounts_dir).await?;
    async_fs::remove_dir_all(&storages_dir).await?;

    info!(
        block = pivot.number,
        "snap/2 state download complete, pivot frozen"
    );
    Ok(pivot)
}

/// Make everything buffered readable through the flat state.
async fn publish(
    writer: &mut ChunkWriter,
    flat: &FlatState,
    accounts_dir: &Path,
    storages_dir: &Path,
) -> Result<(), SyncError> {
    writer.flush()?;
    flat.absorb_account_chunks(accounts_dir).await?;
    flat.absorb_storage_chunks(storages_dir).await?;
    Ok(())
}

/// Hand out as much work as there are free peers for, returning whether
/// anything was scheduled.
async fn schedule(
    peers: &mut PeerHandler,
    tasks: &mut BTreeMap<RangeId, RangeTask>,
    pivot: &BlockHeader,
    tx: &tokio::sync::mpsc::Sender<Response>,
) -> Result<bool, SyncError> {
    let mut scheduled = false;

    for (&index, task) in tasks.iter_mut() {
        // Storage first: it is what holds a range back from advancing.
        while !task.awaiting_storage.is_empty() {
            let Some(batch) = next_storage_batch(task) else {
                break;
            };
            let Some((peer_id, connection, permit)) = peers
                .peer_table
                .get_best_peer(SUPPORTED_SNAP_CAPABILITIES.to_vec())
                .await?
            else {
                // No peer free; put the batch back rather than dropping it.
                for (account_hash, work) in batch {
                    task.awaiting_storage.insert(account_hash, work);
                }
                return Ok(scheduled);
            };

            task.storage_inflight += 1;
            scheduled = true;

            let tx = tx.clone();
            let pivot = pivot.clone();
            let accounts: Vec<H256> = batch.iter().map(|(hash, _)| *hash).collect();
            let roots: Vec<H256> = batch.iter().map(|(_, work)| work.root).collect();
            let start = batch
                .first()
                .map(|(_, work)| work.next_slot)
                .unwrap_or_default();
            let requested = batch;
            tokio::spawn(async move {
                let outcome = request_storage_ranges(
                    peer_id, connection, permit, &pivot, accounts, roots, start,
                )
                .await;
                let _ = tx
                    .send(Response::Storage {
                        range: index,
                        requested,
                        outcome: Box::new(outcome),
                    })
                    .await;
            });
        }

        // One batch at a time: the next account request may only go out once
        // this range's frontier has caught up to the last one, or the two
        // batches would overlap in the window the catch-up gate reads.
        if task.accounts_done || task.accounts_inflight || task.served_through.is_some() {
            continue;
        }
        let Some((peer_id, connection, permit)) = peers
            .peer_table
            .get_best_peer(SUPPORTED_SNAP_CAPABILITIES.to_vec())
            .await?
        else {
            return Ok(scheduled);
        };
        task.accounts_inflight = true;
        scheduled = true;

        let tx = tx.clone();
        let pivot = pivot.clone();
        let (start, end) = (task.next, task.last);
        tokio::spawn(async move {
            let outcome =
                request_account_range(peer_id, connection, permit, &pivot, start, end).await;
            let _ = tx
                .send(Response::Accounts {
                    range: index,
                    outcome: Box::new(outcome),
                })
                .await;
        });
    }

    Ok(scheduled)
}

/// The first hash after `hash`, or `None` at the top of the space.
fn next_hash(hash: H256) -> Option<H256> {
    use ethrex_common::BigEndianHash;
    hash.into_uint()
        .checked_add(U256::one())
        .map(|next| H256::from_uint(&next))
}

/// Take the next set of contracts to request storage for.
///
/// A request carries one starting slot hash for the whole batch, so a contract
/// resuming part-way through its slots goes out on its own; contracts starting
/// from zero are batched.
fn next_storage_batch(task: &mut RangeTask) -> Option<Vec<(H256, StorageWork)>> {
    let resuming = task
        .awaiting_storage
        .iter()
        .find(|(_, work)| !work.next_slot.is_zero())
        .map(|(hash, work)| (*hash, *work));
    if let Some((account_hash, work)) = resuming {
        task.awaiting_storage.remove(&account_hash);
        return Some(vec![(account_hash, work)]);
    }

    let batch: Vec<(H256, StorageWork)> = task
        .awaiting_storage
        .iter()
        .take(STORAGE_BATCH_SIZE)
        .map(|(hash, work)| (*hash, *work))
        .collect();
    if batch.is_empty() {
        return None;
    }
    for (account_hash, _) in &batch {
        task.awaiting_storage.remove(account_hash);
    }
    Some(batch)
}

/// Fold one worker's answer back into its range.
#[allow(clippy::too_many_arguments)]
async fn handle_response(
    response: Response,
    peers: &PeerHandler,
    tasks: &mut BTreeMap<RangeId, RangeTask>,
    writer: &mut ChunkWriter,
    cursor: &mut DownloadCursor,
    code_hash_collector: &mut CodeHashCollector,
    diagnostics: &Arc<tokio::sync::RwLock<SyncDiagnostics>>,
) -> Result<(), SyncError> {
    match response {
        Response::Accounts { range, outcome } => {
            let Some(task) = tasks.get_mut(&range) else {
                return Ok(());
            };
            task.accounts_inflight = false;
            if !outcome.verified {
                // Scoring this is what stops the range download reselecting a
                // peer that cannot answer. A peer stalled behind a fork it
                // could not follow still advertises snap/2 and still wins peer
                // selection, so without this the download retries it forever at
                // full score and never progresses.
                let _ = peers.peer_table.record_failure(outcome.peer_id);
                diagnostics.write().await.snap2_ranges_unverified += 1;
                return Ok(());
            }
            let _ = peers.peer_table.record_success(outcome.peer_id);
            diagnostics.write().await.snap2_ranges_served += 1;
            // A verified response holding no account inside this range means
            // the range is exhausted: every account the peer had from here on
            // belongs to a later range and was filtered out. Treat it as served
            // to the end, or the range is rescheduled forever and the frontier
            // never moves, while every response still verifies.
            let served_through = outcome
                .accounts
                .last()
                .map(|(account_hash, _)| *account_hash)
                .unwrap_or(task.last);

            for (account_hash, account) in &outcome.accounts {
                if account.code_hash != *EMPTY_KECCAK_HASH {
                    code_hash_collector.add(account.code_hash);
                    code_hash_collector.flush_if_needed().await?;
                }
                if account.storage_root == *EMPTY_TRIE_HASH {
                    continue;
                }
                // A contract part-way through its slots resumes where the
                // cursor left it, against the root this pivot just served.
                let next_slot = match cursor.storage_ranges(*account_hash) {
                    Some(ranges) => match ranges.first() {
                        Some(range) => range.next,
                        None => H256::zero(),
                    },
                    None if cursor.is_storage_complete(*account_hash) => continue,
                    None => H256::zero(),
                };
                task.awaiting_storage.insert(
                    *account_hash,
                    StorageWork {
                        root: account.storage_root,
                        next_slot,
                    },
                );
            }
            writer.accounts.extend(outcome.accounts);

            // The frontier moves to here once this batch's storage is in. A
            // response that ends the range still has to wait for that, so the
            // range is only marked done after the commit.
            task.served_through = Some(if outcome.remaining.is_some() {
                served_through
            } else {
                task.last
            });
            debug_assert!(
                task.served_through
                    .is_some_and(|through| through >= task.next)
                    || task.next > task.last,
                "frontier must not move backwards"
            );
        }
        Response::Storage {
            range,
            requested,
            outcome,
        } => {
            let Some(task) = tasks.get_mut(&range) else {
                return Ok(());
            };
            task.storage_inflight -= 1;

            if !outcome.verified {
                // Nothing usable came back; every contract in the batch is
                // still owed. Score the peer for the same reason as above.
                let _ = peers.peer_table.record_failure(outcome.peer_id);
                let requeued = requested.len() as u64;
                for (account_hash, work) in requested {
                    task.awaiting_storage.insert(account_hash, work);
                }
                let mut diag = diagnostics.write().await;
                diag.snap2_ranges_unverified += 1;
                diag.snap2_storage_requeued += requeued;
                return Ok(());
            }
            let _ = peers.peer_table.record_success(outcome.peer_id);
            {
                let mut diag = diagnostics.write().await;
                diag.snap2_ranges_served += 1;
                diag.snap2_storage_requeued += outcome.unserved.len() as u64;
                diag.snap2_storage_partial += outcome
                    .served
                    .iter()
                    .filter(|served| served.remaining.is_some())
                    .count() as u64;
            }

            let owed: BTreeMap<H256, StorageWork> = requested.into_iter().collect();
            for served in outcome.served {
                writer.push_storages(served.account_hash, served.slots);
                match served.remaining {
                    Some(next_slot) => {
                        // A contract too large for one response. Record what is
                        // left so the cursor answers per slot, and keep it
                        // queued with the root it was verified against.
                        cursor.open_storage_ranges(
                            served.account_hash,
                            vec![HashRange::new(next_slot, HASH_MAX)],
                        );
                        if let Some(work) = owed.get(&served.account_hash) {
                            task.awaiting_storage.insert(
                                served.account_hash,
                                StorageWork {
                                    root: work.root,
                                    next_slot,
                                },
                            );
                        }
                    }
                    None => cursor.complete_storage(served.account_hash),
                }
            }
            // The tail the peer truncated is simply owed again.
            for account_hash in outcome.unserved {
                if let Some(work) = owed.get(&account_hash) {
                    task.awaiting_storage.insert(account_hash, *work);
                }
            }
        }
    }
    Ok(())
}

/// Collect every outstanding response before the pivot changes under them.
#[allow(clippy::too_many_arguments)]
async fn collect_inflight(
    tasks: &mut BTreeMap<RangeId, RangeTask>,
    peers: &PeerHandler,
    rx: &mut tokio::sync::mpsc::Receiver<Response>,
    writer: &mut ChunkWriter,
    cursor: &mut DownloadCursor,
    code_hash_collector: &mut CodeHashCollector,
    diagnostics: &Arc<tokio::sync::RwLock<SyncDiagnostics>>,
) -> Result<(), SyncError> {
    while tasks.values().any(RangeTask::is_inflight) {
        let Some(response) = rx.recv().await else {
            debug!("snap/2 download: response channel closed with work in flight");
            break;
        };
        handle_response(
            response,
            peers,
            tasks,
            writer,
            cursor,
            code_hash_collector,
            diagnostics,
        )
        .await?;
    }
    Ok(())
}
