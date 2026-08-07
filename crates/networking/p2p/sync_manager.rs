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

use crate::{
    peer_handler::PeerHandler,
    sync::{SyncCycleOutcome, SyncDiagnostics, SyncMode, Syncer},
};

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
    syncer: Arc<Mutex<Syncer>>,
    last_fcu_head: Arc<Mutex<H256>>,
    store: Store,
    diagnostics: Arc<tokio::sync::RwLock<SyncDiagnostics>>,
    /// Cancellation token from construction. On L1 this is the node token `main`
    /// waits on; cancelling it runs shutdown/flush instead of `process::exit`.
    cancel_token: CancellationToken,
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
            cancel_token.clone(),
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
        let sync_head = self.last_fcu_head.clone();
        let cancel_token = self.cancel_token.clone();

        tokio::spawn(async move {
            // If we can't get hold of the syncer, then it means that there is an active sync in process
            let Ok(mut syncer) = syncer.try_lock() else {
                return;
            };
            let mut waiting_for_fcu_logged = false;
            let mut retry_state = SyncRetryState::new();
            loop {
                let sync_head = {
                    // Read latest fcu head without holding the lock for longer than needed
                    let Ok(sync_head) = sync_head.try_lock() else {
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
                // Start the sync cycle
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
