# Prover Benchmarking Guide

How to measure proving performance on a running L2 localnet.

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
# Terminal 2 — SP1 prover with timing enabled
PROVER_CLIENT_TIMED=true make init-prover-sp1

# With GPU acceleration:
# PROVER_CLIENT_TIMED=true make init-prover-sp1 GPU=true
```

The prover connects to the proof coordinator and begins polling for batches. The `PROVER_CLIENT_TIMED` env var wraps each `prove()` call with timing instrumentation and logs structured fields:

```
batch=3 proving_time_s=47 proving_time_ms=47123 Proved batch 3 in 47.12s
```

### 3. Generate Transactions

```bash
# Terminal 3 — load test (from repo root)
make load-test

# Customize with env vars:
# LOAD_TEST_TX_AMOUNT=50 LOAD_TEST_ENDLESS=true LOAD_TEST_RPC_URL=http://localhost:1729 make load-test
```

This sends transactions per account using the test private keys. The sequencer will include them in blocks and batch them for proving.

> **Warning:** Keep the transaction count below the node's mempool limit. If the mempool fills up, the node will evict the oldest transactions, which removes the lowest nonces and causes all subsequent transactions from that account to become stuck (nonce gap).

| Env Var | Default | Description |
|---------|---------|-------------|
| `LOAD_TEST_RPC_URL` | `http://localhost:8545` | RPC URL of the node |
| `LOAD_TEST_TX_AMOUNT` | `1000` | Transactions per account |
| `LOAD_TEST_ENDLESS` | `false` | Restart load test after each round |

### 4. Collect Results

Once batches have been proved, redirect the prover output to a file (or use `tee`):

```bash
# If you started the prover with output redirection:
make init-prover-sp1 2>&1 | tee prover.log
```

Then run the benchmark script. The L2 must still be running, since the script fetches batch metadata from its Prometheus metrics endpoint:

```bash
./scripts/sp1_bench_metrics.sh prover.log
```

The script outputs a markdown file (`sp1_bench_results.md`) with a results table and summary, and prints it to stdout.

The script fetches batch metadata (gas used, tx count, block count) from the Prometheus metrics endpoint at `localhost:3702/metrics`. Pass a custom URL as the second argument if your metrics are elsewhere:

```bash
./scripts/sp1_bench_metrics.sh prover.log http://localhost:3702/metrics
```
