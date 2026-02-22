# Tokamak Modification Points & Isolation Strategy

*Analyzed: 2026-02-22*

## Modification Points

| # | Tokamak Feature | Target File(s) | Modification Type | Isolation Strategy |
|---|----------------|----------------|-------------------|--------------------|
| 1 | JIT Compiler | `crates/vm/levm/src/vm.rs` (run_execution) | New crate + integration point | `crates/vm/tokamak-jit/` new crate |
| 2 | Time-Travel Debugger | `crates/vm/levm/src/tracing.rs` | Extend existing tracer | `tokamak` feature flag on ethrex-levm |
| 3 | Continuous Benchmarking | `crates/vm/levm/src/timings.rs` | CI connection | Reuse `perf_opcode_timings`, add CI only |
| 4 | Tokamak L2 | `crates/vm/levm/src/hooks/` | New Hook impl | `hooks/tokamak_l2_hook.rs` + `tokamak` feature |
| 5 | Differential Testing | `src/opcodes.rs` (`build_opcode_table()`) | Read-only reference | Separate test crate |

### 1. JIT Compiler

**Current**: `run_execution()` at `vm.rs:528-663` is a pure interpreter loop with dual dispatch (inline match + table fallback).

**Tokamak change**: Add a JIT compilation tier using Cranelift. The JIT would:
- Compile hot bytecode regions to native code
- Replace the table fallback path for compiled functions
- Fall back to interpreter for cold/uncompiled code

**Integration point**: Inside `run_execution()`, before the interpreter loop:
```rust
#[cfg(feature = "tokamak")]
if let Some(compiled) = self.jit_cache.get(&code_hash) {
    return compiled.execute(self);
}
```

**Isolation**: New `crates/vm/tokamak-jit/` crate with Cranelift dependency. Only referenced from `ethrex-levm` behind `tokamak` feature flag.

### 2. Time-Travel Debugger

**Current**: `LevmCallTracer` in `tracing.rs` records call-level traces (entry/exit, gas, return data).

**Tokamak change**: Extend tracing to capture:
- Full state snapshots at configurable intervals
- Opcode-level execution steps (PC, stack, memory)
- Bidirectional navigation (step forward/backward)

**Integration point**: Inside the main loop, after opcode execution:
```rust
#[cfg(feature = "tokamak")]
if self.tracer.is_recording_snapshots() {
    self.tracer.record_step(opcode, &self.current_call_frame, &self.substate);
}
```

**Isolation**: Feature-gated extension to existing `LevmCallTracer`. New debugger CLI/RPC in separate `crates/tokamak-debugger/` crate.

### 3. Continuous Benchmarking

**Current**: `perf_opcode_timings` feature already instruments every opcode with `Instant::now()` / `elapsed()` in `timings.rs`. Global `OPCODE_TIMINGS` mutex aggregates counts and durations.

**Tokamak change**: No code changes needed. Add:
- CI workflow running benchmarks per commit
- Results comparison against baseline (Geth/Reth)
- Regression detection with configurable thresholds

**Isolation**: No source modifications. CI-only addition. Benchmark runner in `crates/tokamak-bench/`.

### 4. Tokamak L2 Hook

**Current**: Hook system dispatches via `VMType`:
- `VMType::L1` → `[DefaultHook]`
- `VMType::L2(FeeConfig)` → `[L2Hook, BackupHook]`

**Tokamak change**: Add `TokamakL2Hook` for Tokamak-specific L2 execution:
- Custom fee handling
- Tokamak-specific system contracts
- Integration with Tokamak sequencer

**Integration point**: `hooks/hook.rs:get_hooks()`:
```rust
#[cfg(feature = "tokamak")]
VMType::TokamakL2(config) => tokamak_l2_hooks(config),
```

**Isolation**: New `hooks/tokamak_l2_hook.rs` file behind `tokamak` feature flag. New `VMType::TokamakL2` variant also feature-gated.

### 5. Differential Testing

**Current**: `build_opcode_table()` builds a fork-gated 256-entry dispatch table. Read-only access is sufficient to verify opcode behavior against reference implementations.

**Tokamak change**: Compare LEVM execution results against:
- Geth's EVM (via JSON-RPC)
- Reth's revm (via WASM or native)
- Ethereum Foundation test vectors

**Isolation**: Entirely separate test crate `crates/tokamak-bench/` (shared with benchmarking). No modifications to `opcodes.rs`.

---

## Isolation Strategy: Hybrid (Option C)

### Feature Flag Scope (small changes in existing crates)

The `tokamak` feature flag gates minimal, surgical changes inside existing crates:

| Change | File | Lines Affected |
|--------|------|---------------|
| `VMType::TokamakL2` variant | `vm.rs:38-44` | ~3 lines |
| `get_hooks()` new branch | `hooks/hook.rs:19-24` | ~2 lines |
| Tracer snapshot extension | `tracing.rs` | ~20 lines |
| JIT cache check in loop | `vm.rs:528` area | ~5 lines |

**Total**: ~30 lines of feature-gated changes in existing files.

### New Crate Scope (large new subsystems)

| Crate | Purpose | Primary Dependency |
|-------|---------|-------------------|
| `crates/vm/tokamak-jit/` | Cranelift JIT compiler | `cranelift-*`, `ethrex-levm` |
| `crates/tokamak-bench/` | Benchmark runner + differential testing | `ethrex-levm`, `ethrex-vm` |
| `crates/tokamak-debugger/` | Time-Travel Debugger CLI/RPC | `ethrex-levm`, `ethrex-rpc` |

### Why Hybrid?

| Approach | Upstream Rebase | Code Duplication | Complexity |
|----------|----------------|------------------|------------|
| Feature flags only | Frequent conflicts in modified files | None | Low |
| New crates only | No conflicts | High (must fork types) | High |
| **Hybrid** | **Minimal conflicts (30 lines)** | **None** | **Medium** |

The hybrid approach minimizes both conflict surface and code duplication:
- Feature-gated changes are small enough to resolve quickly during rebase
- New crates add zero conflict risk (they're entirely new files)
- Types and APIs are shared via existing crate interfaces, no duplication needed

---

## Upstream Conflict Risk Assessment

| File | Upstream Change Frequency | Our Modification | Conflict Risk | Mitigation |
|------|--------------------------|------------------|---------------|------------|
| `vm.rs` | **High** (core execution) | JIT check in `run_execution`, `VMType` variant | **HIGH** | Feature flag isolates to ~8 lines; review upstream changes weekly |
| `hooks/hook.rs` | **Low** (stable API) | New branch in `get_hooks()` | **LOW** | Simple pattern match addition |
| `tracing.rs` | **Low** (rarely changed) | Snapshot recording extension | **MEDIUM** | Feature-gated; additive only |
| `timings.rs` | **Low** (instrumentation) | Read-only usage | **NONE** | No modifications |
| `opcodes.rs` | **Medium** (fork updates) | Read-only (differential testing) | **NONE** | No modifications |
| `Cargo.toml` (levm) | **Medium** (dependency updates) | `tokamak` feature addition | **LOW** | Single line in `[features]` |

### Rebase Strategy

1. **Weekly**: Monitor upstream `lambdaclass/ethrex` for changes to HIGH-risk files
2. **Per-rebase**: Resolve `vm.rs` conflicts first (most likely), then others
3. **Automated**: CI check comparing our feature-gated lines against upstream changes
4. **Escape hatch**: If `vm.rs` diverges too much, extract `run_execution()` into a separate module

---

## Feature Flag Declaration

```toml
# crates/vm/levm/Cargo.toml
[features]
tokamak = []  # Tokamak-specific extensions (JIT hook, debugger snapshots, L2 hook)

# cmd/ethrex/Cargo.toml
[features]
tokamak = ["ethrex-vm/tokamak"]  # Propagate to VM layer
```

The `tokamak` feature enables all Tokamak-specific code paths. Individual features (JIT, debugger, L2) can be further gated if needed in later phases.
