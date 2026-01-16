# Performance Improvement Ideas

This document tracks performance optimization ideas for ethrex.

## Status Legend

- **To do**: Not started
- **In progress**: Currently being worked on
- **Review**: Code ready for review
- **Benches**: Benchmarking in progress
- **Discarded**: Idea rejected after evaluation
- **Done**: Completed and merged

---

## Execution Optimizations (LEVM)

### Low Difficulty

| Idea | Status | Improvements | Regressions | Notes |
|------|--------|--------------|-------------|-------|
| Nibbles 1-byte 2-nibble representation | In progress | | | |
| Nibble fixed storage | To do | | | |
| Use FxHashSet for access lists | In progress | | | |
| Skip memory zero-init | In progress | | | |
| Replace BTreeMap/BTreeSet with FxHashMap/FxHashSet | Benches | | | |
| Remove RefCell from Memory | In progress | | | |
| Inline Hot Opcodes | Review | Memory access, push, dup: 20-40% | NUMBER, BLOBBASEFEE, initcode jumpdest analysis 15-25% | Analyze jumpdest noise, then server |
| Avoid Clone on Account Load | Review | RETURN/REVERT 1KiB: ~1.33-1.45x, LOG*: ~1.18-1.19x, MSIZE: ~1.18x | block_full_of_ether_transfers: ~1.19-1.30x slower | |
| SSTORE double lookup | In progress | | | Cache used, no double DB lookup but two hashmap lookups. Can improve cache for both original/current with tradeoff |
| Hook Cloning Per Opcode | In progress | | | Runs once per tx, clone individual RCs instead of vector |
| keccak caching | To do | | | LRU cache for top 10k hashes, especially contract constants |
| Buffer reuse | To do | | | Free-list pattern with vec of buffers instead of allocating |

### Medium Difficulty

| Idea | Status | Improvements | Regressions | Notes |
|------|--------|--------------|-------------|-------|
| Object Pooling (reuse EVM stack frames, memory buffers) | To do | | | |
| SIMD everywhere | To do | | | |
| Stackalloc for Small Buffers | To do | | | |
| Use Arena Allocator for Substate Backups | Review | CHAINID, SDIV, Swap: 15-20% | CODECOPY, CALLDATACOPY 25-30% | Analyze jumpdest noise, then server |
| Arkworks EC Pairing | Review | bn128 2x, ec pairing 1.6-2x | initcode jumpdest analysis 20%, big memory access 14% | Analyze jumpdest noise, then server |
| Jumptable vs Calltable | In progress | | | Confirmed we have a jump table |
| Mempool Lock Contention | To do | | | mempool pruning O(n^2), parking_lot::RWLock, DashMap for lock-free-per-sender, batch pruning |
| Precompile caching | To do | | | Per-address LRU cache with spec validation. Useful for ec recover |
| Cross-block cache reuse | To do | | | Saved cache pattern with usage guards between blocks |
| Hierarchical storage cache | To do | | | Code cache, storage cache, account cache |
| Parallel proof workers with atomic availability | To do | | | Replace 16-fixed with dynamic |

### High Difficulty

| Idea | Status | Improvements | Regressions | Notes |
|------|--------|--------------|-------------|-------|
| LEVM simplify stack and results | To do | | | |
| Parallel Transaction Execution | To do | | | |
| PGO (Profile-Guided Optimization) | To do | | | |
| ruint crate | To do | | | Brute approach doesn't work, evaluate for specific spots |

### Discarded

| Idea | Status | Notes |
|------|--------|-------|
| Test PEVM | Discarded | Quick test on server showed 10% worse on full block |
| Jumpdest with stored bitmaps | Discarded | Seemed to worsen, tested multiple times |
| RLP Double Encoding for Length | Discarded | Already implemented length for all types |

---

## I/O and Storage Optimizations

### Caching

| Idea | Status | Improvements | Regressions | Notes |
|------|--------|--------------|-------------|-------|
| Transaction pre-warming | To do | | | Pre-execute in parallel to populate caches |
| LRU cache for states and accounts | To do | | | |
| Increase CodeCache Size | To do | | | |
| Cache trie top level (~2 levels) | To do | | | |
| Disklayer memory accumulator | To do | | | |

### Trie / State

| Idea | Status | Improvements | Regressions | Notes |
|------|--------|--------------|-------------|-------|
| Sparse trie | To do | | | Only recompute paths for changed accounts (probably already done) |
| BAL (Balance Access List) | To do | | | Slots accessed |
| Bloom filter revision | To do | | | |
| Test nibbles instead of bloom filter | To do | | | |
| State pruning | To do | | | |
| State Prefetching | To do | | | |
| Increase TrieLayerCache Commit Threshold | To do | | | |

### Database

| Idea | Status | Improvements | Regressions | Notes |
|------|--------|--------------|-------------|-------|
| Implementing Ethrex-DB | To do | | | Custom DB implementation |
| RocksDB tuning | To do | | | |
| Multiget everywhere | To do | | | |

### Merkelization

| Idea | Status | Improvements | Regressions | Notes |
|------|--------|--------------|-------------|-------|
| Parallel merkelization of storages | To do | | | |

### Pipeline Architecture

| Idea | Status | Improvements | Regressions | Notes |
|------|--------|--------------|-------------|-------|
| Deeper pipeline | To do | | | tx_stream -> prewarm -> multiproof (?) -> sparse_trie (merkelization) |
