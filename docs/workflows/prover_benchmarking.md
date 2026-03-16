# Prover Benchmarking Workflow

## Overview

This workflow measures proving time for the ethrex L2 prover on a remote server. The agent deploys a localnet (L1 + L2 + prover), generates load, waits for batches to be proved, and collects structured timing results.

For details on each tool see [Prover Benchmarking Guide](../l2/prover-benchmarking.md).

## Agent Setup Instructions

**Before starting any benchmark session, the agent MUST prompt the user for:**

1. **Server hostname** — SSH alias or full hostname where the benchmark will run
2. **Prover backend** — `sp1`, `risc0`, `exec`, etc. (default: `sp1`)
3. **GPU enabled?** — Whether to build with `GPU=true` (default: yes for sp1)
4. **Transaction count** — Number of transactions per account to generate (default: 50)
5. **Batches to prove** — How many batches to wait for before collecting results
6. **Endless mode?** — Whether to run the load test continuously (default: no)
7. **Branch/commit** — Git ref to benchmark (always ask, no default)
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

### 1b. Pre-compile All Binaries

Build **all** binaries before starting anything. This is the only compilation step in the entire workflow — every subsequent step uses the pre-built binaries directly, making them near-instant.

Build the `ethrex` binary with the features needed for the chosen backend. The `l2,l2-sql` features are always required. Add the backend feature (`sp1`, `risc0`) and optionally `gpu`:

| Backend | Features |
|---------|----------|
| SP1 with GPU | `l2,l2-sql,sp1,gpu` |
| SP1 without GPU | `l2,l2-sql,sp1` |
| RISC0 with GPU | `l2,l2-sql,risc0,gpu` |
| RISC0 without GPU | `l2,l2-sql,risc0` |
| Exec only | `l2,l2-sql` |

```bash
# Build ethrex (adjust --features for your backend)
ssh <server> "bash -l -c 'cd ~/ethrex && COMPILE_CONTRACTS=true cargo build --release --features <features> --bin ethrex'"

# Build load test
ssh <server> "bash -l -c 'cd ~/ethrex && cargo build --release --manifest-path ./tooling/load_test/Cargo.toml'"
```

`COMPILE_CONTRACTS=true` compiles the Solidity contracts and embeds them in the binary. This only needs to happen once — the same binary is used for deploying, running the L2, and proving.

### 2. Start the Localnet

The localnet is started in three stages so that the L1 (Docker) stays up across re-deploys and L2 restarts.

#### 2a. Start the L1

```bash
ssh <server> "bash -l -c 'cd ~/ethrex/crates/l2 && make init-l1-docker'"
```

This only needs to run once. If the L1 container is already running, skip this step.

#### 2b. Deploy L1 Contracts

Run the pre-built binary directly instead of `make deploy-l1` (which would trigger `cargo run` and recompile):

```bash
ssh <server> "bash -l -c 'cd ~/ethrex/crates/l2 && env ETHREX_DEPLOYER_RANDOMIZE_CONTRACT_DEPLOYMENT=true \
  ../../target/release/ethrex l2 deploy \
  --eth-rpc-url http://localhost:8545 \
  --private-key 0x385c546456b6a603a1cfcaa9ec9494ba4832da08dd6bcf4de9a71e4a01b74924 \
  <backend-verifier-flag> \
  --on-chain-proposer-owner 0x4417092b70a3e5f10dc504d0947dd256b965fc62 \
  --bridge-owner 0x4417092b70a3e5f10dc504d0947dd256b965fc62 \
  --bridge-owner-pk 0x941e103320615d394a55708be13e45994c7d93b932b064dbcb2b511fe3254e2e \
  --deposit-rich \
  --private-keys-file-path ../../fixtures/keys/private_keys_l1.txt \
  --genesis-l1-path ../../fixtures/genesis/l1.json \
  --genesis-l2-path ../../fixtures/genesis/l2.json'"
```

Replace `<backend-verifier-flag>` based on the prover backend. This tells the deployer to deploy the on-chain verifier contract and require that proof type:

| Backend | Flag | Effect |
|---------|------|--------|
| SP1 | `--sp1 true` | Deploys SP1 verifier, requires SP1 proofs |
| RISC0 | `--risc0 true` | Deploys RISC0 verifier, requires RISC0 proofs |
| Exec | *(omit flag)* | No on-chain verifier, exec proofs only |

The private keys and addresses above are the default test keys from `fixtures/`. They are hardcoded in the L1 genesis and have no value outside the local devnet.

`ETHREX_DEPLOYER_RANDOMIZE_CONTRACT_DEPLOYMENT` randomizes the CREATE2 salt so that contracts are deployed to fresh addresses on every run, avoiding collisions with previous deployments on the same L1.

#### 2c. Start the L2

Run the pre-built binary directly instead of `make init-l2` (which would trigger `cargo run` and recompile). The contract addresses are read from `cmd/.env` (written by the deployer in step 2b):

```bash
ssh <server> "bash -l -c 'cd ~/ethrex/crates/l2 && \
  export \$(cat ../../cmd/.env | xargs) && \
  nohup env ETHREX_NO_MONITOR=true ../../target/release/ethrex l2 \
  --watcher.block-delay 0 \
  --network ../../fixtures/genesis/l2.json \
  --http.port 1729 \
  --http.addr 0.0.0.0 \
  --metrics \
  --metrics.port 3702 \
  --datadir dev_ethrex_l2 \
  --l1.bridge-address \$ETHREX_WATCHER_BRIDGE_ADDRESS \
  --l1.on-chain-proposer-address \$ETHREX_COMMITTER_ON_CHAIN_PROPOSER_ADDRESS \
  --eth.rpc-url http://localhost:8545 \
  --osaka-activation-time 1761677592 \
  --block-producer.coinbase-address 0x0007a881CD95B1484fca47615B64803dad620C8d \
  --block-producer.base-fee-vault-address 0x000c0d6b7c4516a5b274c51ea331a9410fe69127 \
  --block-producer.operator-fee-vault-address 0xd5d2a85751b6F158e5b9B8cD509206A865672362 \
  --block-producer.operator-fee-per-gas 1000000000 \
  --committer.l1-private-key 0x385c546456b6a603a1cfcaa9ec9494ba4832da08dd6bcf4de9a71e4a01b74924 \
  --proof-coordinator.l1-private-key 0x39725efee3fb28614de3bacaffe4cc4bd8c436257e2c8bb887c4b5c4be45e76d \
  --proof-coordinator.addr 127.0.0.1 \
  > ~/l2.log 2>&1 &'"
```

`ETHREX_NO_MONITOR=true` disables the TUI monitor so that logs are written to stdout (and captured in the log file). The `export $(cat ../../cmd/.env | xargs)` loads the contract addresses that the deployer wrote in step 2b.

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

Available transaction types (passed via `-t`):
| `-t` flag | Description |
|-----------|-------------|
| `eth-transfers` | ETH transfers |
| `erc20` | ERC20 token transfers |
| `fibonacci` | Fibonacci computation transactions |
| `io-heavy` | IO-heavy transactions |

### 4. Start the Prover

Run the pre-built binary directly instead of `make init-prover-sp1` (which would trigger `cargo run` and recompile):

```bash
# SP1:
ssh <server> "bash -l -c 'cd ~/ethrex/crates/l2 && nohup env PROVER_CLIENT_TIMED=true \
  ../../target/release/ethrex l2 prover \
  --proof-coordinators tcp://127.0.0.1:3900 \
  --backend sp1 \
  > ~/prover.log 2>&1 &'"

# RISC0:
ssh <server> "bash -l -c 'cd ~/ethrex/crates/l2 && nohup env PROVER_CLIENT_TIMED=true \
  ../../target/release/ethrex l2 prover \
  --proof-coordinators tcp://127.0.0.1:3900 \
  --backend risc0 \
  > ~/prover.log 2>&1 &'"

# Exec:
ssh <server> "bash -l -c 'cd ~/ethrex/crates/l2 && nohup env PROVER_CLIENT_TIMED=true \
  ../../target/release/ethrex l2 prover \
  --proof-coordinators tcp://127.0.0.1:3900 \
  --backend exec \
  > ~/prover.log 2>&1 &'"
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
ssh <server> "bash -l -c 'cd ~/ethrex && ./scripts/bench_metrics.sh ~/prover.log'"
```

This outputs a markdown file (`bench_results.md`) with a results table and summary. Copy it locally if needed:

```bash
scp <server>:~/ethrex/bench_results.md .
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
