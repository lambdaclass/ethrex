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
9. **Transaction type** — `eth-transfers`, `erc20`, `fibonacci`, or `io-heavy` (default: `eth-transfers`)

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
- Transaction type (eth-transfers/erc20/fibonacci/io-heavy):
```

## Prerequisites

- SSH access to the benchmark server
- Docker installed on the server (for the L1 container)
- ethrex repository cloned at `~/ethrex`
- For SP1 with GPU: CUDA toolkit and SP1 GPU dependencies installed
- For RISC0: `risc0` toolchain installed

## Workflow

All remote commands MUST be run via `ssh <server> "bash -l -c '...'"` to ensure the full shell environment is loaded (PATH, cargo, solc, etc.). Never use tmux — it creates sessions with a minimal environment that is missing critical tools.

### 0. Clean Up Previous Runs

Before starting, check for and clean up any running instances, old databases, and docker containers from previous runs:

```bash
# Check for running ethrex processes
ssh <server> "bash -l -c 'pgrep -a -f ethrex || echo No ethrex processes'"

# Check for running docker containers
ssh <server> "bash -l -c 'docker ps --format \"{{.Names}} {{.Status}}\"'"

# Check for old databases
ssh <server> "bash -l -c 'ls -la ~/ethrex/crates/l2/dev_l2_db* 2>/dev/null; ls -la ~/ethrex/crates/l2/l1_db* 2>/dev/null'"

# Check for docker volumes
ssh <server> "bash -l -c 'docker volume ls | grep -i ethrex'"

# If anything is found, stop it all:
ssh <server> "bash -l -c 'cd ~/ethrex/crates/l2 && make down'"
```

### 1. Connect and Prepare

```bash
ssh <server> "bash -l -c 'cd ~/ethrex && git fetch origin && git checkout <branch> && git pull origin <branch>'"
```

### 1b. Pre-compile Load Test Binary

Build the load test binary **before** starting the L2. This avoids the sequencer producing empty blocks while the load test compiles.

```bash
ssh <server> "bash -l -c 'cd ~/ethrex && cargo build --release --manifest-path ./tooling/load_test/Cargo.toml'"
```

### 2. Start the Localnet

The localnet is started in three stages so that the L1 (Docker) stays up across re-deploys and L2 restarts.

#### 2a. Start the L1

```bash
ssh <server> "bash -l -c 'cd ~/ethrex/crates/l2 && make init-l1-docker'"
```

This only needs to run once. If the L1 container is already running, skip this step.

#### 2b. Deploy L1 Contracts

```bash
# Default deployment (exec prover only):
ssh <server> "bash -l -c 'cd ~/ethrex/crates/l2 && env ETHREX_DEPLOYER_RANDOMIZE_CONTRACT_DEPLOYMENT=true make deploy-l1'"

# For SP1 benchmarks (deploys the SP1 verifier contract):
ssh <server> "bash -l -c 'cd ~/ethrex/crates/l2 && env ETHREX_DEPLOYER_RANDOMIZE_CONTRACT_DEPLOYMENT=true make deploy-l1-sp1'"
```

The `ETHREX_DEPLOYER_RANDOMIZE_CONTRACT_DEPLOYMENT` env var randomizes the CREATE2 salt so that contracts are deployed to fresh addresses on every run. This avoids collisions with previous deployments on the same L1.

Use `deploy-l1-sp1` when benchmarking with the SP1 backend so that the SP1 on-chain verifier is deployed and the proof coordinator requires SP1 proofs.

#### 2c. Start the L2

```bash
ssh <server> "bash -l -c 'cd ~/ethrex/crates/l2 && nohup env ETHREX_NO_MONITOR=true make init-l2 > ~/l2.log 2>&1 &'"
```

`ETHREX_NO_MONITOR=true` disables the TUI monitor so that logs are written to stdout (and captured in the log file).

Wait for the L2 to start:

```bash
ssh <server> "bash -l -c 'tail -f ~/l2.log'" | grep --line-buffered -m1 'Blockchain configured'
```

### 3. Generate Load

Start the load test **immediately after the L2 is up** and **before** the prover to ensure batches contain transactions. The block producer and committer are fast — they will build and commit empty blocks and batches while the load test is not yet sending transactions. The prover then proves those empty batches instead of full ones.

Run the pre-compiled binary directly (built in step 1b) instead of `cargo run` to avoid compilation delays after the L2 is already producing blocks:

```bash
# ETH transfers (default):
ssh <server> "bash -l -c 'cd ~/ethrex && nohup env LOAD_TEST_RPC_URL=http://localhost:1729 LOAD_TEST_TX_AMOUNT=<tx_amount> ./tooling/target/release/load_test -k ./fixtures/keys/private_keys.txt -t eth-transfers > ~/loadtest.log 2>&1 &'"

# ERC20 transactions:
ssh <server> "bash -l -c 'cd ~/ethrex && nohup env LOAD_TEST_RPC_URL=http://localhost:1729 LOAD_TEST_TX_AMOUNT=<tx_amount> ./tooling/target/release/load_test -k ./fixtures/keys/private_keys.txt -t erc20 > ~/loadtest.log 2>&1 &'"

# For continuous load, add LOAD_TEST_ENDLESS=true to env vars.
```

> **Important:** `LOAD_TEST_RPC_URL` must point to the L2 node RPC (port 1729 by default), not the L1.

Available load test targets:
| Target | Description |
|--------|-------------|
| `load-test` | ETH transfers |
| `load-test-erc20` | ERC20 token transfers |
| `load-test-fibonacci` | Fibonacci computation transactions |
| `load-test-io` | IO-heavy transactions |

### 4. Start the Prover

```bash
# SP1 without GPU:
ssh <server> "bash -l -c 'cd ~/ethrex/crates/l2 && nohup env PROVER_CLIENT_TIMED=true make init-prover-sp1 > ~/prover.log 2>&1 &'"

# SP1 with GPU:
ssh <server> "bash -l -c 'cd ~/ethrex/crates/l2 && nohup env PROVER_CLIENT_TIMED=true GPU=true make init-prover-sp1 > ~/prover.log 2>&1 &'"
```

The `PROVER_CLIENT_TIMED` env var enables structured proving time logs that the benchmark script parses.

### 5. Wait for Batches

Monitor the prover log for proving completion:

```bash
ssh <server> "bash -l -c 'tail -f ~/prover.log'" | grep --line-buffered proving_time
```

Wait until the desired number of batches have been proved.

### 6. Collect Results

```bash
ssh <server> "bash -l -c 'cd ~/ethrex && ./scripts/sp1_bench_metrics.sh ~/prover.log'"
```

This outputs a markdown file (`sp1_bench_results.md`) with a results table and summary. Copy it locally if needed:

```bash
scp <server>:~/ethrex/sp1_bench_results.md .
```

### 7. Teardown

```bash
ssh <server> "bash -l -c 'cd ~/ethrex/crates/l2 && make down'"
```

Verify everything is stopped:

```bash
ssh <server> "bash -l -c 'pgrep -a -f ethrex || echo All stopped'"
ssh <server> "bash -l -c 'docker ps --format \"{{.Names}}\" | grep -i ethrex || echo No containers'"
```

## Troubleshooting

| Issue | Solution |
|-------|----------|
| Prover log shows no `proving_time` lines | Ensure prover was started with `PROVER_CLIENT_TIMED=true`. Check `~/prover.log` for errors. |
| Benchmark script shows "(no batches found)" | Prover hasn't finished proving any batch yet. Wait longer or check prover logs for errors. |
| Metrics show `-` for gas/tx/blocks | The benchmark script fetches these from the L2 metrics endpoint (`localhost:3702/metrics`). Ensure the L2 is still running when collecting results. Verify with `curl localhost:3702/metrics`. |
| Load test can't connect | Verify L2 RPC is on the expected port (default 1729). Check with `curl http://localhost:1729` |
| Load test nonce errors | The load test uses pending nonces, so consecutive runs should work. If stuck, restart the localnet. |
| GPU out of memory | Reduce batch size or check if another process is using the GPU: `nvidia-smi` |
| `command not found` errors over SSH | Always use `bash -l -c '...'` to load the full shell environment. |

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
| **Tx Type** | eth-transfers / erc20 / fibonacci / io-heavy |
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
