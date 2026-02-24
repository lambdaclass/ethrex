# BAL-Based Parallel Block Execution — Status Report

**Branch:** `bal-parallel-exec`
**Date:** 2026-02-24
**Tested on:** Kurtosis devnet (5 clients: ethrex, geth, nethermind, besu, nimbus)

## Summary

ethrex executes Amsterdam+ blocks using BAL-derived state diffs: the Block Access List (EIP-7928) provides the complete post-block state, so transactions execute embarrassingly parallel for receipt/gas validation only.

**Performance results (60 blocks, ~113 avg txs/block, ~36 avg Mgas/block):**

| Client | Avg Ggas/s | Avg ms | Notes |
|--------|-----------|--------|-------|
| ethrex | 2.50 | 16.6 | peak 6.44 |
| nethermind | 2.99 | 16.8 | early blocks anomalously high |
| geth | 1.91 | 19.8 | |
| besu | 1.00 | 47.4 | |

All 5 clients in consensus throughout. Zero fallbacks.

## Architecture

### Pipeline

Block execution runs three threads concurrently:

1. **Warmer** — prefetches state using BAL address hints (or speculative tx re-execution for pre-Amsterdam)
2. **Executor** — runs transactions (parallel or sequential)
3. **Merkleizer** — computes state trie updates from BAL-derived AccountUpdates

### Parallel Execution Flow

```
BAL (from header)
    │
    ├──► bal_to_account_updates()   ← extract final state diffs directly from BAL
    │         │
    │         ▼
    │    send to merkleizer         ← single batch, trie computation starts immediately
    │
    ▼
rayon par_iter over ALL txs         ← embarrassingly parallel, each tx on isolated DB
    │                                  (pre-block state only, for receipts/gas)
    ▼
sort by tx_idx, build receipts      ← cumulative gas, validate against header
```

### How It Works

The BAL is **consensus-validated** (part of the block header). We verify:

1. **Per-tx gas/receipts** against block header (from independent parallel execution)
2. **State root** from BAL diffs must match header
3. **BAL hash** itself is validated against header

Transaction execution is only used to produce receipts and validate gas — **not** to produce state. State comes entirely from the BAL.

### `bal_to_account_updates`

Converts BAL into `Vec<AccountUpdate>` for the merkleizer:

- For each `AccountChanges` in the BAL, extracts the **highest `block_access_index`** entry per field (balance, nonce, code, storage)
- Loads pre-state from store for unchanged fields
- Detects account removal (EIP-161): post-state empty but pre-state wasn't
- Handles code deployment (computes `keccak(code)` for hash)
- Storage slots set to zero are included (valid trie deletions)
- `removed_storage = false` always (EIP-6780: SELFDESTRUCT only destroys same-tx accounts)
- Accounts with only `storage_reads` and no writes are skipped

### `execute_block_parallel`

1. Compute BAL-derived AccountUpdates → send to merkleizer (single batch covering ALL state changes: system calls, txs, withdrawals, post-tx)
2. Execute all txs via `rayon::par_iter` — each tx gets its own `GeneralizedDatabase` seeded with post-system-call state
3. Sort by tx_idx, build receipts with cumulative gas

**No coinbase handling needed** — the BAL records the final coinbase balance directly.

**Gas semantics:** Each tx runs against pre-block state. Gas may differ from sequential execution if a tx depends on prior-tx state (e.g., warm vs cold SLOAD). But we validate `gas_used == header.gas_used` and `receipts_root`, so any divergence = invalid block.

## Files

| File | Role |
|------|------|
| `crates/vm/backends/levm/mod.rs` | `bal_to_account_updates`, `execute_block_parallel`, BAL branch in `execute_block_pipeline` |
| `crates/blockchain/blockchain.rs` | Pipeline orchestration (`add_block_pipeline`) |
| `crates/vm/levm/src/db/gen_db.rs` | `GeneralizedDatabase` with shared-base support for parallel tx isolation |

## Next Steps

- **LEVM interpreter optimization** — execution speed is the bottleneck, not parallelism overhead
- **Validate edge cases** — blocks with SELFDESTRUCT, EIP-7702 delegations, heavy contract interactions
- **ef-tests / hive** — run full test suite against BAL parallel path
