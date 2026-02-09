# Prover Benchmarking Workflow

## Overview

This workflow measures proving time for the ethrex L2 prover on a remote server. The agent deploys a localnet (L1 + L2 + prover), generates load, waits for batches to be proved, and collects structured timing results.

For details on each tool see [Prover Benchmarking Guide](../l2/prover-benchmarking.md).

## Agent Setup Instructions

**Before starting any benchmark session, the agent MUST prompt the user for:**

1. **Server hostname** — SSH alias or full hostname where the benchmark will run
2. **Prover backend** — `sp1`, `risc0`, or `exec` (default: `sp1`)
3. **GPU enabled?** — Whether to build with `GPU=true` (default: yes for sp1)
4. **Transaction count** — Number of transactions per account to generate (default: 50)
5. **Batches to prove** — How many batches to wait for before collecting results
6. **Endless mode?** — Whether to run the load test continuously (default: no)
7. **Branch/commit** — Git ref to benchmark (default: current branch on server)
8. **Server already running?** — Skip setup steps if the L2 is already running

Example prompt:
```
Before starting the prover benchmark, I need the following information:
- Server hostname (SSH alias or full hostname):
- Prover backend (sp1/risc0/exec):
- GPU enabled? (yes/no):
- Transaction count per account:
- Number of batches to prove:
- Run load test in endless mode? (yes/no):
- Branch or commit to benchmark:
- Is the L2 already running on this server? (yes/no)
```

## Prerequisites

- SSH access to the benchmark server
- Docker installed on the server (for L1 and metrics containers)
- ethrex repository cloned at `~/ethrex`
- For SP1 with GPU: CUDA toolkit and SP1 GPU dependencies installed
- For RISC0: `risc0` toolchain installed

## Workflow

### 1. Connect and Prepare

```bash
ssh <server>
cd ~/ethrex
git fetch origin && git checkout <branch>
```

### 2. Start the Localnet

All commands from `crates/l2/`:

```bash
cd crates/l2

# Terminal 1 (tmux session "l2") — L1 + contracts + metrics + L2 sequencer
tmux new -s l2
make init
# Detach: Ctrl+B, D
```

Wait for the L2 to start (look for "Started L2 node" or similar in logs).

### 3. Start the Prover

```bash
# Terminal 2 (tmux session "prover") — redirect output to file for later parsing
tmux new -s prover
PROVER_CLIENT_TIMED=true make init-prover-sp1 2>&1 | tee ~/prover.log
# For GPU: PROVER_CLIENT_TIMED=true make init-prover-sp1 GPU=true 2>&1 | tee ~/prover.log
# Detach: Ctrl+B, D
```

The `PROVER_CLIENT_TIMED` env var enables structured proving time logs that the benchmark script parses.

### 4. Generate Load

```bash
# Terminal 3 (tmux session "loadtest")
tmux new -s loadtest
cd ~/ethrex
LOAD_TEST_RPC_URL=http://localhost:1729 LOAD_TEST_TX_AMOUNT=<tx_amount> make load-test
# For continuous load:
# LOAD_TEST_RPC_URL=http://localhost:1729 LOAD_TEST_TX_AMOUNT=<tx_amount> LOAD_TEST_ENDLESS=true make load-test
# Detach: Ctrl+B, D
```

> **Important:** `LOAD_TEST_RPC_URL` must point to the L2 node RPC (port 1729 by default), not the L1.

### 5. Wait for Batches

Monitor the prover log for proving completion:

```bash
ssh <server> "tail -f ~/prover.log | grep --line-buffered proving_time"
```

Wait until the desired number of batches have been proved.

### 6. Collect Results

```bash
ssh <server> "cd ~/ethrex && ./scripts/sp1_bench_metrics.sh ~/prover.log"
```

This outputs a markdown file (`sp1_bench_results.md`) with a results table and summary. Copy it locally if needed:

```bash
scp <server>:~/ethrex/sp1_bench_results.md .
```

### 7. Teardown

```bash
ssh <server>
cd ~/ethrex/crates/l2
make down          # stops L1, L2, metrics
tmux kill-session -t prover
tmux kill-session -t loadtest
```

## Troubleshooting

| Issue | Solution |
|-------|----------|
| Prover log shows no `proving_time` lines | Ensure prover was started with `--timed`. Check `tmux attach -t prover` for errors. |
| Benchmark script shows "(no batches found)" | Prover hasn't finished proving any batch yet. Wait longer or check prover logs for errors. |
| Metrics show `-` for gas/tx/blocks | The Prometheus endpoint may be unreachable. Verify metrics are running: `curl localhost:3702/metrics` |
| Load test can't connect | Verify L2 RPC is on the expected port (default 1729). Check with `curl http://localhost:1729` |
| Load test nonce errors | The load test uses pending nonces, so consecutive runs should work. If stuck, restart the localnet. |
| GPU out of memory | Reduce batch size or check if another process is using the GPU: `nvidia-smi` |

## Results Template

### Test Session Info

| Field | Value |
|-------|-------|
| **Date** | YYYY-MM-DD |
| **Server** | |
| **Backend** | sp1 / risc0 / exec |
| **GPU** | yes / no |
| **Branch/Commit** | |
| **Tx per Account** | |
| **Endless Mode** | yes / no |

### Proving Results

| Batch | Time (s) | Time (ms) | Gas Used | Tx Count | Blocks |
|-------|----------|-----------|----------|----------|--------|
| | | | | | |

### Summary

| Metric | Value |
|--------|-------|
| **Batches Proved** | |
| **Avg Proving Time** | |
| **Min Proving Time** | |
| **Max Proving Time** | |
| **Total Gas** | |
| **Total Txs** | |

### Observations

(Notes about anomalies, GPU temperature, errors, or interesting findings)
