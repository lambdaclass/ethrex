# Tokamak Remaining Work Roadmap

**Created**: 2026-02-24 | **Updated**: 2026-02-26
**Context**: Overall ~90% complete. JIT core done (Phases 2-8). Phase A: ALL P0 COMPLETE (A-1 ✅ A-2 ✅ A-3 ✅ A-4 ✅). Phase B: B-1 ✅ B-2 ✅ B-3 ✅ — ALL COMPLETE. Phase C: C-1 ✅ C-2 ✅ C-3 ✅ — ALL COMPLETE. Phase D: D-1 decided (accept), D-2 ✅ DONE, D-3 ✅ DONE. Phase E: E-1 ✅ DONE, E-2 ✅ DONE, E-3 ✅ DONE — ALL COMPLETE. Phase F: F-1 ✅ DONE, F-2 ✅ DONE, F-3 ✅ DONE (scaffolding), F-4 ✅ DONE, F-5 CI CONFIGURED (awaiting sync run).

---

## Priority Classification

| Grade | Meaning | Rule |
|-------|---------|------|
| **P0** | Must-have | Launch impossible without this |
| **P1** | Important | Launch possible but quality at risk |
| **P2** | Nice-to-have | Improves experience but not blocking |
| **P3** | Backlog | Post-launch |

**Rule: P0 must ALL be done before touching P1.**

---

## Phase A: Production Foundation (P0)

> "Without Hive and sync, this is not an Ethereum client. It's a library."

### A-1. Hive Test Integration [P0] ✅ VERIFIED
- ~~Add Hive test suites to `pr-tokamak.yaml`~~ ✅
- ~~Suites: RPC Compat, Devp2p, Engine Auth, Engine Cancun, Engine Paris, Engine Withdrawals~~ ✅
- ~~Reuse upstream `check-hive-results.sh` + pinned Hive version~~ ✅
- **Verification**: All 6 Hive suites pass — ✅ PR #6260, run 22379067904
- **Done**: `fc720f46f` + `bd8e881` — Hive Gate PASS, all 6 suites green

### A-2. Testnet Sync Verification [P0] ✅ VERIFIED
- ~~Run Hoodi testnet sync using existing `tooling/sync/` infrastructure~~ ✅
- ~~Verify state trie validation passes~~ ✅ (release-with-debug-assertions profile, debug_assert! enabled)
- ~~Document sync time + any failures~~ ✅
- **Verification**: Hoodi snap sync completed in 1h48m35s — ✅ run 22404315946
- **Infra**: `fc720f46f` — `tokamak-sync.yaml` (manual dispatch, Hoodi/Sepolia, Kurtosis + Lighthouse, `--features tokamak-jit`)
- **Done**: `8f0328df7` — URL fix (`${GITHUB_REPOSITORY}`), ubuntu-latest runner, 3h timeout, assertoor synced-check PASS

### A-3. Tokamak Feature Flag Safety [P0] ✅ VERIFIED
- ~~Verify `--features tokamak` does NOT break Hive tests~~ ✅
- ~~Verify `--features tokamak-jit` does NOT break Hive tests~~ ✅
- ~~Key concern: JIT dispatch must not interfere with consensus~~ ✅
- **Verification**: Hive pass rate with tokamak-jit == upstream (both 6/6) — ✅ PR #6260
- **Done**: Quality Gate (all 4 flags) + Hive Gate (tokamak-jit build) all green

### A-4. Phase 1.2 Completion [P0] ✅ VERIFIED (9/9)
- ~~Build verification (Phase 1.2-5): all workspace crates compile with tokamak features~~ ✅
- ~~Record baseline Hive pass rate for Tokamak branch~~ ✅ (6/6 PASS, Hive Gate records baseline)
- ~~Document any regressions vs upstream~~ ✅ (0 regressions — same 6/6 pass rate)
- ~~Snapsync verification~~ ✅ (Hoodi snap sync PASS — run 22404315946)
- **Verification**: Phase 1.2 criteria 1-9 ALL PASS

---

## Phase B: JIT Hardening (P1)

> "JIT works but isn't production-safe yet."

### B-1. JIT Gas Accounting Alignment [P1] ✅ DONE
- Root-cause gas mismatch between JIT and interpreter ✅
- Fixed: negative SSTORE refund bug in `execution.rs` — `u64::try_from` silently dropped negative refunds ✅
- Known: JitOutcome::gas_used excludes intrinsic gas (handled by apply_jit_outcome) ✅
- Edge cases: SSTORE EIP-2200/EIP-3529 (zero→nonzero, nonzero→zero, restore, clear-then-restore) all tested ✅
- Documented: revmc upstream `REFUND_SSTORE_CLEARS = 15000` (pre-EIP-3529) vs LEVM 4800 — execution gas unaffected
- **Verification**: 11 gas alignment tests passing (7 SSTORE edge cases + 3 memory expansion + 1 combined) ✅
- **Dependency**: A-1 (need Hive for comprehensive testing)
- **Estimate**: 8-16h
- **Completed**: Session 71f39d2d7 — Fixed negative refund bug, added `gas_alignment.rs` test module

### B-2. Test Quality (Volkov R24 Recommendations) [P1] ✅ DONE
- R1: Extract `make_test_db()` helper from 4 duplicate test setups ✅
- R2: Replace `let _ =` in rollback with `eprintln!` logging — deferred (low impact)
- R3: Replace `21_000u64` magic number with named constant ✅
- R4: DRY merge `init_vm` / `init_vm_interpreter_only` — deferred (needs subcall.rs refactor)
- **Verification**: All tests pass, clippy clean ✅
- **Dependency**: None
- **Estimate**: 1-2h
- **Completed**: Session 224921e1f — Created `test_helpers.rs`, added `INTRINSIC_GAS` constant, refactored 15+ duplicate test setups

### B-3. EIP-7928 BAL Recording for JIT [P1] ✅ DONE
- Removed 4 TODO comments from host.rs ✅
- Implemented BAL recording in sload/sstore JIT paths (host.rs) ✅
- sload: record_storage_read unconditionally (revmc pre-validates gas) ✅
- sstore: implicit read + conditional write (skip no-op SSTORE) ✅
- **Verification**: 5 differential tests passing (bal_recording.rs) — JIT BAL == interpreter BAL ✅
- **Dependency**: B-1 ✅
- **Estimate**: 4-8h
- **Completed**: Session 2126e232b — BAL recording in host.rs, 5 differential tests (counter, sload-only, sstore-noop, sstore-change, multi-sstore)

---

## Phase C: Benchmark CI & Regression Detection (P1)

> "Performance gains mean nothing without regression prevention."

### C-1. Phase 9: JIT Benchmark CI [P1] ✅ DONE
- Add JIT benchmark job to `pr-tokamak-bench.yaml` ✅
- Compare JIT speedup ratios between PR and base ✅ (`compare_jit()` + `jit-compare` CLI)
- Flag regression if speedup drops >20% ✅ (exit code 1 on regression)
- 3 CI jobs: `jit-bench-pr`, `jit-bench-main`, `compare-jit-results` ✅
- PR comment with JIT speedup regression report ✅
- **Verification**: 10 unit tests passing (regression/improvement/edge cases) ✅
- **Dependency**: None
- **Estimate**: 4h
- **Completed**: Session d17a71c24 — `compare_jit()`, `JitCompare` CLI, `JitRegressionReport` types, CI jobs with LLVM 21 + `continue-on-error`

### C-2. LLVM 21 CI Provisioning [P1] ✅ DONE
- Created reusable `.github/actions/install-llvm/` composite action ✅
- Installs llvm-21, llvm-21-dev, libpolly-21-dev (fixes Polly linking issue) ✅
- Modern GPG key method (tee to trusted.gpg.d, not deprecated apt-key) ✅
- Updated `pr-tokamak.yaml` and `pr-tokamak-bench.yaml` to use the action ✅
- Removed `continue-on-error: true` from jit-backend and jit-bench jobs ✅
- **Verification**: JIT backend job now fails the PR if compilation breaks ✅
- **Dependency**: None
- **Estimate**: 4-8h
- **Completed**: Session 5ea9c8376 — Composite action + workflow updates

### C-3. Benchmark Statistics [P1] ✅ DONE
- Add warmup runs (discard first 2) ✅
- Add stddev + 95% confidence interval to output ✅
- Multiple independent trial invocations (not just loop iterations) ✅
- **Verification**: Benchmark output includes stddev, CI in JSON and markdown ✅
- **Dependency**: None
- **Estimate**: 2-4h
- **Completed**: Session 224921e1f — Created `stats.rs` module, added `--warmup` CLI param, warmup/stddev/CI support to tokamak-bench

---

## Phase D: Performance Optimization (P2)

> "From 2x to 3-5x target."

### D-1. Recursive CALL Performance [P2] — DECISION: (c) Accept for v1.0
- Current: JIT suspend -> LEVM dispatch -> JIT resume is extremely slow
- **Decision**: (c) Accept limitation for v1.0 — non-recursive scenarios already 2-2.5x speedup
- Impact: FibonacciRecursive, ERC20 scenarios remain skipped in benchmarks
- Future options (v1.1+):
  - (a) Inline small calls — inline child bytecode into parent JIT, ~20-30h
  - (b) JIT-to-JIT direct dispatch — skip LEVM for JIT-compiled children, ~30-40h, may need revmc changes
- **Dependency**: B-1 ✅
- **Rationale**: Most real-world ERC20 transfers use 1-2 CALL depth, not deep recursion. Invest effort in D-2 (bytecode fallback) first.

### D-2. Bytecode Size Limit — Graceful Interpreter Fallback [P2] ✅ DONE
- revmc hard limit: 24576 bytes (EIP-170 MAX_CODE_SIZE)
- **Decision**: (b) Explicit interpreter fallback with negative cache
- Added `oversized_hashes` negative cache to JitState — O(1) skip for known-oversized bytecodes ✅
- Early size gate in VM dispatch at compilation threshold ✅
- Belt-and-suspenders size check in background compiler thread ✅
- Benchmarks now report interpreter-only results instead of silently dropping oversized scenarios ✅
- **Verification**: 4 unit tests (dispatch.rs) + 3 integration tests (oversized.rs, revmc-gated) ✅
- **Dependency**: None
- **Completed**: Session ff3396efe

### D-3. Opcode Fusion / Constant Folding [P2] ✅ DONE
- Same-length PUSH+PUSH+OP → single wider PUSH replacement (no offset changes) ✅
- Supports ADD, SUB, MUL, AND, OR, XOR with SUB wrapping edge case handling ✅
- optimizer.rs: detect_patterns() scan + optimize() constant folding ✅
- Pipeline integration between analyze_bytecode() and TokamakCompiler::compile() ✅
- **Verification**: 37 unit tests + 5 integration tests (42 total) ✅
- **Dependency**: D-1 ✅, D-2 ✅
- **Completed**: Session fec956fef

---

## Phase E: Developer Experience (P2)

> "Time-Travel Debugger MVP."

### E-1. Debugger Core: TX Replay Engine [P2] ✅ DONE
- LEVM `OpcodeRecorder` hook trait in `debugger_hook.rs` (feature-gated `tokamak-debugger`) ✅
- `DebugRecorder` captures per-opcode step: opcode, PC, gas, depth, stack top-N, memory size, code address ✅
- `ReplayEngine::record()` executes TX with recorder, builds `ReplayTrace` ✅
- Navigation API: `forward()`, `backward()`, `goto()`, `current_step()`, `steps_range()` ✅
- Stack `peek()` method for non-destructive inspection ✅
- **Verification**: 14 tests passing — basic replay (4), navigation (5), gas tracking (3), nested calls (2) ✅
- **Dependency**: None (uses test-constructed bytecodes, not synced state)
- **Completed**: Session — LEVM hook + tokamak-debugger engine + 14 tests

### E-2. Debugger CLI [P2] ✅ DONE
- GDB-style interactive REPL with 13 commands: step, step-back, continue, reverse-continue, break, delete, goto, info, stack, list, breakpoints, help, quit ✅
- rustyline REPL with auto-history, `--bytecode <hex>` input mode ✅
- Feature-gated `cli` module (clap, rustyline, hex, ethrex-storage/blockchain/vm) ✅
- **Verification**: 27 CLI tests (12 parsing + 6 formatter + 9 execution) — total 41 tests with base 14 ✅
- **Dependency**: E-1 ✅
- **Completed**: Session b6f304de1

### E-3. debug_timeTravel RPC Endpoint [P2] ✅ DONE
- JSON-RPC method: `debug_timeTravel(txHash, { stepIndex, count, reexec })` ✅
- Returns: trace summary (totalSteps, gasUsed, success, output) + step window (opcode, stack, memory, code address) ✅
- Refactored `blockchain/tracing.rs` — extracted `prepare_state_for_tx()` reused by both `trace_transaction_calls` and time travel ✅
- Added `Evm::setup_env_for_tx()` wrapper in `vm/tracing.rs` ✅
- Added `Serialize` derives to `tokamak-debugger` types (StepRecord, ReplayTrace, ReplayConfig) ✅
- Feature-gated `tokamak-debugger` feature in ethrex-rpc ✅
- **Verification**: 6 RPC handler tests + 4 serde tests passing ✅
- **Dependency**: E-1 ✅, E-2 ✅
- **Completed**: Phase E fully complete

---

## Phase F: Ecosystem & Launch (P3)

### F-1. Cross-Client Benchmarking [P3] ✅ DONE
- `cross-client` CLI subcommand in tokamak-bench ✅
- ethrex runs in-process (no RPC overhead), Geth/Reth via `eth_call` with state overrides ✅
- Comparison table with ethrex as 1.00x baseline (JSON + markdown output) ✅
- Feature-gated `cross-client` (reqwest, tokio, url deps) ✅
- **Verification**: 61 tests passing (including 18 cross-client tests) ✅
- **Dependency**: A-2, C-1
- **Completed**: Cross-client benchmarking module with types, async runner, and report generation

### F-2. Public Dashboard [P3] ✅ DONE
- Astro + React islands + Recharts + Tailwind static site at `dashboard/` ✅
- 16 TypeScript interfaces mirroring Rust bench types, Zod runtime validation at fetch boundary ✅
- TrendChart with CI error bands (ComposedChart + Area + Line), BenchTable, MetricCard, ScenarioSelector ✅
- Landing page (metric cards + benchmark table) + Trends page (interactive line chart with scenario selector) ✅
- `rebuild_index.py` script for CI data pipeline (scans date-stamped dirs, generates index.json) ✅
- `publish-dashboard` CI job in `pr-tokamak-bench.yaml` (peaceiris/actions-gh-pages) ✅
- Path traversal protection in fetch layer ✅
- **Verification**: 62 JS/TS tests (Vitest + Testing Library) + 9 Python tests (unittest) ✅
- **Dependency**: F-1 ✅, C-1 ✅
- **Completed**: Session 3294bdf97 — Full dashboard MVP with 71 total tests

### F-3. L2 Integration [P3] ✅ DONE (scaffolding)
- `TokamakFeeConfig` struct with `JitPolicy` enum (composition over `FeeConfig`) ✅
- `VMType::TokamakL2` variant with feature-gated exhaustive match handling ✅
- `TokamakL2Hook` wrapping `L2Hook` via composition + proven execution metadata ✅
- Hook dispatch, Evm constructors, P256Verify precompile support ✅
- `BlockchainType::TokamakL2` + `TokamakL2Config` with all 5 match arm updates ✅
- `--tokamak-l2` CLI flag routing to `BlockchainType::TokamakL2` ✅
- Feature flag propagation: levm → common, vm → levm + common, blockchain → vm + common, cmd → vm + blockchain ✅
- **Verification**: 7 tests (4 TokamakFeeConfig serde + 3 TokamakL2Hook unit), clippy clean both states ✅
- **Dependency**: A-1 ✅
- **Remaining**: Tokamak-specific fee logic when L2 spec arrives (~24-64h)
- **Completed**: L2 scaffolding with composition pattern

### F-4. Security Audit Prep [P3] ✅ DONE
- `cargo-fuzz` harnesses: fuzz_analyzer, fuzz_optimizer, fuzz_differential ✅
- Property-based tests (proptest): analyzer_never_panics, basic_blocks_within_bounds, optimizer_preserves_length, optimizer_converges ✅
- SAFETY_AUDIT.md: catalog of all 9 unsafe blocks with risk assessment + mitigations ✅
- Found real optimizer limitation: not single-pass idempotent (folding creates new patterns) — documented ✅
- **Enhanced**: fuzz_differential rewritten for real dual-path execution (JIT vs interpreter), random EVM bytecode generation, 3-way comparison (status/gas/output), tokamak-jit optional dep with revmc-backend feature wiring ✅
- **Verification**: 31 tests passing (including 4 proptest) ✅
- **Dependency**: B-1, D-1
- **Completed**: Fuzzing harnesses + proptest + safety audit documentation (0e585ca07)
- **Enhanced**: 2026-02-26 — real differential fuzzing harness (b2def75e8)

### F-5. Mainnet Full Sync [P3] — CI CONFIGURED
- Full mainnet state sync as Tokamak client
- Verify state root matches at head
- **CI**: `tokamak-sync.yaml` — mainnet option with 48h timeout, `ethrex-sync` self-hosted runner, Docker cleanup
- **How to run**: `gh workflow run tokamak-sync.yaml -f network=mainnet` (manual dispatch)
- **Dependency**: A-2 ✅, A-3 ✅
- **Estimate**: 24-48h (mostly wait time)
- **Status**: Workflow configured — awaiting manual dispatch and sync completion

---

## Execution Order

```
Week 1:  [P0] A-1 ✅ + A-2 ✅ → A-3 ✅ → A-4 ✅ (9/9 ALL PASS)
Week 2:  [P1] B-2 ✅ + C-2 + C-3 ✅ (parallel) → B-1 ✅
Week 3:  [P1] C-1 ✅ + C-2 ✅ + B-3 ✅
Week 4:  [P2] D-1 decision ✅ + D-2 ✅ + D-3 ✅ → E-1 ✅
Week 5+: [P2] E-2 ✅ + E-3 ✅
Week 6:  [P3] F-1 ✅ + F-4 ✅ (parallel)
Week 7:  [P3] F-2 ✅ (dashboard MVP)
Week 8:  [P3] F-3 ✅ (L2 scaffolding)
Later:   [P3] F-5
```

---

## Decisions Needed

| Decision | Options | Recommendation |
|----------|---------|----------------|
| Recursive CALL strategy | (a) Inline (b) JIT-to-JIT (c) Accept | **(c) Accept for v1.0** ✅ decided — revisit (a)/(b) for v1.1 |
| Bytecode size limit | (a) Chunk (b) Fallback (c) Upstream fix | (b) Fallback -- least effort, already works |
| L2 timeline | (a) Now (b) After mainnet (c) Skip | (b) After mainnet -- L1 correctness first |
| Debugger scope | (a) Full Web UI (b) CLI only (c) Skip | (b) CLI MVP -- prove value, web UI in v1.1 |
