# engine_newPayload Performance Optimizations

## Summary

This document records a series of performance optimization experiments targeting the `engine_newPayload` RPC endpoint, which is critical for block execution in Ethereum clients. Five different optimization strategies were implemented on separate branches and benchmarked against a baseline.

**Result**: All optimizations showed differences within the benchmark's noise margin (~10ms variance). None of the optimizations produced statistically significant improvements for this workload.

## Benchmark Setup

### Hardware & Environment
- **Server**: Ryzen 9 9950X3D
- **Configuration**: 8 CPUs, 64GB RAM (Docker container limits)

### Benchmark Tool
- **Tool**: [expb](https://github.com/lambdaclass/execution-payloads-benchmarks) (execution payloads benchmarks)
- **Scenario**: `ethrex-c100`
- **Warmup**: 4831 blocks before measurement
- **Measured**: 100 blocks with per-payload metrics

### Command
```bash
sudo /home/admin/.local/bin/expb execute-scenario \
    --scenario-name ethrex-c100 \
    --config-file config.yaml \
    --per-payload-metrics
```

### Benchmark Variance
Based on 7 repeated runs of the same branch, the benchmark variance is approximately **10ms for average latency** and **20ms for p99 latency**. This establishes the noise floor for interpreting results.

## Benchmark Results

| Branch | avg | med | p95 | p99 | Δ avg vs baseline |
|--------|-----|-----|-----|-----|-------------------|
| **perf_newpayload_testing (baseline)** | 1.98s | 1.89s | 2.66s | 4.11s | - |
| perf-merkle-exec-overlap | 1.95s | 1.86s | 2.64s | 4.13s | -30ms (1.5%) |
| perf-parallel-db-writes | 1.96s | 1.88s | 2.64s | 4.12s | -20ms (1.0%) |
| perf-incremental-receipts-root | 1.97s | 1.89s | 2.66s | 4.19s | -10ms (0.5%) |
| perf-parent-header-cache | 1.95s | 1.87s | 2.65s | 4.10s | -30ms (1.5%) |
| perf-code-cache-warmup | 1.97s | 1.88s | 2.66s | 4.15s | -10ms (0.5%) |

## Optimizations Attempted

### 1. Adaptive State Transition Flushing (perf-merkle-exec-overlap)

**Branch**: `perf-merkle-exec-overlap`
**Expected Impact**: 10-15%
**Actual Impact**: ~1.5%

**Description**: Modified the heuristic for flushing state transitions during the execution/merkleization pipeline. The optimization flushes state transitions sooner for high-gas transactions (>500k gas), reducing the threshold from 5 transactions to 2 when gas exceeds the threshold.

**Changes** (`crates/vm/backends/levm/mod.rs`):
```rust
const GAS_THRESHOLD_FOR_EARLY_FLUSH: u64 = 500_000;
const BASE_TX_COUNT_THRESHOLD: usize = 5;
const MIN_TX_COUNT_THRESHOLD: usize = 2;

let should_flush = if queue_length.load(Ordering::Relaxed) == 0 {
    let effective_threshold = if gas_since_last_flush >= GAS_THRESHOLD_FOR_EARLY_FLUSH {
        MIN_TX_COUNT_THRESHOLD
    } else {
        BASE_TX_COUNT_THRESHOLD
    };
    tx_since_last_flush > effective_threshold
} else {
    false
};
```

**Rationale**: High-gas transactions typically modify more state, so flushing sooner could improve merkleization parallelism.

---

### 2. Parallel DB Write Preparation (perf-parallel-db-writes)

**Branch**: `perf-parallel-db-writes`
**Expected Impact**: 5-10%
**Actual Impact**: ~1.0%

**Description**: Parallelized the RLP encoding of blocks, transactions, receipts, and contract codes using rayon before performing sequential database writes.

**Changes** (`crates/storage/store.rs`):
```rust
use rayon::prelude::*;

// Parallel preparation of block data
let block_data: Vec<_> = update_batch.blocks.par_iter()
    .map(|block| { /* RLP encoding */ })
    .collect();

let receipt_data: Vec<_> = update_batch.receipts.par_iter()
    .flat_map(/* ... */)
    .collect();

let code_data: Vec<_> = update_batch.code_updates.par_iter()
    .map(/* ... */)
    .collect();

// Sequential DB writes after parallel preparation
```

**Rationale**: Encoding is CPU-bound and can be parallelized, while DB writes remain sequential for consistency.

---

### 3. Incremental Receipts Root Computation (perf-incremental-receipts-root)

**Branch**: `perf-incremental-receipts-root`
**Expected Impact**: 5-8%
**Actual Impact**: ~0.5%

**Description**: Compute the receipts root incrementally during execution instead of rebuilding the entire trie at the end. Each receipt is inserted into the trie as it's created.

**Changes**:

`crates/common/types/block.rs` - Added incremental builder:
```rust
pub struct IncrementalReceiptsRoot {
    trie: Trie,
    count: usize,
}

impl IncrementalReceiptsRoot {
    pub fn new() -> Self {
        Self { trie: Trie::new_temp(), count: 0 }
    }

    pub fn insert(&mut self, receipt: &Receipt) {
        let key = self.count.encode_to_vec();
        let value = receipt.encode_to_vec();
        self.trie.insert(key, value).expect("...");
        self.count += 1;
    }

    pub fn root(self) -> H256 {
        self.trie.hash_no_commit()
    }
}
```

`crates/vm/backends/mod.rs` - Added optional pre-computed root to result:
```rust
pub struct BlockExecutionResult {
    pub receipts: Vec<Receipt>,
    pub requests: Vec<Requests>,
    pub receipts_root: Option<H256>,  // New field
}
```

**Rationale**: Avoids rebuilding the receipts trie from scratch after all transactions are processed.

---

### 4. Parent Header LRU Cache (perf-parent-header-cache)

**Branch**: `perf-parent-header-cache`
**Expected Impact**: 2-5%
**Actual Impact**: ~1.5%

**Description**: Added a 64-entry LRU cache for parent block headers to avoid repeated database lookups during block validation and execution.

**Changes** (`crates/blockchain/blockchain.rs`):
```rust
use lru::LruCache;

const PARENT_HEADER_CACHE_SIZE: usize = 64;

pub struct Blockchain {
    // ... existing fields
    parent_header_cache: Mutex<LruCache<H256, BlockHeader>>,
}

impl Blockchain {
    pub fn find_parent_header_cached(&self, block_header: &BlockHeader)
        -> Result<BlockHeader, ChainError>
    {
        let parent_hash = block_header.parent_hash;

        // Check cache first
        if let Some(header) = self.parent_header_cache
            .lock().unwrap().get(&parent_hash).cloned()
        {
            return Ok(header);
        }

        // Fall back to DB lookup
        let header = self.find_parent_header(block_header)?;
        self.parent_header_cache.lock().unwrap()
            .put(parent_hash, header.clone());
        Ok(header)
    }
}
```

**Rationale**: Parent headers are frequently accessed during block processing; caching avoids redundant DB reads.

---

### 5. Contract Code Cache Warmup (perf-code-cache-warmup)

**Branch**: `perf-code-cache-warmup`
**Expected Impact**: 2-4%
**Actual Impact**: ~0.5%

**Description**: Pre-warm the contract code cache by batch-loading codes for all transaction target addresses before execution starts.

**Changes**:

`crates/storage/store.rs` - Added warmup method:
```rust
pub fn warmup_code_cache_for_addresses(
    &self,
    parent_hash: BlockHash,
    addresses: &[Address],
) -> Result<(), StoreError> {
    for address in addresses {
        if let Some(account_state) = self.get_account_state_by_hash(parent_hash, address)? {
            if account_state.code_hash != *EMPTY_KECCACK_HASH {
                let _ = self.get_account_code(account_state.code_hash);
            }
        }
    }
    Ok(())
}
```

`crates/blockchain/blockchain.rs` - Called before execution:
```rust
let target_addresses: Vec<_> = block.body.transactions.iter()
    .filter_map(|tx| match tx.to() {
        TxKind::Call(addr) => Some(addr),
        TxKind::Create => None,
    })
    .collect();

let _ = self.storage.warmup_code_cache_for_addresses(
    parent_header.hash(),
    &target_addresses
);
```

**Rationale**: Pre-loading contract codes before execution could reduce cache misses during transaction processing.

---

## Conclusions

1. **No Statistically Significant Gains**: All observed improvements (10-30ms) fall within the benchmark's noise margin (~10ms for avg, ~20ms for p99). The differences cannot be distinguished from natural run-to-run variance.

2. **Possible Explanations**:
   - The baseline implementation may already be well-optimized for this workload
   - The bottleneck may lie elsewhere (e.g., EVM execution, trie operations)
   - These optimizations may show better results under different workloads (higher gas blocks, more contract interactions)

3. **Recommendations**:
   - Profile to identify the actual bottleneck before implementing more optimizations
   - Consider testing with higher-gas or contract-heavy blocks where these optimizations might have more impact
   - Focus on optimizations that target the dominant cost centers identified through profiling

## Branch Reference

| Branch Name | Description | Status |
|-------------|-------------|--------|
| `perf_newpayload_testing` | Baseline branch | Benchmarked |
| `perf-merkle-exec-overlap` | Adaptive state transition flushing | Benchmarked |
| `perf-parallel-db-writes` | Parallel DB write preparation | Benchmarked |
| `perf-incremental-receipts-root` | Incremental receipts root | Benchmarked |
| `perf-parent-header-cache` | LRU cache for parent headers | Benchmarked |
| `perf-code-cache-warmup` | Contract code cache warmup | Benchmarked |

## Date

2026-01-19
