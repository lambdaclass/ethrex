# Prover Benchmarking Guide

How to measure proving performance on a running L2 localnet.

## Quick Start

### 1. Start the L2 Localnet

From the `crates/l2/` directory, start the localnet in three stages:

```bash
# Terminal 1 — Start L1 (Docker). Only needed once; skip if already running.
make init-l1-docker

# Deploy contracts. Use ETHREX_DEPLOYER_RANDOMIZE_CONTRACT_DEPLOYMENT to avoid
# address collisions with previous deployments on the same L1.
# Add the verifier flag for your backend (--sp1 true, --risc0 true, or omit for exec):
ETHREX_DEPLOYER_RANDOMIZE_CONTRACT_DEPLOYMENT=true make deploy-l1

# Start the L2 sequencer. Use ETHREX_NO_MONITOR=true to disable the TUI monitor
# and get plain log output (useful for log files and remote runs).
ETHREX_NO_MONITOR=true make init-l2
```

This starts the L1 (Docker), deploys contracts, and runs the L2 sequencer.

### 2. Start the Prover

```bash
# Terminal 2 — Start prover with timing enabled (replace <backend> with sp1, risc0, or exec)
PROVER_CLIENT_TIMED=true make init-prover-<backend>

# With GPU acceleration:
# PROVER_CLIENT_TIMED=true make init-prover-<backend> GPU=true
```

Available Makefile targets: `init-prover-sp1`, `init-prover-risc0`, `init-prover-exec`.

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
| `LOAD_TEST_RPC_URL` | `http://localhost:8545` | RPC URL of the node. **For L2 benchmarking, set to `http://localhost:1729`** (the L2 RPC port). |
| `LOAD_TEST_TX_AMOUNT` | `1000` | Transactions per account |
| `LOAD_TEST_ENDLESS` | `false` | Restart load test after each round |

### 4. Collect Results

Once batches have been proved, redirect the prover output to a file (or use `tee`):

```bash
# If you started the prover with output redirection:
PROVER_CLIENT_TIMED=true make init-prover-<backend> 2>&1 | tee prover.log
```

Then run the benchmark script. The L2 must still be running, since the script fetches batch metadata from its metrics endpoint (`localhost:3702/metrics`):

```bash
./scripts/bench_metrics.sh prover.log
```

The script outputs a markdown file (`bench_results.md`) with a results table and summary, and prints it to stdout.

The script fetches batch metadata (gas used, tx count, block count) from the L2 metrics endpoint at `localhost:3702/metrics` (exposed by `make init-l2` via `--metrics.port`). Pass a custom URL as the second argument if the L2 is on a different host or port:

```bash
./scripts/bench_metrics.sh prover.log http://myhost:3702/metrics
```
