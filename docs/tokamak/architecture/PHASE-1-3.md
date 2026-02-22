# Phase 1.3: Benchmarking Foundation

## Summary

Fills the `tokamak-bench` crate skeleton with a minimal library API + CLI binary that runs LEVM with per-opcode timing, exports structured JSON, and detects performance regressions between commits.

## What Changed

### `timings.rs` — Accessor Methods

Added to `OpcodeTimings` and `PrecompilesTimings`:
- `reset()` — clears accumulated data between benchmark runs
- `raw_totals()` — immutable access to duration totals map
- `raw_counts()` — immutable access to call count map

### `tokamak-bench` Crate

| Module | Purpose |
|--------|---------|
| `types.rs` | `BenchSuite`, `BenchResult`, `OpcodeEntry`, `RegressionReport`, `Thresholds` |
| `runner.rs` | VM initialization (mirrors `revm_comparison/levm_bench.rs`), scenario execution, opcode timing extraction |
| `report.rs` | JSON serialization/deserialization, markdown table generation |
| `regression.rs` | Compare two `BenchSuite`s by opcode averages, classify as Stable/Warning/Regression |
| `bin/runner.rs` | CLI: `run` / `compare` / `report` subcommands via clap |

Key dependency: `ethrex-levm` with `features = ["perf_opcode_timings"]` scoped to this crate only.

### CI Workflow

`.github/workflows/pr-tokamak-bench.yaml`:
- **bench-pr**: Build + run on PR commit
- **bench-main**: Build + run on base commit
- **compare-results**: Compare JSON outputs, generate markdown, post PR comment

Triggers on changes to `crates/vm/levm/**`, `crates/tokamak-bench/**`, or the workflow file.

## Default Scenarios

Same 12 contracts as `revm_comparison/`:
Fibonacci, FibonacciRecursive, Factorial, FactorialRecursive, Push, MstoreBench, SstoreBench_no_opt, ManyHashes, BubbleSort, ERC20Approval, ERC20Transfer, ERC20Mint.

## Thresholds

| Level | Default |
|-------|---------|
| Warning | >= 20% slower |
| Regression | >= 50% slower |

## CLI Usage

```
tokamak-bench run [--scenarios LIST] [--runs N] [--commit HASH] [--output PATH]
tokamak-bench compare --baseline PATH --current PATH [--threshold-warn N] [--threshold-regress N] [--output PATH]
tokamak-bench report --input PATH [--output PATH]
```

## Verification

- `cargo build --release -p tokamak-bench` — builds library + binary
- `cargo test -p tokamak-bench` — 11 tests pass (regression logic, report formatting, JSON roundtrip)
- `cargo test --workspace` — 0 failures (no regressions)
- `cargo check --features tokamak` — umbrella still works

## Deferred

- Geth/Reth comparison via JSON-RPC
- State root differential testing
- Dashboard publishing
- Precompile timing export (trivial to add)
