# Performance Optimization Guide

This directory contains documentation for systematic performance optimization of ethrex.

## Current Priorities

Based on architecture analysis, these are the highest-impact opportunities ranked by expected gain and implementation effort:

### Tier 1: High Impact, Low Effort
| Idea | Why | Key Location |
|------|-----|--------------|
| **Skip memory zero-init** | Memory expansion is in hot path, `resize(n, 0)` can be avoided with careful tracking | `memory.rs:96` |
| **FxHashSet for access lists** | Proven pattern elsewhere in codebase, access lists checked every SLOAD/SSTORE | `substate.rs` |
| **Inline Hot Opcodes** | Already showing 20-40% gains in benchmarks, needs server validation | `vm.rs:554-629` |
| **SSTORE double lookup** | Two hashmap lookups per SSTORE, can be reduced to one | `gen_db.rs:514` |

### Tier 2: High Impact, Medium Effort
| Idea | Why | Key Location |
|------|-----|--------------|
| **Nibbles stack allocation** | Vec allocations on every trie path operation, very hot path | `nibbles.rs:23-28` |
| **Trie cache RwLock** | Current Mutex has contention on read-heavy workload | `store.rs:2328` |
| **Transaction pre-warming** | Populate caches before execution, enables parallelism | Pipeline architecture |
| **Parallel merkelization tuning** | Currently 16 fixed workers, could be dynamic | `blockchain.rs:432-593` |

### Tier 3: High Impact, High Effort
| Idea | Why | Key Location |
|------|-----|--------------|
| **Parallel Transaction Execution** | Requires dependency analysis, complex but large gains possible | Full pipeline |
| **PGO (Profile-Guided Optimization)** | 10-20% typical gains, requires build pipeline changes | Build system |

## Quick Architecture Summary

**LEVM** (`crates/vm/levm/`)
- Hybrid opcode dispatch: 64 hot opcodes in direct match, rest via function pointer table
- Stack: Fixed 1024-element array, unsafe pointer ops, pool reuse across txs
- Memory: `Rc<RefCell<Vec<u8>>>` shared across call frames
- State: Multi-tier FxHashMap caching with backup system for reversion

**Trie** (`crates/common/trie/`, `crates/storage/`)
- Nibbles use Vec<u8> (allocation bottleneck, TODO exists for stack allocation)
- NodeRef uses OnceLock for hash memoization (lost on clone)
- TrieLayerCache with bloom filter, 128 layer commit threshold on-disk
- Parallel merkleization sharded by first nibble (16 workers)

**Block Pipeline** (`crates/blockchain/`)
- Sequential transaction execution within blocks (state dependencies)
- Parallel merkleization across account shards
- Key lock: `trie_cache.lock()` on every state/storage trie open

## How to Benchmark

<!-- TODO: To be completed by team member with mainnet replay setup -->

### Mainnet Block Replay

```bash
# TODO: Add instructions for replaying mainnet blocks
# Include:
# - How to obtain block data
# - Command to run replay benchmark
# - Expected baseline numbers
# - How to compare results
```

### Quick Synthetic Benchmark

```bash
cargo run -p ethrex-benches --bin perf_bench --release
```

Baseline (ETH transfers, 100 blocks, ~404 txs/block):
- Mean block time: ~12.68 ms
- Throughput: ~669 Mgas/s

### Profiling

```bash
# CPU profiling with samply
samply record --save-only target/release/perf_bench

# View profile
samply load profile_*.json
```

## Files

| File | Purpose | Update Frequency |
|------|---------|------------------|
| [ideas.md](ideas.md) | Full idea list with status tracking | Every experiment |
| [architecture.md](architecture.md) | Detailed code notes and bottleneck analysis | When code changes |
| [experiments/](experiments/) | Detailed experiment logs | Per major experiment |

## Workflow for New Experiments

1. **Pick an idea** from Tier 1 priorities above (or ideas.md for full list)
2. **Read relevant section** in architecture.md for code context
3. **Establish baseline** using mainnet replay benchmark
4. **Implement change** on a branch
5. **Measure improvement** with same benchmark
6. **Update ideas.md** with results (improvements, regressions, notes)
7. **Document details** in experiments/ if methodology is complex

## Key Metrics

- **Mgas/s**: Primary throughput metric (gas processed per second)
- **Block time**: Time to execute + merkleize a block
- **Breakdown**: Execution time vs merkleization time vs store time
