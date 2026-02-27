# Tokamak Remaining Work Roadmap

**Created**: 2026-02-24 | **Updated**: 2026-02-27
**Context**: Overall ~95% complete. JIT core done (Phases 2-8). Phase A: ALL P0 COMPLETE (A-1 ✅ A-2 ✅ A-3 ✅ A-4 ✅). Phase B: B-1 ✅ B-2 ✅ B-3 ✅ — ALL COMPLETE. Phase C: C-1 ✅ C-2 ✅ C-3 ✅ — ALL COMPLETE. Phase D: D-1 decided (accept), D-2 ✅ DONE, D-3 ✅ DONE. Phase E: E-1 ✅ DONE, E-2 ✅ DONE, E-3 ✅ DONE — ALL COMPLETE. Phase F: F-1 ✅ DONE, F-2 ✅ DONE, F-3 ✅ DONE (scaffolding), F-4 ✅ DONE, F-5 CI CONFIGURED (awaiting sync run). Phase G: G-1 ✅ DONE (arena allocator), G-2 ✅ DONE (auto-resolved by G-1), G-3 ✅ DONE (CALL/CREATE dual-execution validation), G-4 ✅ DONE (JIT-to-JIT direct dispatch), G-5 ✅ DONE (parallel compilation pool), G-6 ✅ DONE (LRU cache eviction — AtomicU64 timestamps), G-7 ✅ DONE (constant folding enhancement — 22 opcodes + unary patterns).

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

### D-1. Recursive CALL Performance [P2] — DECISION: (c) Accept for v1.0 → G-4 RESOLVES
- Current: JIT suspend -> LEVM dispatch -> JIT resume is extremely slow
- **Decision**: (c) Accept limitation for v1.0 — non-recursive scenarios already 2-2.5x speedup
- **UPDATE**: G-4 (JIT-to-JIT Direct Dispatch) resolves this for JIT-compiled children — child bytecodes in JIT cache execute directly without full suspend/resume overhead. Deep recursive patterns (FibonacciRecursive) still use suspend/resume but shallow CALL patterns (ERC20) benefit from fast dispatch.
- **Dependency**: B-1 ✅
- **Resolved by**: G-4 ✅

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
- **Enhanced by G-7**: expanded from 6 to 22 opcodes + unary pattern support (see G-7 below)
- **Verification**: 68 unit tests + 8 integration tests (76 total, after G-7) ✅
- **Dependency**: D-1 ✅, D-2 ✅
- **Completed**: Session fec956fef (base), 43026d7cf (G-7 enhancement)

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

## Phase G: Memory & Stability (P1)

> "Arena allocator eliminates the 1-5 MB/compilation LLVM memory leak."

### G-1. LLVM Memory Lifecycle [P1] ✅ DONE
- Arena allocator replacing `std::mem::forget(compiler)` with tracked lifecycle ✅
- `ArenaManager`, `ArenaEntry`, `FuncSlot` types in `levm/jit/arena.rs` with CAS-based concurrent eviction tracking ✅
- `ArenaCompiler` in `tokamak-jit/compiler.rs` stores compilers instead of leaking them ✅
- `compile_in_arena()` alongside existing `compile()` for backward compat ✅
- `thread_local! ArenaState` in `lib.rs` handler manages arena rotation + eviction-triggered freeing ✅
- `CompilerRequest::Free{slot}` and `FreeArena{arena_id}` request variants ✅
- `JitConfig` extended: `arena_capacity`, `max_arenas`, `max_memory_mb` ✅
- `JitMetrics` extended: `arenas_created`, `arenas_freed`, `functions_evicted` ✅
- **Verification**: 12 arena + 4 ArenaCompiler tests, all 178 tests pass (94 levm + 36 jit + 48 bench) ✅
- **Dependency**: None
- **Completed**: 2026-02-26 — Arena allocator replacing mem::forget, 178 tests pass (f8e9ba540)

### G-2. Cache Eviction Effectiveness [P0-CRITICAL] ✅ DONE
- G-1 arena system already handles `Free{slot}` and `FreeArena{arena_id}` requests ✅
- `cache.insert()` returns `Option<FuncSlot>` on eviction → `ArenaManager::mark_evicted()` → `free_arena()` when empty ✅
- `CompilerRequest::Free` handler in `lib.rs` decrements arena live count and frees empty arenas ✅
- No additional implementation needed — auto-resolved by G-1 ✅
- **Dependency**: G-1 ✅
- **Completed**: 2026-02-27 — G-1 arena system already handles Free/FreeArena — auto-resolved (f8e9ba540)

### G-3. CALL/CREATE Dual-Execution Validation [P1-SIGNIFICANT] ✅ DONE
- Removed `!compiled.has_external_calls` guard from `vm.rs` validation path ✅
- Dual-execution validation now runs for ALL bytecodes including CALL/CREATE/DELEGATECALL/STATICCALL ✅
- Interpreter replay handles sub-calls natively via `handle_call_opcodes()` ✅
- State-swap mechanism (`swap_validation_state`) works correctly for external-call bytecodes ✅
- Refactored test infrastructure: shared `MismatchBackend`, `make_external_call_bytecode()`, `setup_call_vm()`, `run_g3_mismatch_test()` helpers ✅
- **Verification**: 5 G-3 tests (CALL/STATICCALL/DELEGATECALL mismatch + pure regression + combined), 41 total tokamak-jit tests ✅
- **Dependency**: G-1 ✅, G-2 ✅
- **Completed**: 2026-02-27 — removed has_external_calls guard, 5 tests, 751 lines (8c05d3412)

### G-4. JIT-to-JIT Direct Dispatch [P1-SIGNIFICANT] ✅ DONE
- VM-layer fast dispatch: child CALL bytecodes checked against JIT cache ✅
- `run_subcall_with_jit_dispatch()` in vm.rs — direct JIT execution for cached children ✅
- Recursive suspend/resume loop for nested JIT calls (child also suspends on CALL) ✅
- Precompile guard + CREATE exclusion (init code needs validate_contract_creation) ✅
- Error-safe: JIT failure in child treated as revert (state may be partially mutated) ✅
- `enable_jit_dispatch` config toggle (default: true) + `jit_to_jit_dispatches` metric ✅
- **Verification**: 10 G-4 tests (simple/checked/revert/nested/fallback/differential/CREATE/depth/config/multi), 48 total tokamak-jit tests ✅
- **Dependency**: G-1 ✅
- **Completed**: 2026-02-27 — Fast JIT dispatch in VM layer, 10 tests

### G-5. Parallel Compilation [P2] ✅ DONE
- Replaced single `CompilerThread` (mpsc) with `CompilerThreadPool` (crossbeam-channel multi-consumer) ✅
- Configurable N workers via `JitConfig.compile_workers` (default: `num_cpus / 2`, min 1) ✅
- Each worker has its own `thread_local! ArenaState` — LLVM context thread-affinity preserved ✅
- Deduplication guard (`compiling_in_progress` set) prevents duplicate compilations across workers ✅
- `crossbeam-channel` unbounded channel for fair work distribution ✅
- `num_cpus` crate for automatic worker count selection ✅
- **Verification**: 4 G-5 tests (concurrent compilation, single worker equiv, deduplication guard, different keys), 48 total tokamak-jit tests ✅
- **Dependency**: G-1 ✅
- **Completed**: 2026-02-27 — CompilerThreadPool + deduplication guard + 4 tests

### G-6. LRU Cache Eviction [P2] ✅ DONE
- Replaced FIFO (`VecDeque`) with LRU eviction using per-entry `AtomicU64` timestamps ✅
- `CacheEntry` wrapper: `Arc<CompiledCode>` + `AtomicU64` last_access timestamp ✅
- `CodeCache.access_counter: Arc<AtomicU64>` — monotonic counter outside `RwLock` ✅
- `get()`: atomic timestamp update under read lock (no write lock needed) ✅
- `insert()`: O(n) min_by_key scan to find LRU entry (n ≤ max_cache_entries) ✅
- Removed `insertion_order: VecDeque<CacheKey>` entirely ✅
- **Verification**: 9 cache unit tests + 5 integration tests, 53 total tokamak-jit tests ✅
- **Dependency**: G-1 ✅
- **Completed**: 2026-02-27 — AtomicU64 LRU eviction replacing FIFO

### G-7. Constant Folding Enhancement — Expanded Opcodes [P2] ✅ DONE
- Expanded D-3 optimizer from 6 binary opcodes to 20 binary + 2 unary opcodes (22 total) ✅
- **Binary opcodes added**: DIV, SDIV, MOD, SMOD, EXP, SIGNEXTEND, LT, GT, SLT, SGT, EQ, SHL, SHR, SAR ✅
- **Unary opcodes added**: NOT, ISZERO (new `UnaryPattern` type + `detect_unary_patterns()` scanner) ✅
- Signed arithmetic helpers: `is_negative`, `negate`, `abs_val`, `u256_from_bool` (exact LEVM semantics) ✅
- Refactored eval_op into 6 extracted helpers (eval_sdiv, eval_smod, eval_signextend, signed_compare, eval_sar, eval_shift) ✅
- Extracted shared `write_folded_push()` helper eliminating duplicate rewrite logic ✅
- `OptimizationStats` extended with `unary_patterns_folded` field ✅
- Proptest convergence check updated for unary patterns ✅
- **Verification**: 68 unit tests + 8 integration tests (76 total), clippy clean both states ✅
- **Dependency**: D-3 ✅
- **Completed**: 2026-02-27 — 22 opcodes + unary patterns + refactored helpers (43026d7cf)

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
Week 9:  [P1] G-1 ✅ (arena allocator) + G-3 ✅ (CALL/CREATE validation) + G-5 ✅ (parallel compilation) + G-7 ✅ (constant folding 22 opcodes)
Week 10: [P1] G-4 ✅ (JIT-to-JIT direct dispatch) + G-6 ✅ (LRU cache eviction)
Later:   [P3] F-5
```

---

## Decisions Needed

| Decision | Options | Recommendation |
|----------|---------|----------------|
| Recursive CALL strategy | (a) Inline (b) JIT-to-JIT (c) Accept | **(c) Accept for v1.0** ✅ decided — **G-4 resolves** via VM-layer fast JIT dispatch |
| Bytecode size limit | (a) Chunk (b) Fallback (c) Upstream fix | (b) Fallback -- least effort, already works |
| L2 timeline | (a) Now (b) After mainnet (c) Skip | (b) After mainnet -- L1 correctness first |
| Debugger scope | (a) Full Web UI (b) CLI only (c) Skip | (b) CLI MVP -- prove value, web UI in v1.1 |
