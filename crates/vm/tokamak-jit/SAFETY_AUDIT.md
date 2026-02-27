# tokamak-jit Safety Audit

This document catalogs every `unsafe` block in the tokamak-jit crate and
its supporting infrastructure in ethrex-levm's JIT modules. It is intended
as a reference for security auditors evaluating the JIT compilation pipeline.

Last updated: 2026-02-27

## Attack Surface

User-controlled EVM bytecode flows through the following pipeline:

```
  user bytecode (arbitrary bytes)
       |
       v
  analyzer.rs    -- basic block detection, opcode counting
       |
       v
  optimizer.rs   -- constant folding (same-length rewriting)
       |
       v
  compiler.rs    -- revmc/LLVM JIT compilation (unsafe)
       |
       v
  cache.rs       -- compiled fn ptr storage (unsafe Send/Sync)
       |
       v
  execution.rs   -- transmute + FFI call into native code (unsafe)
       |
       v
  native execution on host CPU
```

The analyzer and optimizer operate on byte slices using safe Rust. The
critical trust boundary is `compiler.rs`, where user bytecode is handed to
LLVM for compilation into native machine code, and `execution.rs`, where
that native code is invoked via raw function pointers.

### Optimizer Scope (G-7 Enhancement, 2026-02-27)

The constant folding optimizer (`optimizer.rs`) rewrites `PUSH+PUSH+OP` and
`PUSH+UNARY_OP` patterns into pre-computed single PUSH instructions. It now
supports **22 opcodes** (expanded from 6 in D-3):

- **Binary (20)**: ADD, SUB, MUL, DIV, SDIV, MOD, SMOD, EXP, SIGNEXTEND,
  LT, GT, SLT, SGT, EQ, AND, OR, XOR, SHL, SHR, SAR
- **Unary (2)**: NOT, ISZERO

**Safety properties** of the new opcodes:
- All arithmetic uses wrapping/checked operations — no undefined behavior
- Division by zero returns `U256::zero()` per EVM spec (DIV, SDIV, MOD, SMOD)
- EXP overflow handled via `overflowing_pow` (wraps, no panic)
- Signed arithmetic (SDIV, SMOD, SLT, SGT, SAR, SIGNEXTEND) uses two's
  complement via `!x + 1` negate — matches exact LEVM semantics
- Same-length rewrite constraint preserved — results exceeding original byte
  count are skipped (no bytecode offset corruption)
- 68 unit tests + 8 integration tests + 4 proptest property tests verify
  optimizer invariants (length preservation, convergence, no panics)

## Unsafe Block Inventory

| # | File | Lines | Category | Risk | Description |
|---|------|-------|----------|------|-------------|
| 1 | compiler.rs | 47-52 | JIT compilation | CRITICAL | `compiler.jit()` -- invokes LLVM JIT compiler on user-controlled bytecode |
| 2 | compiler.rs | 59-67 | Pointer wrapping | MEDIUM | `CompiledCode::new()` wraps raw fn ptr from JIT as type-erased `*const ()` |
| 3 | compiler.rs | 80 | Memory leak | HIGH | `mem::forget(compiler)` intentionally leaks LLVM context to keep JIT code alive |
| 4 | execution.rs | 73-74 | Send impl | LOW | Manual `Send` for `JitResumeStateInner` containing Interpreter + EvmCompilerFn |
| 5 | execution.rs | 141-142 | Transmute | CRITICAL | `transmute` from `*const ()` to `RawEvmCompilerFn` -- restores type-erased fn ptr |
| 6 | execution.rs | 148-149 | FFI call | CRITICAL | `call_with_interpreter` -- calls JIT-compiled native code via function pointer |
| 7 | execution.rs | 203-204 | FFI call | CRITICAL | `call_with_interpreter` -- resume call after CALL/CREATE sub-call |
| 8 | cache.rs | 79-80 | Send impl | LOW | Manual `Send` for `CompiledCode` containing raw function pointer |
| 9 | cache.rs | 81-82 | Sync impl | LOW | Manual `Sync` for `CompiledCode` -- JIT code is immutable after creation |

## Detailed Analysis

### 1. JIT Compilation (compiler.rs:47-52) -- CRITICAL

```rust
let f: EvmCompilerFn = unsafe {
    compiler
        .jit(&hash_hex, bytecode, spec_id)
        .map_err(|e| JitError::CompilationFailed(format!("{e}")))?
};
```

**Risk**: The revmc `jit()` method compiles arbitrary EVM bytecode into native
x86-64 machine code using LLVM. If revmc or LLVM has a code-generation bug,
the resulting native code could corrupt memory, escape the sandbox, or execute
unintended instructions.

**Mitigations**:
- revmc validates EVM bytecode semantics before compilation
- LLVM's optimizer and code generator are extensively tested
- Dual-execution validation compares JIT output against interpreter
- Compilation is restricted to bytecodes under `max_bytecode_size` (24576 bytes)
- Oversized bytecodes are rejected via `oversized_hashes` negative cache

**Recommendation**: Fuzz the revmc compiler with arbitrary bytecode inputs.
Implement W^X page permissions for JIT code pages. Consider LLVM sandbox
modes in production.

### 2. Pointer Wrapping (compiler.rs:59-67) -- MEDIUM

```rust
let compiled = unsafe {
    CompiledCode::new(
        raw_fn as *const (),
        analyzed.bytecode.len(),
        analyzed.basic_blocks.len(),
        None,
        analyzed.has_external_calls,
    )
};
```

**Risk**: Type erasure loses the function signature. If the pointer is later
cast to the wrong type, calling it would be undefined behavior.

**Mitigations**:
- Only one cast-back site exists (execution.rs:142)
- The cast-back uses `EvmCompilerFn::new()` which enforces the correct signature
- No other code path accesses the raw pointer directly

**Recommendation**: Consider a wrapper type with a `PhantomData` marker to
prevent accidental misuse.

### 3. Memory Leak (compiler.rs:80) -- HIGH

```rust
std::mem::forget(compiler);
```

**Risk**: Each compilation leaks one `EvmCompiler` + `EvmLlvmBackend`
(~1-5 MB per contract). In a long-running node, memory grows proportionally
to the number of unique contracts compiled.

**Mitigations**:
- Cache has a bounded capacity (`max_cache_entries = 1024`)
- Oversized bytecodes (>24KB) are excluded from compilation
- Acceptable for PoC; documented as requiring production fix

**Recommendation**: ✅ RESOLVED — Arena allocator (G-1) manages LLVM memory lifecycle,
bounded LRU eviction policy (G-6) with per-entry AtomicU64 timestamps ensures
frequently-accessed entries survive longer. Evicted entries return FuncSlot for
arena memory reclamation.

### 4. Manual Send for JitResumeStateInner (execution.rs:73-74) -- LOW

```rust
unsafe impl Send for JitResumeStateInner {}
```

**Risk**: If `Interpreter` or `EvmCompilerFn` contained non-Send types (e.g.,
`Rc`, thread-local references), sending across threads would cause data races.

**Mitigations**:
- `Interpreter` contains `SharedMemory` (Arc-backed) and owned types
- `EvmCompilerFn` wraps a raw function pointer (inherently Send)
- Resume state is only transferred from JIT executor to LEVM dispatcher
  within the same transaction processing pipeline

**Recommendation**: Add a compile-time assertion or doc-test that verifies
the inner types remain Send-compatible across dependency updates.

### 5. Transmute (execution.rs:141-142) -- CRITICAL

```rust
let f = unsafe { EvmCompilerFn::new(std::mem::transmute::<*const (), _>(ptr)) };
```

**Risk**: Transmuting a raw pointer to a function pointer is the most dangerous
operation in the crate. If the pointer is null, dangling, or points to
non-executable memory, calling it is immediate undefined behavior.

**Mitigations**:
- Null check at line 90-93 rejects null pointers before reaching this code
- The pointer originates exclusively from `TokamakCompiler::compile()`,
  which only stores valid LLVM-produced function pointers
- `CompiledCode` is only created in compiler.rs and test code

**Recommendation**: Add a debug assertion verifying the pointer falls within
known JIT code page ranges. Consider using `NonNull` in `CompiledCode`.

### 6-7. FFI Calls (execution.rs:148-149, 203-204) -- CRITICAL

```rust
let action = unsafe { f.call_with_interpreter(&mut interpreter, &mut host) };
```

**Risk**: Calls JIT-compiled native code. If the compiled code is malformed
(due to compiler bugs), this could corrupt the interpreter's stack, memory,
or the host's state.

**Mitigations**:
- revmc's `call_with_interpreter` follows a well-defined ABI contract
- The interpreter and host are freshly constructed with valid state
- Gas accounting limits execution duration
- Dual-execution validation catches output mismatches
- Revert handling undoes storage writes via journal rollback

**Recommendation**: Implement stack canaries or guard pages around the JIT
interpreter's stack and memory regions. Monitor for unexpected signals
(SIGSEGV, SIGBUS) during JIT execution.

### 8-9. Manual Send/Sync for CompiledCode (cache.rs:79-82) -- LOW

```rust
unsafe impl Send for CompiledCode {}
unsafe impl Sync for CompiledCode {}
```

**Risk**: `CompiledCode` contains a `*const ()` which is neither Send nor Sync
by default. Incorrect Send/Sync can cause data races.

**Mitigations**:
- JIT-compiled code is immutable after creation (no writes to code pages)
- The pointer itself is never dereferenced for mutation
- Cache uses `Arc<CompiledCode>` for shared ownership
- The `RwLock` in `CodeCache` provides proper synchronization for metadata

**Recommendation**: Acceptable as-is. The compiled code pages are read-only
executable memory. Consider wrapping in a newtype that documents the
Send/Sync invariants.

### JIT-to-JIT Direct Dispatch (G-4, 2026-02-27)

The JIT-to-JIT dispatch path (`run_subcall_with_jit_dispatch()` in `vm.rs`) introduces
a new execution path where child CALL targets are executed directly via JIT if their
bytecode is already in the JIT cache.

**New trust boundary**: When the parent JIT code suspends on CALL, the VM now checks
the child bytecode hash against the JIT cache and may execute the child via
`JitState.execute_jit()` instead of the LEVM interpreter. This means:

1. **Child JIT errors treated as reverts**: If child JIT execution fails after
   potentially mutating state, the error is treated as a revert (gas consumed,
   `CALL` returns 0), NOT as a fallback to interpreter. This prevents double-execution
   of partially-mutated state.

2. **Precompile guard**: Precompile addresses always bypass JIT dispatch to avoid
   running JIT code against precompile semantics.

3. **CREATE exclusion**: CREATE opcode init code always uses the interpreter because
   `validate_contract_creation` (code size check, EOF prefix, code deposit cost)
   is handled by the interpreter's `handle_opcode_result`.

4. **Recursive nesting**: If a child JIT execution also suspends (nested CALL),
   the dispatch re-enters `handle_jit_subcall()` recursively. Stack depth is bounded
   by EVM's 1024 call depth limit.

5. **Configuration**: `enable_jit_dispatch` defaults to `true`; operators can disable
   it via `JitConfig` if correctness issues are discovered.

**Risk assessment**: LOW — no new unsafe code introduced. The dispatch logic uses
existing safe APIs (`try_jit_dispatch`, `execute_jit`, `execute_jit_resume`). The
only new risk is incorrect VM state management during child dispatch, mitigated by
dual-execution validation (G-3) and 10 dedicated tests.

## Summary

| Risk Level | Count | Categories |
|------------|-------|------------|
| CRITICAL | 4 | JIT compilation, transmute, FFI calls (x2) |
| HIGH | 1 | Memory leak (intentional) |
| MEDIUM | 1 | Pointer type erasure |
| LOW | 3 | Manual Send/Sync impls (x3) |

The CRITICAL-risk blocks are inherent to any JIT compilation system: compiling
user-controlled bytecode to native code and invoking it requires unsafe operations
that cannot be eliminated. The primary defense is revmc's correctness, LLVM's
code generation reliability, and the dual-execution validation layer that
catches output mismatches before trusting JIT results.

## Production Hardening Recommendations

1. **Memory management**: ✅ RESOLVED — Arena allocator (G-1) replaces `mem::forget`,
   LRU cache eviction (G-6) ensures bounded memory with AtomicU64-based access tracking.

2. **W^X enforcement**: Ensure JIT code pages are mapped as RX (read-execute)
   only, never RWX. Verify via `/proc/self/maps` audit on Linux.

3. **Signal handling**: Install SIGSEGV/SIGBUS handlers around JIT execution
   to gracefully fall back to the interpreter on crashes.

4. **Fuzzing**: Run `cargo fuzz` targets continuously in CI to detect
   analyzer/optimizer panics and invariant violations. The `fuzz_differential`
   target performs real JIT-vs-interpreter comparison (gas alignment, execution
   status, output equivalence) on randomly generated EVM bytecode.

5. **Address space isolation**: Consider running JIT code in a separate
   process or using seccomp/landlock to restrict syscalls.

6. **Code signing**: Hash compiled native code and verify before execution
   to detect memory corruption.
