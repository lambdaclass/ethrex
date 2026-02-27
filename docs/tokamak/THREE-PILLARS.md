# Tokamak Three Pillars: Completion Report

**Date**: 2026-02-27
**Branch**: `feat/tokamak-proven-execution`

---

## Architecture Overview

```
   JIT-Compiled EVM (be the fastest)
           |
           v
   Continuous Benchmarking (prove it every commit)
           |
           v
   Time-Travel Debugger (show exactly why)
           |
           +---> feeds back into JIT optimization
```

Three pillars form a closed feedback loop: JIT compiles EVM bytecode to native code for speed, benchmarking proves the speedup on every commit, and the debugger explains execution behavior down to individual opcodes — informing the next round of JIT optimization.

---

## Pillar 1: JIT-Compiled EVM

**Status**: ~88% complete | **Codebase**: ~9,266 lines Rust | **Tests**: 187+

> "Be the fastest."

### What It Does

Tokamak JIT compiles frequently-executed EVM bytecodes into native machine code via revmc (Paradigm) + LLVM 21. Hot contracts are detected at runtime, compiled in a background thread, and executed natively on subsequent calls — achieving **1.5-2.5x speedup** over the LEVM interpreter.

### Completed Components

| Component | Location | Lines | Tests |
|-----------|----------|-------|-------|
| LEVM JIT Infrastructure | `crates/vm/levm/src/jit/` (9 files) | 2,773 | 80 |
| tokamak-jit Crate | `crates/vm/tokamak-jit/src/` (14 files) | 6,493 | 73 |
| **Total** | | **9,266** | **153** |

### Key Achievements

1. **Tiered Execution Pipeline**
   - Execution counter tracks per-bytecode-hash call frequency
   - Crosses threshold -> enqueues for background compilation (mpsc channel)
   - Compiled native fn cached in `DashMap<(H256, Fork), JitFunction>`
   - Next call dispatches to native code instead of interpreter loop

2. **CALL/CREATE Suspend/Resume** (Phase 6)
   - JIT native code suspends at CALL/CREATE opcodes
   - LEVM dispatches the child call (interpreter or JIT)
   - Parent resumes from exactly where it left off
   - State (stack, memory, gas) preserved across boundary

3. **Dual-Execution Validation** (Phase 7)
   - JIT runs first, interpreter re-runs same TX
   - State roots compared — divergence triggers alert
   - Volkov R20 score: 8.25 — PROCEED

4. **Gas Accounting Alignment** (B-1)
   - Fixed negative SSTORE refund bug (`u64::try_from` silently dropped negatives)
   - 11 gas alignment tests covering EIP-2200/EIP-3529 edge cases
   - JIT gas == interpreter gas for all tested scenarios

5. **EIP-7928 BAL Recording** (B-3)
   - Block Access List recording in JIT host sload/sstore paths
   - 5 differential tests confirm JIT BAL == interpreter BAL

6. **Bytecode Size Limit Fallback** (D-2)
   - revmc hard limit: 24,576 bytes (EIP-170)
   - `oversized_hashes` negative cache — O(1) skip for known-oversized
   - Early size gate in VM dispatch + background thread guard
   - Graceful interpreter-only fallback (no silent failures)

7. **Constant Folding Optimizer** (D-3 + G-7)
   - Same-length PUSH+PUSH+OP -> single PUSH replacement + PUSH+UNARY_OP unary folding
   - 22 opcodes: 20 binary (ADD, MUL, SUB, DIV, SDIV, MOD, SMOD, EXP, SIGNEXTEND, LT, GT, SLT, SGT, EQ, AND, OR, XOR, SHL, SHR, SAR) + 2 unary (NOT, ISZERO)
   - Signed arithmetic helpers, extracted eval helpers, write_folded_push
   - 76 tests (68 unit + 8 integration)

8. **Security Audit Prep** (F-4)
   - 3 cargo-fuzz harnesses (analyzer, optimizer, differential)
   - 4 proptest property tests
   - SAFETY_AUDIT.md cataloging all 9 unsafe blocks
   - Real differential fuzzing: JIT vs interpreter dual-path execution

### Benchmark Results

10 runs each, `--profile jit-bench`, Fork::Cancun:

| Scenario | Interpreter | JIT | Speedup |
|----------|------------|-----|---------|
| Fibonacci | 3.55ms | 1.40ms | **2.53x** |
| BubbleSort | 357.69ms | 159.84ms | **2.24x** |
| Factorial | 2.36ms | 1.41ms | **1.67x** |
| ManyHashes | 2.26ms | 1.55ms | **1.46x** |

**Interpreter-only** (bytecode > 24KB): Push, MstoreBench, SstoreBench
**Skipped** (recursive CALL too slow): FibonacciRecursive, FactorialRecursive, ERC20*

### Known Limitations & Resolution Roadmap

> Full roadmap: [`JIT-LIMITATIONS-ROADMAP.md`](./JIT-LIMITATIONS-ROADMAP.md)

#### Critical (Production Blockers) — ✅ ALL RESOLVED

| ID | Issue | Status | Resolution |
|----|-------|--------|------------|
| **G-1** | **LLVM Memory Leak** | ✅ DONE (f8e9ba540) | Arena allocator — ArenaManager + ArenaCompiler + thread_local ArenaState |
| **G-2** | **Cache Eviction No-Op** | ✅ DONE | Auto-resolved by G-1 — Free/FreeArena handlers reclaim memory |

#### Significant (v1.1 Targets) — 2/3 RESOLVED

| ID | Issue | Status | Resolution |
|----|-------|--------|------------|
| **G-3** | **CALL/CREATE Validation Gap** | ✅ DONE (8c05d3412) | Removed has_external_calls guard — validation runs for ALL bytecodes |
| **G-4** | **Recursive CALL Overhead** | REMAINING | Inline small calls for depth 1 (20-30h) |
| **G-5** | **Single Compiler Thread** | ✅ DONE (299d03720) | CompilerThreadPool — crossbeam-channel multi-consumer, N workers |

#### Moderate (v1.2 Optimization) — 1/3 RESOLVED

| ID | Issue | Status | Resolution |
|----|-------|--------|------------|
| **G-6** | **FIFO Cache** (not LRU) | REMAINING | Atomic timestamp LRU (8-12h) |
| **G-7** | **Constant Folding** | ✅ DONE (43026d7cf) | Expanded from 6 to 22 opcodes (14 binary + 2 unary), refactored eval helpers |
| **G-8** | **No Precompile Acceleration** | REMAINING | LLVM IR extern calls (16-24h) |

#### Resolution Timeline

```
v1.0.1  G-1 + G-2 (memory safety)           ✅ ALL DONE
v1.1    G-3 + G-5 done, G-4 remaining        20-30h left
v1.2    G-7 done, G-6 + G-8 remaining        24-36h left
                                        Remaining: 44-66h
```

---

## Pillar 2: Continuous Benchmarking

**Status**: ~80% complete | **Codebase**: ~4,411 lines (2,907 Rust + 1,504 TS/Astro) | **Tests**: 134

> "Prove it every commit."

### What It Does

Every PR triggers automated benchmark runs comparing interpreter and JIT performance. Results are posted as PR comments with regression detection. A public dashboard visualizes performance trends over time.

### Completed Components

| Component | Location | Lines | Tests |
|-----------|----------|-------|-------|
| tokamak-bench Crate | `crates/tokamak-bench/src/` (11 files) | 2,907 | 61 |
| Public Dashboard | `dashboard/` (Astro + React) | 1,504 | 73 |
| **Total** | | **4,411** | **134** |

### Key Achievements

1. **Benchmark Harness** (Phase 8)
   - 12 benchmark scenarios (Fibonacci, BubbleSort, ERC20, etc.)
   - CLI: `run`, `compare`, `report`, `jit-compare` subcommands
   - JSON output + markdown report generation

2. **JIT Benchmark CI** (C-1)
   - 3 CI jobs: `jit-bench-pr`, `jit-bench-main`, `compare-jit-results`
   - `compare_jit()` compares speedup ratios between PR and base
   - Regression flagged if JIT speedup drops > 20%
   - PR comment with JIT speedup regression report

3. **LLVM 21 CI Provisioning** (C-2)
   - Reusable `.github/actions/install-llvm/` composite action
   - llvm-21 + llvm-21-dev + libpolly-21-dev
   - Modern GPG key method (no deprecated apt-key)
   - `continue-on-error` removed — JIT failures now block PRs

4. **Statistical Rigor** (C-3)
   - Warmup runs (discard first 2)
   - Standard deviation + 95% confidence intervals
   - Multiple independent trial invocations
   - `--warmup` CLI parameter

5. **Cross-Client Benchmarking** (F-1)
   - ethrex runs in-process (zero RPC overhead)
   - Geth/Reth via `eth_call` with state overrides
   - Comparison table with ethrex as 1.00x baseline

6. **Public Dashboard** (F-2)
   - Astro + React islands + Recharts + Tailwind static site
   - 16 TypeScript interfaces mirroring Rust bench types
   - Zod runtime validation at fetch boundary
   - TrendChart with CI error bands (ComposedChart + Area + Line)
   - Landing page (metric cards + benchmark table) + Trends page
   - `rebuild_index.py` for CI data pipeline
   - `publish-dashboard` CI job (GitHub Pages via peaceiris/actions-gh-pages)
   - Path traversal protection

### CI Pipeline

```
PR opened/updated
    |
    +-- pr-tokamak.yaml
    |     +-- Hive Gate (6 suites)
    |     +-- Quality Gate (clippy, test, all features)
    |     +-- Docker Build
    |
    +-- pr-tokamak-bench.yaml
          +-- jit-bench-pr (LLVM 21 + tokamak-jit benchmarks)
          +-- jit-bench-main (baseline from main branch)
          +-- compare-jit-results (regression detection + PR comment)
          +-- publish-dashboard (GitHub Pages)
```

### Known Limitations

- **State root differential testing**: Not yet automated in CI.
- **Precompile timing export**: Benchmark harness doesn't yet isolate precompile costs.

---

## Pillar 3: Time-Travel Debugger

**Status**: ~85% complete | **Codebase**: ~1,830 lines Rust | **Tests**: 51

> "Show exactly why."

### What It Does

Records every opcode execution during a transaction, then allows developers to navigate forward and backward through execution history — like a DVR for EVM execution. Available as a CLI REPL and a JSON-RPC endpoint.

### Completed Components

| Component | Location | Lines | Tests |
|-----------|----------|-------|-------|
| tokamak-debugger Crate | `crates/tokamak-debugger/src/` (14 files) | 1,803 | 45 |
| LEVM Debugger Hook | `crates/vm/levm/src/debugger_hook.rs` | 27 | — |
| **Total** | | **1,830** | **45** |

### Key Achievements

1. **TX Replay Engine** (E-1)
   - `OpcodeRecorder` hook trait in LEVM (feature-gated `tokamak-debugger`)
   - `DebugRecorder` captures per-opcode: opcode, PC, gas, depth, stack top-N, memory size, code address
   - `ReplayEngine::record()` executes TX with recorder, builds `ReplayTrace`
   - Navigation: `forward()`, `backward()`, `goto()`, `current_step()`, `steps_range()`
   - Stack `peek()` for non-destructive inspection

2. **GDB-Style CLI** (E-2)
   - 13 commands: `step`, `step-back`, `continue`, `reverse-continue`, `break`, `delete`, `goto`, `info`, `stack`, `list`, `breakpoints`, `help`, `quit`
   - rustyline REPL with auto-history
   - `--bytecode <hex>` input mode for quick debugging
   - Feature-gated `cli` module (clap, rustyline dependencies)

3. **debug_timeTravel RPC** (E-3)
   - JSON-RPC method: `debug_timeTravel(txHash, { stepIndex, count, reexec })`
   - Returns trace summary (totalSteps, gasUsed, success, output) + step window
   - Reusable `prepare_state_for_tx()` extracted from tracing infrastructure
   - Feature-gated in ethrex-rpc

### Debugger Session Example

```
$ tokamak-debugger --bytecode 6005600301
tokamak-debugger> step
[0] PUSH1 0x05  gas=999979000  depth=0
tokamak-debugger> step
[2] PUSH1 0x03  gas=999978997  depth=0
tokamak-debugger> step
[4] ADD          gas=999978994  depth=0
tokamak-debugger> step-back
[2] PUSH1 0x03  gas=999978997  depth=0
tokamak-debugger> info
Step 1/3 | PC: 2 | Gas: 999978997 | Depth: 0
```

### Feedback Loop

The debugger directly feeds back into JIT optimization:

| Debugger Insight | JIT Action |
|------------------|------------|
| Hot opcode sequences identified | Constant folding candidates (D-3) |
| Gas-heavy operations visible | Gas accounting alignment targets (B-1) |
| SSTORE patterns in trace | BAL recording optimization (B-3) |
| CALL depth visibility | Recursive CALL performance analysis (D-1) |
| Bytecode size in trace | Size limit fallback triggers (D-2) |

---

## Summary: Three Pillars Status

| Pillar | Completion | Lines | Tests | Phases |
|--------|-----------|-------|-------|--------|
| JIT-Compiled EVM | **~88%** | 9,266+ | 187+ | 2-8, B-1/2/3, D-1/2/3, F-4, G-1/2/3/5/7 |
| Continuous Benchmarking | **~80%** | 4,411 | 134 | 8-9, C-1/2/3, F-1/2 |
| Time-Travel Debugger | **~85%** | 1,830 | 45 | E-1/2/3 |
| **Total** | **~85%** | **15,507+** | **366+** | |

Plus L2 integration scaffolding (F-3): 7 tests connecting JIT policy to L2 hook system.

### What's Left

#### JIT Limitations Resolution (see [`JIT-LIMITATIONS-ROADMAP.md`](./JIT-LIMITATIONS-ROADMAP.md))

| Phase | Tasks | Status | Remaining |
|-------|-------|--------|-----------|
| v1.0.1 | G-1 + G-2 | **✅ ALL DONE** | 0h |
| v1.1 | G-3 ✅ + G-4 + G-5 ✅ | **2/3 DONE** | 20-30h (G-4) |
| v1.2 | G-6 + G-7 ✅ + G-8 | **1/3 DONE** | 24-36h (G-6, G-8) |

#### Other Remaining Work

| Item | Pillar | Priority | Estimate |
|------|--------|----------|----------|
| State root diff testing | Bench | P2 | 8-12h |
| Precompile timing export | Bench | P3 | 4-8h |
| Web UI | Debugger | P3 | 24-40h |
| Mainnet full sync (F-5) | All | P3 | 24-48h |

### Verified Milestones

- Hive 6/6 PASS (tokamak-jit build)
- Hoodi snap sync PASS (1h48m35s)
- Feature flag safety confirmed (tokamak-jit == upstream)
- 4 Volkov PROCEED reviews (R6, R10, R20, R24)
- JIT speedup: 1.46x - 2.53x across non-recursive scenarios
