# Performance Improvement Ideas

This document tracks performance optimization ideas for ethrex.

See [methodology.md](methodology.md) for the full benchmarking workflow.
See [tracking.md](tracking.md) for tracking formats.

## Status Legend

- **To do**: Not started
- **In progress**: Currently being worked on
- **Review**: Code ready for review
- **Benches**: Benchmarking in progress
- **Discarded**: Idea rejected after evaluation (see reason in Discarded section)
- **Done**: Completed and merged

## Column Legend

| Column | Description |
|--------|-------------|
| Improvements | Measured improvement percentage |
| Regressions | Any regressions observed |
| Conflicts | Ideas that conflict with this one |
| Last Tested | Date of last benchmark |
| Experiment | Link to experiment log |
| Notes | Additional context |

---

## Execution Optimizations (LEVM)

### Low Difficulty

| Idea | Status | Improvements | Regressions | Conflicts | Last Tested | Experiment | Notes |
|------|--------|--------------|-------------|-----------|-------------|------------|-------|
| Nibbles 1-byte 2-nibble representation | In progress | | | | | | |
| Nibble fixed storage | To do | | | | | | |
| Use FxHashSet for access lists | In progress | | | | | | |
| Skip memory zero-init | In progress | | | | | | |
| Replace BTreeMap/BTreeSet with FxHashMap/FxHashSet | Benches | | | | | | |
| Remove RefCell from Memory | In progress | | | | | | |
| Inline Hot Opcodes | Review | 20-40% (memory, push, dup) | 15-25% (NUMBER, BLOBBASEFEE, initcode) | | | | Analyze jumpdest noise, then server |
| Avoid Clone on Account Load | Review | 1.33-1.45x (RETURN/REVERT 1KiB), 1.18-1.19x (LOG*), 1.18x (MSIZE) | 1.19-1.30x slower (block_full_of_ether_transfers) | | | | |
| SSTORE double lookup | In progress | | | | | | Cache used, no double DB lookup but two hashmap lookups |
| Hook Cloning Per Opcode | In progress | | | | | | Runs once per tx, clone individual RCs instead of vector |
| keccak caching | To do | | | | | | LRU cache for top 10k hashes, especially contract constants |
| Buffer reuse | To do | | | Buffer reuse | | | Free-list pattern with vec of buffers instead of allocating |

### Medium Difficulty

| Idea | Status | Improvements | Regressions | Conflicts | Last Tested | Experiment | Notes |
|------|--------|--------------|-------------|-----------|-------------|------------|-------|
| Object Pooling (reuse EVM stack frames, memory buffers) | To do | | | Buffer reuse | | | |
| SIMD everywhere | To do | | | | | | |
| Stackalloc for Small Buffers | To do | | | | | | |
| Use Arena Allocator for Substate Backups | Review | 15-20% (CHAINID, SDIV, Swap) | 25-30% (CODECOPY, CALLDATACOPY) | | | | Analyze jumpdest noise, then server |
| Arkworks EC Pairing | Review | 2x (bn128), 1.6-2x (ec pairing) | 20% (initcode jumpdest), 14% (big memory access) | | | | Analyze jumpdest noise, then server |
| Jumptable vs Calltable | In progress | | | | | | Confirmed we have a jump table |
| Mempool Lock Contention | To do | | | | | | mempool pruning O(n^2), parking_lot::RWLock, DashMap |
| Precompile caching | To do | | | | | | Per-address LRU cache with spec validation |
| Cross-block cache reuse | To do | | | Hierarchical storage cache | | | Saved cache pattern with usage guards between blocks |
| Hierarchical storage cache | To do | | | Cross-block cache reuse | | | Code cache, storage cache, account cache |
| Parallel proof workers with atomic availability | To do | | | | | | Replace 16-fixed with dynamic |

### High Difficulty

| Idea | Status | Improvements | Regressions | Conflicts | Last Tested | Experiment | Notes |
|------|--------|--------------|-------------|-----------|-------------|------------|-------|
| LEVM simplify stack and results | To do | | | | | | |
| Parallel Transaction Execution | To do | | | | | | Requires dependency analysis |
| PGO (Profile-Guided Optimization) | To do | | | | | | 10-20% typical gains |
| ruint crate | To do | | | | | | Brute approach doesn't work, evaluate for specific spots |

### Discarded

| Idea | Reason | Last Tested | Experiment | Notes |
|------|--------|-------------|------------|-------|
| Test PEVM | 10% worse on full block | | | Quick test on server showed regression |
| Jumpdest with stored bitmaps | Worsened performance | | | Tested multiple times |
| RLP Double Encoding for Length | Already implemented | | | Length already implemented for all types |

---

## I/O and Storage Optimizations

### Caching

| Idea | Status | Improvements | Regressions | Conflicts | Last Tested | Experiment | Notes |
|------|--------|--------------|-------------|-----------|-------------|------------|-------|
| Transaction pre-warming | To do | | | | | | Pre-execute in parallel to populate caches |
| LRU cache for states and accounts | To do | | | | | | |
| Increase CodeCache Size | To do | | | | | | |
| Cache trie top level (~2 levels) | To do | | | | | | |
| Disklayer memory accumulator | To do | | | | | | |

### Trie / State

| Idea | Status | Improvements | Regressions | Conflicts | Last Tested | Experiment | Notes |
|------|--------|--------------|-------------|-----------|-------------|------------|-------|
| Sparse trie | To do | | | | | | Only recompute paths for changed accounts |
| BAL (Balance Access List) | To do | | | | | | Slots accessed |
| Bloom filter revision | To do | | | Test nibbles instead of bloom | | | |
| Test nibbles instead of bloom filter | To do | | | Bloom filter revision | | | |
| State pruning | To do | | | | | | |
| State Prefetching | To do | | | | | | |
| Increase TrieLayerCache Commit Threshold | To do | | | | | | |

### Database

| Idea | Status | Improvements | Regressions | Conflicts | Last Tested | Experiment | Notes |
|------|--------|--------------|-------------|-----------|-------------|------------|-------|
| Implementing Ethrex-DB | To do | | | | | | Custom DB implementation |
| RocksDB tuning | To do | | | | | | |
| Multiget everywhere | To do | | | | | | |

### Merkelization

| Idea | Status | Improvements | Regressions | Conflicts | Last Tested | Experiment | Notes |
|------|--------|--------------|-------------|-----------|-------------|------------|-------|
| Parallel merkelization of storages | To do | | | | | | |

### Pipeline Architecture

| Idea | Status | Improvements | Regressions | Conflicts | Last Tested | Experiment | Notes |
|------|--------|--------------|-------------|-----------|-------------|------------|-------|
| Deeper pipeline | To do | | | | | | tx_stream -> prewarm -> multiproof (?) -> sparse_trie |

---

## How to Update This Document

After completing an experiment:

1. **Update the idea row:**
   - Set Status to "Done" or "Discarded"
   - Fill in Improvements with measured percentages
   - Fill in Regressions if any
   - Fill in Conflicts discovered during testing
   - Set Last Tested to the experiment date
   - Add link to Experiment log

2. **If Discarded:**
   - Move to the Discarded section
   - Fill in the Reason column explaining why

3. **Example update:**
   ```
   BEFORE: | Skip memory zero-init | Benches | | | | | | Testing |
   AFTER:  | Skip memory zero-init | Done | 6.5% | None | Buffer reuse | 2026-01-16 | [001](experiments/001-skip-zero-init/) | Merged PR #1234 |
   ```

4. **Update TRACKER.md** with the result
