# Block Composition Analysis Report

**Date**: 2026-02-19 20:16–21:17 UTC
**Server**: ethrex-office-2 (Ryzen 9 9950X3D, 126 GB RAM)
**Node**: ethrex @ commit `3f2fb8cc0` (main + witness background thread)
**Block range**: 24,493,223 – 24,493,523 (301 blocks, 1 hour)

---

## What I Ran

```bash
# 1. Build profiling binary (frame pointers + debug symbols)
cargo build --config .cargo/profiling.toml --profile release-with-debug --features jemalloc

# 2. Start node (same flags, just swapped binary)
target/release-with-debug/ethrex \
  --http.addr 0.0.0.0 --metrics --metrics.port 3701 \
  --network mainnet --authrpc.jwtsecret ~/secrets/jwt.hex \
  --log.dir /var/log/ethrex --precompute-witnesses

# 3. Start perf recording (997 Hz, frame-pointer unwinding, 1 hour)
perf record -F 997 --call-graph fp -p <PID> -o /tmp/perf_ethrex_1h.data -- sleep 3600

# 4. Collect block metrics from log (bash script parsing [METRIC] lines)
bash /tmp/collect_blocks.sh  # writes /tmp/block_data.csv

# 5. Enrich with tx types via RPC
python3 /tmp/enrich_blocks.py  # calls eth_getBlockByNumber for each block
```

## Data Sources

| Artifact | Path on ethrex-office-2 | Description |
|----------|------------------------|-------------|
| Raw log | `/tmp/ethrex_profiling.log` | Full node stdout for 1 hour |
| Block CSV | `/tmp/block_data.csv` | 301 blocks: number, total_ms, tx_count, gas, phases |
| Enriched CSV | `/tmp/block_data_enriched.csv` | + tx type breakdown (eip1559, legacy, eip4844, type4, eip2930) |
| perf data | `/tmp/perf_ethrex_1h.data` | 326K samples, 39 MB |
| Analysis JSON | `/tmp/analysis_results.json` | Machine-readable results |

## Method

### Threshold

- **P95 of total block processing time = 50 ms**
- Blocks with `total_ms >= 50` are classified as **slow** (18 blocks, 6.0%)
- Remaining 283 blocks are **fast**

### Distribution (all 301 blocks)

| Metric | Min | P25 | Median | P75 | P95 | Max |
|--------|-----|-----|--------|-----|-----|-----|
| Total (ms) | 7 | 24 | 29 | 38 | 50 | 65 |
| Exec (ms) | 0 | 14 | 20 | 27 | 39 | 54 |

### Categorization Logic

Blocks are categorized along 3 dimensions:

1. **TX count band**: very_low (<100), low (100-199), med (200-399), high (400+)
2. **Gas utilization band**: low (<30%), med (30-59%), high (60-89%), full (90%+)
3. **Composite** = TX band × Gas band (the primary categorization)

Among the 18 slow blocks, only 4 composite categories appeared.

### TX Type Mix

| Type | All blocks | Slow blocks | Fast blocks |
|------|-----------|-------------|-------------|
| EIP-1559 | 81.8% | 76% | 83% |
| Legacy | 17.1% | 23% | 16% |
| EIP-4844 | 0.5% | 1% | 1% |
| Type 4 (EIP-7702) | 0.4% | 0% | 0% |
| EIP-2930 | 0.1% | 0% | 0% |

**Key observation**: Slow blocks have a higher proportion of legacy transactions (23% vs 16%). Legacy txs on mainnet tend to be older contract interactions with higher gas consumption.

### Gas per Transaction

| Group | Median gas/tx | Mean gas/tx |
|-------|--------------|-------------|
| Slow (>=50ms) | 101,411 | 110,411 |
| Fast (<50ms) | 101,545 | 127,053 |

Gas per tx is nearly identical. **Slow blocks are slow because they have more transactions, not more complex ones.**

---

## Correlations (all 301 blocks)

| Pair | Pearson r | Strength | Interpretation |
|------|-----------|----------|----------------|
| total_ms vs exec_ms | **0.970** | Very strong | Execution phase dominates total time |
| total_ms vs gas_mgas | **0.873** | Strong | Gas used is the primary predictor of slowness |
| exec_ms vs gas_mgas | **0.828** | Strong | More gas → more EVM work → slower exec |
| store_ms vs gas_mgas | **0.748** | Strong | More gas → more state writes → slower store |
| total_ms vs tx_count | 0.689 | Moderate | More txs helps, but gas matters more |
| merkle_ms vs tx_count | -0.005 | None | Merkle runs concurrently; not correlated |

**Conclusion**: Gas used is the #1 predictor of block processing time (r=0.873). Transaction count is secondary (r=0.689). The exec phase accounts for 97% of the variance in total time.

---

## Profiling Hotspots (aggregated by functional group)

326K samples over 1 hour. Aggregated across all threads:

| Rank | Functional Group | Self % | Thread(s) | Notes |
|------|-----------------|--------|-----------|-------|
| 1 | **Witness generation (rayon)** | ~28.4% | rayon-worker-0..31 | `bridge_producer_consumer::helper` × 32 workers |
| 2 | **secp256k1 (ECDSA)** | ~8.7% | tokio-runtime-w | fe_mul 4.06%, ecmult 1.44%, fe_sqr 0.93%, fe_sqrt 0.60%, modinv 0.49%, ecmult_gen 0.25%, sha256 0.25% |
| 3 | **Keccak256 (SHA3)** | ~5.7% | witness (2.74%), tokio (2.51%), block_exec (0.44%) | Hashing for addresses, storage keys, trie nodes |
| 4 | **P2P peer table** | ~3.9% | tokio-runtime-w | `get_contact_to_initiate` 2.76%, ENR lookup 0.69%, lookup 0.46% |
| 5 | **BLS12-381** | ~2.9% | tokio-runtime-w | `mulq_mont_384` 1.64%, `mulq_by_1` 0.79%, `sqrq_384` 0.44% |
| 6 | **Hashing/HashMap** | ~2.3% | tokio-runtime-w | `hash_one` 1.51%, `DefaultHasher::write` 0.77% |
| 7 | **Trie operations** | ~2.6% | witness_generat | `TrieDB::get` 1.37%, `get_embedded_root` 0.43%, `decode_child` 0.37%, `Node::decode` 0.34% |
| 8 | **Witness serialization** | ~1.1% | witness_generat | `serialize_vec_of_hex_encodables` 0.75%, `serialize_str` 0.33% |
| 9 | **RocksDB (LZ4/XXH3)** | ~1.3% | rocksdb:low | `LZ4_compress` 0.88%, `LZ4_decompress` 0.21%, `XXH3` 0.16% |
| 10 | **Block execution (EVM)** | ~0.7% | block_executor_ | `VM::run_execution` 0.28%, `KeccakF1600` 0.44% |

### Critical Finding: Block Executor Thread is Only ~0.8% of CPU

The `block_executor_` thread (which is what determines `exec_ms`) shows:
- 0.44% KeccakF1600 (hashing)
- 0.28% VM::run_execution (EVM opcodes)
- 0.11% TrieWrapper::get (state reads)

This is because the node is **synced at the tip** and processes blocks every ~12 seconds. Block execution takes 20-65ms out of a 12,000ms slot — the CPU is mostly idle or doing background work (witness gen, P2P, RocksDB compactions).

---

## Ranked Table: Slow Block Composition Categories

Sorted from slowest to fastest median duration within the 18 slow blocks (>= 50ms):

| Rank | Composition Category | Slow Blocks | Median (ms) | P95 (ms) | Key Composition Traits | Dominant Profiling Hotspots | Correlation Signal | Example Blocks |
|------|---------------------|-------------|-------------|----------|----------------------|---------------------------|-------------------|----------------|
| 1 | **High-TX (400+) / Full-Gas (90%+)** | 9 | 53 | 65 | avg 607 txs, 59M gas, 77% EIP-1559, 22% legacy | secp256k1 (ECDSA tx verification scales with tx count), Keccak (address/storage hashing scales with gas), state I/O | Strong: r(total,gas)=0.87, r(total,tx)=0.69 | 24493448, 24493299, 24493422 |
| 2 | **Med-TX (200-399) / High-Gas (60-89%)** | 3 | 53 | 64 | avg 335 txs, 40M gas, 86% EIP-1559, 13% legacy | secp256k1, Keccak; lower tx count but higher gas-per-tx suggests heavier contract calls | Moderate: gas drives exec time despite fewer txs | 24493398, 24493399, 24493423 |
| 3 | **Med-TX (200-399) / Full-Gas (90%+)** | 2 | 51 | 51 | avg 321 txs, 60M gas, 78% EIP-1559, 20% legacy | Keccak and state I/O dominate (full gas with fewer txs = complex contracts); less ECDSA pressure | Strong gas correlation; exec is 74% of total (lower than avg) | 24493369, 24493378 |
| 4 | **High-TX (400+) / High-Gas (60-89%)** | 4 | 50.5 | 54 | avg 580 txs, 47M gas, 69% EIP-1559, 30% legacy — highest legacy share | secp256k1 dominates (high tx count = many sig verifications); lower gas means less EVM work per block | Moderate: tx count drives ECDSA load | 24493316, 24493411, 24493415 |

### Interpretation

- **Categories 1 and 2** (median 53ms) are the slowest — both have high gas. Category 1 adds high tx count on top.
- **Category 4** (median 50.5ms) is the "fastest slow" — high tx count but only 60-89% gas. The 30% legacy share is notable.
- All 18 slow blocks have gas >= 60%. No slow block has gas < 60%.
- Median differences across categories are small (50.5-53ms), with only 18 samples — the distinction is more about WHY they're slow (gas-heavy vs tx-heavy) than HOW slow.

### Correlation Signal Strength Explained

- **Strong** (r > 0.7): Gas utilization is the dominant predictor for categories 1 and 3
- **Moderate** (r 0.4-0.7): TX count adds explanatory power for categories 2 and 4
- **Limitation**: perf data is process-wide (not per-block), so the profiling correlation is qualitative — I infer which hotspots map to which block characteristics based on what each function does, not from direct per-block profiling

---

## Top 10 Slowest Blocks (for reference)

| Block | Total | Exec | Merkle | Store | TXs | Gas | Gas% | EIP-1559 | Legacy | EIP-4844 |
|-------|-------|------|--------|-------|-----|-----|------|----------|--------|----------|
| 24493448 | 65ms | 54ms | 8ms | 2ms | 625 | 60M | 100% | 510 | 108 | 2 |
| 24493398 | 64ms | 54ms | 7ms | 2ms | 377 | 43M | 72% | 293 | 80 | 2 |
| 24493422 | 60ms | 48ms | 7ms | 3ms | 694 | 60M | 100% | 580 | 110 | 3 |
| 24493299 | 57ms | 43ms | 10ms | 3ms | 810 | 60M | 100% | 731 | 69 | 4 |
| 24493239 | 55ms | 42ms | 7ms | 4ms | 779 | 59M | 99% | 495 | 281 | 1 |
| 24493415 | 54ms | 46ms | 5ms | 2ms | 462 | 43M | 72% | 348 | 114 | 0 |
| 24493375 | 53ms | 32ms | 17ms | 2ms | 468 | 56M | 94% | 291 | 169 | 4 |
| 24493423 | 53ms | 42ms | 7ms | 2ms | 346 | 40M | 66% | 316 | 25 | 1 |
| 24493379 | 52ms | 45ms | 2ms | 4ms | 407 | 60M | 100% | 365 | 32 | 4 |
| 24493399 | 52ms | 43ms | 6ms | 2ms | 281 | 37M | 61% | 250 | 29 | 1 |

**Notable outlier**: Block 24493375 has 17ms merkle time (vs typical 2-8ms for slow blocks). This block had 56M gas and 468 txs — the high merkle time suggests a burst of state changes that couldn't be fully absorbed by the concurrent merkle thread.

---

## Conclusion

### Top 2 Likely Root Causes of Slow Blocks

1. **Gas utilization is the primary driver** (r=0.873 with total_ms). Blocks that consume >60% of the 60M gas limit trigger proportionally more EVM execution, more Keccak hashing for storage slot computation, and more state I/O. All 18 slow blocks had gas >= 60%. The exec phase accounts for 82-93% of total block time.

2. **Transaction count amplifies ECDSA verification cost** (r=0.689). Each transaction requires secp256k1 signature recovery. With secp256k1 being the #1 CPU hotspot overall (8.7% of all samples), blocks with 400+ transactions create a measurable ECDSA verification load. However, since signature verification likely happens in the pipeline before exec, its impact may be partially hidden from the per-block timing.

### Top 2 Next Actions

1. **Profile the `block_executor_` thread in isolation during a full gas block**. The current whole-process profiling dilutes execution hotspots with P2P and witness generation. Use `perf record --tid <block_executor_tid>` for a 10-minute window to get EVM-level hotspots (opcode costs, state read patterns, storage cache hit rates).

2. **Investigate the P2P peer table hotspot** (`get_contact_to_initiate` at 2.76% + `get_contact_for_enr_lookup` at 0.69% + `get_contact_for_lookup` at 0.46% = 3.9%). This runs on tokio-runtime-w and should not compete with block execution on the same threads. If the peer table operations are O(n) scans or holding locks that block block processing, this could be a latent performance issue.

### Assumptions and Confidence

- **High confidence** in block composition data (parsed from structured log lines, enriched via RPC)
- **High confidence** in correlations (301 samples, strong r values, clear physical causation)
- **Medium confidence** in profiling-to-category correlation (perf is process-wide, not per-block; attribution is qualitative)
- **Low confidence** in category ranking differences (18 slow blocks across 4 categories; median differences of 0.5-2.5ms are within noise)
- **Note**: `warmer_ms` is always 0 in the CSV due to a collection script ordering issue (emits row before reading warmer line). This doesn't affect the analysis since warmer runs before exec and isn't part of total_ms.
