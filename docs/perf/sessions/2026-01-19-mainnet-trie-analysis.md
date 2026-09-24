# Session: 2026-01-19 - Mainnet State Trie Shape Analysis

## Objective

Analyze the shape of the Ethereum mainnet state trie to understand:
- Account and storage slot distribution
- Storage concentration patterns
- Implications for trie optimization

## Environment

- Data source: Synced mainnet node (~432 GB)
- Block height: ~21.8M (Jan 2026)
- Tool: `trie_analyzer` binary

## Raw Statistics

### Account Summary

| Metric | Value |
|--------|-------|
| Total accounts | 349,725,275 |
| Accounts with storage (contracts) | 25,076,455 (7.2%) |
| Accounts without storage (EOAs) | 324,648,820 (92.8%) |
| Account key length | 65 bytes (nibbles) |

### Storage Summary

| Metric | Value |
|--------|-------|
| Total storage slots | 1,478,870,586 |
| Average slots per contract | 58.97 |
| Median slots per contract | 1 |
| Max slots (single contract) | 139,244,689 |
| Storage key length | 131 bytes (nibbles) |

## Distribution Analysis

### Storage Concentration (Power Law)

The storage distribution follows an extreme power law (Pareto/Zipf distribution).

| Top N Accounts | Storage Slots | % of Total |
|----------------|---------------|------------|
| Top 1 | 139,244,689 | 9.42% |
| Top 10 | 276,701,133 | 18.71% |
| Top 100 | 530,886,000 | 35.89% |
| Top 1,000 | 783,345,000 | 52.97% |
| Top 10,000 | 1,087,353,000 | 73.51% |

**Gini Coefficient: 0.9544** (0 = perfect equality, 1 = max inequality)

This is comparable to the most unequal wealth distributions globally, indicating extreme concentration.

### Slots per Account Histogram

```
            1:   13,809,276 ( 55.1%) ██████████████████████████████████████████████████
          2-5:    7,417,999 ( 29.6%) ██████████████████████████
         6-10:    1,544,374 (  6.2%) █████
        11-50:    1,804,577 (  7.2%) ██████
       51-100:      169,394 (  0.7%)
      101-500:      213,884 (  0.9%)
       501-1K:       42,675 (  0.2%)
       1K-10K:       60,795 (  0.2%)
     10K-100K:       12,336 (  0.0%)
      100K-1M:        1,031 (  0.0%)
       1M-10M:          105 (  0.0%)
         >10M:            9 (  0.0%)
```

### Percentile Distribution

| Percentile | Slots per Account |
|------------|-------------------|
| p50 (median) | 1 |
| p90 | 10 |
| p95 | 16 |
| p99 | 154 |
| p99.9 | 4,678 |
| p99.99 | ~50,000 |
| max | 139,244,689 |

## Top 20 Contracts by Storage

| Rank | Storage Slots | % Total | Hashed Address (nibbles) |
|------|---------------|---------|--------------------------|
| 1 | 139,244,689 | 9.416% | 0105090e040809000c0d... |
| 2 | 20,765,768 | 1.404% | 0a0b01040d06080800... |
| 3 | 20,669,980 | 1.398% | 0f0e010c020c030b0f... |
| 4 | 16,991,670 | 1.149% | 010f0f000800000a06... |
| 5 | 16,294,950 | 1.102% | 0f0b0d010509020b06... |
| 6 | 15,731,344 | 1.064% | 040e060f01090b0d0f... |
| 7 | 13,479,639 | 0.911% | 090f01030f08080203... |
| 8 | 12,656,832 | 0.856% | 070b050805050b0b09... |
| 9 | 10,836,537 | 0.733% | 01030e050202010b03... |
| 10 | 9,991,466 | 0.676% | 000e00020b0b040c05... |
| 11 | 9,128,033 | 0.617% | 0a05030b040a0e0001... |
| 12 | 9,060,738 | 0.613% | 090b0a080d01000d0d... |
| 13 | 8,834,286 | 0.597% | 030a05050e06070008... |
| 14 | 8,681,664 | 0.587% | 0d00090803020c0a05... |
| 15 | 7,979,462 | 0.540% | 0c020a0e0c07010c0f... |
| 16 | 7,834,867 | 0.530% | 0105090406040504... |
| 17 | 6,824,988 | 0.462% | 080607090e080e0d0a... |
| 18 | 6,226,468 | 0.421% | 0000080f06050c0003... |
| 19 | 5,704,267 | 0.386% | 05040a0f000c000006... |
| 20 | 5,303,466 | 0.359% | 0b0106070b0b0d0f03... |

Note: Addresses shown are keccak256(address) in nibble format. Need reverse lookup to identify actual contracts.

## Key Findings

### 1. Bimodal Distribution

The state is divided into two very different populations:
- **92.8% EOAs**: No storage, just nonce/balance
- **7.2% Contracts**: Highly variable storage (1 to 139M slots)

### 2. Extreme Concentration

- **Top 100 contracts hold 36% of all storage**
- **Top 10,000 contracts hold 74% of all storage**
- The Gini coefficient (0.95) indicates near-maximal inequality

### 3. Long Tail

- 55% of contracts have exactly 1 storage slot
- 85% of contracts have ≤5 storage slots
- Only 0.5% have >100 slots
- Only 114 contracts have >1M slots

### 4. Whale Contracts

A tiny number of contracts dominate:
- 9 contracts with >10M slots
- 105 contracts with 1M-10M slots
- 1,031 contracts with 100K-1M slots

## Implications for Optimization

### 1. Cache Strategy

**Hot contract caching is highly effective:**
- Caching top 100 contracts covers 36% of storage ops
- Caching top 10,000 contracts covers 74% of storage ops
- LRU cache should work well given power law access patterns

### 2. Trie Structure

**Storage trie depth varies enormously:**
- Most contracts: 1-4 levels deep (≤16 slots)
- Whale contracts: Full 64-level depth (millions of slots)

**Optimization opportunity:** Special-case small storage tries (1-16 slots) with simpler data structure.

### 3. Merkle Computation

**Parallel merkleization benefits:**
- Whale contracts dominate merkle time
- Top 100 contracts likely consume majority of merkle CPU
- Pre-computing or caching merkle roots for stable contracts could help

### 4. EOA Optimization

**93% of accounts have no storage:**
- Fast path for EOA state updates (just nonce/balance)
- Skip storage trie operations entirely for EOAs
- Consider separate storage for EOAs vs contracts

### 5. Batch Processing

**Storage updates are concentrated:**
- Most blocks touch a small subset of contracts
- Batch updates to same contract's storage trie
- Defer merkle recomputation until end of batch

## Data Structure Recommendations

Based on this analysis:

| Account Type | Count | Recommended Structure |
|--------------|-------|----------------------|
| EOAs (no storage) | 325M | Flat map, no trie needed |
| Tiny contracts (1-16 slots) | 24M | Inline array or small map |
| Medium contracts (17-10K slots) | 72K | Standard Patricia trie |
| Whale contracts (>10K slots) | 13K | Sharded/cached Patricia trie |

## Future Work

1. **Identify top contracts** - Reverse lookup hashes to identify actual contracts (USDT, Uniswap, etc.)
2. **Access pattern analysis** - Which contracts are accessed most frequently?
3. **Temporal analysis** - How does storage grow over time?
4. **Depth distribution** - Actual trie depth histogram for storage tries

## Appendix: Analysis Tool

The analysis was performed using `benches/src/trie_analyzer.rs`:

```bash
# Run the analyzer
cargo run -p ethrex-benches --bin trie_analyzer --release

# Or with custom data directory
./target/release/trie_analyzer /path/to/ethrex/data
```

Scan time: ~7 minutes for 432 GB database.
