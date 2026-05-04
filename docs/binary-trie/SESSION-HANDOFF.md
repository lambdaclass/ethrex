# Binary-Trie PR — Session Handoff

**Purpose**: orient a fresh session with no prior chat context. Read this first.

## Current status (2026-05-05)

Phase 7 round 5 (Bugs 0A + 0B + design reversal + Arc-wrap + Bug 1 obsolete + Bug 2 fix + doc reversals + TEMP `[BACKEND=...]` instrumentation) **landed and amend-pushed** as `edb9b18a2` onto `origin/shared-trie-binary`.

Phase 7 round 6 (Bug 3 — `frozen_mpt_root` was one block stale during catchup because `activate()` read from `LatestBlockNumber` which lags block execution) is **implemented in the working tree**, ready for amend-and-push and a second hoodi sign-off.

Phase 8 (`--binary-from-genesis`) is **still deferred** until the hoodi sign-off lands cleanly. Plan at `docs/binary-trie/genesis-binary-plan.md` (untracked, awaiting Phase 8 start).

### History recap (in order)

1. **Bug 0A (CRITICAL, fixed round 5)** — `crates/blockchain/vm.rs::StoreVmDatabase::new` unconditionally called `store.new_state_reader(state_root)` (MPT-only). Transition mode was cosmetic. Fix: dispatch on `store.backend_kind()` in both constructors. Live hoodi 2026-05-05 confirmed `[BACKEND=Transition]` lands on the BLOCK metric line after activation.
2. **Bug 0B (CRITICAL, fixed round 5)** — `StateBackend::account_state_info` returned `Err("AccountStateInfo is MPT-specific...")` for Binary/Transition, blocking every account read on the post-switch hot path even after Bug 0A's dispatch fix. The error was justified by a stale doc claim that `AccountStateInfo` had a `storage_root` field; the struct actually has `{info, has_storage}`, both fully derivable. Fix: delegate to `self.account(addr)?` and wrap with conservative `has_storage = true`.
3. **Design reversal (round 5)** — process exit on activation → in-process hot-swap. Operator no longer needs to restart with `--binary-transition`. `backend_kind` and `transition_metadata` became `Arc<AtomicU8>` and `Arc<RwLock<...>>` so all `Store` clones see the swap.
4. **Bug 1 (round 5)** — obsolete after the design reversal. The `cancel_token.cancelled()` arm in `cmd/ethrex/ethrex.rs` was discarded; nothing fires the token in-runtime any more.
5. **Bug 2 (round 5)** — misleading "binary transition may fire" log; rephrased.
6. **Bug 3 (HIGH, fixed round 6, ready to push)** — `frozen_mpt_root` one block stale. Live hoodi run on 2026-05-05 had Bug 0 confirmed structurally (BLOCK lines tagged `[BACKEND=Transition]`) but block 2752450 (the switch block) failed with `Nonce mismatch: expected 45, got 46`. Sender's account state was one nonce behind. Diagnosed via the MPT-only bisect script (`scripts-local/run-hoodi-mpt-only.sh`): plain MPT advanced past the equivalent block range cleanly, confirming the bug is activation-flow-specific. Root cause: `activate()` step 5 read `frozen_mpt_root` via `get_latest_canonical_block_header()`, which loads `CHAIN_DATA::LatestBlockNumber`. That key (and the in-memory `Store::latest_block_header` cache) are advanced by `apply_fork_choice` (engine_forkchoiceUpdated from the CL), NOT by block execution. During catchup the EL outruns the CL by one or more blocks, so `LatestBlockNumber=N-1` while `head_block_number=N`. The activator persisted `frozen_mpt_root=state_root(N-1)` and `switch_block=N+1`, off by one. Fix: pass `head_state_root: H256` from the caller (`Blockchain::add_block_pipeline`) into `tick()` and `activate()`. Caller has the just-committed block in scope; capture `block.header.state_root` before moving the block into `store_block`. New regression test `activator_uses_caller_state_root_when_chain_data_lags` makes the dependency on `head_state_root` explicit. See `phases/phase-7-handoff.md` Round-5 section for detail.

### Outstanding before the PR is ready to merge

1. Amend the Bug 3 fix onto `edb9b18a2` and force-with-lease push.
2. Re-run hoodi sign-off (`scripts-local/run-hoodi-stack.sh transition`). Expected: `[BACKEND=Transition]` on every BLOCK line after activation, no `Nonce mismatch`, FullSync advancing past the switch block.
3. Remove the TEMP `[BACKEND=...]` tag in Phase 9 (replaced by proper metrics) — left in for now per round-5 plan.
4. Open Phase 8 (`docs/binary-trie/genesis-binary-plan.md`).

### Bug 1 (HIGH, partially fixed in working tree) — `cancel_token.cancel()` doesn't terminate main runtime

`cmd/ethrex/ethrex.rs:189`'s `tokio::select!` only watched `ctrl_c` + `SIGTERM`. Activator's `cancel_token.cancel()` (step 9 of `activate()`) was silently ignored; process kept running blocks for several minutes until user Ctrl-C'd.

**Working-tree partial fix (uncommitted)**: `cmd/ethrex/ethrex.rs` now has a `_ = cancel_token.cancelled() => { ... }` arm. This is defense-in-depth and useful regardless of the hot-swap reversal below; if hot-swap lands, the activator no longer fires `cancel()` but the arm is still useful for future programmatic shutdowns. Decide: keep the edit, or discard if the design reversal makes it unnecessary.

### Where the per-bug detail lives

Round 5 plan, deviations, and post-mortem (Bugs 0A/0B/1/2 + design reversal + Arc-wrap) live in `phases/phase-7-handoff.md` "Round-5 deviations" section. Round 6 (Bug 3 + caller-provided `head_state_root`) is appended to that same section. This file (`SESSION-HANDOFF.md`) is the bridge between sessions — open `phases/phase-7-handoff.md` for the engineering record.

### Working-tree state at handoff (after Bug 3 fix)

```
 M crates/blockchain/blockchain.rs              # capture block.header.state_root, pass to tick()
 M crates/blockchain/transition_activator.rs    # tick/activate take head_state_root: H256
 M docs/binary-trie/SESSION-HANDOFF.md          # this file
 M docs/binary-trie/phases/phase-7-handoff.md   # Bug 3 paragraph appended
?? docs/binary-trie/genesis-binary-plan.md      # Phase 8 plan (untracked, awaits Phase 8 start)
```

Amend onto `edb9b18a2` and force-with-lease push, then re-run hoodi.

---

## State snapshot

- **Repo**: `/home/edgar/dev/ethrex` (also runs from `/data2/edgar/work/ethrex` and other clones)
- **Branch**: `shared-trie-binary` (single squashed commit, project pattern: 1 commit per PR branch)
- **HEAD (pre-Bug-3-fix)**: `edb9b18a2 feat(l1): binary trie backend (EIP-7864)` (force-with-lease pushed; `origin/shared-trie-binary` matches)
- **HEAD (post-Bug-3-fix, ready to amend)**: working tree on top of `edb9b18a2`
- **Base**: `04279f146c` on `shared-trie` (rebased onto `origin/main`)
- **Phases done**: 0–7
- **Phases remaining** (renumbered as of 2026-05-04 user decision):
  - **Phase 8** (NEW): `--binary-from-genesis` — full-sync from genesis directly into binary trie. Plan saved at `docs/binary-trie/genesis-binary-plan.md` (untracked, awaits implementation).
  - Phase 9 (was 8): RPC + metrics.
  - Phase 10 (was 9): integration tests + docs.
  - Phase 11 (was 10): polish.

### Tests (post-Phase-7)

- `ethrex-binary-trie` lib → 143 unit + 1 ignored bench
- `ethrex-storage` lib → 46 (incl. `peek_backend_format_byte_does_not_populate_fresh_datadir` hotfix regression test, `binary_transition_restart_cycle`, `binary_transition_locked_without_flag`)
- `ethrex-blockchain` lib → 5 (incl. 4 transition_activator + `store_block_skips_state_root_validation_for_non_mpt_backend`)
- `ethrex-p2p` lib → 21
- `cargo check --workspace`, `cargo clippy -p ethrex-blockchain -p ethrex-storage -p ethrex-p2p --lib --no-deps -- -D warnings`, `make check-cargo-lock` all clean

### Test vector regen

- JSONs under `crates/common/binary-trie/testgen/*.json` are git-ignored.
- Regenerate via `make binary-trie-vectors` (requires `python3` + `pip install blake3`).
- Without regenerated JSONs, the 7 vector tests skip with a `SKIP:` log line; everything else still passes.

## Read first (in order)

1. `docs/binary-trie/overview.md` — scope, activation state machine, key invariants.
2. `docs/binary-trie/design-decisions.md` — locked choices (§13 merkleization, §11 genesis-from-binary now reversed by Phase 8).
3. `docs/binary-trie/plan.md` §2a (hard rules), §3 (locked decisions), §6 (phases 0–7 done; 8/9/10 in plan.md are stale numbering — see "Phase numbering note" below).
4. `docs/binary-trie/genesis-binary-plan.md` — the new Phase 8 plan, locked decisions, 15 tasks, 7 mandatory tests, audit task X.0.
5. `docs/binary-trie/phases/phase-7-handoff.md` — round-by-round deviations for the most recent phase. Phases 1–6 handoffs were removed once their work was squashed into the single PR commit; their content (atomic CoW invariant, tombstone shape, layer-cache thresholds, etc.) lives in `design-decisions.md` and `overview.md`.
6. `docs/binary-trie/{rpc,testing,operational}.md` — Phase 9/10 reference.
7. `docs/shared-trie/{spec,adding-a-backend}.md` — abstraction layer this PR builds on.

Don't dive into source until the doc pass is done. The plan's locked constraints are how to read the source meaningfully.

### Phase numbering note

`plan.md` §6 still calls them Phases 8/9/10 (RPC, integration, polish). The user approved renumbering to 9/10/11 to make room for genesis-binary as Phase 8. **The renumbering edit to `plan.md` itself is a Phase 11 docs-pass task; do not edit `plan.md` mid-phase.** Refer to `genesis-binary-plan.md` as authoritative for "Phase 8" until then.

## Hard rules (plan §2a, non-negotiable)

1. No deferrals. No skipping. Escalate per §11 if blocked.
2. No `TODO`/`todo!()`/`unimplemented!()`/`FIXME` in merged code. `unreachable!()` only with a justifying comment.
3. No "v1 fallback" shortcuts. Locked decisions don't have escape hatches.
4. Every checkpoint is a hard gate. Failing tests = phase not complete.
5. "Already covered"/"redundant" are not acceptable justifications for code-reviewer findings unless the plan explicitly marks the item implicit-in-X.
6. Re-run `code-reviewer` after non-trivial follow-up commits.

## Process applied this session (Phase 7)

Phase 7 needed **4 plan-implementer rounds + 4 code-reviewer rounds + 1 planner-found Blocker** before close. Trend: implementer rounds 1–3 each shipped substituted/dead/tautological code that passed the previous reviewer round. Round 4 fixed a Blocker (state-root gate ungated) caught by the feature-planner during genesis-binary planning, NOT by code-reviewer.

When resuming for Phase 8:
1. Run environment verification (below). All must pass.
2. Read `docs/binary-trie/genesis-binary-plan.md` end-to-end. Resolve any "Resolved Audit" stubs in §11 first (Task X.0).
3. Launch `plan-reviewer` (Sonnet) on `genesis-binary-plan.md` before implementation. Per CLAUDE.md the auto-rule applies to 2+ phase plans; we extend it here because the previous phase's review-round-3 still missed a Blocker.
4. Launch `plan-implementer` (Sonnet) for Phase 8. Prompt must include: starting commit SHA `5ef4068c33`, plan reference, §2a quoted inline, explicit out-of-scope list (RPC=Phase 9, EF tests excluded by user, no dual-node diff), handoff-doc requirement.
5. After plan-implementer returns, **independently verify** (don't trust the agent):
   - `cargo check --workspace`
   - `cargo test -p ethrex-storage -p ethrex-blockchain -p ethrex-binary-trie`
   - `cargo clippy -p <crate> --lib --no-deps -- -D warnings`
   - `make check-cargo-lock`
   - No `TODO`/`todo!()`/`unimplemented!()`/`FIXME` introduced.
   - Test counts match the handoff claim. **Read test bodies — names lie.**
6. Run `code-reviewer` on the diff. Fix every finding (Blocker/Major before phase-close; Minor/Nit unless user defers).
7. Re-run `code-reviewer` on the follow-up commits. Plan for 2–4 rounds.
8. Update `phases/phase-8-handoff.md` honestly. "Deviations: None" is a smell when non-trivial work was done.
9. When the phase closes, amend onto the single `shared-trie-binary` commit and `git push --force-with-lease=shared-trie-binary:<old SHA> origin shared-trie-binary`.

## Environment verification

```bash
cd /data2/edgar/work/ethrex
git rev-parse --abbrev-ref HEAD                # expect: shared-trie-binary
git log --oneline -1                           # expect: 5ef4068c33 (or whatever HEAD is now)
git status --porcelain                         # expect: only "?? docs/binary-trie/genesis-binary-plan.md" until Phase 8 lands
cargo check --workspace                        # clean
cargo test -p ethrex-binary-trie               # 143 unit + 1 ignored
cargo test -p ethrex-storage --lib             # 46 unit (rocksdb feature gates one)
cargo test -p ethrex-blockchain --lib transition_activator  # 4 tests
make check-cargo-lock                          # clean
gh api repos/lambdaclass/ethrex/git/trees/eip-7864-plan --jq '.sha' >/dev/null  # reference branch ping
```

## Phase 7 in two paragraphs (now done)

`--binary-transition` flag in `cmd/ethrex/cli.rs`. `SyncManager` now owns `caught_up: Arc<AtomicBool>` (one-shot latch, `Release`/`Acquire`) and `last_fcu_finalized: Arc<Mutex<(H256, u64)>>` populated by `engine_forkchoiceUpdated`. After every successful block commit, `check_and_latch_caught_up` flips `caught_up = true` once `committed_number >= finalized_number`. `TransitionActivator::tick(store, head_block_number)` polls the two preconditions; when both hold, runs the 10-step `activate()` sequence (acquire `Store::activation_lock()`, re-verify, stop FKV gen, `force_commit_layers()`, `drain_trie_update_worker()` via the rendezvous channel, read `frozen_mpt_root` from `get_latest_canonical_block_header()` (returns `None` on a fresh store; activator errors out instead of silently writing zero), persist 3 transition meta keys + flip `STATE_BACKEND_FORMAT_KEY` to `2` atomically, log, `cancel_token.cancel()` for graceful shutdown). Process exits 0; operator relaunches with the same flag and the restart path detects byte 2 → constructs `TransitionBackend`. Format-byte-2 + flag-absent returns `StoreError::Custom("...format byte 2 (transition) but --binary-transition was not passed...")`. `validate_state_root` is gated on `BackendKind == Mpt` at both call sites in `blockchain.rs` (`store_block` line 937 + batch path line 1448) — Binary/Transition modes compute the root for storage but never compare it against the header.

Hotfix shipped in the same commit: `Store::peek_backend_format_byte` previously called `RocksDBBackend::open(path)` unconditionally, which writes RocksDB bootstrap files (CURRENT, MANIFEST, OPTIONS-*) into the datadir. The next `Store::new` then saw a non-empty dir without `metadata.json` and bailed with `NotFoundDBVersion`. Fix: gate the open on `has_valid_db(path)` — fresh datadirs return `None` immediately. Regression test `peek_backend_format_byte_does_not_populate_fresh_datadir` asserts the dir stays empty after the call (rocksdb-feature-gated).

## Locked design decisions for Phase 8 (genesis-binary; from `genesis-binary-plan.md` §3)

- **`--binary-from-genesis`** boolean flag. Mutually exclusive with `--binary-transition` (clap `conflicts_with`) and with `--syncmode=snap` (manual check in `init_l1`).
- **Genesis block 0 header `state_root` = canonical MPT genesis root** (computed via existing `genesis_root()`). Binary genesis root computed internally only — never overwrites the header. Network-compatible: peer-following works.
- **From block 1 onward**: peer headers stored verbatim; binary state computed internally; no comparison (Phase 7 gate).
- **Pre-existing-DB**: format byte `0` (Mpt) or `2` (Transition) + flag → startup error. Format byte `1` (Binary) + flag → restart.
- **Cross-validation**: LEVM unit tests + per-block always-on header verification (receipts root, transactions root, withdrawals root, gas, logs bloom). **No EF tests against binary backend** (user excluded). **No dual-node diff** (user rejected).
- **`Genesis::alloc` iteration**: sorted by address before iterating (deterministic node_id allocation).
- **Format byte**: `1` (`Binary`); `Store::from_backend` already handles it.
- **Hardfork system contracts at genesis** (beacon roots, history storage, etc.): written via the same `Genesis::alloc` walk used for MPT genesis — no special-case logic.
- **Network compatibility**: genesis-binary nodes CAN follow public mainnet/testnets (genesis hash matches because we emit canonical MPT root in block 0).

## Phase 7 lessons (apply to Phase 8 forward)

1. **Read test bodies, not names.** Round 1 implementer substituted `binary_transition_restart_cycle` (must reopen same backend → assert `Transition`) with an "second tick returns Skip" idempotency test, AND `binary_transition_locked_without_flag` (must call `Store::from_backend` with mismatched kind → assert specific `StoreError`) with another fast-path-skip test. Both passed under their original names. Caught only by reading the bodies. The Phase 6 `Arc<InMemoryBackend>` shared-backend pattern (`test_transition_restart_with_overlay`) was the precedent the implementer ignored.
2. **Dead code in fixes.** Round 2's `unwrap_or_default → ok_or_else` on `get_latest_canonical_block_header` was unreachable because the function always returned `Ok(Some(...))`. Round 3 had to fix the upstream signal (read `CHAIN_DATA/LatestBlockNumber`) AND keep the guard as defense-in-depth.
3. **Tautological tests.** Round 2's Task 7.8 test asserted what `Blockchain::new` always does (initializes `transition_activator: None`). Bidirectional companion (`set_transition_activator_installs_activator`) added in round 3 to give meaningful coverage. Honest naming (`transition_activator_starts_none` for the constructor invariant) over the lying name.
4. **Audit completeness.** Round 1 ordered `disable_snap` to `Release`; missed two more `snap_enabled.store(_, Relaxed)` writes in `snap_sync.rs:225,264` and one in `sync_manager.rs:81`. Round 3 audited all four sites + tightened reads to `Acquire`.
5. **"Deviations: None" is a smell** when non-trivial work was done. The implementer wrote it twice in Phase 7 round-1 / round-2 handoffs; both times reviewers found real deviations.
6. **Plans miss things.** Phase 7's `validate_state_root` gate was a locked design decision in §3 but had NO task in §6's task list. Three reviewer rounds approved Phase 7 before feature-planner caught it during genesis-binary planning. Round 4 fixed it as a Blocker. **For Phase 8: cross-check the locked decisions table against the task list during plan-reviewer. If any locked decision has no task implementing it, that's a Blocker.**
7. **Hotfix discipline.** The `peek_backend_format_byte` bootstrap-files bug shipped to the user in the first push; they hit it within minutes. Lesson: any function that opens a backend at all should gate on `has_valid_db` if it's part of the startup detect-format path. Probably worth a defensive review of similar paths in Phase 8.

## Open questions

- **Snap sync post-switch is not supported**. Snap sync stays MPT-only; transition activates after snap completes. A fresh node cannot snap-sync the binary era. Acceptable for follower-only research. Documented in `operational.md`.
- **Snap sync into binary-from-genesis is also not supported** (user-locked: `--binary-from-genesis` requires `--syncmode=full`). Documented in `genesis-binary-plan.md` §3 row 3.

## Phase 8 next steps

1. Run plan-reviewer on `docs/binary-trie/genesis-binary-plan.md` (auto per CLAUDE.md spirit; Phase 7 demonstrated review value).
2. Address any plan-reviewer findings.
3. Launch plan-implementer for Phase 8 with the prompt template above.
4. Iterate implementer + reviewer rounds until clean.
5. Amend onto `5ef4068c33` and `git push --force-with-lease=shared-trie-binary:5ef4068c33 origin shared-trie-binary`.
6. Update this `SESSION-HANDOFF.md` again at the close of Phase 8.
