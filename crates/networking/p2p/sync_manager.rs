use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use ethrex_blockchain::Blockchain;
use ethrex_common::H256;
use ethrex_storage::Store;
use tokio::{
    sync::Mutex,
    time::{Duration, sleep},
};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use std::time::{SystemTime, UNIX_EPOCH};

use crate::{
    peer_handler::PeerHandler,
    sync::{SyncDiagnostics, SyncMode, Syncer, sync_head_executed},
};

/// How long to wait for a forkchoice head to arrive via `engine_newPayload` before
/// starting a peer-download sync cycle, when the node was recently at the chain tip.
/// Post-ePBS consensus clients move their forkchoice head to a payload hash learned
/// from a bid before the payload itself is delivered to the EL; measured on
/// glamsterdam-devnet-7 the payload consistently follows within 10-13s (about one
/// slot), and peers cannot serve the block earlier either — it has not propagated
/// anywhere yet. Waiting is strictly cheaper than cycling against peers.
const NEWPAYLOAD_HEAL_WAIT: Duration = Duration::from_secs(15);

/// Poll interval while waiting for the forkchoice head to become locally executed.
const NEWPAYLOAD_HEAL_POLL: Duration = Duration::from_millis(500);

/// A node whose latest executed block is at most this old was following the chain tip
/// moments ago; only then is the pre-cycle heal wait applied. Nodes that are cold,
/// restarting, or genuinely behind (older executed tip) start their sync cycle
/// immediately, so initial sync and catch-up latency are unaffected.
const RECENT_TIP_MAX_AGE_SECS: u64 = 30;

/// Abstraction to interact with the active sync process without disturbing it
#[derive(Debug)]
pub struct SyncManager {
    /// This is also held by the Syncer and allows tracking it's latest syncmode
    /// It is a READ_ONLY value, as modifications will disrupt the current active sync progress
    snap_enabled: Arc<AtomicBool>,
    syncer: Arc<Mutex<Syncer>>,
    last_fcu_head: Arc<Mutex<H256>>,
    store: Store,
    diagnostics: Arc<tokio::sync::RwLock<SyncDiagnostics>>,
}

impl SyncManager {
    pub async fn new(
        peer_handler: PeerHandler,
        sync_mode: &SyncMode,
        cancel_token: CancellationToken,
        blockchain: Arc<Blockchain>,
        store: Store,
        datadir: PathBuf,
    ) -> Self {
        let snap_enabled = Arc::new(AtomicBool::new(matches!(sync_mode, SyncMode::Snap)));

        // Fetch checkpoint once to avoid duplicate DB reads
        let has_checkpoint = store
            .get_header_download_checkpoint()
            .await
            .unwrap_or_else(|e| {
                warn!("Failed to read header download checkpoint: {e}");
                None
            })
            .is_some();

        // Auto-switch from snap to full sync if node already has synced state.
        // For post-merge networks (terminal_total_difficulty_passed), any stored
        // block > 0 means the node has previously synced. For pre-merge networks,
        // use merge_netsplit_block as threshold to avoid false positives in hive tests.
        if snap_enabled.load(Ordering::Relaxed) {
            let latest_block = store.get_latest_block_number().await.unwrap_or(0);
            let chain_config = store.get_chain_config();
            let is_synced = if chain_config.terminal_total_difficulty_passed {
                latest_block > 0
            } else if let Some(merge_block) = chain_config.merge_netsplit_block {
                latest_block > merge_block
            } else {
                false
            };
            if is_synced {
                info!("Node has synced state (block {latest_block}), switching to full sync");
                snap_enabled.store(false, Ordering::Relaxed);
                if has_checkpoint && let Err(e) = store.clear_snap_state().await {
                    warn!("Failed to clear stale snap state: {e}");
                }
            }
        }

        let diagnostics = Arc::new(tokio::sync::RwLock::new(SyncDiagnostics::default()));
        let syncer = Arc::new(Mutex::new(Syncer::new(
            peer_handler,
            snap_enabled.clone(),
            cancel_token,
            blockchain,
            datadir,
            diagnostics.clone(),
        )));
        let sync_manager = Self {
            snap_enabled,
            syncer,
            last_fcu_head: Arc::new(Mutex::new(H256::zero())),
            store: store.clone(),
            diagnostics,
        };
        // If the node was in the middle of a sync and then re-started we must resume syncing
        // Otherwise we will incorreclty assume the node is already synced and work on invalid state
        // Skip if the auto-switch already transitioned to full sync (snap_enabled is now false)
        if has_checkpoint && sync_manager.snap_enabled.load(Ordering::Relaxed) {
            sync_manager.start_sync();
        }
        sync_manager
    }

    /// Sets the latest fcu head and starts the next sync cycle if the syncer is currently inactive
    pub fn sync_to_head(&self, fcu_head: H256) {
        self.set_head(fcu_head);
        if !self.is_active() {
            self.start_sync();
        }
    }

    /// Returns the syncer's current syncmode (either snap or full)
    pub fn sync_mode(&self) -> SyncMode {
        if self.snap_enabled.load(Ordering::Relaxed) {
            SyncMode::Snap
        } else {
            SyncMode::Full
        }
    }

    /// Disables snapsync mode
    pub fn disable_snap(&self) {
        self.snap_enabled.store(false, Ordering::Relaxed);
    }

    /// Returns a snapshot of the current sync diagnostics with live values.
    pub async fn get_sync_diagnostics(&self) -> SyncDiagnostics {
        use crate::metrics::METRICS;
        use std::sync::atomic::Ordering::Relaxed;

        let mut diag = self.diagnostics.read().await.clone();

        // Compute live pivot age
        if let Some(ts) = diag.pivot_timestamp {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            diag.pivot_age_seconds = Some(now.saturating_sub(ts));
        }

        // Populate live progress from METRICS atomics
        let headers = METRICS.downloaded_headers.get();
        let accounts_downloaded = METRICS.downloaded_account_tries.load(Relaxed);
        let accounts_inserted = METRICS.account_tries_inserted.load(Relaxed);
        let storage_downloaded = METRICS.storage_leaves_downloaded.get();
        let storage_inserted = METRICS.storage_leaves_inserted.get();

        if headers > 0 {
            diag.phase_progress
                .insert("headers_downloaded".into(), headers);
        }
        if accounts_downloaded > 0 {
            diag.phase_progress
                .insert("accounts_downloaded".into(), accounts_downloaded);
        }
        if accounts_inserted > 0 {
            diag.phase_progress
                .insert("accounts_inserted".into(), accounts_inserted);
        }
        if storage_downloaded > 0 {
            diag.phase_progress
                .insert("storage_slots_downloaded".into(), storage_downloaded);
        }
        if storage_inserted > 0 {
            diag.phase_progress
                .insert("storage_slots_inserted".into(), storage_inserted);
        }

        diag
    }

    /// Returns a reference to the diagnostics RwLock for updating from the sync code.
    pub fn diagnostics(&self) -> &Arc<tokio::sync::RwLock<SyncDiagnostics>> {
        &self.diagnostics
    }

    /// Updates the last fcu head. This may be used on the next sync cycle if needed
    fn set_head(&self, fcu_head: H256) {
        if let Ok(mut latest_fcu_head) = self.last_fcu_head.try_lock() {
            *latest_fcu_head = fcu_head;
        } else {
            warn!("Failed to update latest fcu head for syncing")
        }
    }

    /// Returns true is the syncer is active
    fn is_active(&self) -> bool {
        self.syncer.try_lock().is_err()
    }

    /// Attempts to sync to the last received fcu head
    /// Will do nothing if the syncer is already involved in a sync process
    /// If the sync process would require multiple sync cycles (such as snap sync), starts all required sync cycles until the sync is complete
    fn start_sync(&self) {
        let syncer = self.syncer.clone();
        let store = self.store.clone();
        let last_fcu_head = self.last_fcu_head.clone();
        let diagnostics = self.diagnostics.clone();

        tokio::spawn(async move {
            // If we can't get hold of the syncer, then it means that there is an active sync in process
            let Ok(mut syncer) = syncer.try_lock() else {
                return;
            };
            let mut waiting_for_fcu_logged = false;
            let mut heal_wait_done = false;
            loop {
                let sync_head = {
                    // Read latest fcu head without holding the lock for longer than needed
                    let Ok(sync_head) = last_fcu_head.try_lock() else {
                        error!("Failed to read latest fcu head, unable to sync");
                        return;
                    };
                    *sync_head
                };
                // Edge case: If we are resuming a sync process after a node restart, wait until the next fcu to start
                if sync_head.is_zero() {
                    if waiting_for_fcu_logged {
                        debug!(
                            "Still waiting for a forkchoice update from the consensus client to resume sync"
                        );
                    } else {
                        info!(
                            "Resuming sync after node restart, waiting for a forkchoice update from the consensus client"
                        );
                        waiting_for_fcu_logged = true;
                    }
                    sleep(Duration::from_secs(5)).await;
                    continue;
                }
                // A node that was following the tip and is asked to sync to an unknown
                // head is almost always seeing the FCU/newPayload ordering race: the
                // payload behind that head simply has not reached us (or anyone) yet.
                // Give `engine_newPayload` a bounded window to deliver it before paying
                // for a peer-download sync cycle.
                if !heal_wait_done && was_recently_at_tip(&store).await {
                    heal_wait_done = true;
                    if wait_for_newpayload_heal(&store, &last_fcu_head).await {
                        return;
                    }
                    // The wait expired: re-read the latest head (it may have advanced
                    // while we waited) and start a real cycle.
                    continue;
                }
                heal_wait_done = true;
                // Start the sync cycle
                diagnostics.write().await.sync_cycles_started += 1;
                syncer.start_sync(sync_head, store.clone()).await;
                // Continue to the next sync cycle if we have an ongoing snap sync (aka if we still have snap sync checkpoints stored)
                if store
                    .get_header_download_checkpoint()
                    .await
                    .ok()
                    .flatten()
                    .is_none()
                {
                    break;
                }
            }
        });
    }

    pub fn get_last_fcu_head(&self) -> Result<H256, tokio::sync::TryLockError> {
        Ok(*self.last_fcu_head.try_lock()?)
    }
}

/// True when the latest executed block is recent enough that this node was following the
/// chain tip moments ago — as opposed to being cold, restarting, or mid-sync. Only then
/// is a missing forkchoice head likely to be a payload that has not reached us through
/// `engine_newPayload` yet rather than a real gap to download.
async fn was_recently_at_tip(store: &Store) -> bool {
    let latest = match store.get_latest_block_number().await {
        Ok(number) if number > 0 => number,
        _ => return false,
    };
    let Ok(Some(header)) = store.get_block_header(latest) else {
        return false;
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    now.saturating_sub(header.timestamp) <= RECENT_TIP_MAX_AGE_SECS
}

/// Polls for up to `NEWPAYLOAD_HEAL_WAIT` for the latest forkchoice head to become locally
/// executed (delivered via `engine_newPayload`). Returns true if it did, in which case a
/// sync cycle would be wasted work. Re-reads the head on every poll so newer FCUs arriving
/// during the wait are accounted for.
async fn wait_for_newpayload_heal(store: &Store, last_fcu_head: &Arc<Mutex<H256>>) -> bool {
    let deadline = tokio::time::Instant::now() + NEWPAYLOAD_HEAL_WAIT;
    while tokio::time::Instant::now() < deadline {
        let head = match last_fcu_head.try_lock() {
            Ok(head) => *head,
            Err(_) => H256::zero(),
        };
        if !head.is_zero() {
            match sync_head_executed(store, head) {
                Ok(true) => {
                    info!(
                        ?head,
                        "Forkchoice head arrived via engine_newPayload while waiting; skipping sync cycle"
                    );
                    return true;
                }
                Ok(false) => {}
                Err(error) => {
                    warn!(
                        %error,
                        "Failed to check local state for forkchoice head; starting sync cycle"
                    );
                    return false;
                }
            }
        }
        sleep(NEWPAYLOAD_HEAL_POLL).await;
    }
    false
}
