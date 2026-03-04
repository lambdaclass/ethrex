# ZK-DEX Localnet Setup

One-command local environment for the ZK-DEX E2E pipeline.

## Quick Start

```bash
cd crates/l2

# Start full environment (L1 + contracts + L2 + SP1 prover)
make zk-dex-localnet

# Or without prover (faster, for app/frontend testing)
make zk-dex-localnet-no-prover
```

## Commands

| Command | Description |
|---------|-------------|
| `make zk-dex-localnet` | Start full localnet (L1 + deploy + L2 + prover) |
| `make zk-dex-localnet-no-prover` | Start without prover (app testing) |
| `make zk-dex-localnet-stop` | Stop all components |
| `make zk-dex-localnet-status` | Show status of each component |

You can also use the script directly:

```bash
./scripts/zk-dex-localnet.sh start [--no-prover]
./scripts/zk-dex-localnet.sh stop
./scripts/zk-dex-localnet.sh status
./scripts/zk-dex-localnet.sh logs [l1|l2|prover|deploy]
```

## What It Does

The script automates the 4-step manual process:

1. **Start L1** — Launches ethrex in `--dev` mode on port 8545
2. **Deploy Contracts** — Deploys OnChainProposer, Bridge, SP1 Verifier, and registers the ZK-DEX guest program
3. **Start L2** — Launches ethrex L2 on port 1729 with ZK-DEX guest program
4. **Start Prover** — Launches SP1 prover with `programs-zk-dex.toml` config

Each step includes health checks before proceeding to the next.

## Endpoints

| Service | URL |
|---------|-----|
| L1 RPC | `http://localhost:8545` |
| L2 RPC | `http://localhost:1729` |
| Proof Coordinator | `tcp://127.0.0.1:3900` |
| Prometheus Metrics | `http://localhost:3702` |

## Verification

```bash
# Check L1
curl -s -X POST -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}' \
  http://localhost:8545

# Check L2
curl -s -X POST -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}' \
  http://localhost:1729
```

## Logs

All logs are stored in `crates/l2/.zk-dex-localnet/`:

```bash
# Tail all logs
./scripts/zk-dex-localnet.sh logs

# Tail specific component
./scripts/zk-dex-localnet.sh logs l1
./scripts/zk-dex-localnet.sh logs l2
./scripts/zk-dex-localnet.sh logs prover
./scripts/zk-dex-localnet.sh logs deploy
```

## Prerequisites

- Rust toolchain (with `cargo`)
- SP1 toolchain (`sp1up` installed)
- Ports 8545, 8551, 1729, 3702, 3900 available

## File Layout

```
crates/l2/
  scripts/
    zk-dex-localnet.sh    # Main script
  programs-zk-dex.toml    # Prover program config
  .zk-dex-localnet/       # Runtime directory (created automatically)
    l1.pid / l2.pid / prover.pid
    l1.log / l2.log / prover.log / deploy.log
```
