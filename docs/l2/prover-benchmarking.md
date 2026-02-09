# Prover Benchmarking Guide

How to measure proving performance on a running L2 localnet.

## Prerequisites

- ethrex repository cloned and buildable
- Docker (for L1 and metrics containers)
- `curl` and `bash` (for the benchmark script)

## Quick Start

### 1. Start the L2 Localnet

From the `crates/l2/` directory:

```bash
# Terminal 1 — L1 + deploy contracts + metrics + L2 sequencer
make init
```

This starts the L1 (Docker), deploys contracts, starts Prometheus/Grafana, and runs the L2 sequencer with metrics on port 3702.

### 2. Start the Prover

```bash
# Terminal 2 — SP1 prover (default: --timed enabled)
make init-prover-sp1
```

The prover connects to the proof coordinator and begins polling for batches. By default, the `--timed` flag is enabled, which wraps each `prove()` call with timing instrumentation and logs structured fields:

```
batch=3 proving_time_s=47 proving_time_ms=47123 Proved batch 3 in 47.12s
```

To disable timing (for production or to reduce overhead):

```bash
cargo run --release --features "l2,l2-sql,sp1" --manifest-path ../../Cargo.toml -- \
    l2 prover --proof-coordinators tcp://127.0.0.1:3900 --backend sp1 --no-timed
```

### 3. Generate Transactions

```bash
# Terminal 3 — load test
cd tooling/load_test
cargo run -- -k ../../crates/l2/test_data/private_keys.txt -N 500
```

This sends 500 transactions per account using the test private keys. The sequencer will include them in blocks and batch them for proving.

**Env var alternative** (useful for scripts or CI):

```bash
export LOAD_TEST_RPC_URL=http://localhost:1729
export LOAD_TEST_TX_AMOUNT=500
export LOAD_TEST_ENDLESS=true
cargo run -- -k ../../crates/l2/test_data/private_keys.txt
```

| Env Var | CLI Flag | Default |
|---------|----------|---------|
| `LOAD_TEST_RPC_URL` | `--node` / `-n` | `http://localhost:8545` |
| `LOAD_TEST_TX_AMOUNT` | `--tx-amount` / `-N` | `1000` |
| `LOAD_TEST_ENDLESS` | `--endless` | `false` |

The `--endless` flag restarts the load test after each round, useful for continuous benchmarking sessions.

### 4. Collect Results

Once batches have been proved, redirect the prover output to a file (or use `tee`):

```bash
# If you started the prover with output redirection:
make init-prover-sp1 2>&1 | tee prover.log
```

Then run the benchmark script:

```bash
./scripts/sp1_bench_metrics.sh prover.log
```

**Example output:**

```
===== SP1 Proving Benchmark Results =====

Batch       Time (s)      Time (ms)       Gas Used   Tx Count   Blocks
-----       --------      ---------       --------   --------   ------
1              47          47123          1234567        50        3
2              52          52456          2345678        75        4
3              39          39012           987654        30        2

Batches: 3 | Avg: 46s (46197ms) | Min: 39s | Max: 52s
Total gas: 4567899 | Total txs: 155

Results written to sp1_bench_results.csv
```

The script fetches batch metadata (gas used, tx count, block count) from the Prometheus metrics endpoint at `localhost:3702/metrics`. Pass a custom URL as the second argument if your metrics are elsewhere:

```bash
./scripts/sp1_bench_metrics.sh prover.log http://myserver:3702/metrics
```

## Reference

### Prover `--timed` Flag

| Flag | Env Var | Default | Effect |
|------|---------|---------|--------|
| `--timed` | `PROVER_CLIENT_TIMED` | `true` | Measure and log proving time per batch |
| `--no-timed` | — | — | Skip timing measurement |

When timed, each batch proof logs structured fields (`proving_time_s`, `proving_time_ms`) that the benchmark script parses.

### Metrics

The L2 sequencer exposes Prometheus metrics on port 3702 (configurable with `--metrics.port`). Relevant metrics for benchmarking:

| Metric | Description |
|--------|-------------|
| `batch_gas_used{batch_number="N"}` | Total gas used in batch N |
| `batch_tx_count{batch_number="N"}` | Transaction count in batch N |
| `batch_size{batch_number="N"}` | Number of blocks in batch N |

### Benchmark Script

```
./scripts/sp1_bench_metrics.sh <PROVER_LOG_FILE> [METRICS_URL]
```

Outputs:
- A summary table to stdout
- A CSV file (`sp1_bench_results.csv`) with columns: `batch,proving_time_s,proving_time_ms,gas_used,tx_count,blocks`
