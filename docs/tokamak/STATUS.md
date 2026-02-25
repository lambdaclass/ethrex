# Tokamak Client Status Report

**Date**: 2026-02-25
**Branch**: `feat/tokamak-proven-execution`
**Overall Completion**: ~35-40%

---

## Phase Completion

| Phase | Description | Completion | Status |
|-------|-------------|-----------|--------|
| Phase 0 | Research & Decision | **100%** | ethrex fork confirmed (FINAL) |
| Phase 1 | Foundation | **~95%** | CI infra built (fc720f46f), Hive/Sync verification pending |
| Phase 2 | JIT Foundation (revmc) | **100%** | LLVM backend integrated |
| Phase 3 | JIT Execution Wiring | **100%** | LevmHost + execution bridge |
| Phase 4 | Production JIT Hardening | **100%** | LRU cache, auto-compile, tracing bypass |
| Phase 5 | Advanced JIT | **100%** | Multi-fork, async compile, validation mode |
| Phase 6 | CALL/CREATE Resume | **100%** | Suspend/resume + LLVM memory mgmt |
| Phase 7 | Dual-Execution Validation | **100%** | State-swap validation, Volkov R20 PROCEED |
| Phase 8 | JIT Benchmarking | **100%** | Infrastructure + benchmark execution |
| Phase 9 | Benchmark CI & Dashboard | **0%** | Not started |

---

## Tier S Features

### Feature #9: JIT-Compiled EVM (~70%)

**Completed:**
- revmc/LLVM backend integration (Phases 2-8)
- Tiered execution (counter threshold -> compile -> execute)
- Multi-fork support (cache key includes Fork)
- Background async compilation (CompilerThread)
- LRU cache eviction
- CALL/CREATE suspend/resume
- Dual-execution validation (JIT vs interpreter)
- Benchmarking infrastructure + initial results
- 39 LEVM JIT tests + 19 tokamak-jit tests passing

**Remaining:**
- Gas accounting full alignment (JIT gas differs in edge cases)
- Recursive CALL performance (suspend/resume is slow)
- Bytecode size limit (revmc 24KB limit)
- Tiered optimization (profile-guided optimization)
- Opcode fusion, constant folding
- Fuzzing + security audit
- Production deployment

### Feature #10: Continuous Benchmarking (~35%)

**Completed:**
- `tokamak-bench` crate with 12 scenarios
- CLI: `run` / `compare` / `report` subcommands
- Regression detection with thresholds
- CI workflow (`pr-tokamak-bench.yaml`)
- JIT benchmark infrastructure
- JSON output + markdown report generation

**Remaining:**
- Geth/Reth comparison via JSON-RPC
- State root differential testing
- Public dashboard (clients.tokamak.network)
- PR-level regression blocking
- Precompile timing export

### Feature #21: Time-Travel Debugger (~2%)

**Completed:**
- `tokamak-debugger` skeleton crate (feature flag only)

**Remaining:**
- TX replay + state reconstruction
- Interactive CLI (step, breakpoint, inspect)
- `debug_timeTravel` RPC endpoint
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

**Skipped**: Push/MstoreBench/SstoreBench (bytecode > 24KB revmc limit),
FibonacciRecursive/FactorialRecursive/ERC20* (recursive CALL suspend/resume too slow).

---

## Tokamak-Specific Codebase

| Component | Location | Lines |
|-----------|----------|-------|
| LEVM JIT infra | `crates/vm/levm/src/jit/` (8 files) | ~1,966 |
| tokamak-jit crate | `crates/vm/tokamak-jit/src/` (13 files) | ~5,470 |
| tokamak-bench crate | `crates/tokamak-bench/src/` (7 files) | ~1,305 |
| tokamak-debugger | `crates/tokamak-debugger/src/` (1 file) | 2 |
| **Total** | | **~8,743** |

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
- Test quality improvements (B-2) — `test_helpers.rs`, `INTRINSIC_GAS` constant, 15+ test DRY refactors (224921e1f)
- Benchmark statistics (C-3) — `stats.rs` module, warmup/stddev/95% CI support, `--warmup` CLI param (224921e1f)

### Awaiting CI Verification
- Hive 6 suites 실행 및 통과 확인 (commit push 후 자동 트리거)
- Hoodi testnet sync 실행 (workflow_dispatch 수동 트리거 필요)
- Hive pass rate 비교: tokamak features on vs off
- Phase 1.2 criteria 6-9 확인

### Not Started
- Mainnet full sync as Tokamak client
- L2 integration (`tokamak-l2` flag declared, no implementation)
- Time-Travel Debugger (empty skeleton)
- Cross-client benchmark (Geth/Reth comparison)
- Public benchmark dashboard
- EF grant application
- External node operator adoption

### In Progress
- JIT gas accounting edge cases
- EIP-7928 BAL recording for JIT path (TODO comments only)

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
