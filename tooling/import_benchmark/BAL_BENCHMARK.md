# BAL Parallel Execution Benchmark

## Overview

Benchmark the BAL (Block Access List, EIP-7928) parallel execution validation path
and, secondarily, BAL construction performance.

The approach:
1. Run a kurtosis devnet with spamoor to produce blocks with diverse transaction patterns
2. Export the chain as RLP
3. Run a sequential bootstrap pass to generate BALs for each block
4. Benchmark the parallel validation path by replaying blocks with their BALs

```
┌─ ONE TIME ────────────────────────────────────────────────┐
│  Kurtosis devnet + spamoor → diverse blocks               │
│  ethrex export → chain.rlp                                │
├─ BOOTSTRAP (once per chain.rlp) ─────────────────────────┤
│  import-bench chain.rlp --export-bal bals.rlp             │
│    sequential execution, records BALs to single file      │
├─ BENCHMARK (repeatable, deterministic) ──────────────────┤
│  import-bench chain.rlp --with-bal bals.rlp  (parallel)   │
│  import-bench chain.rlp                      (sequential) │
│  Compare Ggas/s and phase breakdown between both runs     │
└───────────────────────────────────────────────────────────┘
```

## Prerequisites

- [Kurtosis](https://docs.kurtosis.com/install/#ii-install-the-cli)
- [Docker](https://docs.docker.com/engine/install/)
- ethrex built with Amsterdam support
- spamoor (bundled in the kurtosis ethereum-package)

## Phase 1: Generate the chain fixture

### 1.1 Start the devnet

Use the ethrex-only benchmark config (`fixtures/networks/bal-bench.yaml`).
This runs a single ethrex node as the supernode — it builds all blocks and
produces BALs, with no cross-client noise:

```bash
make localnet KURTOSIS_CONFIG_FILE=./fixtures/networks/bal-bench.yaml
```

This starts:
- ethrex (supernode) + lighthouse
- spamoor with three scenarios:
  - `eoatx` (throughput: 15) — simple transfers, balance changes only
  - `evm-fuzz` (throughput: 8) — random EVM opcodes, storage-heavy
  - `deploytx` (throughput: 1) — contract creation, code changes

> For multi-client devnets (e.g. with geth), use `bal-devnet-3.yaml` instead.

### 1.2 Let it run

Let the devnet run until you have enough blocks. For a meaningful benchmark:
- **Minimum**: ~100 blocks (quick smoke test)
- **Recommended**: ~1000 blocks (stable averages)
- **Ideal**: ~5000+ blocks (captures variance, cache effects, state growth)

At 6s slots, 1000 blocks ≈ 100 minutes.

Monitor progress via dora (the block explorer included in the devnet config)
or ethrex logs.

### 1.3 Export the chain

Find the ethrex container and current head block:

```bash
# Get the ethrex container ID
CID=$(docker ps -q --filter ancestor=ethrex:local | head -n1)

# Check current head via RPC
curl -s http://$(kurtosis port print lambdanet el-1-ethrex-lighthouse rpc) \
  -X POST -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}' | jq -r '.result'
```

Export blocks from inside the container:

```bash
docker exec $CID ethrex export --first 1 --last <HEAD> /tmp/chain.rlp
docker cp $CID:/tmp/chain.rlp ~/.local/share/ethrex_bal_bench/chain.rlp
```

### 1.4 Stop the devnet

```bash
make stop-localnet
```

### 1.5 Copy the genesis file

You'll need the devnet's genesis file for the import-bench passes. Extract it
from the kurtosis enclave **before stopping**:

```bash
kurtosis files download lambdanet genesis_file ~/bal-bench-genesis.json
```

## Phase 2: Bootstrap — generate BALs

This pass executes the chain sequentially and writes all BALs to a single file.

### 2.1 Prepare the state database

The import-bench needs a store with the genesis state:

```bash
mkdir -p ~/.local/share/ethrex_bal_bench
```

### 2.2 Run the bootstrap pass

```bash
cargo run --release -- \
  --network ~/bal-bench-genesis.json \
  --datadir ~/.local/share/ethrex_bal_bench \
  import-bench chain.rlp --export-bal bals.rlp
```

This will:
1. Execute each block sequentially (the normal `add_block_pipeline(block, None)` path)
2. Record the BAL produced during execution
3. Write all BALs as concatenated RLP to `bals.rlp` (same format as chain.rlp)

The BALs file is now your fixture for the parallel path.

### 2.3 Verify BAL integrity

Quick sanity check — the BAL hash should match what's in each block header:

```bash
# The import-bench should log any hash mismatches.
# If a BAL hash doesn't match the block header's block_access_list_hash,
# something is wrong with execution or recording.
```

## Phase 3: Benchmark

### 3.1 Parallel path (primary objective)

```bash
# Copy the base state (from after genesis init, before block 1)
cp -r ~/.local/share/ethrex_bal_bench/ethrex ~/.local/share/temp

cargo run --release -- \
  --network ~/bal-bench-genesis.json \
  --datadir ~/.local/share/temp \
  import-bench chain.rlp --with-bal bals.rlp
```

This loads all BALs into memory upfront (no per-block I/O overhead), then calls
`add_block_pipeline(block, Some(&bal))` which activates:
- `validate_header_bal_indices()`
- `build_validation_index()`
- `warm_block_from_bal()` (prefetch accounts, storage, codes)
- `execute_block_parallel()` (rayon-parallelized tx execution)
- BAL validation (reads, accounts, withdrawals)

The existing perf logs (`perf_logs_enabled: true`) will output per-block:
- Ggas/s throughput
- Phase breakdown: validate, exec, merkle (concurrent + drain), store, warmer
- Merkle overlap percentage
- Bottleneck identification

### 3.2 Sequential baseline (for comparison)

```bash
cp -r ~/.local/share/ethrex_bal_bench/ethrex ~/.local/share/temp

cargo run --release -- \
  --network ~/bal-bench-genesis.json \
  --datadir ~/.local/share/temp \
  import-bench chain.rlp
```

Same blocks, no BAL → sequential `execute_block()` path.

### 3.3 Multiple runs

Use the existing benchmark.sh pattern for multiple repetitions:

```bash
# Parallel (3 runs)
for i in 1 2 3; do
  cp -r ~/.local/share/ethrex_bal_bench/ethrex ~/.local/share/temp
  cargo run --release -- \
    --network ~/bal-bench-genesis.json \
    --datadir ~/.local/share/temp \
    import-bench chain.rlp --with-bal bals.rlp \
    2>&1 | tee bench_results/parallel-${i}.log
done

# Sequential (3 runs)
for i in 1 2 3; do
  cp -r ~/.local/share/ethrex_bal_bench/ethrex ~/.local/share/temp
  cargo run --release -- \
    --network ~/bal-bench-genesis.json \
    --datadir ~/.local/share/temp \
    import-bench chain.rlp \
    2>&1 | tee bench_results/sequential-${i}.log
done
```

### 3.4 Compare results

```bash
python3 tooling/import_benchmark/parse_bench.py \
  bench_results/parallel-*.log bench_results/sequential-*.log
```

> **NOTE**: `parse_bench.py` may need updates to parse the pipeline log format
> (7 instants) vs the old sequential format (3 instants). Consider extending it
> to report parallel-specific metrics like warmer time and merkle overlap.

## Metrics to Track

| Metric | Source | What it tells you |
|--------|--------|-------------------|
| Ggas/s | `[METRIC] BLOCK EXECUTION THROUGHPUT` | Overall throughput |
| Exec time (ms) | Pipeline phase breakdown | Raw execution time |
| Warmer time (ms) | `warmer_duration` | BAL prefetch cost |
| Merkle overlap % | Pipeline log | How well merkle overlaps with exec |
| Merkle drain (ms) | Pipeline log | Residual merkle after exec finishes |
| Parallel speedup | Ggas/s(parallel) / Ggas/s(sequential) | The headline number |
| Store time (ms) | Pipeline phase | DB write overhead |

## CLI Flags

### `--export-bal <FILE>`

During sequential execution, save all BALs to a single concatenated RLP file:

```
import-bench chain.rlp --export-bal bals.rlp
```

BALs are collected in memory during execution and written in one shot at the end.

### `--with-bal <FILE>`

Load pre-computed BALs and use the parallel execution path:

```
import-bench chain.rlp --with-bal bals.rlp
```

All BALs are loaded into memory upfront before block execution begins,
avoiding per-block I/O overhead during the benchmark.

### Both flags are mutually exclusive

`--export-bal` = sequential bootstrap pass.
`--with-bal` = parallel benchmark pass.

## Spamoor Scenario Tuning

The `bal-bench.yaml` config runs three spamoor scenarios. You can adjust these
to stress different aspects of the parallel pipeline:

| Scenario | BAL Impact | Parallel Impact |
|----------|-----------|-----------------|
| `eoatx` | Small BAL (balance-only) | Minimal contention, easy to parallelize |
| `evm-fuzz` | Large BAL (random storage) | Stresses storage prefetch, validation |
| `deploytx` | Code change entries | Tests code change recording/validation |
| `erctx` | Token transfers (SLOAD+SSTORE) | Shared contract = contention test |
| `uniswaptx` | DEX swaps (heavy storage) | Realistic DeFi, high contention |

To add more scenarios, edit the spamoor config in the yaml:

```yaml
spamoor_params:
  spammers:
  - scenario: eoatx
    config: {throughput: 15}
  - scenario: evm-fuzz
    config: {throughput: 8}
  - scenario: deploytx
    config: {throughput: 1}
  # Add for contention testing:
  - scenario: erctx
    config: {throughput: 10}
```

## Fixture Management

Once generated, the fixtures are self-contained:

```
~/.local/share/ethrex_bal_bench/
├── chain.rlp              # RLP-encoded blocks (concatenated)
├── bals.rlp               # RLP-encoded BALs (concatenated, 1:1 with blocks)
├── ethrex/                # RocksDB state at genesis (base for each run)
└── genesis.json           # Network genesis
```

These can be:
- Shared across team members (deterministic results on same hardware)
- Versioned (generate new fixtures when BAL format or spamoor config changes)
- Stored in CI for regression testing

## Troubleshooting

### Blocks are empty / low gas usage
Spamoor may need time to ramp up. Skip the first ~20 blocks or increase
`genesis_delay` in the devnet config.

### BAL hash mismatch during parallel benchmark
The BAL was generated with a different execution than what the block expects.
Regenerate BALs from the same chain.rlp.

### RocksDB lock errors
Make sure you're copying the base state to a temp dir before each run,
not reusing the same datadir.

### 500ms sleep in import-bench
The current code has a `tokio::time::sleep(Duration::from_millis(500))` between
blocks to wait for background DB writes. For benchmarking, this adds ~500ms
overhead per block. Consider:
- Reducing it for faster iteration
- Excluding it from timing measurements
- Or removing it and handling the DB flush synchronously
