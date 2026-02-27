# Tokamak Client Status Report

**Date**: 2026-02-27
**Branch**: `feat/tokamak-proven-execution`
**Overall Completion**: ~94%

---

## Phase Completion

| Phase | Description | Completion | Status |
|-------|-------------|-----------|--------|
| Phase 0 | Research & Decision | **100%** | ethrex fork confirmed (FINAL) |
| Phase 1 | Foundation | **100%** | Hive 6/6 PASS, Hoodi sync PASS (1h48m), all P0 complete |
| Phase 2 | JIT Foundation (revmc) | **100%** | LLVM backend integrated |
| Phase 3 | JIT Execution Wiring | **100%** | LevmHost + execution bridge |
| Phase 4 | Production JIT Hardening | **100%** | LRU cache, auto-compile, tracing bypass |
| Phase 5 | Advanced JIT | **100%** | Multi-fork, async compile, validation mode |
| Phase 6 | CALL/CREATE Resume | **100%** | Suspend/resume + LLVM memory mgmt |
| Phase 7 | Dual-Execution Validation | **100%** | State-swap validation, Volkov R20 PROCEED |
| Phase 8 | JIT Benchmarking | **100%** | Infrastructure + benchmark execution |
| Phase 9 | Benchmark CI & Dashboard | **100%** | C-1 ✅ C-2 ✅ C-3 ✅ — All Phase C tasks complete. F-2 Dashboard ✅ DONE. |

---

## Tier S Features

### Feature #9: JIT-Compiled EVM (~80%)

**Completed:**
- revmc/LLVM backend integration (Phases 2-8)
- Tiered execution (counter threshold -> compile -> execute)
- Multi-fork support (cache key includes Fork)
- Background async compilation (CompilerThreadPool — multi-worker, G-5)
- LRU cache eviction
- CALL/CREATE suspend/resume
- Dual-execution validation (JIT vs interpreter)
- Benchmarking infrastructure + initial results
- Bytecode size limit graceful fallback (D-2) — negative cache + early size gate + interpreter-only bench results
- Constant folding optimizer (D-3 + G-7) — PUSH+PUSH+OP → single PUSH, 22 opcodes (6 original + 14 binary + 2 unary), 76 tests
- 76 LEVM JIT tests + 27 tokamak-jit tests passing (104 total)
- Arena-based LLVM memory lifecycle (G-1) — eliminates `mem::forget` memory leak, 178 tests pass
- CALL/CREATE dual-execution validation (G-3) — removed `has_external_calls` guard, validation runs for all bytecodes, 5 tests
- Parallel compilation thread pool (G-5) — crossbeam-channel multi-consumer, N workers (default num_cpus/2), deduplication guard, 4 new tests
- Constant folding enhancement (G-7) — expanded to 22 opcodes (DIV/SDIV/MOD/SMOD/EXP/SIGNEXTEND/LT/GT/SLT/SGT/EQ/SHL/SHR/SAR + NOT/ISZERO unary), refactored eval helpers, 68 unit + 8 integration tests
- JIT-to-JIT direct dispatch (G-4) — VM-layer fast dispatch for child CALL bytecodes, cache lookup + direct JIT execution, recursive suspend/resume, configurable + metrics, 10 tests
- LRU cache eviction (G-6) — replaced FIFO with AtomicU64-based LRU timestamps, lock-free get() hot path, O(n) eviction scan on insert(), 9 cache unit + 5 integration tests
- Precompile JIT acceleration (G-8) — `precompile_fast_dispatches` metric in JitMetrics, `enable_precompile_fast_dispatch` config toggle, metric tracking in `handle_jit_subcall()` precompile path, 9 tests (5 interpreter + 4 JIT differential)

**Remaining:**
- Tiered optimization (profile-guided optimization)
- Production deployment

### Feature #10: Continuous Benchmarking (~80%)

**Completed:**
- `tokamak-bench` crate with 12 scenarios
- CLI: `run` / `compare` / `report` / `jit-compare` subcommands
- Regression detection with thresholds (opcode + JIT speedup)
- CI workflow (`pr-tokamak-bench.yaml`) with JIT benchmark jobs
- JIT benchmark infrastructure
- JSON output + markdown report generation
- JIT speedup regression detection with PR comments
- Public dashboard (F-2) — Astro + React islands + Recharts + Tailwind at `dashboard/`, 62 JS/TS + 9 Python tests, `publish-dashboard` CI job

**Remaining:**
- State root differential testing
- Precompile timing export

### Feature #21: Time-Travel Debugger (~85%)

**Completed:**
- `tokamak-debugger` crate with replay engine (E-1)
- LEVM `OpcodeRecorder` hook trait (feature-gated `tokamak-debugger`)
- Per-opcode step recording: opcode, PC, gas, depth, stack top-N, memory size, code address
- Forward/backward/goto navigation API (`ReplayEngine`)
- Stack `peek()` for non-destructive stack inspection
- GDB-style interactive CLI (E-2) — 13 commands: step, step-back, continue, reverse-continue, break, delete, goto, info, stack, list, breakpoints, help, quit
- rustyline REPL with auto-history, `--bytecode <hex>` input mode
- `debug_timeTravel` JSON-RPC endpoint (E-3) — full TX replay over RPC with step windowing
- Serde serialization for all debugger types (StepRecord, ReplayTrace, ReplayConfig)
- 51 tests: basic replay (4), navigation (5), gas tracking (3), nested calls (2), serde (4), CLI parsing (12), formatter (6), execution (9), RPC handler (6)

**Remaining:**
- Web UI (optional)

---

## JIT Benchmark Results

Measured after Volkov R21-R23 fixes (corrected measurement order).
10 runs each, `--profile jit-bench`, Fork::Cancun.

| Scenario | Interpreter | JIT | Speedup |
|----------|------------|-----|---------|
| Fibonacci | 3.55ms | 1.40ms | **2.53x** |
| BubbleSort | 357.69ms | 159.84ms | **2.24x** |
| Factorial | 2.36ms | 1.41ms | **1.67x** |
| ManyHashes | 2.26ms | 1.55ms | **1.46x** |

**Interpreter-only**: Push/MstoreBench/SstoreBench (bytecode > 24KB, graceful fallback via D-2).
**Skipped**: FibonacciRecursive/FactorialRecursive (deep recursive CALL). ERC20* scenarios now benefit from G-4 JIT-to-JIT dispatch.

---

## Tokamak-Specific Codebase

| Component | Location | Lines |
|-----------|----------|-------|
| LEVM JIT infra | `crates/vm/levm/src/jit/` (9 files) | ~2,700 |
| tokamak-jit crate | `crates/vm/tokamak-jit/src/` (14 files) | ~5,650 |
| tokamak-bench crate | `crates/tokamak-bench/src/` (11 files) | ~1,700 |
| tokamak-debugger | `crates/tokamak-debugger/src/` (14 files) | ~1,310 |
| LEVM debugger hook | `crates/vm/levm/src/debugger_hook.rs` | ~27 |
| **Total** | | **~10,990** |

Base ethrex codebase: ~103K lines Rust.

---

## Volkov Review History

Three PROCEED milestones achieved:

| Review | Subject | Score | Verdict |
|--------|---------|-------|---------|
| R6 | DECISION.md | 7.5 | **PROCEED** |
| R10 | Architecture docs | 8.25 | **PROCEED** |
| R20 | Phase 7 dual-execution | 8.25 | **PROCEED** |
| R24 | Phase 8B cumulative | 8.0 | **PROCEED** |

Full review history: R1(3.0) -> R2(3.0) -> R3(5.25) -> R4(4.5) -> R5(4.0) ->
R6(7.5) -> R8(5.5) -> R9(6.5) -> R10(8.25) -> R13(3.0) -> R14(4.0) ->
R16(4.0) -> R17(4.0) -> R18(5.5) -> R19(7.0) -> R20(8.25) -> R22(3.5) ->
R23(5.0) -> R24(8.0)

---

## Outstanding Items

### Recently Completed (Infra)
- Hive CI infra — 6 suites in `pr-tokamak.yaml`, Docker build, Hive Gate (fc720f46f)
- Sync CI infra — `tokamak-sync.yaml` with Hoodi/Sepolia (fc720f46f)
- Feature flag CI — Quality Gate checks all 4 feature flags (fc720f46f)

### Recently Completed (Phase B/C)
- LLVM 21 CI provisioning (C-2) — Reusable composite action `.github/actions/install-llvm/`, removed `continue-on-error`, Polly fix (5ea9c8376)
- JIT benchmark CI (C-1) — `compare_jit()`, `JitCompare` CLI, 3 CI jobs, 10 tests, PR comment integration (d17a71c24)
- JIT gas alignment (B-1) — Fixed negative SSTORE refund bug in `execution.rs`, added `gas_alignment.rs` with 11 tests (71f39d2d7)
- Test quality improvements (B-2) — `test_helpers.rs`, `INTRINSIC_GAS` constant, 15+ test DRY refactors (224921e1f)
- Benchmark statistics (C-3) — `stats.rs` module, warmup/stddev/95% CI support, `--warmup` CLI param (224921e1f)
- EIP-7928 BAL recording (B-3) — BAL recording in host.rs sload/sstore JIT paths, 5 differential tests (2126e232b)
- Bytecode size limit fallback (D-2) — oversized_hashes negative cache, early size gate, bench interpreter-only results, 4+3 tests (ff3396efe)

### Recently Completed (Phase D)
- Constant folding optimizer (D-3) — same-length PUSH+PUSH+OP → single PUSH, 6 opcodes (ADD/MUL/SUB/AND/OR/XOR), pipeline integration in backend.rs, 37 unit + 5 integration tests (fec956fef)

### Recently Completed (Phase E)
- TX Replay Engine (E-1) — LEVM OpcodeRecorder hook, DebugRecorder, ReplayEngine with forward/backward/goto navigation, 14 tests
- Debugger CLI (E-2) — GDB-style REPL with 13 commands, rustyline, cli feature gate, 27 CLI tests (b6f304de1)
- debug_timeTravel RPC (E-3) — JSON-RPC endpoint, prepare_state_for_tx refactor, Evm::setup_env_for_tx, serde derives, feature-gated tokamak-debugger in ethrex-rpc, 10 tests (6 RPC + 4 serde)

### CI Verified (PR #6260, run 22379067904)
- Hive 6/6 suites PASS (tokamak-jit build) — RPC, Devp2p, Auth, Cancun, Paris, Withdrawals
- Quality Gate PASS — cargo check/clippy/test with all tokamak features
- Docker Build (tokamak-jit) PASS
- Feature flag safety confirmed — tokamak-jit Hive == upstream (both 6/6)

### Hoodi Sync Verified (run 22404315946)
- Hoodi snap sync PASS — 1h48m35s, `release-with-debug-assertions`, `--features tokamak-jit`
- assertoor `synced-check`: EL + CL both synced
- Ran on `ubuntu-latest` with Kurtosis + Lighthouse v8.0.1

### Recently Completed (Phase F)
- Cross-client benchmarking (F-1) — `cross-client` CLI subcommand, ethrex in-process + Geth/Reth via eth_call state overrides, comparison table with ethrex as 1.00x baseline, 18 tests
- Security audit prep (F-4) — cargo-fuzz harnesses (analyzer, optimizer, differential), 4 proptest property tests, SAFETY_AUDIT.md cataloging all 9 unsafe blocks with risk assessment; enhanced: real differential fuzzing (JIT vs interpreter dual-path, random bytecode gen, gas/status/output comparison) (b2def75e8)
- Public dashboard MVP (F-2) — Astro + React islands + Recharts + Tailwind at `dashboard/`, 16 TS interfaces + Zod schemas, TrendChart with CI bands, BenchTable, landing + trends pages, rebuild_index.py, publish-dashboard CI job, path traversal protection, 62 JS/TS + 9 Python tests (3294bdf97)

### Recently Completed (Phase F continued)
- L2 integration scaffolding (F-3) — `TokamakFeeConfig` + `JitPolicy` types, `VMType::TokamakL2` variant, `TokamakL2Hook` (wraps L2Hook via composition), hook dispatch + Evm constructors, `BlockchainType::TokamakL2` + 5 match arm updates, `--tokamak-l2` CLI flag, feature propagation across 6 Cargo.toml files, 7 tests
- Mainnet full sync CI (F-5) — Added `mainnet` option to `tokamak-sync.yaml`, `ethrex-sync` self-hosted runner for 48h timeout, Docker cleanup step, conditional Kurtosis install

### Recently Completed (Phase G)
- LLVM Memory Lifecycle (G-1) — Arena allocator replacing `mem::forget`, ArenaManager + ArenaCompiler + thread_local ArenaState, 12+4 arena tests, all 178 tests pass (f8e9ba540) (2026-02-26)
- Cache Eviction Effectiveness (G-2) — Auto-resolved by G-1 arena system: Free/FreeArena handlers, cache eviction returns FuncSlot for arena cleanup (2026-02-27)
- CALL/CREATE Dual-Execution Validation (G-3) — Removed `has_external_calls` guard, validation runs for ALL bytecodes (CALL/STATICCALL/DELEGATECALL), shared MismatchBackend + helpers, 5 tests (8c05d3412) (2026-02-27)
- Constant Folding Enhancement (G-7) — Expanded optimizer from 6 to 22 opcodes (14 new binary + 2 unary), signed arithmetic helpers, extracted eval helpers + write_folded_push, 68 unit + 8 integration tests (43026d7cf) (2026-02-27)
- JIT-to-JIT Direct Dispatch (G-4) — VM-layer fast dispatch: child CALL bytecodes checked against JIT cache and executed directly via `execute_jit()`, recursive suspend/resume for nested JIT calls, `enable_jit_dispatch` config + `jit_to_jit_dispatches` metric, 10 tests (2026-02-27)
- LRU Cache Eviction (G-6) — Replaced FIFO (VecDeque) with LRU eviction: per-entry AtomicU64 timestamps, monotonic access_counter outside RwLock, atomic-only get() hot path, O(n) min_by_key eviction on insert(), 9+5 tests (2026-02-27)
- Precompile JIT Acceleration (G-8) — `precompile_fast_dispatches` metric in JitMetrics, `enable_precompile_fast_dispatch` config toggle in JitConfig, `is_precompile_fast_dispatch_enabled()` on JitState, metric tracking in `handle_jit_subcall()` precompile path, 9 tests (5 interpreter correctness + 4 JIT differential), 58 total tokamak-jit tests (2026-02-27)

### Not Started
- EF grant application
- External node operator adoption

### In Progress
- Mainnet full sync (F-5) — CI configured (`tokamak-sync.yaml` with mainnet option, `ethrex-sync` self-hosted runner, 48h timeout), awaiting manual dispatch
- F-3 scaffolding done, Tokamak-specific fee logic awaits L2 spec

---

## Architecture Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Base client | ethrex (LambdaClass) | Rust, LEVM custom EVM, active development |
| JIT backend | revmc (Paradigm) + LLVM 21 | Only functional backend (Cranelift lacks i256) |
| Cache key | `(H256, Fork)` | Fork-specific compiled code |
| Compilation | Background thread (mpsc) | Non-blocking hot path |
| Validation | State-swap dual execution | JIT runs first, interpreter re-runs to verify |
| Memory | Arena allocator (G-1) | Groups compiled fns into arenas; free LLVM resources when all evicted |
