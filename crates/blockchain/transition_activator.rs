//! Automatic MPT→binary trie transition activator.
//!
//! [`TransitionActivator`] is constructed when `--binary-transition` is passed
//! at startup and ticks after each successful block commit. When both
//! preconditions hold simultaneously — `snap_enabled == false` (snap sync
//! done) and `caught_up == true` (follower reached finalized head) — it calls
//! [`activate`](TransitionActivator::activate), which:
//!
//! 1. Acquires the activation lock (blocks concurrent block execution).
//! 2. Re-verifies the preconditions inside the lock.
//! 3. Force-flushes the MPT layer cache to disk.
//! 4. Writes the three transition metadata keys + format byte 2 atomically.
//! 5. Updates the in-memory backend kind to `Transition` (hot-swap).
//! 6. Logs a prominent message.
//!
//! The node continues running in Transition mode — no restart required.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use ethrex_binary_trie::EMPTY_BINARY_ROOT;
use ethrex_common::H256;
use ethrex_state_backend::BackendKind;
use ethrex_storage::{Store, error::StoreError};
use tracing::{error, info, warn};

/// Returned by [`TransitionActivator::tick`] to indicate whether the
/// activation fired or was skipped.
#[derive(Debug, PartialEq, Eq)]
pub enum TickResult {
    /// Preconditions not yet met, or activation already completed.
    Skip,
    /// Activation fired; the node continues running in Transition mode.
    Activate(u64),
}

/// Observer that polls snap_enabled + caught_up after each committed block
/// and fires the one-way MPT→binary transition once when both are true.
#[derive(Debug)]
pub struct TransitionActivator {
    snap_enabled: Arc<AtomicBool>,
    caught_up: Arc<AtomicBool>,
}

impl TransitionActivator {
    /// Creates a new activator.
    ///
    /// - `snap_enabled`: shared with `SyncManager`; reads whether snap sync
    ///   is still active.
    /// - `caught_up`: shared with `SyncManager`; one-shot latch that fires
    ///   once the follower has reached or exceeded the CL-finalized head.
    pub fn new(snap_enabled: Arc<AtomicBool>, caught_up: Arc<AtomicBool>) -> Self {
        Self {
            snap_enabled,
            caught_up,
        }
    }

    /// Checks the two activation preconditions and fires [`activate`] when both
    /// are simultaneously true.
    ///
    /// - Returns [`TickResult::Skip`] when:
    ///   - snap sync is still running (`snap_enabled == true`), or
    ///   - the follower has not caught up yet (`caught_up == false`), or
    ///   - the DB format byte is already `2` (idempotent: already activated).
    /// - Returns [`TickResult::Activate(block_number)`] when activation fired.
    ///
    /// `head_state_root` MUST be the `state_root` field of the block at
    /// `head_block_number` that was just committed by the caller (typically
    /// `Blockchain::execute_block_pipeline` followed by `store_block`). It is
    /// the source of truth for `frozen_mpt_root` — `LatestBlockNumber` and
    /// `Store::latest_block_header` are NOT, because both are advanced by
    /// `apply_fork_choice` (engine_forkchoiceUpdated from the CL), not by
    /// block execution. Using either of those produces a one-block-stale
    /// frozen root and breaks post-switch reads (Bug 3, hoodi 2026-05-05).
    pub fn tick(&self, store: &Store, head_block_number: u64, head_state_root: H256) -> TickResult {
        // Fast path: already activated (format byte 2 on disk).
        match store.read_state_backend_format_byte() {
            Ok(Some(2)) => return TickResult::Skip,
            Ok(_) => {}
            Err(e) => {
                warn!("TransitionActivator: failed to read format byte: {e}");
                return TickResult::Skip;
            }
        }

        // Check preconditions.
        if self.snap_enabled.load(Ordering::Acquire) {
            return TickResult::Skip;
        }
        if !self.caught_up.load(Ordering::Acquire) {
            return TickResult::Skip;
        }

        // Both preconditions met — activate.
        match self.activate(store, head_block_number, head_state_root) {
            Ok(()) => TickResult::Activate(head_block_number),
            Err(e) => {
                error!("TransitionActivator: activation failed (will retry on next block): {e}");
                TickResult::Skip
            }
        }
    }

    /// Executes the activation sequence (plan §6 Task 7.4, revised for hot-swap).
    ///
    /// Steps executed in order:
    /// 0. Acquire `activation_lock` (blocks concurrent `execute_block_pipeline`).
    /// 1. Re-verify preconditions inside the lock.
    /// 2. Send `FKVGeneratorControlMessage::Stop` to the MPT FKV generator
    ///    (binary/transition stores have no FKV generator; the call is a no-op
    ///    for them).
    /// 3. Drain the `trie_update_worker` queue by sending a flush sentinel,
    ///    ensuring the worker has applied block N's diffs to the layer cache.
    /// 4. Force-flush the MPT `TrieLayerCache` to disk walking from
    ///    `head_state_root` (NOT `Store::latest_block_header`) via
    ///    `store.force_commit_layers(head_state_root)`. Walking from the
    ///    forkchoice-tracked root would skip block N's layer.
    /// 5. Use the caller-provided `head_state_root` as `frozen_mpt_root`.
    /// 6. Set `binary_root` = `EMPTY_BINARY_ROOT` (`H256([0u8; 32])`).
    /// 7. Persist metadata atomically (`persist_transition_metadata`), which also
    ///    updates the in-memory `transition_metadata` RwLock.
    /// 8. Set `backend_kind` to `Transition` in-memory (hot-swap; no restart needed).
    /// 9. Log the activation message.
    fn activate(
        &self,
        store: &Store,
        head_block_number: u64,
        head_state_root: H256,
    ) -> Result<(), StoreError> {
        // Step 0: Acquire the activation lock.
        let activation_lock = store.activation_lock();
        let _guard = activation_lock
            .lock()
            .map_err(|_| StoreError::Custom("activation_lock poisoned".to_string()))?;

        // Step 1: Re-verify preconditions inside the lock.
        if self.snap_enabled.load(Ordering::Acquire) {
            // Re-checked under the lock; tick() already verified, but this is
            // the canonical re-verification per plan §6 Task 7.4 step 1.
            // snap_enabled is one-way (true → false), so this branch does not
            // retrigger once it has been cleared.
            return Ok(());
        }
        if !self.caught_up.load(Ordering::Acquire) {
            // Re-checked under the lock.  caught_up is a one-shot latch; this
            // branch can only fire if the latch had not yet been set between
            // tick()'s pre-check and the lock acquisition.  Acceptable; activate
            // will retry on the next tick.
            return Ok(());
        }
        // Check idempotency: if already activated, skip.
        if let Ok(Some(2)) = store.read_state_backend_format_byte() {
            return Ok(());
        }

        // Step 2: Stop the MPT FKV generator.
        // `stop_fkv_generator` is a best-effort call; ignored if disconnected.
        store.stop_fkv_generator();

        // Step 3 (was 4): Drain the trie_update_worker queue FIRST.
        //
        // execute_block_pipeline → store_block_updates dispatches block N's diffs
        // to the worker; the worker's Phase 1 (cache update via put_batch) may
        // still be running when we get here. Force-committing before the cache
        // contains block N's layer would silently drop those diffs to disk.
        // Drain ensures the worker has acked Phase 1 for everything in flight,
        // so the cache is up to date with block N before we walk it.
        store.drain_trie_update_worker()?;

        // Step 4 (was 3): Force-flush the MPT TrieLayerCache to disk, walking
        // from the just-committed head root (NOT Store::latest_block_header,
        // which is advanced by apply_fork_choice and lags). Walking from the
        // stale forkchoice-tracked root would skip block N's layer entirely
        // because that layer is keyed by N's root and is a *child* of the
        // stale root, never reached via an ancestor walk.
        store.force_commit_layers(head_state_root)?;

        // Step 5: Use the caller-provided state_root of the block we just
        // committed. CHAIN_DATA::LatestBlockNumber and Store::latest_block_header
        // are both advanced by apply_fork_choice (engine_forkchoiceUpdated from
        // the CL), not by block execution, so reading from either of them here
        // yields a one-block-stale frozen root during catchup. The caller —
        // execute_block_pipeline followed by store_block — has the canonical
        // state root for the block at head_block_number, so it passes it in.
        let frozen_mpt_root = head_state_root;

        // Step 6: Initial binary_root is EMPTY_BINARY_ROOT.
        let binary_root = EMPTY_BINARY_ROOT;

        // Step 7: Persist metadata atomically (format byte 2 + 3 meta keys).
        // persist_transition_metadata also updates the in-memory RwLock.
        // switch_block = head_block_number + 1 (first block whose execution
        // writes go to the binary overlay).
        let switch_block = head_block_number.saturating_add(1);
        store.persist_transition_metadata(switch_block, frozen_mpt_root, binary_root)?;

        // Step 8: Hot-swap the in-memory backend kind to Transition so that
        // subsequent block executions immediately use the transition reader
        // without requiring a restart.
        store.set_backend_kind(BackendKind::Transition);

        // Step 9: Log a clear activation message.
        info!(
            switch_block,
            frozen_mpt_root = %format!("{frozen_mpt_root:#x}"),
            "Binary trie transition activated at block {}. Frozen MPT root: {:#x}. \
             Node continues running in Transition mode.",
            switch_block,
            frozen_mpt_root,
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    use ethrex_common::H256;
    use ethrex_state_backend::BackendKind;
    use ethrex_storage::{EngineType, Store};

    use super::{TickResult, TransitionActivator};

    fn make_activator(
        snap_enabled: bool,
        caught_up: bool,
    ) -> (TransitionActivator, Arc<AtomicBool>, Arc<AtomicBool>) {
        let snap = Arc::new(AtomicBool::new(snap_enabled));
        let caught = Arc::new(AtomicBool::new(caught_up));
        let activator = TransitionActivator::new(Arc::clone(&snap), Arc::clone(&caught));
        (activator, snap, caught)
    }

    /// When both preconditions are satisfied (snap done, caught up), the first
    /// tick must write the four metadata keys, hot-swap backend_kind to Transition,
    /// and return Activate.
    #[test]
    fn binary_transition_auto_activation() {
        let store = Store::new(".", EngineType::InMemory, BackendKind::Mpt)
            .expect("failed to open in-memory store");

        let (activator, _snap, _caught) = make_activator(false, true);

        // Caller (execute_block_pipeline → store_block) supplies the state_root
        // of the block it just committed at head_block_number=100.
        let head_state_root = H256::repeat_byte(0xAA);
        let result = activator.tick(&store, 100, head_state_root);

        assert_eq!(result, TickResult::Activate(100), "expected Activate(100)");

        // Format byte 2 must be on disk.
        let format_byte = store
            .read_state_backend_format_byte()
            .expect("read_state_backend_format_byte failed");
        assert_eq!(
            format_byte,
            Some(2),
            "format byte must be 2 (Transition) after activation"
        );

        // In-memory backend_kind must be Transition (hot-swap).
        assert_eq!(
            store.backend_kind(),
            BackendKind::Transition,
            "backend_kind must be Transition after hot-swap"
        );

        // In-memory transition_metadata must be set (updated by persist_transition_metadata).
        let meta_in_memory = store
            .transition_metadata()
            .expect("transition_metadata must be set in-memory after activation");

        // Also verify disk matches.
        let meta_disk = store
            .load_transition_metadata()
            .expect("load_transition_metadata failed")
            .expect("transition metadata must be present on disk");

        // switch_block = head_block_number + 1
        assert_eq!(meta_disk.0, 101, "switch_block must be head+1");
        assert_eq!(
            meta_in_memory.0, 101,
            "in-memory switch_block must be head+1"
        );

        // frozen_mpt_root must equal the caller-provided head_state_root, NOT
        // anything looked up from CHAIN_DATA::LatestBlockNumber. This is the
        // contract that prevents Bug 3 (stale frozen root during catchup).
        assert_eq!(
            meta_disk.1, head_state_root,
            "frozen_mpt_root must equal caller-provided head_state_root"
        );
        assert_eq!(
            meta_in_memory.1, head_state_root,
            "in-memory frozen_mpt_root must match disk"
        );
        assert_eq!(
            meta_disk.2,
            ethrex_binary_trie::EMPTY_BINARY_ROOT,
            "binary_root must be EMPTY_BINARY_ROOT on fresh activation"
        );
        assert_eq!(
            meta_in_memory.2,
            ethrex_binary_trie::EMPTY_BINARY_ROOT,
            "in-memory binary_root must be EMPTY_BINARY_ROOT"
        );
    }

    /// Bug 3 regression — activate() must use the caller-provided head_state_root,
    /// not anything derived from CHAIN_DATA::LatestBlockNumber or
    /// Store::latest_block_header (both of which lag during catchup because they
    /// are advanced by apply_fork_choice on engine_forkchoiceUpdated, not by
    /// block execution).
    ///
    /// Reproduction: simulate the live hoodi 2026-05-05 condition where the CL
    /// finalized hash points at block N-1 (so LatestBlockNumber stays at N-1)
    /// but the EL has already executed block N. tick(store, N, root_N) must
    /// freeze root_N, not root_(N-1).
    #[test]
    fn activator_uses_caller_state_root_when_chain_data_lags() {
        let store = Store::new(".", EngineType::InMemory, BackendKind::Mpt)
            .expect("failed to open in-memory store");

        // Simulate the stale view CHAIN_DATA::LatestBlockNumber would expose:
        // forkchoice has only declared block 99 canonical so far.
        store
            .test_set_latest_block_number(99)
            .expect("test_set_latest_block_number failed");

        // But the EL just finished executing block 100 via execute_block_pipeline,
        // and that block's state_root is root_100. Caller passes it to tick().
        let root_99_stale = H256::zero(); // value LatestBlockNumber would resolve to
        let root_100 = H256::repeat_byte(0xCC);
        assert_ne!(
            root_99_stale, root_100,
            "test precondition: roots must differ to make the bug observable"
        );

        let (activator, _snap, _caught) = make_activator(false, true);
        let result = activator.tick(&store, 100, root_100);
        assert_eq!(result, TickResult::Activate(100));

        let (switch_block, frozen_mpt_root, _) = store
            .transition_metadata()
            .expect("transition_metadata must be set");

        // switch_block aligns with head_block_number passed to tick.
        assert_eq!(
            switch_block, 101,
            "switch_block must be head+1, not LatestBlockNumber+1"
        );
        // frozen_mpt_root MUST be the caller-provided root (block 100), not the
        // root LatestBlockNumber would resolve to (block 99 ⇒ H256::zero()).
        // Pre-Bug-3-fix this assertion would fail because the activator looked
        // up frozen_mpt_root via get_latest_canonical_block_header().
        assert_eq!(
            frozen_mpt_root, root_100,
            "frozen_mpt_root must track caller's head_state_root, not stale CHAIN_DATA"
        );
    }

    // Plan §6 Task 7.6 — binary_transition_restart_cycle.
    //
    // This test requires opening two Store instances against the same physical
    // backend (shared Arc<InMemoryBackend>) which requires the pub(crate)
    // Store::from_backend API.  It lives in ethrex-storage's transition_wiring
    // test module:
    //   crates/storage/transition_wiring.rs::tests::binary_transition_restart_cycle

    /// When `caught_up` is false the activator must skip regardless of snap
    /// state. Once `caught_up` is latched true, the very next tick fires.
    #[test]
    fn binary_transition_waits_for_caught_up() {
        let store = Store::new(".", EngineType::InMemory, BackendKind::Mpt)
            .expect("failed to open in-memory store");

        let (activator, snap, caught) = make_activator(false, false);

        // snap_enabled=false but caught_up=false → must skip.
        let head_state_root = H256::repeat_byte(0xBB);
        let r1 = activator.tick(&store, 10, head_state_root);
        assert_eq!(r1, TickResult::Skip, "must Skip when caught_up is false");
        // backend_kind still Mpt after Skip.
        assert_eq!(store.backend_kind(), BackendKind::Mpt);

        // Flip caught_up to true.
        caught.store(true, Ordering::Release);
        // snap is already false, both preconditions now met.
        drop(snap); // explicit: snap_enabled remains false

        let r2 = activator.tick(&store, 11, head_state_root);
        assert_eq!(
            r2,
            TickResult::Activate(11),
            "must Activate once caught_up is latched"
        );
        // After activation: backend_kind == Transition (hot-swap).
        assert_eq!(
            store.backend_kind(),
            BackendKind::Transition,
            "backend_kind must be Transition after activation"
        );
        assert!(
            store.transition_metadata().is_some(),
            "transition_metadata must be set after activation"
        );
    }

    // Plan §6 Task 7.9 — binary_transition_locked_without_flag.
    //
    // This test exercises Store::from_backend returning StoreError::Custom when
    // format byte 2 is on disk and BackendKind::Mpt is requested (flag absent).
    // It lives in ethrex-storage's backend_format_tests module where
    // from_backend is accessible:
    //   crates/storage/store.rs::backend_format_tests::binary_transition_locked_without_flag

    /// Plan §6 Task 7.8 (part 1) — `Blockchain::new` always initialises
    /// `transition_activator` to `None`.
    ///
    /// This is the **constructor invariant**: the activator field starts empty
    /// regardless of any external flag.  The flag-driven wiring lives in
    /// `cmd/ethrex/initializers.rs` (binary crate) and cannot be unit-tested at
    /// this library layer without pulling in unrelated binary-only dependencies.
    /// See part 2 below for the complementary setter test.
    #[test]
    fn transition_activator_starts_none() {
        use crate::{Blockchain, BlockchainOptions};

        let store = Store::new(".", EngineType::InMemory, BackendKind::Mpt)
            .expect("failed to open in-memory store");

        let bc = Blockchain::new(store, BlockchainOptions::default());

        assert!(
            bc.transition_activator.try_lock().unwrap().is_none(),
            "Blockchain::new must leave transition_activator as None; \
             set_transition_activator is called only when --binary-transition is present"
        );
    }

    /// Plan §6 Task 7.8 (part 2) — `set_transition_activator` populates the field.
    ///
    /// This proves that `set_transition_activator` actually installs the
    /// activator, completing the bidirectional test: `None` before the call,
    /// `Some` after.  Together with part 1 this gives meaningful coverage of the
    /// `transition_activator` field lifecycle.
    ///
    /// The actual flag → `set_transition_activator` wiring is in
    /// `cmd/ethrex/initializers.rs` (binary crate); that path requires
    /// `SyncManager`, `CancellationToken`, and `Options` and cannot be tested in
    /// this library crate without introducing circular or binary-only
    /// dependencies.
    #[test]
    fn set_transition_activator_installs_activator() {
        use crate::{Blockchain, BlockchainOptions};

        let store = Store::new(".", EngineType::InMemory, BackendKind::Mpt)
            .expect("failed to open in-memory store");

        let bc = Blockchain::new(store, BlockchainOptions::default());

        // Confirm starts empty.
        assert!(
            bc.transition_activator.try_lock().unwrap().is_none(),
            "precondition: must start None"
        );

        // Install an activator via the public setter.
        let (activator, _snap, _caught) = make_activator(false, false);
        bc.set_transition_activator(activator);

        // Confirm it's now Some.
        assert!(
            bc.transition_activator.try_lock().unwrap().is_some(),
            "set_transition_activator must install the activator (Some after call)"
        );
    }
}
