## Why

The current `warm_block` implementation (used for pre-Amsterdam blocks without BAL) re-executes every transaction speculatively just to discover which accounts and storage slots will be accessed. This is wasteful — we're running the full EVM (gas accounting, opcode dispatch, stack manipulation) twice per transaction: once in warming, once in actual execution.

We can replace this with static analysis that extracts the same information directly from transaction data and bytecode, avoiding the redundant computation.

## What Changes

- Replace `warm_block` speculative re-execution with static extraction:
  1. Extract call targets directly from `tx.to()` — no execution needed
  2. Predict CREATE/CREATE2 addresses from sender + nonce (CREATE) or salt (CREATE2)
  3. Scan called contracts' bytecode for static storage keys (PUSH1/PUSH2 followed by SLOAD)
  4. Batch prefetch accounts and storage slots via existing `store.prefetch_accounts()` / `store.prefetch_storage()`

- Add a new `static_warming` module in `crates/vm/backends/levm/` with:
  - `extract_call_targets()` — collect addresses from tx.to()
  - `predict_create_addresses()` — compute CREATE/CREATE2 addresses
  - `extract_static_storage_keys()` — bytecode analysis for SLOAD keys
  - `warm_block_static()` — orchestrates the above

- Modify `execute_block_pipeline` to use `warm_block_static` instead of `warm_block` for non-BAL path

## Capabilities

### New Capabilities
- `static-warming`: Replace speculative transaction re-execution with static analysis to pre-warm state before block execution. Reads accounts, storage, and code without running the EVM.

### Modified Capabilities
- (none) — this is an optimization within existing block execution, no spec-level behavior changes

## Impact

- **Affected code**: `crates/vm/backends/levm/mod.rs` — `warm_block` function and its callers
- **Performance**: Expected 50%+ reduction in warming phase CPU time
- **Storage I/O**: Same total reads, but more efficient batch prefetch
- **Compatibility**: No behavioral changes — produces identical block results
