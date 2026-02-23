# BAL-Based Parallel Block Execution — Status Report

**Branch:** `bal-parallel-exec`
**Date:** 2026-02-23
**Tested on:** Kurtosis devnet (5 clients: ethrex, geth, nethermind, besu, nimbus)

## Summary

ethrex now executes Amsterdam+ blocks in parallel using the Block Access List (EIP-7928) to build a conflict graph and group non-conflicting transactions. A two-tier fallback system handles cases where the heuristic grouping misses dependencies.

**Results across 160 Amsterdam blocks on Kurtosis devnet:**

| Client | Avg Ggas/s | Avg ms/block | vs ethrex |
|--------|-----------|-------------|-----------|
| **ethrex** | **2.83** | **14.2** | — |
| nethermind | 2.14 | 16.3 | -24% |
| geth | 2.08 | 18.0 | -26% |
| besu | 0.78 | 45.7 | -72% |

ethrex achieves **7.2x average parallelism** (min 3.1x, max 8.7x) with ~110 txs/block average.

## Architecture

### Pipeline

Block execution runs three threads concurrently:

1. **Warmer** — prefetches state using BAL address hints (or speculative tx re-execution for pre-Amsterdam)
2. **Executor** — runs transactions (parallel or sequential)
3. **Merkleizer** — computes state trie updates as AccountUpdates stream in

### Parallel Execution Flow

```
BAL (from header)
    │
    ▼
build_parallel_groups()     ← conflict graph via Union-Find
    │                         W-W conflicts: same address written by 2+ txs
    │                         RAW conflicts: tx reads address another wrote
    │                         Same-sender: chained by nonce dependency
    ▼
rayon par_iter over groups  ← each group: sequential execution on isolated DB
    │
    ▼
merge AccountUpdates        ← reconcile coinbase deltas, merge per-group state
    │
    ▼
send to merkleizer          ← trie computation starts
```

### Grouping Algorithm (`build_parallel_groups`)

1. **Phase 1 — Write-write conflicts:** Extract per-tx write sets from BAL. Union txs that write to the same resource (Balance, Code, Storage slot, Nonce). Coinbase writes are excluded (handled separately via per-group delta tracking).

2. **Phase 2 — Read-after-write conflicts:** Approximate per-tx read sets and union any tx that reads a resource another tx wrote. Two modes:
   - **Heuristic mode** (first attempt): reads approximated from static tx metadata (sender, `to`, access_list, authorization_list) and BAL-derived addresses.
   - **Refined mode** (retry): uses actual per-tx read addresses captured from the failed first attempt.

3. **Phase 3 — Same-sender chaining:** Txs from the same sender are unioned (nonce ordering).

4. **Extract groups:** Connected components from the Union-Find become execution groups.

## Fallback System

The fallback exists because we cannot build a perfect conflict graph from the BAL alone. The BAL records *which addresses each tx writes to* (with per-tx attribution via the BAL index), but `storage_reads` is a flat per-account list with **no BAL index** — we cannot tell which transaction performed a given read. This means read-after-write (RAW) dependencies must be approximated from static tx metadata (sender, `to`, access_list, etc.), which can miss reads that happen through internal CALL chains. When a missed RAW dependency causes incorrect parallel execution, the fallback catches it.

### Two-Tier Retry

When parallel execution produces a mismatch (gas, receipts root, or state root doesn't match the block header):

```
Tier 0: Heuristic parallel grouping
  │
  ├─ succeeds (~90%) → done
  │
  ▼ mismatch detected
Tier 1: Refined parallel grouping (using actual per-tx read addresses)
  │
  ├─ succeeds (100% so far) → done
  │
  ▼ mismatch detected
Tier 2: Sequential fallback (BAL disabled)
  │
  └─ always correct → done
```

### Observed Fallback Rates (160 blocks)

- **Tier 0 success:** ~90% (142/160 blocks)
- **Tier 1 success:** 100% of fallbacks recovered here (18/18)
- **Tier 2 (sequential):** 0 blocks needed this
- **Total wasted time:** 275ms across 18 fallbacks, avg 15ms per fallback

### How Refined Retry Works

During the failed parallel execution, each tx's actual read addresses are captured from `GeneralizedDatabase.current_accounts_state.keys()` (the set of accounts the tx loaded). These are stored on the DB and consumed by the retry attempt.

On retry, `build_parallel_groups` uses these actual reads instead of the heuristic approximation, producing tighter groups that correctly separate false-positive conflicts while preserving true dependencies.

### Cache Reuse Across Retries

Each retry creates a fresh `GeneralizedDatabase` via `fresh_with_same_store()`. This gives a clean write slate while preserving the `CachingDatabase` (read cache backed by an `Arc` to the underlying store). Reads from the parent state are still valid across retries — only the computed writes were wrong.

## Limitations

### BAL `storage_reads` Lacks Per-Tx Attribution

The BAL records `storage_reads` per account as a flat `Vec<U256>` — there is no BAL index (tx index) associated with each read. This means we cannot determine *which transaction* read a given storage slot from the BAL alone.

This is the root cause of the ~10% fallback rate: CALL transactions that sub-call contracts (reading their state without writing) have those addresses invisible to the heuristic grouper. When another tx writes to that address, the read-after-write dependency is missed.

The heuristic compensates by using "address-narrowed" read sets — each tx's reads are approximated from its known addresses (sender, `to`, access_list entries, authorization_list targets, BAL-derived write addresses). This over-approximates (some txs are grouped together unnecessarily, reducing parallelism slightly) but can also under-approximate (a tx may CALL an address not in any of these sets).

### Coinbase Handling Complexity

Every transaction implicitly reads and writes the coinbase balance (gas fees). Naively, this would serialize all txs into one group. Instead:

- Coinbase writes are **excluded** from the conflict graph
- Each parallel group tracks its own coinbase balance delta
- After all groups finish, deltas are reconciled on the main DB
- Credits and debits are tracked separately to handle the edge case where coinbase is also a tx sender

### Merkleizer Wasted Work on Fallback

The execution and merkleizer threads run concurrently. When parallel execution fails, the merkleizer has already started computing the trie for incorrect data. The `std::thread::scope` must wait for the merkleizer to finish before the error can propagate and trigger a retry. This means the wasted time includes both execution + merkle computation for the failed attempt.

A potential optimization: validate block gas before sending updates to the merkleizer (the gas sum is just a few additions, essentially free). This would skip merkle work entirely on the ~10% of blocks that fail. Not yet implemented.

## Timing Breakdown (Instrumented)

Temporary timing spans show where time is spent within `execute_block_parallel`:

| Phase | Typical | Notes |
|-------|---------|-------|
| `groups` | 0.2–0.7ms | Building conflict graph — negligible |
| `exec` | 1–31ms | **Dominates** — rayon parallel tx execution |
| `merge` | 0.1–0.3ms | AccountUpdate merging — negligible |
| `send` | 0.0–0.1ms | Channel send to merkleizer — negligible |

The bottleneck is purely execution speed. The parallel infrastructure (grouping, merging, coinbase reconciliation) adds <1ms combined.

Variance in `exec` is driven by specific tx complexity within the largest group. Block 92 hit 31ms due to a heavy group; most blocks complete in 2–15ms.

## Files Changed

| File | Changes |
|------|---------|
| `crates/vm/levm/src/db/gen_db.rs` | `per_tx_read_addresses` field for refined retry |
| `crates/vm/backends/levm/mod.rs` | Parallel grouping (heuristic + refined), per-tx read capture, timing instrumentation |
| `crates/blockchain/blockchain.rs` | Two-tier retry logic |

## Next Steps

- **Remove timing instrumentation** — temporary, for profiling only
- **LEVM interpreter optimization** — execution speed is the bottleneck, not parallelism overhead
- **Pre-merkle gas validation** — skip merkle work on failed attempts (saves ~15ms per fallback)
- **BAL spec improvement** — if `storage_reads` gains per-tx attribution in a future EIP revision, the heuristic grouping could become exact, eliminating fallbacks entirely
