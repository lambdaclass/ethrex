# Phase 2: JIT Foundation (revmc Integration)

## Decision: revmc over Cranelift

Cranelift **cannot** be used for EVM JIT — it lacks i256 (256-bit integer) support.
revmc (Paradigm) confirms this: their Cranelift backend is non-functional.
Their LLVM backend works and is the only production-proven EVM JIT.

**Decision**: Use revmc (LLVM backend) as the JIT library.

## Architecture: Two-Location Strategy

revmc + LLVM cannot be added to `ethrex-levm` (too heavy). But `vm.rs` needs
JIT dispatch logic. Solution: split infrastructure from heavy dependencies.

```
ethrex-levm (feature = "tokamak-jit")
  └── src/jit/          ← Lightweight infra (cache, counter, dispatch)
                           Zero new external deps. Only std + existing.

tokamak-jit (separate crate, feature = "revmc-backend")
  ├── ethrex-levm       ← Depends on LEVM (reads types, populates cache)
  ├── revmc (LLVM)      ← Heavy compilation backend
  └── adapter layer     ← Bridges LEVM state ↔ revmc/revm model
```

LEVM never depends on tokamak-jit. The dispatch in `vm.rs` checks the global cache.

## LEVM JIT Infrastructure (`src/jit/`)

All behind `#[cfg(feature = "tokamak-jit")]`. No new external deps.

| Module | Purpose | Lines |
|--------|---------|-------|
| `types.rs` | `JitConfig`, `JitOutcome`, `AnalyzedBytecode` | ~55 |
| `analyzer.rs` | Basic block boundary identification | ~85 |
| `counter.rs` | `ExecutionCounter` (Arc<RwLock<HashMap>>) | ~50 |
| `cache.rs` | `CompiledCode` + `CodeCache` (type-erased fn ptrs) | ~120 |
| `dispatch.rs` | `JitState` + `try_jit_dispatch()` | ~60 |

### vm.rs Integration

Global `JIT_STATE` via `lazy_static`. In `run_execution()`, after precompile
check and before the interpreter loop:

```rust
#[cfg(feature = "tokamak-jit")]
{
    let bytecode_hash = self.current_call_frame.bytecode.hash;
    JIT_STATE.counter.increment(&bytecode_hash);
    // Phase 3: check cache, execute JIT, return result
}
```

## tokamak-jit Crate

### Dependencies (behind `revmc-backend` feature)

- `revmc` (git, LLVM backend) — EVM JIT compiler
- `revm-primitives` v22, `revm-interpreter` v32 — revm type ecosystem
- LLVM 18+ required on build system

### Adapter Layer

Bridges LEVM ↔ revm type models:

| LEVM Type | revm Type | Strategy |
|-----------|-----------|----------|
| `U256` (ethereum_types) | `U256` (ruint) | Limb-level copy (same layout) |
| `H256` | `B256` | Byte slice copy |
| `Address` (H160) | `Address` | Byte slice copy |
| `gas_remaining: i64` | `Gas { remaining: u64 }` | Clamp i64→u64 |
| `Memory (Rc<RefCell<Vec<u8>>>)` | `SharedMemory` | Copy active slice |

### Compiler Wrapper

```rust
TokamakCompiler::compile(analyzed: &AnalyzedBytecode) -> Result<CompiledCode, JitError>
```

Uses `revmc_llvm::with_llvm_context` for thread-local LLVM context.
Calls `EvmCompiler::jit()` to produce native function pointers.

### Validation Mode

`validate_outcomes()` compares JIT result against interpreter result.
Mandatory during PoC — every JIT execution verified vs interpreter.

## Proof of Concept

Hand-crafted Fibonacci EVM bytecode:
- Pure computation: PUSH, DUP, SWAP, ADD, SUB, JUMP, JUMPI, CALLDATALOAD, MSTORE, RETURN
- No CALL, CREATE, SLOAD, SSTORE (deferred to Phase 3)
- Tested for fib(0)..fib(20) against LEVM interpreter

## Phase 2 Scope Limitations (NOT included)

- **Automatic compilation trigger** — counter tracks but doesn't trigger
- **CALL/CREATE** — suspend/resume mechanism deferred
- **State-accessing opcodes** (SLOAD, SSTORE) — needs Host impl validation
- **LRU eviction** — cache grows unbounded in PoC
- **Production error recovery** — JIT failures simply fall back

## Files Created/Modified

| File | Action |
|------|--------|
| `crates/vm/levm/src/jit/mod.rs` | Created |
| `crates/vm/levm/src/jit/types.rs` | Created |
| `crates/vm/levm/src/jit/analyzer.rs` | Created |
| `crates/vm/levm/src/jit/counter.rs` | Created |
| `crates/vm/levm/src/jit/cache.rs` | Created |
| `crates/vm/levm/src/jit/dispatch.rs` | Created |
| `crates/vm/levm/src/lib.rs` | Modified (+2 lines) |
| `crates/vm/levm/src/vm.rs` | Modified (+15 lines) |
| `crates/vm/tokamak-jit/Cargo.toml` | Replaced |
| `crates/vm/tokamak-jit/src/lib.rs` | Replaced |
| `crates/vm/tokamak-jit/src/error.rs` | Created |
| `crates/vm/tokamak-jit/src/adapter.rs` | Created |
| `crates/vm/tokamak-jit/src/compiler.rs` | Created |
| `crates/vm/tokamak-jit/src/backend.rs` | Created |
| `crates/vm/tokamak-jit/src/validation.rs` | Created |
| `crates/vm/tokamak-jit/src/tests/mod.rs` | Created |
| `crates/vm/tokamak-jit/src/tests/fibonacci.rs` | Created |
| `crates/tokamak-bench/src/jit_bench.rs` | Created |
| `crates/tokamak-bench/src/lib.rs` | Modified (+1 line) |
| `.github/workflows/pr-tokamak.yaml` | Modified (added jit-backend job) |
