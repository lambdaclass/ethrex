# zkVM Optimization Logbook

This file tracks optimization attempts, successful or failed, for the ethrex zkVM guest program.

## Format

Each entry should include:
- **Date**: When the optimization was attempted
- **Branch**: Git branch name
- **Description**: What was optimized and why
- **Baseline**: Starting metrics (steps, cost)
- **Result**: Ending metrics and % change
- **Status**: Success/Regression/No Change
- **Notes**: Any important observations

---

## Entry #1: Block-Level Base Blob Fee Caching (P2 Revised)

**Date**: 2026-01-22
**Branch**: `opt/p2-fake-exp`
**Commit**: `40885a49d`
**Block**: 24291039 (mainnet)

### Description

Optimized EIP-4844 blob gas calculation by computing `base_blob_fee_per_gas` once per block instead of per transaction. This replaced a previous approach that used i128 conditional compilation but still computed the value ~1536 times per block.

### Changes

- Removed `fake_exponential_i128()` and all zkVM-specific conditional compilation
- Modified `calculate_blob_gas_cost()` to accept pre-computed value from Environment
- Compute `base_blob_fee_per_gas` once in `execute_block()`/`execute_block_pipeline()` before transaction loop
- Thread the value through `setup_env()`, `execute_tx()`, and `execute_tx_in_block()`

### Metrics

**Baseline** (f5524c2d7):
- Total Steps: 641,671,652
- Total Cost: 72,815,736,824

**Optimized** (40885a49d):
- Total Steps: 591,264,620
- Total Cost: 68,142,008,314

**Change**:
- Steps: **-50,407,032 (-7.86%)**
- Cost: **-4,673,728,510 (-6.42%)**

### Breakdown

| Function | Baseline | Optimized | Change |
|----------|----------|-----------|--------|
| `LEVM::execute_tx` | 38,910,195,044 | 34,254,599,227 | **-11.96%** |
| `LEVM::execute_block` | 41,245,142,369 | 36,571,413,773 | **-11.33%** |
| `VM::execute` | 35,119,228,101 | 32,766,859,301 | **-6.70%** |

Memory operations:
- memcpy calls: 921,521 → 832,937 (-88,584, **-9.6%**)
- memcpy cost: 13.5B → 12.6B (-900M, **-6.7%**)

### Status

✅ **SUCCESS** - Significant improvement with cleaner code

### Notes

- Previous i128 approach achieved ~598M steps but with conditional compilation complexity
- This block-level approach is simpler (-62 net lines of code) and more efficient
- The optimization eliminates ~1536 redundant `fake_exponential` calls per block (reduces to 1)
- Architecturally correct: block-level constants should be computed at block level
- No functional changes - all tests pass

### Files Modified

- `crates/common/types/block.rs`
- `crates/vm/levm/src/utils.rs`
- `crates/vm/levm/src/hooks/default_hook.rs`
- `crates/vm/backends/levm/mod.rs`
- `crates/vm/backends/levm/tracing.rs`
- `crates/vm/backends/mod.rs`

### Related

- PR: #6001
- Branch points to: `zerocopy_trie`
