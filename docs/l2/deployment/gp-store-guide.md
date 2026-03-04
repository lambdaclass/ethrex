# GP Store — L2 Launch Guide

## Prerequisites

- [Docker Desktop](https://www.docker.com/get-started/) installed and running
- Git

## Quick Start

```bash
git clone https://github.com/tokamak-network/ethrex.git
cd ethrex

# Run with default guest program (evm-l2)
make -C crates/l2 init-guest-program

# Or specify a guest program
make -C crates/l2 init-guest-program PROGRAM=zk-dex
```

## Stop

```bash
make -C crates/l2 down-guest-program
```

## Endpoints

| Service | URL |
|---------|-----|
| L1 RPC | `http://localhost:8545` |
| L2 RPC | `http://localhost:1729` |

## Built-in Guest Programs

| Program | Type ID | Description |
|---------|---------|-------------|
| `evm-l2` | 1 | Default EVM execution |
| `zk-dex` | 2 | DEX order matching circuits |
| `tokamon` | 3 | Gaming state transition circuits |

## Configuration

### Guest Program Selection

The guest program is set via the `ETHREX_GUEST_PROGRAM_ID` environment variable:

```bash
# Via Makefile
make -C crates/l2 init-guest-program PROGRAM=tokamon

# Via docker compose directly
cd crates/l2
ETHREX_GUEST_PROGRAM_ID=tokamon \
DOCKER_ETHREX_WORKDIR=/usr/local/bin \
docker compose -f docker-compose.yaml \
  -f docker-compose-guest-program.overrides.yaml \
  up -d --build
```

### Prover Programs Config

The prover reads `programs.toml` to decide which guest programs to register:

```toml
default_program = "zk-dex"
enabled_programs = ["evm-l2", "zk-dex", "tokamon"]
```

## AI / Automation

```bash
git clone https://github.com/tokamak-network/ethrex.git && cd ethrex
make -C crates/l2 init-guest-program PROGRAM=evm-l2
# L1 RPC: http://localhost:8545
# L2 RPC: http://localhost:1729
# Stop: make -C crates/l2 down-guest-program
```

## Troubleshooting

| Issue | Solution |
|-------|----------|
| `docker: command not found` | Install [Docker Desktop](https://www.docker.com/get-started/) |
| Port in use | Stop other services on 8545/1729 |
| Container fails to start | Check logs: `docker compose logs` |
| Build takes too long | First build compiles Rust from source (~10min) |

## For Developers

To run without Docker (native build):

```bash
git clone https://github.com/tokamak-network/ethrex.git
cd ethrex/crates/l2
make init    # Starts L1 + deploys contracts + L2

# In another terminal, start prover with guest program config:
make init-prover-exec
```

See [deployment overview](./overview.md) for full details.
