# Performance Optimization

This directory is the source of truth for ethrex performance optimization work.

## Quick Links

| Document | Purpose |
|----------|---------|
| [methodology.md](methodology.md) | Complete benchmarking workflow |
| [ideas.md](ideas.md) | All performance ideas with status tracking |
| [architecture.md](architecture.md) | Code architecture and bottleneck analysis |
| [tracking.md](tracking.md) | Session and historical tracking formats |
| [observability.md](observability.md) | Dashboard and notification setup |
| [zkvm-guidelines.md](zkvm-guidelines.md) | zkVM-specific benchmarking guidance |

## Directory Structure

```
docs/perf/
├── README.md                    # This file
├── methodology.md               # Full benchmarking workflow
├── ideas.md                     # Ideas tracking with status
├── architecture.md              # Code architecture notes
├── tracking.md                  # Tracking formats
├── observability.md             # Dashboard and notifications
├── zkvm-guidelines.md           # zkVM-specific guidance
├── templates/
│   ├── experiment.md            # Experiment log template
│   ├── environment.yaml         # Environment documentation template
│   ├── inputs.yaml              # Input diversity template
│   └── plan.md                  # Experiment plan template
├── scripts/
│   └── generate_report.py       # HTML report generator
└── experiments/                 # Experiment logs
```

---

## Current Priorities

Based on architecture analysis, these are the highest-impact opportunities:

### Tier 1: High Impact, Low Effort

| Idea | Why | Key Location |
|------|-----|--------------|
| **Skip memory zero-init** | Memory expansion is in hot path | `memory.rs:96` |
| **FxHashSet for access lists** | Proven pattern, checked every SLOAD/SSTORE | `substate.rs` |
| **Inline Hot Opcodes** | 20-40% gains in benchmarks | `vm.rs:554-629` |
| **SSTORE double lookup** | Two hashmap lookups per SSTORE | `gen_db.rs:514` |

### Tier 2: High Impact, Medium Effort

| Idea | Why | Key Location |
|------|-----|--------------|
| **Nibbles stack allocation** | Vec allocations on every trie path | `nibbles.rs:23-28` |
| **Trie cache RwLock** | Current Mutex has contention | `store.rs:2328` |
| **Transaction pre-warming** | Populate caches before execution | Pipeline |
| **Parallel merkelization tuning** | Currently 16 fixed workers | `blockchain.rs:432-593` |

### Tier 3: High Impact, High Effort

| Idea | Why | Key Location |
|------|-----|--------------|
| **Parallel Transaction Execution** | Requires dependency analysis | Full pipeline |
| **PGO** | 10-20% typical gains | Build system |

See [ideas.md](ideas.md) for the complete list with status tracking.

---

## Quick Start

### 1. Pick an Idea

Start with Tier 1 priorities or check [ideas.md](ideas.md) for the full list.

### 2. Read the Workflow

Follow [methodology.md](methodology.md) for the complete process:
- Critical rules (never violate these)
- Knowledge gathering
- Baseline establishment
- Experiment execution
- Decision criteria
- Tracking updates

### 3. Use Templates

Copy templates from `templates/` for your experiment:
- `experiment.md` - Full experiment log
- `environment.yaml` - Machine/toolchain documentation
- `inputs.yaml` - Input diversity documentation
- `plan.md` - Experiment planning

### 4. Run Benchmarks

```bash
# Quick synthetic benchmark
cargo run -p ethrex-benches --bin perf_bench --release

# With hyperfine for statistical rigor
hyperfine --warmup 3 --runs 10 'cargo run -p ethrex-benches --bin perf_bench --release'

# Profiling
samply record --save-only target/release/perf_bench
```

### 5. Update Tracking

After each experiment:
- Update [ideas.md](ideas.md) with results
- Update TRACKER.md in experiments/
- Rename branch (`-KEEP` or `-DISCARD`)

---

## Key Metrics

| Metric | Description | How to Measure |
|--------|-------------|----------------|
| **Mgas/s** | Gas processed per second | Primary throughput metric |
| **Block time** | Time to execute + merkleize | perf_bench output |
| **CV** | Coefficient of variation | Should be <10% for valid benchmarks |

### Baseline (Synthetic Benchmark)

ETH transfers, 100 blocks, ~404 txs/block:
- Mean block time: ~12.68 ms
- Throughput: ~669 Mgas/s

---

## Architecture Summary

**LEVM** (`crates/vm/levm/`)
- Hybrid opcode dispatch: 64 hot opcodes in direct match
- Stack: Fixed 1024-element array with pool reuse
- Memory: `Rc<RefCell<Vec<u8>>>` shared across call frames
- State: Multi-tier FxHashMap caching

**Trie** (`crates/common/trie/`, `crates/storage/`)
- Nibbles use Vec<u8> (allocation bottleneck)
- NodeRef uses OnceLock for hash memoization
- Parallel merkleization sharded by first nibble

See [architecture.md](architecture.md) for detailed analysis.

---

## Critical Rules

From [methodology.md](methodology.md):

1. **Only run parallel benchmarks when they won't affect results**
2. **Never skip baseline**
3. **Never modify code during a benchmark run**
4. **Always record before deciding**
5. **Always test correctness after optimization**
6. **Always use multiple inputs**
