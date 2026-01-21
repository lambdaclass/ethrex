# ZisK Profiling Report: zerocopy_trie Branch

**Date:** 2026-01-21
**Block:** 24283607
**Branch:** zerocopy_trie

## Summary

| Version | Commit | Steps | Total Cost | vs Main |
|---------|--------|-------|------------|---------|
| **Main baseline** | main | 699,706,720 | 78.5B | - |
| **Previous commit** | 672c908 | 663,189,254 | 74.7B | **-5.22%** |
| **Latest commit** | 95a73c6 | 673,892,675 | 75.7B | **-3.69%** |

## Regression Analysis: 672c908 → 95a73c6

The latest commit `95a73c6ec` ("refactor(trie): store pruned child hashes directly in NodeType") introduced a **1.61% regression** compared to the previous commit.

### Cost Distribution Change

| Category | 672c908 | 95a73c6 | Change |
|----------|---------|---------|--------|
| MAIN | 45,096,869,272 | 45,824,701,900 | +1.61% |
| MEMORY | 5,501,683,005 | 5,581,480,793 | +1.45% |
| OPCODES | 10,137,214,856 | 10,239,318,024 | +1.01% |

### Hot Functions Affected

| Function | 672c908 | 95a73c6 | Change |
|----------|---------|---------|--------|
| `EncodedTrie::hash::recursive` | 23,778,263,127 | 24,278,203,568 | **+2.10%** |
| `EncodedTrie::hash` | 12,003,427,740 | 12,255,687,583 | **+2.10%** |
| memcpy (calls) | 1,001,396 | 1,053,721 | **+52,325 calls** |
| memcpy (cost) | 14,071,097,808 | 14,794,732,819 | +5.14% |

### New Functions in Latest

- `ethrex_vm::witness_db::GuestProgramStateWrapper::state_trie_root`: 6,452,848,276 cost

### Removed Functions

- `ethrex_common::types::block_execution_witness::GuestProgramState::*`: 6,387,848,210 cost

## Root Cause Hypothesis

The regression appears to stem from the refactoring that stores pruned child hashes directly in `NodeType`. This likely causes:

1. **More memcpy operations** (+52k calls) - possibly from copying hash data into the node structure
2. **Increased hashing cost** (+2.10%) - the recursive hash function is doing more work

## Investigation Areas

1. **`crates/common/trie/encodedtrie.rs`** - Check `hash::recursive` implementation
2. Look for new `clone()` or copy operations on hash data
3. Check if `PrunedChild` variant is causing additional allocations

## Profile Files

```
scripts/zkvm-bench/profiles/zisk/stats_20260121_110625_main_baseline.txt
scripts/zkvm-bench/profiles/zisk/stats_20260121_111017_zerocopy_672c908.txt
scripts/zkvm-bench/profiles/zisk/stats_20260121_111211_zerocopy_95a73c6_latest.txt
```

## Input Files

```
scripts/zkvm-bench/inputs/ethrex_mainnet_24283607_main_input.bin      (9.7 MB)
scripts/zkvm-bench/inputs/ethrex_mainnet_24283607_672c908_input.bin   (8.9 MB)
scripts/zkvm-bench/inputs/ethrex_mainnet_24283607_zerocopy_input.bin  (9.0 MB)
```

## Next Steps

1. Review the `hash::recursive` function in `encodedtrie.rs`
2. Identify where the additional memcpy calls originate
3. Consider if the pruned child hash storage can avoid copying
