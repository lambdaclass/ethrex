//! Streaming storage download.
//!
//! Storage ranges historically downloaded only after the full account phase
//! (download, trie build, state heal), because the downloader resolved
//! storage roots from the built trie. But every account leaf already carries
//! its storage root, so storage can download as soon as account ranges
//! deliver leaves. This module runs the existing storage downloader in
//! "waves" over incrementally discovered accounts, concurrently with the
//! account download and trie build; whatever a wave cannot finish (stale
//! pivot, big-account remainders, repeated failures) is carried back to the
//! post-build storage loop, which keeps today's heal-based reconciliation.

use crate::peer_handler::PeerHandler;
use crate::snap::request_storage_ranges;
use crate::sync::{AccountStorageRoots, SyncError};
use ethrex_common::H256;
use ethrex_common::types::BlockHeader;
use ethrex_storage::Store;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tracing::{debug, info, warn};

use super::block_is_stale;

/// Accounts that fail to complete within this many waves stop being retried
/// here and are carried to the post-build loop (whose heal fallback covers
/// them). Bounds the retry work for accounts whose root keeps moving.
const MAX_WAVE_ATTEMPTS: u8 = 3;

pub(crate) struct StorageWaveOutcome {
    /// Next storage snapshot file index (continues the wave numbering).
    pub chunk_index: u64,
    /// Accounts fully downloaded by the waves; the post-build loop must not
    /// re-download them.
    pub done: HashSet<H256>,
    /// Unfinished bookkeeping (interval state, repeated failures) plus the
    /// healed-account marks accumulated by the waves.
    pub carry: AccountStorageRoots,
}

/// Runs storage-range downloads over accounts as they are discovered by the
/// account-range download. `feed_rx` delivers `(account_hash, storage_root)`
/// batches; closing it signals that no more discoveries are coming, after
/// which remaining work is attempted once more and the rest is carried out.
/// `pivot_rx` provides the freshest known pivot; when the borrowed pivot goes
/// stale the runner waits for a newer one instead of re-pivoting itself.
/// `storage_ingest_tx` announces each finished snapshot file to the
/// background ingest task; the post-build loop shares the same channel, so
/// this clone is dropped when the waves finish, not the whole sequence.
pub(crate) async fn run_storage_waves(
    mut peers: PeerHandler,
    store: Store,
    snapshots_dir: PathBuf,
    mut feed_rx: UnboundedReceiver<Vec<(H256, H256)>>,
    mut pivot_rx: tokio::sync::watch::Receiver<BlockHeader>,
    storage_ingest_tx: Option<UnboundedSender<(u64, PathBuf)>>,
) -> Result<StorageWaveOutcome, SyncError> {
    let mut chunk_index = 0u64;
    let mut seen: HashSet<H256> = HashSet::new();
    let mut attempts: HashMap<H256, u8> = HashMap::new();
    let mut carry = AccountStorageRoots::default();
    let mut pending = AccountStorageRoots::default();
    let mut feed_open = true;

    loop {
        // Gather new discoveries: block for the first batch while the feed is
        // open and there is nothing pending, then drain whatever is queued.
        if feed_open {
            if pending.accounts_with_storage_root.is_empty() {
                match feed_rx.recv().await {
                    Some(batch) => admit(batch, &mut seen, &mut pending),
                    None => feed_open = false,
                }
            }
            while let Ok(batch) = feed_rx.try_recv() {
                admit(batch, &mut seen, &mut pending);
            }
        }
        if pending.accounts_with_storage_root.is_empty() {
            if feed_open {
                continue;
            }
            break;
        }

        // A stale pivot would only produce empty responses. While the account
        // phase is still running, its dispatcher publishes fresher pivots, so
        // we can wait for one. But once the feed is closed the account phase
        // is over and NOBODY will publish again here — the post-build storage
        // loop owns pivot updates from that point and is blocked waiting for
        // us to return. Waiting on `changed()` then would deadlock (we wait
        // for a pivot only the awaiting main thread could send). Carry the
        // remaining work to the post-build loop instead.
        let mut pivot = pivot_rx.borrow().clone();
        if block_is_stale(&pivot) {
            if !feed_open {
                debug!(
                    "Storage waves: pivot stale and feed closed; carrying remaining accounts to the post-build loop"
                );
                break;
            }
            debug!("Storage waves waiting for a fresh pivot");
            if pivot_rx.changed().await.is_err() {
                // Publisher dropped: leave remaining work to the carry.
                break;
            }
            continue;
        }

        let wave_accounts: Vec<H256> = pending.accounts_with_storage_root.keys().copied().collect();
        debug!(
            accounts = wave_accounts.len(),
            "Starting storage download wave"
        );
        chunk_index = request_storage_ranges(
            &mut peers,
            &mut pending,
            &snapshots_dir,
            chunk_index,
            &mut pivot,
            store.clone(),
            storage_ingest_tx.clone(),
        )
        .await?;

        // Whatever was removed from the map completed; the rest retries in a
        // later wave until its attempt budget runs out.
        for account in wave_accounts {
            if !pending.accounts_with_storage_root.contains_key(&account) {
                done_insert(&mut attempts, account);
            }
        }
        carry
            .healed_accounts
            .extend(pending.healed_accounts.drain());
        let leftovers: Vec<H256> = pending.accounts_with_storage_root.keys().copied().collect();
        for account in leftovers {
            let tries = attempts.entry(account).or_insert(0);
            *tries = tries.saturating_add(1);
            if *tries >= MAX_WAVE_ATTEMPTS
                && let Some(entry) = pending.accounts_with_storage_root.remove(&account)
            {
                carry.accounts_with_storage_root.insert(account, entry);
            }
        }
    }

    // Anything still pending when the loop exits (a stale-pivot break with the
    // feed closed) is unfinished — hand it to the post-build storage loop,
    // which re-pivots on its own. Without this, accounts left in `pending`
    // would be silently dropped (never downloaded, never carried, never
    // healed).
    carry
        .healed_accounts
        .extend(pending.healed_accounts.drain());
    for (account, entry) in std::mem::take(&mut pending.accounts_with_storage_root) {
        carry
            .accounts_with_storage_root
            .entry(account)
            .or_insert(entry);
    }

    let done: HashSet<H256> = attempts
        .iter()
        .filter(|(_, tries)| **tries == DONE_SENTINEL)
        .map(|(account, _)| *account)
        .collect();
    info!(
        downloaded = done.len(),
        carried = carry.accounts_with_storage_root.len(),
        healed_marks = carry.healed_accounts.len(),
        "Storage waves finished"
    );
    Ok(StorageWaveOutcome {
        chunk_index,
        done,
        carry,
    })
}

/// Attempt-counter value marking an account as fully downloaded.
const DONE_SENTINEL: u8 = u8::MAX;

fn done_insert(attempts: &mut HashMap<H256, u8>, account: H256) {
    attempts.insert(account, DONE_SENTINEL);
}

fn admit(batch: Vec<(H256, H256)>, seen: &mut HashSet<H256>, pending: &mut AccountStorageRoots) {
    let mut admitted = 0;
    for (account, root) in batch {
        if seen.insert(account) {
            pending
                .accounts_with_storage_root
                .insert(account, (Some(root), Vec::new()));
            admitted += 1;
        }
    }
    if admitted > 0 {
        debug!(admitted, "Storage waves admitted new accounts");
    } else if !seen.is_empty() {
        // Re-delivered chunks after a re-pivot are expected; their accounts
        // keep the first-seen root and reconcile through healing if it moved.
        warn!("Storage wave feed delivered only already-seen accounts");
    }
}
