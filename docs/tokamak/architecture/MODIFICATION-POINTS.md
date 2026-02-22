# Tokamak Modification Points & Isolation Strategy

*Analyzed: 2026-02-22*

## Modification Points

| # | Tokamak Feature | Target File(s) | Modification Type | Isolation Strategy |
|---|----------------|----------------|-------------------|--------------------|
| 1 | JIT Compiler | `crates/vm/levm/src/vm.rs` (run_execution) | New crate + integration point | `crates/vm/tokamak-jit/` new crate |
| 2 | Time-Travel Debugger | `crates/vm/levm/src/tracing.rs` | Extend existing tracer | `tokamak-debugger` feature flag on ethrex-levm |
| 3 | Continuous Benchmarking | `crates/vm/levm/src/timings.rs` | CI connection | Reuse `perf_opcode_timings`, add CI only |
| 4 | Tokamak L2 | `crates/vm/levm/src/hooks/` | New Hook impl | `hooks/tokamak_l2_hook.rs` + `tokamak-l2` feature |
| 5 | Differential Testing | `src/opcodes.rs` (`build_opcode_table()`) | Read-only reference | Separate test crate |

### 1. JIT Compiler

**Current**: `run_execution()` at `vm.rs:528-663` is a pure interpreter loop with dual dispatch (inline match + table fallback).

**Tokamak change**: Add a JIT compilation tier using Cranelift. The JIT would:
- Compile hot bytecode regions to native code
- Replace the table fallback path for compiled functions
- Fall back to interpreter for cold/uncompiled code

**Integration point**: Inside `run_execution()`, before the interpreter loop. The following is **pseudocode** — `jit_cache` does not exist yet and the actual API will be designed in Phase 3:
```rust
// PSEUDOCODE — illustrative only, not compilable
#[cfg(feature = "tokamak-jit")]
if let Some(compiled) = jit_cache.get(&code_hash) {
    return compiled.execute(self);
}
```

**JIT-VM Interface Complexity**: Integrating a JIT tier into the interpreter is non-trivial. Key challenges:
- **State consistency**: The JIT must maintain identical gas metering, stack, and memory semantics as the interpreter. Any divergence causes consensus failures.
- **Revert handling**: When JIT-compiled code triggers a revert, the VM must seamlessly restore state (Substate checkpoints, CallFrameBackup) as if the interpreter had executed.
- **Boundary transitions**: Calls between JIT-compiled and interpreted code (e.g., a JIT function calling CREATE which falls back to interpretation) require careful stack/context marshaling.
- **Precompile interaction**: JIT-compiled code calling precompiles must use the same interface as the interpreter path.
- **Debugging support**: JIT execution must still produce traces compatible with `LevmCallTracer` for `debug_traceTransaction`.

These challenges will be addressed in Phase 3 design. The skeleton crate exists now to reserve the workspace slot.

**Isolation**: New `crates/vm/tokamak-jit/` crate with Cranelift dependency. Only referenced from `ethrex-levm` behind `tokamak-jit` feature flag.

### 2. Time-Travel Debugger

**Current**: `LevmCallTracer` in `tracing.rs` records call-level traces (entry/exit, gas, return data).

**Tokamak change**: Extend tracing to capture:
- Full state snapshots at configurable intervals
- Opcode-level execution steps (PC, stack, memory)
- Bidirectional navigation (step forward/backward)

**Integration point**: Inside the main loop, after opcode execution. The following is **pseudocode** — `is_recording_snapshots()` and `record_step()` do not exist yet on `LevmCallTracer`:
```rust
// PSEUDOCODE — illustrative only, not compilable
#[cfg(feature = "tokamak-debugger")]
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

**Integration point**: `hooks/hook.rs:get_hooks()`. The following is **pseudocode** — `VMType::TokamakL2` and `tokamak_l2_hooks()` do not exist yet:
```rust
// PSEUDOCODE — illustrative only, not compilable
#[cfg(feature = "tokamak-l2")]
VMType::TokamakL2(config) => tokamak_l2_hooks(config),
```

**Isolation**: New `hooks/tokamak_l2_hook.rs` file behind `tokamak-l2` feature flag. New `VMType::TokamakL2` variant also feature-gated.

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

Each feature flag gates minimal, surgical changes inside existing crates:

| Change | Feature | File | Lines Affected |
|--------|---------|------|---------------|
| `VMType::TokamakL2` variant | `tokamak-l2` | `vm.rs:38-44` | ~3 lines |
| `get_hooks()` new branch | `tokamak-l2` | `hooks/hook.rs:19-24` | ~2 lines |
| Tracer snapshot extension | `tokamak-debugger` | `tracing.rs` | ~20 lines |
| JIT cache check in loop | `tokamak-jit` | `vm.rs:528` area | ~5 lines |

**Total**: ~30 lines of feature-gated changes in existing files, spread across 3 independent features.

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

### Current State (Phase 1.1)

Single `tokamak` feature for build verification:

```toml
# crates/vm/levm/Cargo.toml
[features]
tokamak = []  # Placeholder — will be split in Phase 1.2

# cmd/ethrex/Cargo.toml
[features]
tokamak = ["ethrex-vm/tokamak"]  # Propagate to VM layer
```

### Planned Split (Phase 1.2)

The single `tokamak` feature **must** be split into 3 independent features. A monolithic flag for 3 unrelated subsystems (JIT, debugger, L2 hooks) violates separation of concerns:

```toml
# crates/vm/levm/Cargo.toml — target state
[features]
tokamak-jit = []       # JIT compilation tier (run_execution integration)
tokamak-debugger = []  # Time-travel debugger (tracer snapshot extension)
tokamak-l2 = []        # Tokamak L2 hooks (VMType::TokamakL2, TokamakL2Hook)

# Convenience umbrella
tokamak = ["tokamak-jit", "tokamak-debugger", "tokamak-l2"]
```

**Rationale**: An operator running a Tokamak L2 node should not be forced to compile Cranelift JIT. A developer using the debugger should not need L2 hook code. Independent features enable:
- Faster compile times for targeted builds
- Cleaner `#[cfg]` blocks (each feature gates only its own code)
- Independent testing per subsystem

---

## Failure Scenarios & Mitigations

### Hybrid Strategy Risks

| Scenario | Impact | Mitigation |
|----------|--------|------------|
| **Upstream vm.rs major refactor** | Feature-gated lines conflict; manual resolution required | Weekly upstream monitoring. If `run_execution()` moves or splits, update our `#[cfg]` blocks within 1 week. Escape hatch: extract our integration points into a separate `tokamak_integration.rs` module |
| **Feature flag rot** | Unused `#[cfg(feature = "tokamak-*")]` blocks accumulate, break on upstream API changes | CI must build both `--features tokamak` and default. Breakage in tokamak-only code is caught immediately |
| **New crate API mismatch** | `tokamak-jit` depends on LEVM internals that change upstream | Pin to specific LEVM APIs via a thin adapter layer. Avoid depending on `pub(crate)` items |
| **Merge conflict cascade** | Rebase touches multiple Tokamak files at once | Keep feature-gated changes minimal (~30 lines). Each modified file has at most 1 `#[cfg]` block |
| **Build time regression** | Cranelift dependency adds significant compile time to workspace builds | `tokamak-jit` is a separate crate, not default. Only compiled when `--features tokamak-jit` is used |
