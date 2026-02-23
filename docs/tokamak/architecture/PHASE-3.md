# Phase 3: JIT Execution Wiring

## Overview

Phase 3 wires the JIT compilation output from Phase 2 into LEVM's execution
pipeline so that JIT-compiled bytecode actually runs. The core challenge is
that LEVM cannot depend on `tokamak-jit` (circular dependency), but JIT
execution requires revm types that live in `tokamak-jit`.

## Dependency Inversion Pattern

```
ethrex-levm                          tokamak-jit
  jit/dispatch.rs                      backend.rs
    trait JitBackend ◄─────────────── impl JitBackend for RevmcBackend
    JitState {                           execution.rs
      backend: RwLock<Option<Arc<        host.rs (LevmHost: Host)
        dyn JitBackend>>>
    }
```

LEVM defines the `JitBackend` trait; `tokamak-jit` implements it. At startup,
`register_jit_backend()` stores the implementation in `JIT_STATE.backend`.

## Execution Flow

```
VM::run_execution()
  │
  ├─ JIT_STATE.counter.increment(&hash)
  ├─ try_jit_dispatch(&JIT_STATE, &hash)  → Option<Arc<CompiledCode>>
  ├─ JIT_STATE.execute_jit(compiled, ...)  → Option<Result<JitOutcome>>
  │    └─ backend.execute(compiled, call_frame, db, substate, env)
  │         └─ execution::execute_jit()
  │              ├─ Build revm Interpreter (ExtBytecode, InputsImpl, Gas)
  │              ├─ Build LevmHost (db, substate, env)
  │              ├─ transmute CompiledCode ptr → EvmCompilerFn
  │              ├─ f.call_with_interpreter(&mut interpreter, &mut host)
  │              └─ Map InterpreterAction → JitOutcome
  └─ apply_jit_outcome() → ContextResult
```

On failure at any step, execution falls through to the interpreter loop.

## LevmHost: revm Host Implementation

Maps revm `Host` trait (v14.0, 22 required methods) to LEVM state:

| Host Method | LEVM Delegation |
|-------------|-----------------|
| `basefee()` | `env.base_fee_per_gas` |
| `blob_gasprice()` | `env.base_blob_fee_per_gas` |
| `gas_limit()` | `env.block_gas_limit` |
| `difficulty()` | `env.difficulty` |
| `prevrandao()` | `env.prev_randao` |
| `block_number()` | `env.block_number` |
| `timestamp()` | `env.timestamp` |
| `beneficiary()` | `env.coinbase` |
| `chain_id()` | `env.chain_id` |
| `effective_gas_price()` | `env.gas_price` |
| `caller()` | `env.origin` |
| `blob_hash(n)` | `env.tx_blob_hashes[n]` |
| `max_initcode_size()` | `49152` (EIP-3860) |
| `gas_params()` | `GasParams::new_spec(CANCUN)` |
| `block_hash(n)` | `db.store.get_block_hash(n)` |
| `load_account_info_skip_cold_load()` | `db.get_account()` + `db.get_code()` |
| `sload_skip_cold_load()` | `db.get_storage_value()` |
| `sstore_skip_cold_load()` | `db.update_account_storage()` |
| `tload/tstore` | `substate.get_transient/set_transient` |
| `log()` | `substate.add_log()` |
| `selfdestruct()` | `substate.add_selfdestruct()` |

## Type Conversion

All conversions use existing functions from `adapter.rs`:

| LEVM Type | revm Type | Function |
|-----------|-----------|----------|
| `ethereum_types::U256` | `ruint::Uint<256, 4>` | `levm_u256_to_revm` / `revm_u256_to_levm` |
| `ethereum_types::H256` | `B256` | `levm_h256_to_revm` / `revm_b256_to_levm` |
| `ethereum_types::H160` | `Address` | `levm_address_to_revm` / `revm_address_to_levm` |
| `i64` (gas_remaining) | `Gas` | `levm_gas_to_revm` / `revm_gas_to_levm` |

## Safety

The `execute_jit` function uses `unsafe` in two places:

1. **`EvmCompilerFn::new(transmute(ptr))`** — Casts the type-erased `*const ()`
   back to `RawEvmCompilerFn`. Safety is maintained by the compilation pipeline:
   only valid function pointers from revmc/LLVM are stored in `CompiledCode`.

2. **`f.call_with_interpreter()`** — Calls JIT-compiled machine code. Safety
   relies on the revmc compiler producing correct code for the given bytecode.

## Files Changed

| File | Action | Lines |
|------|--------|-------|
| `crates/vm/levm/src/jit/dispatch.rs` | Modified | +65 |
| `crates/vm/levm/src/vm.rs` | Modified | +45 |
| `crates/vm/tokamak-jit/src/host.rs` | **New** | ~250 |
| `crates/vm/tokamak-jit/src/execution.rs` | **New** | ~125 |
| `crates/vm/tokamak-jit/src/backend.rs` | Modified | +20 |
| `crates/vm/tokamak-jit/src/lib.rs` | Modified | +15 |
| `crates/vm/tokamak-jit/src/tests/fibonacci.rs` | Modified | +175 |

## Phase 3 Scope Limitations

- **CALL/CREATE**: Returns error, falls back to interpreter (Phase 4)
- **Auto-compilation**: Counter tracks but doesn't trigger compile (Phase 4)
- **Cache eviction**: Unbounded growth (Phase 4)
- **is_static**: Hardcoded `false` (Phase 4)
- **SpecId**: Hardcoded `CANCUN` (Phase 4: fork-aware)

## Verification

1. `cargo check --features tokamak-jit` (LEVM)
2. `cargo check -p tokamak-jit` (without LLVM)
3. `cargo test -p ethrex-levm --features tokamak-jit -- jit::` (8 tests pass)
4. `cargo test -p tokamak-jit` (7 tests pass)
5. `cargo clippy -p ethrex-levm --features tokamak-jit -- -D warnings` (clean)
6. `cargo test -p tokamak-jit --features revmc-backend` (CI only, requires LLVM 21)
