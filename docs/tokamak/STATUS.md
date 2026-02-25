# Tokamak Client Status Report

**Date**: 2026-02-26
**Branch**: `feat/tokamak-proven-execution`
**Overall Completion**: ~55-60%

---

## Phase Completion

| Phase | Description | Completion | Status |
|-------|-------------|-----------|--------|
| Phase 0 | Research & Decision | **100%** | ethrex fork confirmed (FINAL) |
| Phase 1 | Foundation | **~98%** | Hive 6/6 PASS (PR #6260), Snapsync 수동 실행 필요 |
| Phase 2 | JIT Foundation (revmc) | **100%** | LLVM backend integrated |
| Phase 3 | JIT Execution Wiring | **100%** | LevmHost + execution bridge |
| Phase 4 | Production JIT Hardening | **100%** | LRU cache, auto-compile, tracing bypass |
| Phase 5 | Advanced JIT | **100%** | Multi-fork, async compile, validation mode |
| Phase 6 | CALL/CREATE Resume | **100%** | Suspend/resume + LLVM memory mgmt |
| Phase 7 | Dual-Execution Validation | **100%** | State-swap validation, Volkov R20 PROCEED |
| Phase 8 | JIT Benchmarking | **100%** | Infrastructure + benchmark execution |
| Phase 9 | Benchmark CI & Dashboard | **~75%** | C-1 ✅ C-2 ✅ C-3 ✅ — All Phase C tasks complete. Dashboard remaining (F-2). |

---

## Tier S Features

### Feature #9: JIT-Compiled EVM (~80%)

**Completed:**
- revmc/LLVM backend integration (Phases 2-8)
- Tiered execution (counter threshold -> compile -> execute)
- Multi-fork support (cache key includes Fork)
- Background async compilation (CompilerThread)
- LRU cache eviction
- CALL/CREATE suspend/resume
- Dual-execution validation (JIT vs interpreter)
- Benchmarking infrastructure + initial results
- Bytecode size limit graceful fallback (D-2) — negative cache + early size gate + interpreter-only bench results
- Constant folding optimizer (D-3) — PUSH+PUSH+OP → single PUSH, 6 opcodes (ADD/MUL/SUB/AND/OR/XOR), 42 tests
- 76 LEVM JIT tests + 27 tokamak-jit tests passing (104 total)

**Remaining:**
- Recursive CALL performance (suspend/resume is slow — accepted for v1.0)
- Tiered optimization (profile-guided optimization)
- Fuzzing + security audit
- Production deployment

### Feature #10: Continuous Benchmarking (~60%)

**Completed:**
- `tokamak-bench` crate with 12 scenarios
- CLI: `run` / `compare` / `report` / `jit-compare` subcommands
- Regression detection with thresholds (opcode + JIT speedup)
- CI workflow (`pr-tokamak-bench.yaml`) with JIT benchmark jobs
- JIT benchmark infrastructure
- JSON output + markdown report generation
- JIT speedup regression detection with PR comments

**Remaining:**
- Geth/Reth comparison via JSON-RPC
- State root differential testing
- Public dashboard (clients.tokamak.network)
- Precompile timing export

### Feature #21: Time-Travel Debugger (~25%)

**Completed:**
- `tokamak-debugger` crate with replay engine (E-1)
- LEVM `OpcodeRecorder` hook trait (feature-gated `tokamak-debugger`)
- Per-opcode step recording: opcode, PC, gas, depth, stack top-N, memory size, code address
- Forward/backward/goto navigation API (`ReplayEngine`)
- Stack `peek()` for non-destructive stack inspection
- 14 tests: basic replay (4), navigation (5), gas tracking (3), nested calls (2)

**Remaining:**
- Interactive CLI (step, breakpoint, inspect) — E-2
- `debug_timeTravel` RPC endpoint — E-3
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
**Skipped**: FibonacciRecursive/FactorialRecursive/ERC20* (recursive CALL suspend/resume too slow).

---

## Tokamak-Specific Codebase

| Component | Location | Lines |
|-----------|----------|-------|
| LEVM JIT infra | `crates/vm/levm/src/jit/` (9 files) | ~2,700 |
| tokamak-jit crate | `crates/vm/tokamak-jit/src/` (14 files) | ~5,650 |
| tokamak-bench crate | `crates/tokamak-bench/src/` (7 files) | ~1,305 |
| tokamak-debugger | `crates/tokamak-debugger/src/` (8 files) | ~450 |
| LEVM debugger hook | `crates/vm/levm/src/debugger_hook.rs` | ~27 |
| **Total** | | **~10,130** |

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

### CI Verified (PR #6260, run 22379067904)
- Hive 6/6 suites PASS (tokamak-jit build) — RPC, Devp2p, Auth, Cancun, Paris, Withdrawals
- Quality Gate PASS — cargo check/clippy/test with all tokamak features
- Docker Build (tokamak-jit) PASS
- Feature flag safety confirmed — tokamak-jit Hive == upstream (both 6/6)

### Awaiting Manual Verification
- Hoodi testnet sync (`tokamak-sync.yaml` workflow_dispatch 수동 트리거 필요)

### Not Started
- Mainnet full sync as Tokamak client
- L2 integration (`tokamak-l2` flag declared, no implementation)
- Time-Travel Debugger CLI (E-2) + RPC endpoint (E-3)
- Cross-client benchmark (Geth/Reth comparison)
- Public benchmark dashboard
- EF grant application
- External node operator adoption

### In Progress
- (none — Phase B, C, D, E-1 complete; next: E-2 Debugger CLI or A-2 Hoodi sync)

---

## Architecture Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Base client | ethrex (LambdaClass) | Rust, LEVM custom EVM, active development |
| JIT backend | revmc (Paradigm) + LLVM 21 | Only functional backend (Cranelift lacks i256) |
| Cache key | `(H256, Fork)` | Fork-specific compiled code |
| Compilation | Background thread (mpsc) | Non-blocking hot path |
| Validation | State-swap dual execution | JIT runs first, interpreter re-runs to verify |
| Memory | `mem::forget(compiler)` | Leak LLVM context to keep fn ptrs alive |
