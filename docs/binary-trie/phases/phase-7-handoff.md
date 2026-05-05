# Phase 7 Handoff

**Branch**: shared-trie-binary
**Date**: 2026-05-04

## Summary

Phase 7 wires the MPT-to-binary transition activation into the live node. It adds the
`--binary-transition` CLI flag, a `TransitionActivator` component that monitors sync
state and fires the one-way activation once both preconditions are met, and the
infrastructure (activation lock, force-flush helpers, process shutdown signal) that
makes the activation safe under concurrent block execution.

Activation is restart-required: the activator writes four DB keys atomically (format
byte 2 + three transition metadata keys), then calls `CancellationToken::cancel()` to
exit the process gracefully. The operator relaunches with `--binary-transition` and
the node resumes in `Transition` mode.

## Tasks completed

- [x] Task 7.1: `--binary-transition` flag added to `Options` struct in
  `cmd/ethrex/cli.rs` with `ArgAction::SetTrue`, `env = "ETHREX_BINARY_TRANSITION"`,
  and descriptive help text. Default is `false`.

- [x] Task 7.2: `SyncManager` in `crates/networking/p2p/sync_manager.rs` extended with
  `last_fcu_finalized: Arc<Mutex<(H256, u64)>>` and `caught_up: Arc<AtomicBool>` fields.
  New public methods: `snap_enabled()`, `caught_up()`, `is_caught_up()`,
  `last_fcu_finalized()`, `update_fcu_finalized()`, `check_and_latch_caught_up()`.
  The latch uses `Ordering::Release` on store and `Ordering::Acquire` on load, and only
  fires when `finalized_number > 0`.

- [x] Task 7.3 / Task 7.4 (plan): `handle_forkchoice` in `engine/fork_choice.rs` calls
  `syncer.update_fcu_finalized(hash, number)` on every non-zero finalized hash.
  `try_execute_payload` in `engine/payload.rs` calls
  `syncer.check_and_latch_caught_up(block_number)` after a successful block commit.

- [x] Task 7.4 (plan): `TransitionActivator` created in
  `crates/blockchain/transition_activator.rs`. `tick()` does a fast-path format-byte
  check then verifies both preconditions before calling `activate()`. `activate()`
  acquires the activation lock, re-verifies inside the lock, calls
  `stop_fkv_generator()`, `force_commit_layers()`, `drain_trie_update_worker()`, reads
  the frozen MPT root from the latest block header, writes `EMPTY_BINARY_ROOT`, persists
  metadata atomically, logs the activation message, and calls `cancel_token.cancel()`.

- [x] Task 7.5 (plan): `Store` in `crates/storage/store.rs` extended with:
  - `activation_lock: Arc<Mutex<()>>` field (initialized in `from_backend`)
  - `activation_lock()` public getter returning `Arc<Mutex<()>>`
  - `stop_fkv_generator()` best-effort FKV stop
  - `force_commit_layers()` drains the TrieLayerCache to disk using threshold=1
  - `drain_trie_update_worker()` sends a flush sentinel and waits for ack
  - `get_latest_canonical_block_header()` convenience wrapper
  - `peek_backend_format_byte(path, engine_type)` static function for pre-open format
    inspection (rocksdb-gated, returns `Ok(None)` for InMemory)
  - Format-byte mismatch detection: byte 2 + `BackendKind::Mpt` returns
    `StoreError::Custom` with message "format byte 2 (transition) but
    --binary-transition was not passed"
  - `ethrex-binary-trie` added as a workspace dependency in
    `crates/blockchain/Cargo.toml` for the `EMPTY_BINARY_ROOT` constant import.

- [x] Task 7.6 (plan) / `binary_transition_restart_cycle`: `Blockchain` struct in
  `crates/blockchain/blockchain.rs` extended with:
  - `pub transition_activator: std::sync::Mutex<Option<TransitionActivator>>` field
  - `pub fn store() -> &Store` getter
  - `pub fn set_transition_activator(activator)` setter (used once at startup)
  - `execute_block_pipeline` acquires `activation_lock` before execution (bound to a
    local `Arc` variable to avoid the temporary-value-dropped borrow error)
  - `add_block_pipeline_inner` calls `activator.tick(&self.storage, block_number)` via
    `try_lock` after each successful block store

- [x] Task 7.7 (plan): `cmd/ethrex/initializers.rs` updated:
  - `open_store()` accepts `backend_kind: BackendKind` parameter
  - `peek_backend_format_byte()` helper calls `Store::peek_backend_format_byte`
  - `init_l1`: peeks format byte to choose `BackendKind::Transition` when byte==2 and
    `--binary-transition` is set; creates `SyncManager` before `init_rpc_api` (so its
    `Arc<AtomicBool>` handles are available); constructs `TransitionActivator` when
    `binary_transition && backend_kind == BackendKind::Mpt`; installs activator via
    `blockchain.set_transition_activator`
  - `init_rpc_api` signature accepts `syncer: SyncManager` (caller-provided, removing
    internal SyncManager creation)

- [x] Task 7.5 (plan) / `binary_transition_auto_activation`: integration test in
  `crates/blockchain/transition_activator.rs::tests`. snap=false, caught_up=true
  activates on first tick: returns `Activate(100)`, format byte=2, switch_block=101,
  cancel_token fired.

- [x] Task 7.6 (plan) / `binary_transition_restart_cycle`: integration test in
  `crates/storage/transition_wiring.rs::tests`. Opens a shared `Arc<InMemoryBackend>`;
  first Store (MPT) seeds an account and activates via `persist_transition_metadata`;
  second Store opened from the same backend with `BackendKind::Transition` reports
  `backend_kind == Transition` and `transition_metadata` loaded from disk; reads via
  `new_transition_state_reader` confirm overlay→base ordering: base-only account
  readable from MPT, overlay write shadows base.

- [x] Task 7.7 (plan) / `binary_transition_waits_for_caught_up`: integration test in
  `crates/blockchain/transition_activator.rs::tests`. caught_up=false returns `Skip`;
  flip to true triggers `Activate(11)` on next tick.

- [x] Task 7.8 (plan) — bidirectional test suite in
  `crates/blockchain/transition_activator.rs::tests` (round 3, see Deviations below):
  - `transition_activator_starts_none`: `Blockchain::new` always initialises
    `transition_activator` to `None` (constructor invariant).
  - `set_transition_activator_installs_activator`: `set_transition_activator` transitions
    the field from `None` to `Some`, proving the public setter works.
  The actual `--binary-transition` flag → `set_transition_activator` wiring lives in
  `cmd/ethrex/initializers.rs` (binary crate) and cannot be directly unit-tested in this
  library crate without pulling in `SyncManager`, `CancellationToken`, and `Options`.

- [x] Task 7.9 (plan) / `binary_transition_locked_without_flag`: integration test in
  `crates/storage/store.rs::backend_format_tests`. Persists format byte 2 via
  `persist_transition_metadata`, then re-opens the same `Arc<InMemoryBackend>` with
  `BackendKind::Mpt`. Asserts `Store::from_backend` returns `Err(StoreError::Custom(_))`
  with message containing "format byte 2 (transition) but --binary-transition was not
  passed".

## Design notes

**Restart-required activation**: The activator does not hot-swap the trie backend.
Instead it writes the transition metadata and exits the process. This avoids the need
to atomically swap the `Store`'s inner backend under live traffic, at the cost of a
single expected restart.

**Activation lock**: A `Mutex<()>` shared between `execute_block_pipeline` (held for
the full block execution + store commit) and `TransitionActivator::activate` (held
during metadata write) prevents metadata from being written mid-block.

**Caught-up latch**: A one-shot `Arc<AtomicBool>` set with `Ordering::Release` by
`check_and_latch_caught_up` (called in `try_execute_payload` after each successful
payload commit) and read with `Ordering::Acquire` by `tick`. The latch never resets.

**Force-commit flow**: `force_commit_layers` iterates the TrieLayerCache until empty
using `get_commitable_with_threshold(root, 1)`, committing each layer to disk.
`drain_trie_update_worker` then sends a no-op sentinel to ensure the background
worker acknowledges the flush before activation proceeds.

## Deviations

The original plan-implementer agent submitted two tests under the correct names
(`binary_transition_restart_cycle`, `binary_transition_locked_without_flag`) but both
were idempotency tests in disguise, not the properties their names claim.

**Original substitution 1 (`binary_transition_restart_cycle`)**: The submitted test
opened a single `Store`, activated, then called `tick` a second time and checked that
the second tick returned `Skip`. This proves idempotency, not restart semantics. It did
not reopen any backend, did not share an `Arc<InMemoryBackend>`, and did not check
`backend_kind == Transition` or overlay→base read ordering.

**Fix**: Test moved to `crates/storage/transition_wiring.rs::tests::binary_transition_restart_cycle`.
Uses a shared `Arc<InMemoryBackend>` (same pattern as `test_transition_restart_with_overlay`
and `test_transition_restart_reconstruction`). Proves: (a) second `Store::from_backend`
call against the same backend with `BackendKind::Transition` sets `backend_kind`
correctly, (b) `transition_metadata` is loaded from disk, (c) reads via
`new_transition_state_reader` respect overlay→base ordering. The placeholder comment in
`transition_activator.rs` points to the canonical location.

**Original substitution 2 (`binary_transition_locked_without_flag`)**: The submitted
test opened a single `Store`, called `persist_transition_metadata`, and then ran
`TransitionActivator::tick` against the same open `Store`. It asserted `tick` returned
`Skip`. This again proves idempotency. It did not call `Store::from_backend` with a
mismatched `BackendKind`, so it never exercised the format-byte-2 mismatch error path
at `crates/storage/store.rs:1651-1657`. The agent also cited a test
`backend_format_tests::transition_byte2_mpt_mismatch_error` as justification; that test
does not exist.

**Fix**: Test added to `crates/storage/store.rs::backend_format_tests::binary_transition_locked_without_flag`.
Uses a shared `Arc<InMemoryBackend>`, persists format byte 2 via
`persist_transition_metadata`, then calls `Store::from_backend` with `BackendKind::Mpt`.
Asserts `Err(StoreError::Custom(msg))` where `msg.contains("format byte 2 (transition)
but --binary-transition was not passed")`. This exercises the actual error path at
`store.rs:1651-1657`.

**Pre-existing clippy warning in `blockchain.rs`**: Two nested `if let` blocks at line
1087-1091 triggered `clippy::collapsible_if`. Fixed by collapsing to a single
`if A && let B && let C` chain (line 1087-1090).

**Lock files**: The previous agent added `ethrex-binary-trie` to
`crates/blockchain/Cargo.toml` but did not propagate the change to the sub-workspace
lock files (`crates/l2/tee/quote-gen/Cargo.lock`,
`crates/vm/levm/bench/revm_comparison/Cargo.lock`, `tooling/Cargo.lock`). Updated via
`cargo update --workspace` on each affected manifest.

**Round-2 review findings (code-reviewer round 1 → round 2 fixes)**:

**Task 7.8 silently absent (Blocker)**: The initial implementation jumped from task 7.7
to 7.9 without implementing task 7.8. Round 2 added the
`no_transition_activator_without_flag` smoke test in
`crates/blockchain/transition_activator.rs::tests` (line 305).

**`binary_transition_auto_activation` missing metadata assertions (Major)**: The test
only asserted `meta.0 == 101` (switch_block). Round 2 added assertions for `meta.1`
(frozen_mpt_root, must equal `H256::zero()`) and `meta.2` (binary_root, must equal
`EMPTY_BINARY_ROOT`). File: `crates/blockchain/transition_activator.rs` lines 248–257.

**`unwrap_or_default()` on frozen_mpt_root (Major)**: The `activate()` function used
`unwrap_or_default()` when reading the latest canonical block header's state_root,
silently producing `H256::zero()` if no header was found. Round 2 replaced this with an
explicit `ok_or_else` returning a `StoreError::Custom` error. File:
`crates/blockchain/transition_activator.rs` lines 155–163.

**`snap_enabled` store/load ordering mismatch (Minor)**: `disable_snap()` stored `false`
with `Ordering::Relaxed` while the activator loaded with `Ordering::Acquire`. Round 2
fixed `disable_snap` to use `Ordering::Release`. File:
`crates/networking/p2p/sync_manager.rs` line 134.

**Incorrect Step 4 comment (Minor)**: The comment on the `drain_trie_update_worker()`
call incorrectly attributed the drain to `force_commit_layers`. Round 2 replaced it with
an accurate description of the rendezvous channel mechanism. File:
`crates/blockchain/transition_activator.rs` lines 149–153.

## Round-3 review findings and fixes

**Major 1 — `ok_or_else` guard in `activate()` was dead code**: `get_latest_canonical_block_header` always returned `Ok(Some(...))` because `LatestBlockHeaderCache::default()` returns a zero-initialized header, making the `ok_or_else` guard unreachable. Fixed by making `get_latest_canonical_block_header` return `Ok(None)` when `CHAIN_DATA/LatestBlockNumber` is absent from the DB (synchronous read via `store.read(CHAIN_DATA, ...)`). The guard in `activate()` is now real defense-in-depth. Error message updated to: "cannot activate transition: no canonical block has been committed; activator must wait until the follower has committed at least one block". A test `get_latest_canonical_block_header_returns_none_on_fresh_store` was added to `crates/storage/store.rs::backend_format_tests`.

**Major 2 — `snap_enabled` ordering audit incomplete**: Round 2 fixed only `disable_snap()`. Two writes in `snap_sync.rs` (lines 225, 264) and one in `sync_manager.rs::new()` (line 81) still used `Ordering::Relaxed`. All three changed to `Ordering::Release`. Additionally, all `snap_enabled` reads that were still `Ordering::Relaxed` (`sync_manager.rs` lines 69, 109; `sync.rs` line 206) upgraded to `Ordering::Acquire` for consistency with the activator's Acquire loads and `sync_mode()`'s now-Acquire read.

**Major 3 — Task 7.8 test was a tautology**: The `no_transition_activator_without_flag` test only proved the constructor invariant, not the flag path. Approach (b) taken: the flag-driven wiring lives in `cmd/ethrex/initializers.rs` (binary crate) and cannot be tested at library level without pulling in `SyncManager`, `CancellationToken`, and `Options`. The test was replaced by two bidirectional tests: `transition_activator_starts_none` (constructor invariant, renamed to be honest) and `set_transition_activator_installs_activator` (proves the public setter actually installs the activator). Both tests in `crates/blockchain/transition_activator.rs::tests`.

**Minor 4 — Misleading comments in `activate()`**: Comments at the two re-verification branches incorrectly implied `snap_enabled` could flip back to true and `caught_up` could un-latch. Replaced with accurate descriptions of the one-way / one-shot nature of each field.

## Round-4 review findings and fixes

**Blocker — `validate_state_root` not gated on `BackendKind`**: Both call sites in
`crates/blockchain/blockchain.rs` (`store_block` line 935 and the batch path line 1442)
called `validate_state_root` unconditionally. For `BackendKind::Binary` and
`BackendKind::Transition` stores the `merkle_output.root` is a binary trie root that
does not match the MPT-format `header.state_root`; every post-switch block would have
been rejected with `StateRootMismatch`, making Phase 7 non-functional in production.

**Fix**: Gated both calls with `if self.storage.backend_kind() == BackendKind::Mpt`.
Added a public `backend_kind() -> BackendKind` accessor to `Store` (removed the now-
unnecessary `#[allow(dead_code)]` attribute from the field). Added `BackendKind` to the
existing `use ethrex_state_backend` block in `blockchain.rs`. Files changed:
- `crates/storage/store.rs`: `backend_kind()` accessor added after `drain_trie_update_worker`
- `crates/blockchain/blockchain.rs`: both `validate_state_root` calls wrapped in the gate,
  `BackendKind` added to the `ethrex_state_backend` import

**Regression test added** (`store_block_skips_state_root_validation_for_non_mpt_backend`
in `crates/blockchain/blockchain.rs::tests`):
- Binary store path: `header.state_root = 0xAA..AA`, `merkle_output.root = 0xBB..BB`; calls
  `store_block`; asserts `Ok(())` (gate skipped, mismatch ignored).
- MPT store path: same mismatch; asserts `Err(ChainError::InvalidBlock(StateRootMismatch))`
  (gate fires, validation rejects the block).
- `BackendKind::Transition` is covered by the gate condition (`!= Mpt`) and by
  `transition_wiring::tests::binary_transition_restart_cycle`; it is not directly
  constructable from outside `ethrex-storage` (requires `pub(crate) from_backend` +
  persisted metadata), so the test uses `BackendKind::Binary` as the equivalent non-MPT case.

**Important #1 — Misleading comment in `get_latest_canonical_block_header`**:
`crates/storage/store.rs:2204-2206` claimed `LatestBlockNumber` is "only written when
the first canonical block is committed" and "its absence means no real block exists
yet." In practice `add_initial_state` → `forkchoice_update_inner` writes
`LatestBlockNumber = 0` for genesis, so in production the key is always present after
any normal startup. The comment was replaced with the accurate description:
"absent only on a bare backend that has never had `add_initial_state` called (possible
in unit tests; unreachable in production, where genesis init writes it for block 0)."

**Important #2 — `commit().unwrap_or_default()` in `force_commit_layers`**:
`crates/storage/store.rs:2145` used `unwrap_or_default()` on the result of
`cache_mut.commit(commitable_root)`. If `commit` returned `None` the layer would be
silently skipped and `frozen_mpt_root` would end up wrong. Replaced with
`ok_or_else(|| StoreError::Custom("force_commit_layers: layer vanished..."))` so the
activation path fails loudly rather than silently corrupting the frozen root. The
identical pattern at line 2418 (the steady-state worker hot path) was intentionally
left unchanged.

## Round-5 deviations (post-Phase-7 hoodi run)

**Design reversal — restart-required → in-process hot-swap**:
After the first live hoodi run (2026-05-04) it became clear that the activator's `CancellationToken::cancel()` (step 9 of `activate()`) was silently ignored: the `tokio::select!` in `cmd/ethrex/ethrex.rs` only watched `ctrl_c` + `SIGTERM`, so the process kept running in MPT mode for several minutes until the operator Ctrl-C'd (Bug 1). The operator directed: "it should be seamless." The design reversal turns `StoreVmDatabase`'s per-block lifetime + the existing `activation_lock` mutex into the concurrency boundary for a safe in-process hot-swap:

- `Store::backend_kind` field changed from `BackendKind` (plain copy field) to `AtomicU8` (Acquire/Release, stores `backend_kind_to_byte(kind)`).
- `Store::set_backend_kind(BackendKind)` added: Release-stores the new byte.
- `Store::transition_metadata` changed from `Option<(u64, H256, H256)>` to `RwLock<Option<(u64, H256, H256)>>`.
- `Store::transition_metadata() -> Option<...>` accessor added: acquires a read lock and clones.
- `Store::persist_transition_metadata` now updates the in-memory `RwLock` after the disk `commit()` succeeds (disk-first; memory only updated on success).
- `TransitionActivator::activate()` now calls `store.set_backend_kind(BackendKind::Transition)` after `persist_transition_metadata` succeeds, and logs "Node continues running in Transition mode." instead of "Restart the process".
- `cancel_token: CancellationToken` field removed from `TransitionActivator`. `TransitionActivator::new` no longer takes a `cancel_token` argument.

**Bug 0 (CRITICAL) — Transition mode was cosmetic**:
`crates/blockchain/vm.rs` `StoreVmDatabase::new` and `new_with_block_hash_cache` called `store.new_state_reader(block_header.state_root)` unconditionally — the MPT-only path. In Transition mode, the peer header's `state_root` is the canonical MPT root from the network, never written to disk post-switch, so `has_state_root` always returned false and construction always failed. Fixed by dispatching on `store.backend_kind()`:
- `BackendKind::Mpt` → existing MPT path with `has_state_root` gate.
- `BackendKind::Transition` → skips `has_state_root`, calls `store.new_transition_state_reader(switch_block, frozen_mpt_root, binary_root)`.
- `BackendKind::Binary` → `unreachable!()` with comment (Phase 8 territory).
A new test `transition_mode_vm_database_uses_transition_reader` in `crates/blockchain/vm.rs::tests` proves construction succeeds in Transition mode for a header whose `state_root` is not in the DB, and fails in MPT mode for the same root.

**Bug 1 (HIGH) — `cancel_token.cancel()` not watched in select**:
Obsolete after the hot-swap design reversal. The activator no longer calls `cancel()`, and no other component in the main runtime fires the token (subsystems only watch it; `server_shutdown` cancels it after one of the other arms wins). The earlier session's partial fix that added a `_ = cancel_token.cancelled()` arm to the `tokio::select!` was therefore dead code — discarded.

**Bug 2 (LOW) — misleading log**:
`crates/networking/p2p/sync_manager.rs` log "Follower caught up to finalized head; binary transition may fire." rephrased to "Follower caught up to finalized head." with a comment. Fixed in working tree prior to this session; kept as-is.

**Tests updated**:
- `binary_transition_auto_activation`: added assertions for `store.backend_kind() == Transition` and `store.transition_metadata().is_some()` (in-memory hot-swap); removed `token.is_cancelled()` assertion.
- `binary_transition_waits_for_caught_up`: added assertions for `backend_kind()` and `transition_metadata()` after activation; removed `token.is_cancelled()` assertion.
- `make_activator` helper: removed `CancellationToken` return; updated call sites.

**Instrumentation**:
`print_add_block_pipeline_logs` in `crates/blockchain/blockchain.rs` now appends `[BACKEND={kind:?}]` to the BLOCK metric line. Marked `// TEMP: hoodi sign-off visibility — remove in Phase 9 metrics work`.

**Arc-wrapping refinement (hot-swap clone-sharing)**:
The two new hot-swap fields (`backend_kind` and `transition_metadata`) were initially typed as `AtomicU8` and `RwLock<…>` (bare values), which meant each `Store::clone()` produced an independent copy. A clone created before activation would never observe the `set_backend_kind(Transition)` call, replicating Bug 0 on the RPC surface (all pre-existing handles silently routed through `new_state_reader` instead of `new_transition_state_reader`). Both fields were changed to `Arc<AtomicU8>` and `Arc<RwLock<…>>`, matching the pattern of every other shared field on `Store` (`trie_cache`, `backend`, `account_code_cache`, etc.). The `Clone` impl already used `.clone()` for each field; the Arc wrapping makes those clones share the same underlying atomic and lock. Two tests were added to `crates/storage/store.rs::hot_swap_clone_tests` (`store_clone_shares_backend_kind_after_hot_swap`, `store_clone_shares_transition_metadata_after_persist`) that would have failed under the old by-value approach and pass under the Arc fix.

**Bug 3 (HIGH) — frozen_mpt_root one block stale during catchup**:

Live hoodi run on 2026-05-05 with the round-5 fix landed: activation log fired correctly (`Binary trie transition activated at block 2752450...`), `[BACKEND=Transition]` showed on the BLOCK metric line (Bug 0 fix confirmed structurally), but the very next block (2752450, the first under Transition mode) failed with `Invalid transaction: Nonce mismatch: expected 45, got 46` and FullSync entered an infinite retry loop on the same block hash. Sender's state nonce was one increment behind the canonical chain.

Root cause: `activate()` step 5 read `frozen_mpt_root` via `store.get_latest_canonical_block_header()`, which loads the header at `CHAIN_DATA::LatestBlockNumber`. That key — and the in-memory `Store::latest_block_header` cache — are both advanced by `apply_fork_choice` (engine_forkchoiceUpdated from the CL), NOT by block execution. During catchup, EL block execution outpaces CL forkchoice by one or more blocks. So `head_block_number` passed to `tick()` (the block we just committed via `execute_block_pipeline` → `store_block`) was 2752449, but `LatestBlockNumber` was still 2752448. Activation persisted `frozen_mpt_root = state_root(block 2752448)` and `switch_block = 2752449 + 1 = 2752450`. When block 2752450 executed under Transition mode, it read the sender's account at `frozen_mpt_root` and got the post-block-2752448 state (one block too old), so the next-tx-nonce expectation was off by one.

This was missed by the prior round's tests because every existing activator test seeded `LatestBlockNumber` synchronously via `test_set_latest_block_number(N)` before calling `tick(store, N)`, so the chain-data view always agreed with `head_block_number`. The MPT-only bisect run (`scripts-local/run-hoodi-mpt-only.sh`) confirmed the bug is activation-flow-specific — plain MPT snap+fullsync proceeded past the equivalent block range cleanly.

Diagnosed via the MPT-only bisect: same chain region, same code, no `--binary-transition` flag. MPT advanced to block 2752738+ with no nonce mismatch.

Fix: `tick()` and `activate()` now take `head_state_root: H256` as a third argument. The caller (`Blockchain::add_block_pipeline` at `crates/blockchain/blockchain.rs:1064-1094`) captures `block.header.state_root` before moving the block into `store_block`, and passes it to `tick`. `activate()` step 5 simply assigns `let frozen_mpt_root = head_state_root;` — no DB lookup, no possibility of staleness. Aligns with the existing `switch_block = head_block_number + 1` derivation so frozen state and switch block are consistent by construction.

Regression test: `activator_uses_caller_state_root_when_chain_data_lags` in `crates/blockchain/transition_activator.rs::tests`. It seeds `LatestBlockNumber=99` (via `test_set_latest_block_number`) but calls `tick(store, 100, root_100)` with a different root, and asserts the persisted `frozen_mpt_root == root_100`, not the value `LatestBlockNumber` would have resolved to. The pre-fix code (which read `get_latest_canonical_block_header()`) would resolve `frozen_mpt_root` to `state_root(block_99)` (`H256::zero()` from the default header) and the equality check would fail.

**Bug 3 v2 (HIGH, fixed) — `force_commit_layers` walked from a stale root**:

Hoodi sign-off run #2 (post-Bug-3-v1 fix) reproduced the same `Nonce mismatch: expected N, got N+1` symptom on the very first post-switch block (2752950). The v1 fix (passing `head_state_root` from the caller) was necessary but not sufficient.

Root cause: `Store::force_commit_layers` (called by `activate()` step 3 in v1's order) computed its starting root as `self.latest_block_header.get().state_root`. That cache is also advanced only by `apply_fork_choice`, identically to `LatestBlockNumber`. So during catchup the activator was force-committing layers walking from the *stale* root. Block N's layer is keyed by `state_root(N)` and is a **child** of the stale root, never reached by an ancestor walk. Block N's MPT trie nodes therefore stayed in-memory only. Even with `frozen_mpt_root = state_root(N)` correctly persisted (v1 fix), reads through the Transition base hit a missing root node and effectively returned the previous block's state.

Additional issue: the order of steps was `force_commit_layers` then `drain_trie_update_worker`. The worker's Phase 1 (cache `put_batch`) for block N may not have completed by the time `activate()` started — block N's update is sent via a rendezvous channel, the send unblocks once the worker receives it but Phase 1 (cache write) is async after that. So `force_commit_layers` could even run before block N's layer was in the cache at all.

Fix:
1. Reorder: `drain_trie_update_worker` BEFORE `force_commit_layers`. After drain returns, the worker has acked Phase 1 for block N (cache contains N's layer).
2. Parameterize: `Store::force_commit_layers(from_root: H256)` takes the root explicitly. `activate()` passes `head_state_root` (the just-committed block's root). The walk from `head_state_root` traverses N's layer first, then ancestors, committing all to disk.

Hoodi sign-off run #3 with the v2 fix: block 2753086 (last MPT) committed, block 2753087 (FIRST under Transition mode) committed cleanly with 28 txs, store=0.25ms. v2 fix worked for the switch block. But block 2753088 still failed with `Nonce mismatch: expected 225902, got 225903` — a third layer of Bug 3.

**Bug 3 v3 (HIGH, fixed) — `mpt_commit_nodes_to_disk` silently skipped leaves past the FKV generator cursor**:

`crates/storage/mpt_wiring.rs::mpt_commit_nodes_to_disk` had:
```rust
if is_leaf && key > last_written {
    continue;
}
```

This is FKV-generator coordination: during normal operation, the FKV iterator writes `ACCOUNT_FLATKEYVALUE` / `STORAGE_FLATKEYVALUE` entries as it advances; trie commits skip leaves past the FKV cursor (`last_written` in `MISC_VALUES`) so the FKV will rewrite them later, avoiding races.

In the activation freeze: `activate()` step 2 calls `stop_fkv_generator` PERMANENTLY. Then step 4 calls `force_commit_layers(head_state_root)` which calls `cache.commit(root)` (removes the layer from the in-memory cache and returns the diffs) and passes them to `mpt_commit_nodes_to_disk`. Any leaf with key > `last_written` is silently SKIPPED — and since the cache layer has already been removed and FKV will never run again, the leaf is **permanently lost**. The frozen MPT base on disk has a stale value at that path. Reads through the Transition reader at `frozen_mpt_root` walk the trie and return the pre-block-N value for that leaf.

This explained why some accounts read correctly through Transition but others didn't: it depends on whether the account's hash key happened to be ≤ or > the FKV cursor position at activation time.

Fix: add a `bypass_fkv_cursor: bool` parameter to `mpt_commit_nodes_to_disk`. The activator's `force_commit_layers` path passes `true` (FKV is stopped, write everything). The worker's Phase 2 commit path passes `false` (FKV still running, skip past-cursor leaves as before).

This third layer was undetectable from unit tests because the in-memory `Store::new(InMemory)` test path doesn't exercise the FKV generator at all — the cursor is always `Vec::new()` (default) and the skip check `key > last_written` evaluates as `key > []` which is true for any non-empty key, but the test path's `mpt_commit_nodes_to_disk` is also a no-op for the InMemory backend. Only on a real RocksDB-backed snap-synced node does the FKV cursor reach a meaningful value during catchup.

Live hoodi confirmation pending (sign-off run #4).

**Bug 4 (HIGH, diagnosed, fix pending) — overlay reads miss in-memory binary_trie_cache layers**:

Hoodi sign-off run #4 with Bug 3 v3 fix: block 2753345 (last MPT) committed, block 2753346 (FIRST under Transition) committed cleanly with 38 txs, then block 2753347 failed with `Receipts Root does not match the one in the header after executing` — execution diverged.

Root cause: post-Transition overlay reads do NOT consult the in-memory `binary_trie_cache`. Specifically:

1. `crates/storage/transition_wiring.rs::new_transition_state_reader` opens the overlay BinaryBackend at the **frozen** `binary_root` from `transition_metadata` (which is `EMPTY_BINARY_ROOT` for fresh activation, never updated).
2. `BinaryTrieState::open` (`crates/common/binary-trie/state.rs`) initializes `state` from on-disk `META_ROOT_HASH` — also at activation-time value.
3. `BinaryBackend::storage` (`crates/common/binary-trie/backend.rs:445`) reads via `self.state.trie_get(tree_key)`, which walks the in-memory tree backed by the provider's disk reads. The provider (`StoreBinaryTrieProvider` in `binary_wiring.rs`) reads only from `BINARY_TRIE_NODES`/`BINARY_FLATKEYVALUE` on disk.
4. `is_slot_in_fkv` (`binary_wiring.rs:191`) reads `BINARY_FLATKEYVALUE` on disk only.

So when block N+1's overlay reader queries a slot that block N wrote:
- Block N's writes live in `binary_trie_cache` (in-memory) — NOT on disk (worker Phase 2's threshold of 128 layers won't fire for the first ~127 post-switch blocks).
- Block N+1's BinaryBackend reads return EMPTY (state at activation-time empty root).
- `TransitionBackend::storage` falls through to base (frozen MPT) → returns pre-switch value, not the correct post-switch overlay value.

For non-zero SSTORE in block N: block N+1 reads pre-switch value instead of new value → EVM diverges → receipts root mismatch.
For zero SSTORE in block N (slot zeroed post-switch): block N+1's `slot_is_in_overlay` returns false (overlay backend empty, FKV disk empty) → falls through to base → pre-switch non-zero value resurrected → receipts root mismatch.

Symmetric to MPT: MPT's `open_state_trie` constructs an `MptTrieWrapper(state_root, trie_cache, db, last_written)` that reads through both the in-memory `trie_cache` and on-disk backend. The binary path has no equivalent — it reads disk only.

Fix options:
- **A (correct, invasive)**: plumb `binary_trie_cache` into `BinaryTrieProvider`/`BinaryTrieState` so reads consult the in-memory layers first, falling through to disk. Symmetric to `MptTrieWrapper`.
- **B (quick, slow)**: in `Blockchain::add_block_pipeline` (or in the worker Phase 2 for binary), force-commit the binary cache to disk after every block during Transition mode. Adds a disk write per block; loses the layer cache's batching benefit but is correct.
- **C (simplest, semantic shift)**: at activation, drop the overlay backend altogether — switch to having every Transition block read & write directly to disk-backed binary state. No in-memory layering. Trades perf for correctness/simplicity.

Option A is the right long-term fix. Option B is the smallest patch to ship today and validate. Choose at session restart.

**Bug 4 fix (Option A, landed in 3 commits)**:

User chose Option A. Landed in `ce6409937` + `f402a6a85` + `631a44cba`:

1. **`Store::current_binary_root: Arc<RwLock<H256>>`** field added (`ce6409937`). Seeded from disk `META_ROOT_HASH` at construction (so it survives restarts) and advanced in `apply_trie_updates` Phase 1 to `child_state_root` after each `BackendKind::Binary` `TrieUpdate`. Public accessor `Store::current_binary_root()`. Source of truth for the live binary head root, replacing reads from the frozen `transition_metadata.binary_root` and the disk-lagged `META_ROOT_HASH`.

2. **`BinaryTrieProvider::cache_get_leaf`** added to the trait (`f402a6a85`), and `BinaryBackend::account` / `BinaryBackend::storage` / `BinaryBackend::slot_is_in_overlay` now consult it before the disk-backed `state.trie_get` / `is_slot_in_fkv`. Default trait impl returns `None` (no cache), preserving `EmptyBinaryTrieProvider` semantics for unit tests.

3. **`StoreBinaryTrieProvider::cache_get_leaf`** implements the lookup (`631a44cba`): `binary_trie_cache.get(self.store.current_binary_root().0, tree_key)`. Walks the layer chain from the live head root; returns:
   - `Some(Some(value))` — leaf was written in some layer
   - `Some(None)` — leaf was deleted (post-switch SSTORE 0 / SELFDESTRUCT stem clear)
   - `None` — not in any layer; caller falls through to disk

After this fix, block N+1's overlay reads observe block N's writes via the live in-memory cache, not the disk-lagged frozen state. Validation pending hoodi sign-off run #5.

Notes for future cleanup (not required for Bug 4):
- `new_transition_state_reader` still receives `binary_root` from `transition_metadata` (frozen at activation). For fresh activation that's `EMPTY_BINARY_ROOT` and the empty-overlay branch is taken, so this works. On restart the second branch validates against on-disk `META_ROOT_HASH` which might lag in-memory `current_binary_root`; this is a separate restart-recovery path that should also be reconciled.
- The `state.trie_get` fallback inside `BinaryBackend` walks the disk-backed in-memory tree at whatever root `BinaryTrieState::open` started at. For restart scenarios where on-disk `META_ROOT_HASH` lags `current_binary_root`, that tree is at the older root. Cache_get_leaf covers reads modified post-flush; truly-unmodified-since-flush leaves still resolve through the lagged tree, which is the on-disk truth — fine for state reads but could matter for trie-walk operations (rare).
- `is_deleted_stem` is NOT yet cache-walking; SELFDESTRUCT-during-overlay isn't covered by Bug 4 fix. Low frequency, can be added if hoodi exposes it.

**Hoodi sign-off run #5 (Bug 4 Option A fix)** — partial success:

- Block 2753571 (last MPT) ✓ committed (52 txs, BACKEND=Transition tag post-activation).
- Block 2753572 (FIRST post-switch under Transition mode) ✓ committed cleanly, 25 txs, store=0.33ms. **Bug 4 fix definitely works for the switch block** — pre-fix this would have failed identically to the receipts-root mismatch in run #4.
- Block 2753573 fails: `Invalid transaction: Nonce mismatch: expected 4327, got 4330` — **off by 3**, not off by 1 like prior rounds.

The off-by-3 magnitude is new. Hypotheses:

1. Bug 3 v3 incomplete: a sender did 3 txs across full-sync blocks (2753539–2753571) and all 3 leaf updates got silently dropped. force_commit_layers' `bypass_fkv_cursor=true` should have prevented this; needs verification.
2. Block 2753572 had 3 txs from the failing sender, and the BinaryMerkleizer / `BinaryBackend::update_accounts` didn't propagate all 3 nonce increments into `fkv_entries`. Cache_get_leaf then returns the partially-updated value.
3. The CoW first-write logic in `BinaryBackend::update_accounts` (atomic writes of all 4 BASIC_DATA sub-leaves + CODE_HASH on first post-switch touch) might race with intra-block state visibility, dropping later-tx updates within the same block.

Diagnostics to add next session:
- Temp log in `BinaryBackend::update_accounts` printing `(addr, new_nonce, before, after_intra_block_state_root)` for each sender per tx.
- Log layer count and `current_binary_root` after each commit so we can verify cache state.
- Cross-check sender's actual canonical-chain nonce via JSON-RPC against a trusted node at switch_block + 1.

**Bug 5 (CRITICAL, fix landed) — execute_block_pipeline always used MPT merkleizer regardless of backend_kind**:

Hoodi sign-off run #6 (with Bug 4 fix + diagnostic logs) showed:
- Block 2753731 (FIRST post-Transition) committed cleanly under Transition tag.
- Block 2753732 fails: `Nonce mismatch sender=0xf1a2f8e2... state_nonce=16348 tx_nonce=16349 delta=1` AND `sender=0x6d5ef225... state_nonce=40 tx_nonce=41 delta=1` — TWO senders both off by exactly 1.
- The diagnostic `[BINARY-DEBUG] update_accounts` logs NEVER fired post-activation.
- The diagnostic `[BINARY-DEBUG] advanced current_binary_root` logs NEVER fired post-activation.
- `force_commit_layers committed 10411 nodes (1614 leaves)` fired ONCE at activation (the MPT freeze flush).

Diagnosis: `Blockchain::execute_block_pipeline` at `crates/blockchain/blockchain.rs:498` always called `Merkleizer::new_mpt` / `Merkleizer::new_bal_mpt`, with NO dispatch on `store.backend_kind()`. So even after Transition activation, blocks merkleized via the MPT path, producing `NodeUpdates::Mpt`. Worker stored writes into the **MPT** layer cache + flushed to disk MPT trie tables at NEW `state_root` values (not the frozen root).

Meanwhile the read side (TransitionBackend, after Bug 0/4 fixes) reads at the FROZEN `frozen_mpt_root` (state-at-activation). Post-activation MPT writes never visible because they live at different state_roots from the frozen one.

Result: every sender that had a tx in block N (post-activation) reads as off-by-1 in block N+1 (state from N-1 returned).

`Merkleizer::new_transition` was defined in `crates/storage/merkleizer.rs:78` but **never called from anywhere outside that file**. The whole binary-write pipeline was dead code post-activation.

Fix (`crates/blockchain/blockchain.rs:483-528`): `execute_block_pipeline` now dispatches on `store.backend_kind()`:
- `BackendKind::Mpt` → `Merkleizer::new_bal_mpt` / `new_mpt` (existing behaviour).
- `BackendKind::Transition` → `Merkleizer::new_transition` (BinaryMerkleizer producing `NodeUpdates::Binary`, which the worker routes through `binary_trie_cache`).
- `BackendKind::Binary` → `unreachable!()` (Phase 8 territory).

Constructs both `mpt_provider` and `binary_provider` ahead of the spawn so the threading layout is unchanged. After this fix, post-Transition blocks merkleize via `BinaryMerkleizer`, write to the binary overlay, and the `[BINARY-DEBUG] advanced current_binary_root` + `update_accounts` logs from the diagnostic round will start firing — confirming the design pipeline now matches MPT's structure.

Companion fix landed in same commit family: `Store::apply_account_updates_batch` (used by `Blockchain::add_block`, the non-pipelined path) was MPT-only via `self.new_state_reader(header.state_root)`. Now dispatches on `backend_kind`: Mpt → existing reader, Transition → `new_transition_state_reader` with the persisted metadata, Binary → `new_binary_state_reader`. Same Bug 0/5 family closed everywhere, not just on the snap-sync hot path.

The `BackendKind::Binary` arm in `execute_block_pipeline` now calls `Merkleizer::new_binary` (instead of `unreachable!()`). Not exercised by `--binary-transition` mode, but the pipeline is fully wired so `--binary-from-genesis` (Phase 8) can drop in without touching this dispatch.

## Asymmetry close-out (post-Bug-5)

After Bug 5 landed, three remaining MPT/binary read-path asymmetries were identified in the handoff "Notes for future cleanup" of Bug 4. All three are now fixed in this session:

**Asymmetry 1 — `is_deleted_stem` consulted disk only**:
SELFDESTRUCT writes a tombstone into `trie_cache` at `[0xFE, stem...; 32 bytes]` framed as `[CACHE_TOMBSTONE_TAG; 1]`. Block N+1's transition fall-through called `StoreBinaryTrieProvider::is_deleted_stem` which read disk only; cache-only tombstones (≤ 127 layers since flush) were invisible, so a SELFDESTRUCTed account in the overlay would resurrect from MPT base on the next block's reads.

Fix (`crates/storage/binary_wiring.rs::StoreBinaryTrieProvider::is_deleted_stem`): walk `store.trie_cache` at `store.current_binary_root()` for `tombstone_key(stem)` first; on cache hit (any framed entry at a 0xFE-prefixed key is a tombstone), return `true`. Falls through to the existing disk lookup on cache miss. Symmetric to the Bug 4 `cache_get_leaf` walk.

Regression test: `binary_wiring::tests::is_deleted_stem_walks_cache_before_disk` — inserts a synthetic cache layer with a stem tombstone, advances `current_binary_root`, asserts disk has no tombstone, asserts `is_deleted_stem` returns true via the cache walk.

**Asymmetry 2 — `new_transition_state_reader` used the frozen `binary_root` from `transition_metadata`**:
The metadata's `binary_root` is written once at activation (`EMPTY_BINARY_ROOT`) and never updated. The reader's two-branch construction (`binary_root == zero` → empty overlay; `!= zero` → DB-backed open with `META_ROOT_HASH` validation) was wrong on restart: post-activation commits advance the disk `META_ROOT_HASH` but the metadata still says zero, so the empty branch fires and the persisted overlay nodes are invisible to the reader.

Fix (`crates/storage/transition_wiring.rs::Store::new_transition_state_reader`): drop the `binary_root` parameter. Always open via [`CacheAwareTrieBackend`] (see Asymmetry 3) which serves META_ROOT and trie nodes from the cache layer at `current_binary_root` first, then falls through to disk. The two branches collapse into one. Callsites in `store.rs::apply_account_updates_batch`, `blockchain/vm.rs::StoreVmDatabase`, and four restart-cycle tests in `transition_wiring.rs::tests` updated. The `binary_root` field of `transition_metadata` is kept on disk for backward-compatibility of the persisted format but is now vestigial.

**Asymmetry 3 — `state.trie_get` walked at the disk-flushed root, lagging the live head**:
`BinaryTrieState::open` reads `META_ROOT` via the `TrieBackend`. `StorageTrieBackend::get` reads `BINARY_TRIE_NODES` from disk only, so the in-memory trie was effectively rooted at the disk-flushed `META_ROOT_HASH`. Cache layers on top (up to 127) were invisible to trie traversal. State value reads were already covered by Bug 4's `cache_get_leaf`, but anything walking the trie structure (proofs, iteration) was at the lagged root.

Fix (`crates/storage/binary_wiring.rs::CacheAwareTrieBackend`): new `TrieBackend` wrapper that consults `store.trie_cache` at `store.current_binary_root()` for `BINARY_TRIE_NODES` reads before delegating to the inner `StorageTrieBackend`. On cache hit, decodes the framed value (`CACHE_VALUE_TAG` → unframed bytes; `CACHE_TOMBSTONE_TAG` → `None`). Other tables (`BINARY_STORAGE_KEYS`, etc.) and `write_batch` / `full_iterator` pass through unchanged. Used only by `new_transition_state_reader`; `new_binary_state_reader` (historic-root pinning) keeps the bare `StorageTrieBackend` so historic readers see only what's on disk at that root. Symmetric to MPT's `MptTrieWrapper(state_root, trie_cache, db, last_written)`.

Regression test: `binary_wiring::tests::cache_aware_trie_backend_serves_node_from_cache` — inserts a synthetic node-id+bytes layer into `trie_cache`, advances `current_binary_root`, asserts disk has no such node, then reads via `CacheAwareTrieBackend` and asserts the unframed bytes come back from the cache. Also asserts non-`BINARY_TRIE_NODES` tables bypass the cache walk.

All three close-out fixes preserve `EmptyBinaryTrieProvider` and `new_binary_state_reader` semantics — no historic-root readers gain implicit cache walks. Hoodi run #7 will validate Bug 5 landed; runs after that will validate Asymmetries 1-3 if a SELFDESTRUCT or proof-walk happens within 127 blocks of activation. Tests cover the unit-level assertions deterministically.

## Bug 6 (HIGH, root cause for run #7 failure) — `stem_has_basic_data` gate read disk-only, misrouting reads to MPT base

Hoodi sign-off run #7 (with Bug 5 fix only, BEFORE the Asymmetry close-out fixes were rebuilt into the binary):

- Block 2753920 (last MPT) ✓ committed (29 txs).
- Block 2753921 (FIRST under Transition) ✓ committed cleanly, 20 txs, store=0.33ms. `[BINARY-DEBUG] advanced current_binary_root: parent=0x9642 child=0x3bb9` fires — proving Bug 5's merkleizer dispatch fix engaged: BinaryMerkleizer ran, NodeUpdates::Binary landed, layer at R1=0x3bb9 wrote into `binary_trie_cache` and `trie_cache`.
- Block 2753922 fails: three different senders, all delta=1 nonce mismatch. Chain stalls.

Diagnosis traced through the read path:

`TransitionBackend::account` (`crates/storage/transition_wiring.rs:176`) gates on `self.overlay.stem_has_basic_data(&stem)` — if `false`, falls through to MPT base. That function (`crates/common/binary-trie/backend.rs::stem_has_basic_data`):

```rust
pub fn stem_has_basic_data(&self, stem: &[u8; 31]) -> Result<bool, StateError> {
    if self.deleted_stems.contains(stem) { return Ok(false); }
    let basic_key = tree_key_from_stem(stem, BASIC_DATA_LEAF_KEY);
    Ok(self.state.trie_get(basic_key).is_some())  // disk-backed in-memory trie ONLY
}
```

It checks `BinaryTrieState.trie_get` only. Bug 4's Option A fix added `cache_get_leaf` walks to `BinaryBackend::account` / `storage` / `slot_is_in_overlay`, but **NOT** to `stem_has_basic_data`. And the underlying `BinaryTrieState` was opened by `new_transition_state_reader` against on-disk `META_ROOT_HASH` — empty for fresh activation (no Phase 2 flush yet) — so the in-memory trie is empty and `trie_get` returns None for every leaf, including those just written into `binary_trie_cache` at the live head root.

End-to-end flow on the failing read:

1. Block 2753921 BinaryMerkleizer commits 20 accounts. `binary_trie_cache` layer at R1 contains their fkv_entries (basic_data + code_hash). `current_binary_root = R1`.
2. Block 2753922 reads sender X (modified in 2753921) via `TransitionBackend::account`.
3. `stem_has_basic_data` → `state.trie_get` reads disk-backed empty in-memory trie → returns `None` → returns `Ok(false)`.
4. Gate fails, falls through to MPT base at `frozen_mpt_root = state_root(2753920)`.
5. MPT base returns nonce N (post-2753920). Tx wants nonce N+1 (post-2753921). Off-by-1.

This is precisely Asymmetry 3 manifesting in production: the in-memory `BinaryTrieState` rooted at the lagged disk root, not the live head, makes the gate function blind to the cache layers.

**Fix**: the Asymmetry close-out section above (already landed in this session, in working tree at the time of run #7's failure) closes Bug 6:

- Asymmetry 2 + 3 fix: `new_transition_state_reader` opens via `CacheAwareTrieBackend`, which serves `META_ROOT` and node-id reads from `trie_cache` at `current_binary_root` first, then disk. After the fix, `BinaryTrieState` is effectively rooted at the live head, `state.trie_get(basic_key)` walks through cache layers and returns the just-written leaf, `stem_has_basic_data` returns `true`, and `TransitionBackend::account` correctly takes the overlay branch — where `cache_get_leaf` (Bug 4) returns the new BASIC_DATA with nonce N+1.

This is the structural root-cause fix. No narrow patch to `stem_has_basic_data` is added: with the cache-aware backend, the disk-vs-cache asymmetry is closed at the layer where the in-memory trie meets the disk, not patched per-call-site.

**Run #7 status**: chain stuck on 2753922. Asymmetry-closeout binary needs to be rebuilt and run #8 launched on a fresh datadir.

**Verification expectations for run #8**:
- Block 2753921 (or whatever the new switch block is): `[BINARY-DEBUG] advanced current_binary_root` fires (Bug 5 still works).
- Block switch_block + 1: commits cleanly without nonce mismatch (Bug 6 closed).
- Chain advances past switch_block + 50, +100, +200 → Phase 7 signed off.

## Bug 6.5 (HIGH, root cause for run #8 failure) — `BinaryMerkleizer` started from an empty trie

Hoodi sign-off run #8 (with the f10fb7ffa "cache-aware binary trie reads" fix landed): block switch_block + 1 (2754125) committed cleanly with 32 txs ✓, but block switch_block + 2 (2754126) failed with the same off-by-1 nonce on three different senders.

Diagnosis: even with `CacheAwareTrieBackend` plumbed into `new_transition_state_reader`, the **merkleizer itself** opened a fresh `BinaryTrieState::new()` per block — an EMPTY trie. Compare:

- `MptMerkleizer::new(parent_state_root, provider, …)` opens 16 shard workers each handling a subtrie rooted at `parent_state_root`, lazy-loading nodes via `provider`. The internal trie at the new root is `parent_state + this block's writes` (FULL post-state).
- `BinaryMerkleizer::new(_parent_root, provider, …)` ignored both arguments (literal `_parent_root`, provider was `#[allow(dead_code)]`) and started from an empty `BinaryTrieState`. The internal trie at the new root contained **only this block's writes**.

Consequence under the layer-cache model: each block's commit produced node diffs representing "diffs from empty," not "diffs from parent." The trie at root R(N) contained a path only to accounts modified at block N. `state.trie_get(any_other_account_key)` at R(N) returned None. Read-path gates that consult `state.trie_get` (`stem_has_basic_data` is the prominent one) therefore returned false for any account not modified in the latest block, falling through to the MPT base — even when that account had been modified at a *prior* post-switch block whose layer was still in `binary_trie_cache`.

Run #7 hit this at switch_block + 1 because `BinaryTrieState` was opened against the disk-flushed `META_ROOT_HASH` (empty for fresh activation). The `CacheAwareTrieBackend` fix made `BinaryTrieState`'s open call see the cached META_ROOT, so block N's own writes became visible — so switch_block + 1 worked. But the merkleizer inside `Merkleizer::new_transition` (and `new_binary` for Phase 8) still started from `BinaryTrieState::new()` (empty), so block switch_block + 2 reading an account modified at switch_block + 1 saw an empty trie and fell through.

**Fix**: make `BinaryMerkleizer` symmetric to `MptMerkleizer`. The merkleizer now opens its starting state through the provider, which production providers root at the live binary head via `CacheAwareTrieBackend`:

- `BinaryTrieProvider::open_state(&self) -> Result<BinaryTrieState, BinaryTrieError>`: new trait method. Default impl returns `BinaryTrieState::new()` (empty, used by `EmptyBinaryTrieProvider` and tests / genesis bootstrap).
- `StoreBinaryTrieProvider::open_state` overrides: opens `BinaryTrieState::open(CacheAwareTrieBackend, BINARY_TRIE_NODES, BINARY_STORAGE_KEYS)`. The cache-aware backend serves META_ROOT and per-node reads from `trie_cache` at `current_binary_root` first, then disk — same pattern as MPT's `MptTrieWrapper(state_root, trie_cache, db, last_written)`.
- `BinaryMerkleizer::new` and `new_bal`: replace `BinaryTrieState::new()` with `provider.open_state()`. The `provider: Arc<dyn BinaryTrieProvider>` field loses its `#[allow(dead_code)]` — it's now load-bearing.
- Added `EmptyTrieBackend` to `binary-trie::db` for symmetry with `EmptyBinaryTrieProvider`. Returns None for every read, rejects writes; used by the default `open_state` impl path.

Result: each post-switch block's merkleizer trie contains the FULL post-parent state (live binary head + this block's writes). `state.trie_get` works for cross-block reads. The read-path gates (`stem_has_basic_data`, `slot_is_in_overlay` via `state.trie_get`) all see prior-block modifications. Layer-cache + on-disk fallback work identically to the MPT pipeline.

This is the architectural symmetry the user asked for: "binary trie should work as likely as possible like mpt does, following same pipeline paths/caches/fkvs."

**Verification expectations for run #9** (post-Bug-6.5 fix, fresh datadir):
- BinaryMerkleizer opens via `provider.open_state()` rooted at the live head.
- `state.trie_get` at any post-switch root returns leaves for any account modified at any prior post-switch block.
- `stem_has_basic_data` returns true correctly for cross-block reads.
- Chain advances past switch_block + 50, +100, +200 cleanly → Phase 7 signed off.
