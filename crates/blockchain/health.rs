//! Whether the chain is advancing, and if not, saying so.
//!
//! ## Why this exists
//!
//! Three separate investigations on this branch were made materially harder by
//! the same gap: a node that has stopped advancing looks exactly like a node
//! that is running normally with nothing to do. Nothing above `WARN` is emitted
//! either way.
//!
//! - The chain halted permanently at the binary-tree activation block, because
//!   `apply_fork_choice` gated on the MPT-only `has_state_root` and so refused
//!   every binary-committed header. The node kept running and kept logging
//!   normally. It was found by noticing the head number had stopped.
//! - A node resumed on absent state after a restart and served genesis-alloc
//!   values indefinitely, silently.
//! - Judging whether a devnet node was healthy required querying it over RPC
//!   and diffing head numbers over time, because the logs of a wedged node and
//!   an idle-but-healthy node are indistinguishable.
//!
//! The node had the information in every case and did not say it.
//!
//! ## What it reports, and what it deliberately does not
//!
//! A status signal that is *wrong* is worse than one that is absent, because
//! people act on it — `eth_syncing` was "fixed" twice on this branch and the
//! devnet refuted both fixes. So this renders no verdict it cannot back:
//!
//! - **A frozen head is not a halt.** A devnet between slots, a chain whose
//!   other validators are down, a node with no peers — all have a frozen head
//!   and all are fine. That case is reported as [`ProgressEvent::Idle`], at
//!   `INFO`, stating facts (head, how long unchanged, when the last forkchoice
//!   update arrived) and drawing no conclusion.
//! - **A frozen head *plus* declined forkchoice updates is a halt.** The
//!   consensus client is asking the node to move to a head and the node keeps
//!   declining, and has not moved meanwhile. That is
//!   [`ProgressEvent::Stalled`], and it is the halt case exactly: the CL kept
//!   sending, `apply_fork_choice` kept refusing, the head number kept still.
//!
//! The `Stalled` level is `ERROR` only for a node that had previously synced
//! (see [`ProgressObservation::synced`]). A node still in its initial sync can
//! legitimately decline forkchoice updates for a long time while a header
//! download runs and the head does not move; that gets the same facts at
//! `WARN`, not an `ERROR` claiming a halt that has not happened.
//!
//! ## Cadence
//!
//! This runs forever, on every node, so it must not become noise — that is how
//! real warnings get filtered out. The monitor ticks every
//! [`PROGRESS_TICK`] but emits on state *transitions*, plus a bounded repeat
//! per state: `Stalled` re-states itself every [`STALLED_REPEAT`] because the
//! halt case is precisely the one where a single line scrolls away and the
//! operator sees nothing thereafter; `Idle` every [`IDLE_REPEAT`]. A node whose
//! head is advancing emits nothing above `DEBUG` from here at all.

use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use crate::Blockchain;

/// How often [`monitor_chain_progress`] samples the head.
///
/// One in-memory read of the latest block header plus one uncontended mutex, so
/// the interval is set by how promptly a halt should surface rather than by
/// cost.
pub const PROGRESS_TICK: Duration = Duration::from_secs(15);

/// How long the head must sit still before the monitor leaves
/// [`Phase::Advancing`].
///
/// Five slots at 12s. Long enough that ordinary block-time jitter and a couple
/// of missed slots never trip it, short enough that a real halt surfaces within
/// a minute rather than after an operator notices.
pub const NO_PROGRESS_AFTER: Duration = Duration::from_secs(60);

/// How often a sustained stall re-states itself.
///
/// The halt case is the one where a single line scrolls away, so this does not
/// log once and go quiet. It is deliberately slower than [`PROGRESS_TICK`]:
/// once a minute is unmissable in a `docker logs -f` tail without flooding it.
pub const STALLED_REPEAT: Duration = Duration::from_secs(60);

/// How often a sustained idle re-states itself. Rare, because an idle node is
/// the healthy case and a healthy node must not be noisy.
pub const IDLE_REPEAT: Duration = Duration::from_secs(600);

/// A forkchoice update the node declined to apply, classified for grepping.
///
/// The kind is a fixed string rather than the error's `Display` so a log tail
/// can be filtered on it and so the same situation reads identically every
/// time; the free-form detail carries the error text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RefusalKind {
    /// The head's state could not be reached from the database.
    ///
    /// The flip-block halt's kind: past `binaryTreeTime` a header's
    /// `state_root` is a binary-trie root, the reachability gate asked the
    /// MPT about it, and every post-activation head was refused.
    StateNotReachable,
    /// The node reported itself as syncing rather than applying the update.
    Syncing,
    /// The head could not be linked to the canonical chain.
    UnlinkedHead,
    /// The reorg was deeper than the node can physically reconstruct.
    TooDeepReorg,
    /// The head, or an ancestor of it, is invalid.
    InvalidHead,
    /// The forkchoice elements were inconsistent with each other.
    Inconsistent,
    /// A storage error prevented the update from being evaluated.
    StoreError,
}

impl RefusalKind {
    /// The stable, greppable name emitted in logs.
    pub const fn as_str(self) -> &'static str {
        match self {
            RefusalKind::StateNotReachable => "state_not_reachable",
            RefusalKind::Syncing => "syncing",
            RefusalKind::UnlinkedHead => "unlinked_head",
            RefusalKind::TooDeepReorg => "too_deep_reorg",
            RefusalKind::InvalidHead => "invalid_head",
            RefusalKind::Inconsistent => "inconsistent_forkchoice",
            RefusalKind::StoreError => "store_error",
        }
    }
}

/// Whether a refusal of this kind should count against progress, given whether
/// the node has ever completed a sync.
///
/// Everything counts except one case: [`RefusalKind::Syncing`] from a node that
/// has never synced. That is the node's own honest self-report while its initial
/// sync runs, the syncer is the thing making progress, and the sync path reports
/// on itself in detail already. Counting it would put a `chain_stalled` line in
/// front of an operator once a minute for the entire duration of a mainnet sync,
/// which is how real warnings get filtered out.
///
/// The same answer from a node that *has* synced is kept: a node that finished
/// syncing and then starts answering SYNCING to every forkchoice update while
/// its head sits still is wedged, and that is worth saying.
pub const fn countable_refusal(kind: RefusalKind, synced: bool) -> bool {
    !matches!(kind, RefusalKind::Syncing) || synced
}

/// The last declined forkchoice update, as reported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub kind: RefusalKind,
    pub detail: String,
}

/// Which of the three progress states the monitor last concluded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// The head moved within [`NO_PROGRESS_AFTER`].
    Advancing,
    /// The head has not moved, and no forkchoice update has been declined
    /// since it last did.
    Idle,
    /// The head has not moved and forkchoice updates are being declined.
    Stalled,
}

/// What one tick concluded, and what (if anything) should be said about it.
///
/// The monitor returns this rather than only logging so that the decision can
/// be tested directly. Log-level and message assembly stay in
/// [`ProgressEvent::emit`], which is what the running node calls.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProgressEvent {
    /// Nothing worth saying this tick.
    Quiet,
    /// The head moved after a spell of not moving.
    Resumed {
        head: u64,
        /// How long the head had been still, in seconds.
        was_still_for: u64,
        /// Whether the spell had been reported as a stall.
        was_stalled: bool,
    },
    /// The head has not moved, and nothing has been declined. Facts only: this
    /// is what a healthy chain with nothing to do looks like, and also what a
    /// chain whose *producers* have stopped looks like. The node cannot tell
    /// those apart and does not try.
    Idle {
        head: u64,
        unchanged_for: u64,
        /// Seconds since the last forkchoice update arrived, or `None` if none
        /// ever has — a node with no consensus client attached.
        last_fcu_ago: Option<u64>,
    },
    /// The head has not moved and forkchoice updates are being declined.
    Stalled {
        head: u64,
        unchanged_for: u64,
        /// Declined updates since the head last moved.
        refusals: u64,
        last_refusal: Refusal,
        /// Seconds since the last forkchoice update arrived. Always `Some` in
        /// practice — a refusal implies an update arrived — but carried as the
        /// same optional so the two events read alike.
        last_fcu_ago: Option<u64>,
        /// Whether the node had completed a sync before this began. Decides
        /// `ERROR` versus `WARN`; see the module docs.
        synced: bool,
    },
}

impl ProgressEvent {
    /// Writes this event to the log at the level the situation warrants.
    ///
    /// Messages lead with a fixed token (`chain_stalled`, `chain_idle`,
    /// `chain_resumed`) so they can be grepped out of `docker logs` and
    /// kurtosis output, and carry their numbers as structured fields.
    pub fn emit(&self) {
        match self {
            ProgressEvent::Quiet => {}
            ProgressEvent::Resumed {
                head,
                was_still_for,
                was_stalled,
            } => {
                if *was_stalled {
                    info!(
                        head,
                        was_still_for_secs = was_still_for,
                        "chain_resumed: head advanced again after a stall"
                    );
                } else {
                    info!(
                        head,
                        was_still_for_secs = was_still_for,
                        "chain_resumed: head advanced again"
                    );
                }
            }
            ProgressEvent::Idle {
                head,
                unchanged_for,
                last_fcu_ago,
            } => {
                // INFO, and deliberately not a verdict: a frozen head with
                // nothing declined is what an idle-but-healthy node looks like.
                info!(
                    head,
                    unchanged_for_secs = unchanged_for,
                    last_forkchoice_update_secs_ago =
                        last_fcu_ago.map_or_else(|| "never".to_string(), |s| s.to_string()),
                    forkchoice_refusals = 0,
                    "chain_idle: head has not moved; no forkchoice update has been declined"
                );
            }
            ProgressEvent::Stalled {
                head,
                unchanged_for,
                refusals,
                last_refusal,
                last_fcu_ago,
                synced,
            } => {
                if *synced {
                    error!(
                        head,
                        unchanged_for_secs = unchanged_for,
                        forkchoice_refusals = refusals,
                        last_refusal_kind = last_refusal.kind.as_str(),
                        last_refusal = %last_refusal.detail,
                        last_forkchoice_update_secs_ago =
                            last_fcu_ago.map_or_else(|| "never".to_string(), |s| s.to_string()),
                        "chain_stalled: head has not advanced and forkchoice updates are being declined"
                    );
                } else {
                    // Same facts, no halt claim: a node still in its initial
                    // sync can decline for a long time while a header download
                    // runs and the head legitimately does not move.
                    warn!(
                        head,
                        unchanged_for_secs = unchanged_for,
                        forkchoice_refusals = refusals,
                        last_refusal_kind = last_refusal.kind.as_str(),
                        last_refusal = %last_refusal.detail,
                        last_forkchoice_update_secs_ago =
                            last_fcu_ago.map_or_else(|| "never".to_string(), |s| s.to_string()),
                        "chain_stalled: head has not advanced and forkchoice updates are being declined (node has not completed a sync)"
                    );
                }
            }
        }
    }
}

/// One sample of the things the monitor cannot observe for itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProgressObservation {
    /// The canonical head's block number.
    pub head: u64,
    /// Whether the node has ever completed a sync ([`crate::Blockchain::is_synced`]).
    pub synced: bool,
}

/// Monotonic time source. Real clock in production; a manually driven one in
/// tests, so a stall spanning minutes can be exercised without waiting.
#[derive(Debug)]
enum Clock {
    System(Instant),
    Manual(Mutex<Duration>),
}

impl Clock {
    fn now(&self) -> Duration {
        match self {
            Clock::System(origin) => origin.elapsed(),
            // A poisoned manual clock is a test-only condition; treating it as
            // zero would silently make every elapsed check pass, so panic.
            Clock::Manual(t) => *t.lock().expect("manual clock poisoned"),
        }
    }
}

#[derive(Debug)]
struct Inner {
    /// The head as of the last tick. `None` before the first tick.
    head: Option<u64>,
    /// When the head was last observed to change.
    head_changed_at: Duration,
    /// When a forkchoice update last arrived, whatever its outcome.
    last_fcu_at: Option<Duration>,
    /// Declined forkchoice updates since the head last moved.
    refusals: u64,
    /// The most recent decline since the head last moved.
    last_refusal: Option<Refusal>,
    phase: Phase,
    /// When the current phase was last reported, for the per-phase repeat.
    last_emit_at: Duration,
}

/// Tracks whether the chain is advancing and, when it is not, what the node is
/// declining to do about it.
///
/// Cheap to update: one uncontended mutex per forkchoice update and one per
/// monitor tick.
#[derive(Debug)]
pub struct ChainHealth {
    clock: Clock,
    inner: Mutex<Inner>,
}

impl Default for ChainHealth {
    fn default() -> Self {
        Self::new()
    }
}

impl ChainHealth {
    pub fn new() -> Self {
        Self::with_clock(Clock::System(Instant::now()))
    }

    /// A `ChainHealth` whose clock is driven by [`Self::advance_clock`] rather
    /// than by wall time. For tests: a stall takes minutes to develop and no
    /// test should wait them out.
    pub fn manual() -> Self {
        Self::with_clock(Clock::Manual(Mutex::new(Duration::ZERO)))
    }

    fn with_clock(clock: Clock) -> Self {
        let now = clock.now();
        Self {
            clock,
            inner: Mutex::new(Inner {
                head: None,
                head_changed_at: now,
                last_fcu_at: None,
                refusals: 0,
                last_refusal: None,
                phase: Phase::Advancing,
                last_emit_at: now,
            }),
        }
    }

    /// Moves a manual clock forward. Panics on a system-clock instance, which
    /// would otherwise silently no-op and make a test assert against a stall
    /// that never developed.
    pub fn advance_clock(&self, by: Duration) {
        match &self.clock {
            Clock::Manual(t) => {
                let mut t = t.lock().expect("manual clock poisoned");
                *t += by;
            }
            Clock::System(_) => panic!("advance_clock called on a system-clock ChainHealth"),
        }
    }

    /// Records that a forkchoice update arrived, before its outcome is known.
    ///
    /// Separate from [`Self::record_refusal`] so that "the CL is talking to us
    /// and we are accepting" and "the CL is not talking to us at all" stay
    /// distinguishable — the second is a node with no consensus client, which
    /// is not a halt.
    pub fn record_forkchoice_update(&self) {
        let now = self.clock.now();
        if let Ok(mut inner) = self.inner.lock() {
            inner.last_fcu_at = Some(now);
        }
    }

    /// Records that a forkchoice update was declined.
    ///
    /// Counted against the current head: [`Self::observe`] clears the count
    /// when the head moves, so a stall verdict always means "declined *and*
    /// went nowhere", never "declined at some point in the past".
    pub fn record_refusal(&self, kind: RefusalKind, detail: impl Into<String>) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.refusals = inner.refusals.saturating_add(1);
            inner.last_refusal = Some(Refusal {
                kind,
                detail: detail.into(),
            });
        }
    }

    /// Number of declined forkchoice updates since the head last moved.
    /// Exposed for tests and for callers that want the raw fact.
    pub fn refusals_at_current_head(&self) -> u64 {
        self.inner.lock().map(|i| i.refusals).unwrap_or(0)
    }

    /// The phase concluded by the most recent [`Self::observe`].
    pub fn phase(&self) -> Phase {
        self.inner
            .lock()
            .map(|i| i.phase)
            .unwrap_or(Phase::Advancing)
    }

    /// Folds one sample into the state machine and returns what should be said
    /// about it, if anything.
    ///
    /// Pure with respect to logging; [`ProgressEvent::emit`] does that. The
    /// split is what makes the transition rules testable without scraping log
    /// output.
    pub fn observe(&self, obs: ProgressObservation) -> ProgressEvent {
        let now = self.clock.now();
        let Ok(mut inner) = self.inner.lock() else {
            return ProgressEvent::Quiet;
        };

        // First sample establishes the baseline; there is no "unchanged for"
        // to report yet.
        let Some(previous_head) = inner.head else {
            inner.head = Some(obs.head);
            inner.head_changed_at = now;
            return ProgressEvent::Quiet;
        };

        if obs.head != previous_head {
            let was_still_for = now.saturating_sub(inner.head_changed_at).as_secs();
            let previous_phase = inner.phase;
            inner.head = Some(obs.head);
            inner.head_changed_at = now;
            // Refusals are per-head: whatever was declined applied to a head we
            // have now left behind.
            inner.refusals = 0;
            inner.last_refusal = None;
            inner.phase = Phase::Advancing;
            inner.last_emit_at = now;
            return match previous_phase {
                Phase::Advancing => {
                    debug!(head = obs.head, "chain_advancing");
                    ProgressEvent::Quiet
                }
                Phase::Idle | Phase::Stalled => ProgressEvent::Resumed {
                    head: obs.head,
                    was_still_for,
                    was_stalled: previous_phase == Phase::Stalled,
                },
            };
        }

        let unchanged = now.saturating_sub(inner.head_changed_at);
        if unchanged < NO_PROGRESS_AFTER {
            return ProgressEvent::Quiet;
        }

        let last_fcu_ago = inner.last_fcu_at.map(|at| now.saturating_sub(at).as_secs());

        let (phase, repeat) = match inner.last_refusal {
            Some(_) => (Phase::Stalled, STALLED_REPEAT),
            None => (Phase::Idle, IDLE_REPEAT),
        };

        // Emit on entering the phase, then at the phase's own repeat cadence.
        let is_transition = inner.phase != phase;
        if !is_transition && now.saturating_sub(inner.last_emit_at) < repeat {
            return ProgressEvent::Quiet;
        }
        inner.phase = phase;
        inner.last_emit_at = now;

        match phase {
            Phase::Stalled => ProgressEvent::Stalled {
                head: obs.head,
                unchanged_for: unchanged.as_secs(),
                refusals: inner.refusals,
                // Set: the match arm above selected this phase on `Some`.
                last_refusal: inner.last_refusal.clone().unwrap_or_else(|| Refusal {
                    kind: RefusalKind::StoreError,
                    detail: "unknown".to_string(),
                }),
                last_fcu_ago,
                synced: obs.synced,
            },
            Phase::Idle => ProgressEvent::Idle {
                head: obs.head,
                unchanged_for: unchanged.as_secs(),
                last_fcu_ago,
            },
            // Unreachable: the match above only yields Stalled or Idle.
            Phase::Advancing => ProgressEvent::Quiet,
        }
    }
}

/// Samples the head every [`PROGRESS_TICK`] and reports what the samples mean.
///
/// The head is read from the store rather than pushed in by the forkchoice
/// handler, so every way the chain can advance is covered — engine-API
/// forkchoice, full-sync block import, the L2 block producer — and a path that
/// forgets to report cannot make the node look halted when it is not.
/// `Store::get_latest_block_number` is an in-memory read of the cached latest
/// header, so the tick costs that read plus one uncontended mutex.
///
/// Runs until `cancel` fires.
pub async fn monitor_chain_progress(blockchain: Arc<Blockchain>, cancel: CancellationToken) {
    let mut ticker = tokio::time::interval(PROGRESS_TICK);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            _ = cancel.cancelled() => return,
            _ = ticker.tick() => {}
        }
        let head = match blockchain.store().get_latest_block_number().await {
            Ok(head) => head,
            Err(err) => {
                // Not fatal, and not silent either: failing to read the head is
                // itself something an operator chasing a wedged node needs.
                warn!(error = %err, "chain progress monitor could not read the head");
                continue;
            }
        };
        blockchain
            .health()
            .observe(ProgressObservation {
                head,
                synced: blockchain.is_synced(),
            })
            .emit();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obs(head: u64) -> ProgressObservation {
        ProgressObservation { head, synced: true }
    }

    /// Baseline sample says nothing: there is no previous head to compare to.
    #[test]
    fn first_observation_is_quiet() {
        let h = ChainHealth::manual();
        assert_eq!(h.observe(obs(10)), ProgressEvent::Quiet);
        assert_eq!(h.phase(), Phase::Advancing);
    }

    /// A node whose head keeps moving must emit nothing. This is the "do not
    /// spam" requirement, asserted rather than assumed.
    #[test]
    fn an_advancing_head_never_emits() {
        let h = ChainHealth::manual();
        h.observe(obs(0));
        for n in 1..=200u64 {
            h.advance_clock(PROGRESS_TICK);
            assert_eq!(
                h.observe(obs(n)),
                ProgressEvent::Quiet,
                "advancing head emitted at block {n}"
            );
        }
        assert_eq!(h.phase(), Phase::Advancing);
    }

    /// A frozen head with nothing declined is idle, not stalled. A devnet
    /// between slots and a node with no peers both land here and neither is
    /// halted.
    #[test]
    fn a_frozen_head_with_no_refusals_is_idle_not_stalled() {
        let h = ChainHealth::manual();
        h.observe(obs(42));
        h.advance_clock(NO_PROGRESS_AFTER + Duration::from_secs(1));
        let event = h.observe(obs(42));
        assert_eq!(
            event,
            ProgressEvent::Idle {
                head: 42,
                unchanged_for: NO_PROGRESS_AFTER.as_secs() + 1,
                last_fcu_ago: None,
            }
        );
        assert_eq!(h.phase(), Phase::Idle);
    }

    /// Below the threshold nothing is said, however many ticks pass.
    #[test]
    fn a_briefly_frozen_head_says_nothing() {
        let h = ChainHealth::manual();
        h.observe(obs(42));
        let mut elapsed = Duration::ZERO;
        while elapsed + PROGRESS_TICK < NO_PROGRESS_AFTER {
            h.advance_clock(PROGRESS_TICK);
            elapsed += PROGRESS_TICK;
            assert_eq!(
                h.observe(obs(42)),
                ProgressEvent::Quiet,
                "emitted after only {elapsed:?}"
            );
        }
        assert!(elapsed > Duration::ZERO, "loop never ran");
    }

    /// The flip-block halt: forkchoice updates arrive, get declined, the head
    /// does not move. That is a stall and it is an ERROR-level one, because the
    /// node had synced.
    #[test]
    fn refusals_against_a_frozen_head_are_a_stall() {
        let h = ChainHealth::manual();
        h.observe(obs(100));
        h.record_forkchoice_update();
        h.record_refusal(
            RefusalKind::StateNotReachable,
            "State root of the new head is not reachable from the database",
        );
        h.advance_clock(NO_PROGRESS_AFTER + Duration::from_secs(5));
        h.record_forkchoice_update();
        h.record_refusal(
            RefusalKind::StateNotReachable,
            "State root of the new head is not reachable from the database",
        );

        let event = h.observe(obs(100));
        assert_eq!(
            event,
            ProgressEvent::Stalled {
                head: 100,
                unchanged_for: NO_PROGRESS_AFTER.as_secs() + 5,
                refusals: 2,
                last_refusal: Refusal {
                    kind: RefusalKind::StateNotReachable,
                    detail: "State root of the new head is not reachable from the database"
                        .to_string(),
                },
                last_fcu_ago: Some(0),
                synced: true,
            }
        );
        assert_eq!(h.phase(), Phase::Stalled);
    }

    /// A stall keeps saying so, at its own cadence rather than every tick. The
    /// halt case is exactly the one where a single line scrolls away.
    #[test]
    fn a_stall_repeats_at_its_own_cadence_not_every_tick() {
        let h = ChainHealth::manual();
        h.observe(obs(7));
        h.record_forkchoice_update();
        h.record_refusal(RefusalKind::StateNotReachable, "detail");
        h.advance_clock(NO_PROGRESS_AFTER);
        assert!(matches!(h.observe(obs(7)), ProgressEvent::Stalled { .. }));

        // Ticks inside the repeat window stay quiet.
        let mut since_emit = Duration::ZERO;
        let mut quiet_ticks = 0;
        while since_emit + PROGRESS_TICK < STALLED_REPEAT {
            h.advance_clock(PROGRESS_TICK);
            since_emit += PROGRESS_TICK;
            assert_eq!(
                h.observe(obs(7)),
                ProgressEvent::Quiet,
                "stall re-emitted after only {since_emit:?}"
            );
            quiet_ticks += 1;
        }
        assert!(quiet_ticks >= 3, "expected quiet ticks, got {quiet_ticks}");

        // Crossing the window re-states it, with the accumulated elapsed time.
        h.advance_clock(PROGRESS_TICK);
        match h.observe(obs(7)) {
            ProgressEvent::Stalled { unchanged_for, .. } => {
                assert!(
                    unchanged_for >= NO_PROGRESS_AFTER.as_secs() + STALLED_REPEAT.as_secs(),
                    "stall repeat reported unchanged_for={unchanged_for}, \
                     expected at least {}",
                    NO_PROGRESS_AFTER.as_secs() + STALLED_REPEAT.as_secs()
                );
            }
            other => panic!("expected a repeated stall, got {other:?}"),
        }
    }

    /// Idle repeats far more slowly than a stall does. A healthy node must not
    /// become noisy.
    #[test]
    fn idle_repeats_far_more_slowly_than_a_stall() {
        assert!(
            IDLE_REPEAT >= STALLED_REPEAT * 5,
            "IDLE_REPEAT {IDLE_REPEAT:?} is not meaningfully slower than STALLED_REPEAT {STALLED_REPEAT:?}"
        );
        let h = ChainHealth::manual();
        h.observe(obs(1));
        h.advance_clock(NO_PROGRESS_AFTER);
        assert!(matches!(h.observe(obs(1)), ProgressEvent::Idle { .. }));

        // A whole stall-repeat window later, still nothing.
        h.advance_clock(STALLED_REPEAT);
        assert_eq!(h.observe(obs(1)), ProgressEvent::Quiet);

        h.advance_clock(IDLE_REPEAT);
        assert!(matches!(h.observe(obs(1)), ProgressEvent::Idle { .. }));
    }

    /// An idle node that then starts declining escalates to a stall
    /// immediately, without waiting out the idle repeat window.
    #[test]
    fn idle_escalates_to_stalled_on_the_first_refusal() {
        let h = ChainHealth::manual();
        h.observe(obs(5));
        h.advance_clock(NO_PROGRESS_AFTER);
        assert!(matches!(h.observe(obs(5)), ProgressEvent::Idle { .. }));

        h.advance_clock(PROGRESS_TICK);
        h.record_forkchoice_update();
        h.record_refusal(RefusalKind::UnlinkedHead, "no link to canonical chain");
        match h.observe(obs(5)) {
            ProgressEvent::Stalled {
                refusals,
                last_refusal,
                ..
            } => {
                assert_eq!(refusals, 1);
                assert_eq!(last_refusal.kind, RefusalKind::UnlinkedHead);
            }
            other => panic!("expected escalation to Stalled, got {other:?}"),
        }
    }

    /// A node that has not completed a sync gets the same facts without the
    /// halt claim: a header download can legitimately hold the head still while
    /// forkchoice updates are declined.
    #[test]
    fn an_unsynced_node_reports_the_stall_without_claiming_a_halt() {
        let h = ChainHealth::manual();
        let unsynced = ProgressObservation {
            head: 3,
            synced: false,
        };
        h.observe(unsynced);
        h.record_forkchoice_update();
        h.record_refusal(RefusalKind::Syncing, "The node has not finished syncing.");
        h.advance_clock(NO_PROGRESS_AFTER);
        match h.observe(unsynced) {
            ProgressEvent::Stalled { synced, .. } => assert!(!synced),
            other => panic!("expected Stalled, got {other:?}"),
        }
    }

    /// The head moving clears the refusal count. Otherwise a node that stumbled
    /// once and recovered would be reported as stalled for ever after.
    #[test]
    fn a_moving_head_clears_refusals() {
        let h = ChainHealth::manual();
        h.observe(obs(1));
        h.record_forkchoice_update();
        h.record_refusal(RefusalKind::StateNotReachable, "detail");
        assert_eq!(h.refusals_at_current_head(), 1);

        h.advance_clock(PROGRESS_TICK);
        assert_eq!(h.observe(obs(2)), ProgressEvent::Quiet);
        assert_eq!(h.refusals_at_current_head(), 0);

        // And a later freeze at the new head is idle, not stalled.
        h.advance_clock(NO_PROGRESS_AFTER);
        assert!(matches!(h.observe(obs(2)), ProgressEvent::Idle { .. }));
    }

    /// Recovery is announced once, and says whether what it recovered from was
    /// a stall.
    #[test]
    fn recovery_from_a_stall_is_announced_once() {
        let h = ChainHealth::manual();
        h.observe(obs(1));
        h.record_forkchoice_update();
        h.record_refusal(RefusalKind::StateNotReachable, "detail");
        h.advance_clock(NO_PROGRESS_AFTER);
        assert!(matches!(h.observe(obs(1)), ProgressEvent::Stalled { .. }));

        h.advance_clock(PROGRESS_TICK);
        assert_eq!(
            h.observe(obs(2)),
            ProgressEvent::Resumed {
                head: 2,
                was_still_for: NO_PROGRESS_AFTER.as_secs() + PROGRESS_TICK.as_secs(),
                was_stalled: true,
            }
        );
        // Said once.
        h.advance_clock(PROGRESS_TICK);
        assert_eq!(h.observe(obs(3)), ProgressEvent::Quiet);
    }

    /// Recovery from a plain idle spell is distinguishable from recovery from a
    /// stall, so a log reader learns which happened.
    #[test]
    fn recovery_from_idle_is_not_reported_as_a_stall_recovery() {
        let h = ChainHealth::manual();
        h.observe(obs(1));
        h.advance_clock(NO_PROGRESS_AFTER);
        assert!(matches!(h.observe(obs(1)), ProgressEvent::Idle { .. }));
        h.advance_clock(PROGRESS_TICK);
        match h.observe(obs(2)) {
            ProgressEvent::Resumed { was_stalled, .. } => assert!(!was_stalled),
            other => panic!("expected Resumed, got {other:?}"),
        }
    }

    /// A head that goes *backwards* (a reorg to a shorter chain) counts as
    /// movement. It is not a halt, and reporting it as one would be a signal
    /// that is wrong.
    #[test]
    fn a_head_moving_backwards_counts_as_movement() {
        let h = ChainHealth::manual();
        h.observe(obs(100));
        h.advance_clock(NO_PROGRESS_AFTER);
        assert!(matches!(h.observe(obs(100)), ProgressEvent::Idle { .. }));
        h.advance_clock(PROGRESS_TICK);
        match h.observe(obs(95)) {
            ProgressEvent::Resumed { head, .. } => assert_eq!(head, 95),
            other => panic!("expected Resumed on a backwards head, got {other:?}"),
        }
        assert_eq!(h.phase(), Phase::Advancing);
    }

    /// A node with no consensus client attached reports "never" for the last
    /// forkchoice update rather than a fabricated age.
    #[test]
    fn a_node_that_never_saw_a_forkchoice_update_reports_never() {
        let h = ChainHealth::manual();
        h.observe(obs(0));
        h.advance_clock(NO_PROGRESS_AFTER);
        match h.observe(obs(0)) {
            ProgressEvent::Idle { last_fcu_ago, .. } => assert_eq!(last_fcu_ago, None),
            other => panic!("expected Idle, got {other:?}"),
        }
    }

    /// The age of the last forkchoice update is a real elapsed time, not a
    /// constant. Distinguishes "the CL stopped talking to us" from "the CL is
    /// talking to us and we keep saying no".
    #[test]
    fn last_forkchoice_update_age_tracks_elapsed_time() {
        let h = ChainHealth::manual();
        h.observe(obs(0));
        h.record_forkchoice_update();
        h.advance_clock(NO_PROGRESS_AFTER + Duration::from_secs(30));
        match h.observe(obs(0)) {
            ProgressEvent::Idle { last_fcu_ago, .. } => {
                assert_eq!(last_fcu_ago, Some(NO_PROGRESS_AFTER.as_secs() + 30));
            }
            other => panic!("expected Idle, got {other:?}"),
        }
    }

    /// The one refusal that does not count against progress is an initial
    /// sync's own self-report; everything else counts either way.
    #[test]
    fn only_an_unsynced_nodes_syncing_answer_is_uncounted() {
        assert!(!countable_refusal(RefusalKind::Syncing, false));
        assert!(countable_refusal(RefusalKind::Syncing, true));
        for kind in [
            RefusalKind::StateNotReachable,
            RefusalKind::UnlinkedHead,
            RefusalKind::TooDeepReorg,
            RefusalKind::InvalidHead,
            RefusalKind::Inconsistent,
            RefusalKind::StoreError,
        ] {
            assert!(
                countable_refusal(kind, false),
                "{} was dropped on an unsynced node",
                kind.as_str()
            );
            assert!(
                countable_refusal(kind, true),
                "{} was dropped on a synced node",
                kind.as_str()
            );
        }
    }

    /// Refusal kinds keep distinct, stable names; a log filter is built on
    /// these.
    #[test]
    fn refusal_kind_names_are_distinct() {
        let kinds = [
            RefusalKind::StateNotReachable,
            RefusalKind::Syncing,
            RefusalKind::UnlinkedHead,
            RefusalKind::TooDeepReorg,
            RefusalKind::InvalidHead,
            RefusalKind::Inconsistent,
            RefusalKind::StoreError,
        ];
        let mut names: Vec<&str> = kinds.iter().map(|k| k.as_str()).collect();
        names.sort_unstable();
        let count = names.len();
        names.dedup();
        assert_eq!(names.len(), count, "duplicate RefusalKind names: {names:?}");
        assert!(names.iter().all(|n| !n.is_empty()));
    }
}
