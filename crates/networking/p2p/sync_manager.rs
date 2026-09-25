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
use tokio_util::{sync::CancellationToken, task::TaskTracker};
use tracing::{error, info, warn};

use std::time::{SystemTime, UNIX_EPOCH};

use crate::{
    peer_handler::PeerHandler,
    sync::{
        BackfillConfig, HistoryChain, SyncCycleOutcome, SyncDiagnostics, SyncMode, Syncer,
        run_history_backfill, sync_head_executed,
    },
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

/// Maximum consecutive PeerTable `RequestTimeout` cycles without an intervening
/// successful cycle before the snap manager loop cancels the construction-time
/// cancellation token (on L1: node shutdown/flush) instead of `process::exit`.
/// The counter resets on each successful cycle. Value matches the pivot-update
/// rotation budget in `snap_sync`.
const MAX_CONSECUTIVE_REQUEST_TIMEOUTS: u64 = 5;

/// Initial backoff after a recoverable sync-cycle failure in the manager loop.
const INITIAL_RETRY_DELAY: Duration = Duration::from_secs(1);

/// Upper bound for exponential backoff between recoverable sync-cycle retries.
const MAX_RETRY_DELAY: Duration = Duration::from_secs(30);

/// Action produced by [`SyncRetryState`] after observing a cycle outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SyncRetryAction {
    /// Continue the manager loop; optionally sleep before the next cycle.
    Continue { sleep: Option<Duration> },
    /// Too many consecutive PeerTable request timeouts; escalate to fatal.
    Fatal,
}

/// Tracks consecutive PeerTable request timeouts and backoff for the snap
/// manager retry loop.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SyncRetryState {
    consecutive_timeouts: u64,
    backoff: Duration,
}

impl SyncRetryState {
    fn new() -> Self {
        Self {
            consecutive_timeouts: 0,
            backoff: INITIAL_RETRY_DELAY,
        }
    }

    fn on_outcome(&mut self, outcome: SyncCycleOutcome) -> SyncRetryAction {
        match outcome {
            SyncCycleOutcome::Success => {
                self.consecutive_timeouts = 0;
                self.backoff = INITIAL_RETRY_DELAY;
                SyncRetryAction::Continue { sleep: None }
            }
            SyncCycleOutcome::RecoverableTimeout => {
                self.consecutive_timeouts += 1;
                if self.consecutive_timeouts >= MAX_CONSECUTIVE_REQUEST_TIMEOUTS {
                    return SyncRetryAction::Fatal;
                }
                let sleep_for = self.backoff;
                self.backoff = next_backoff(self.backoff);
                SyncRetryAction::Continue {
                    sleep: Some(sleep_for),
                }
            }
            SyncCycleOutcome::RecoverableOther => {
                // Back off on other recoverable errors, but only RequestTimeouts
                // count toward the fatal cap. Only Success resets that counter.
                let sleep_for = self.backoff;
                self.backoff = next_backoff(self.backoff);
                SyncRetryAction::Continue {
                    sleep: Some(sleep_for),
                }
            }
        }
    }
}

fn next_backoff(current: Duration) -> Duration {
    current.saturating_mul(2).min(MAX_RETRY_DELAY)
}

/// Decision for one iteration of the manager sync loop after a cycle outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SyncLoopStep {
    /// Escalate: too many consecutive PeerTable timeouts while snap will retry.
    ExitFatal,
    /// Sleep, then continue the loop.
    SleepAndContinue(Duration),
    /// Continue the loop immediately (successful cycle still mid-snap).
    Continue,
    /// Leave the loop (no snap checkpoint: full sync or snap complete).
    Break,
}

/// Combines a retry-policy action with whether a snap checkpoint keeps the loop alive.
fn next_sync_loop_step(action: SyncRetryAction, will_continue: bool) -> SyncLoopStep {
    match action {
        SyncRetryAction::Fatal if will_continue => SyncLoopStep::ExitFatal,
        SyncRetryAction::Continue { sleep: Some(delay) } if will_continue => {
            SyncLoopStep::SleepAndContinue(delay)
        }
        _ if will_continue => SyncLoopStep::Continue,
        _ => SyncLoopStep::Break,
    }
}

/// Abstraction to interact with the active sync process without disturbing it
#[derive(Debug)]
pub struct SyncManager {
    /// This is also held by the Syncer and allows tracking it's latest syncmode
    /// It is a READ_ONLY value, as modifications will disrupt the current active sync progress
    snap_enabled: Arc<AtomicBool>,
    blockchain: Arc<Blockchain>,
    syncer: Arc<Mutex<Syncer>>,
    last_fcu_head: Arc<Mutex<H256>>,
    store: Store,
    diagnostics: Arc<tokio::sync::RwLock<SyncDiagnostics>>,
    /// Cancellation token from construction. On L1 this is the node token `main`
    /// waits on; cancelling it runs shutdown/flush instead of `process::exit`.
    cancel_token: CancellationToken,
}

impl SyncManager {
    #[allow(clippy::too_many_arguments)]
    pub async fn new(
        peer_handler: PeerHandler,
        sync_mode: &SyncMode,
        cancel_token: CancellationToken,
        blockchain: Arc<Blockchain>,
        store: Store,
        datadir: PathBuf,
        backfill_config: BackfillConfig,
        tracker: TaskTracker,
    ) -> Self {
        // Whether snap sync is permitted at all, captured from the configured mode before
        // the auto-switch below can flip `snap_enabled` to full. The unreachable-state
        // recovery uses this to avoid escalating a `--syncmode full` node into snap.
        let snap_permitted = matches!(sync_mode, SyncMode::Snap);
        let snap_enabled = Arc::new(AtomicBool::new(snap_permitted));

        // Clone the shared handles the optional backfill task needs before
        // `peer_handler`/`cancel_token`/`blockchain` are moved into the Syncer below.
        let backfill_peers = peer_handler.clone();
        let backfill_cancel = cancel_token.clone();
        let backfill_blockchain = blockchain.clone();

        // Only a snap/1 state sync depends on `GetTrieNodes`, and that is the
        // sync this node runs exactly when its chain has no Amsterdam, since
        // snap/2 reconciles with access lists that do not exist before it.
        //
        // Keying this on "a snap sync is running" instead would be circular and
        // would strand the snap/2 path permanently: withholding snap/2 means
        // never negotiating it, which means never finding a snap/2 peer, which
        // means always falling back to snap/1 and never clearing the flag.
        let needs_trie_nodes = snap_enabled.load(Ordering::Relaxed)
            && store.get_chain_config().amsterdam_time.is_none();
        blockchain.set_state_sync_needs_trie_nodes(needs_trie_nodes);

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
            let latest_block = store.get_latest_block_number().unwrap_or(0);
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
                blockchain.set_state_sync_needs_trie_nodes(false);
                if has_checkpoint && let Err(e) = store.clear_snap_state().await {
                    warn!("Failed to clear stale snap state: {e}");
                }
            }
        }

        let diagnostics = Arc::new(tokio::sync::RwLock::new(SyncDiagnostics::default()));
        let blockchain_for_manager = blockchain.clone();
        let syncer = Arc::new(Mutex::new(Syncer::new(
            peer_handler,
            snap_enabled.clone(),
            snap_permitted,
            cancel_token.clone(),
            blockchain,
            datadir,
            diagnostics.clone(),
        )));

        // Spawn the optional historical-chain backfill task. It idles until
        // initial sync finishes, then fills bodies/receipts below the pivot down
        // to the configured floor, resuming across restarts via the persisted
        // frontier. A no-op when `--history.chain off`.
        if backfill_config.mode != HistoryChain::Off {
            // Tracked like every other long-lived task, so shutdown waits for it
            // rather than dropping it mid-batch.
            tracker.spawn(run_history_backfill(
                backfill_peers,
                store.clone(),
                backfill_blockchain,
                backfill_config,
                backfill_cancel,
                diagnostics.clone(),
            ));
        }
        let sync_manager = Self {
            snap_enabled,
            blockchain: blockchain_for_manager,
            syncer,
            last_fcu_head: Arc::new(Mutex::new(H256::zero())),
            store: store.clone(),
            diagnostics,
            cancel_token,
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
        self.blockchain.set_state_sync_needs_trie_nodes(false);
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
        let cancel_token = self.cancel_token.clone();

        tokio::spawn(async move {
            // If we can't get hold of the syncer, then it means that there is an active sync in process
            let Ok(mut syncer) = syncer.try_lock() else {
                return;
            };
            let mut heal_wait_done = false;
            let mut retry_state = SyncRetryState::new();
            loop {
                let sync_head = {
                    // Read latest fcu head without holding the lock for longer than needed
                    let Ok(sync_head) = last_fcu_head.try_lock() else {
                        error!("Failed to read latest fcu head, unable to sync");
                        return;
                    };
                    *sync_head
                };
                // No forkchoice head yet (e.g. right after a node restart, before the
                // first forkchoiceUpdate). Return and release the syncer lock instead
                // of spinning here while holding it: while this task is alive `is_active()`
                // stays true, so every `sync_to_head()` becomes a silent no-op and the
                // node never re-arms once a head finally arrives (a single dropped fcu-head
                // update in `set_head` would then wedge it indefinitely). The next
                // forkchoiceUpdate calls `sync_to_head()`, which finds the syncer idle and
                // re-spawns this task with the head populated.
                if sync_head.is_zero() {
                    info!(
                        "No forkchoice head yet (e.g. after a node restart); waiting for the next forkchoice update to start syncing"
                    );
                    return;
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
                let outcome = syncer.start_sync(sync_head, store.clone()).await;
                // Keep looping only while a snap header-download checkpoint remains.
                // Without one (full sync / completed snap), leave the loop as before.
                let will_continue = store
                    .get_header_download_checkpoint()
                    .await
                    .ok()
                    .flatten()
                    .is_some();
                let action = retry_state.on_outcome(outcome);
                match next_sync_loop_step(action, will_continue) {
                    SyncLoopStep::ExitFatal => {
                        error!(
                            consecutive_timeouts = retry_state.consecutive_timeouts,
                            max = MAX_CONSECUTIVE_REQUEST_TIMEOUTS,
                            "Sync cycle failed with {MAX_CONSECUTIVE_REQUEST_TIMEOUTS} consecutive PeerTable request timeouts without a successful cycle; cancelling node"
                        );
                        cancel_token.cancel();
                        break;
                    }
                    SyncLoopStep::SleepAndContinue(delay) => {
                        warn!(
                            ?outcome,
                            consecutive_timeouts = retry_state.consecutive_timeouts,
                            backoff_s = delay.as_secs(),
                            "Backing off before retrying sync cycle"
                        );
                        sleep(delay).await;
                    }
                    SyncLoopStep::Continue => {}
                    SyncLoopStep::Break => break,
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
    let latest = match store.get_latest_block_number() {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn success_resets_timeout_counter_and_skips_sleep() {
        let mut state = SyncRetryState::new();
        assert_eq!(
            state.on_outcome(SyncCycleOutcome::RecoverableTimeout),
            SyncRetryAction::Continue {
                sleep: Some(Duration::from_secs(1))
            }
        );
        assert_eq!(state.consecutive_timeouts, 1);

        assert_eq!(
            state.on_outcome(SyncCycleOutcome::Success),
            SyncRetryAction::Continue { sleep: None }
        );
        assert_eq!(state.consecutive_timeouts, 0);
        assert_eq!(state.backoff, INITIAL_RETRY_DELAY);
    }

    #[test]
    fn fifth_consecutive_timeout_is_fatal() {
        let mut state = SyncRetryState::new();
        let expected_sleeps = [
            Duration::from_secs(1),
            Duration::from_secs(2),
            Duration::from_secs(4),
            Duration::from_secs(8),
        ];
        for (idx, expected_sleep) in expected_sleeps.iter().enumerate() {
            let expected_timeouts = (idx as u64) + 1;
            assert_eq!(
                state.on_outcome(SyncCycleOutcome::RecoverableTimeout),
                SyncRetryAction::Continue {
                    sleep: Some(*expected_sleep)
                },
                "timeout #{expected_timeouts} should sleep {expected_sleep:?}"
            );
            assert_eq!(state.consecutive_timeouts, expected_timeouts);
        }
        assert_eq!(
            state.on_outcome(SyncCycleOutcome::RecoverableTimeout),
            SyncRetryAction::Fatal
        );
        assert_eq!(state.consecutive_timeouts, MAX_CONSECUTIVE_REQUEST_TIMEOUTS);
    }

    #[test]
    fn recoverable_other_backs_off_without_tripping_timeout_cap() {
        let mut state = SyncRetryState::new();
        for _ in 0..MAX_CONSECUTIVE_REQUEST_TIMEOUTS {
            let action = state.on_outcome(SyncCycleOutcome::RecoverableOther);
            assert!(matches!(
                action,
                SyncRetryAction::Continue { sleep: Some(_) }
            ));
        }
        assert_eq!(state.consecutive_timeouts, 0);
        assert_eq!(state.backoff, MAX_RETRY_DELAY);
    }

    #[test]
    fn other_recoverable_does_not_reset_timeout_counter() {
        let mut state = SyncRetryState::new();
        assert!(matches!(
            state.on_outcome(SyncCycleOutcome::RecoverableTimeout),
            SyncRetryAction::Continue { sleep: Some(_) }
        ));
        assert_eq!(state.consecutive_timeouts, 1);

        assert!(matches!(
            state.on_outcome(SyncCycleOutcome::RecoverableOther),
            SyncRetryAction::Continue { sleep: Some(_) }
        ));
        assert_eq!(state.consecutive_timeouts, 1);

        // 1 prior timeout + 3 more continues + 1 fatal = 5 total timeouts.
        assert!(matches!(
            state.on_outcome(SyncCycleOutcome::RecoverableTimeout),
            SyncRetryAction::Continue { sleep: Some(_) }
        ));
        assert!(matches!(
            state.on_outcome(SyncCycleOutcome::RecoverableTimeout),
            SyncRetryAction::Continue { sleep: Some(_) }
        ));
        assert!(matches!(
            state.on_outcome(SyncCycleOutcome::RecoverableTimeout),
            SyncRetryAction::Continue { sleep: Some(_) }
        ));
        assert_eq!(
            state.on_outcome(SyncCycleOutcome::RecoverableTimeout),
            SyncRetryAction::Fatal
        );
    }

    #[test]
    fn backoff_doubles_until_cap() {
        assert_eq!(next_backoff(Duration::from_secs(1)), Duration::from_secs(2));
        assert_eq!(next_backoff(Duration::from_secs(2)), Duration::from_secs(4));
        assert_eq!(
            next_backoff(Duration::from_secs(16)),
            Duration::from_secs(30)
        );
        assert_eq!(
            next_backoff(Duration::from_secs(30)),
            Duration::from_secs(30)
        );
    }

    #[test]
    fn loop_step_gates_fatal_and_sleep_on_will_continue() {
        let delay = Duration::from_secs(4);
        let fatal = SyncRetryAction::Fatal;
        let no_sleep = SyncRetryAction::Continue { sleep: None };
        let with_sleep = SyncRetryAction::Continue { sleep: Some(delay) };

        // Snap will retry (checkpoint present).
        assert_eq!(next_sync_loop_step(fatal, true), SyncLoopStep::ExitFatal);
        assert_eq!(next_sync_loop_step(no_sleep, true), SyncLoopStep::Continue);
        assert_eq!(
            next_sync_loop_step(with_sleep, true),
            SyncLoopStep::SleepAndContinue(delay)
        );

        // No checkpoint: do not escalate or sleep; break like full sync today.
        assert_eq!(next_sync_loop_step(fatal, false), SyncLoopStep::Break);
        assert_eq!(next_sync_loop_step(no_sleep, false), SyncLoopStep::Break);
        assert_eq!(next_sync_loop_step(with_sleep, false), SyncLoopStep::Break);
    }

    #[test]
    fn end_to_end_timeout_cap_with_checkpoint_decides_exit_fatal() {
        let mut state = SyncRetryState::new();
        for _ in 1..MAX_CONSECUTIVE_REQUEST_TIMEOUTS {
            let action = state.on_outcome(SyncCycleOutcome::RecoverableTimeout);
            assert!(matches!(
                next_sync_loop_step(action, true),
                SyncLoopStep::SleepAndContinue(_)
            ));
        }
        let action = state.on_outcome(SyncCycleOutcome::RecoverableTimeout);
        assert_eq!(next_sync_loop_step(action, true), SyncLoopStep::ExitFatal);
        // Same Fatal action without a checkpoint must not escalate.
        assert_eq!(next_sync_loop_step(action, false), SyncLoopStep::Break);
    }
}
