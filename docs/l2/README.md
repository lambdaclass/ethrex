# Ethrex L2 Documentation

This document provides instructions for setting up and managing the Ethrex L2 development environment. The system has been streamlined to offer a flexible workflow, allowing developers to run services using Docker or directly on the host machine.

For a high-level overview of the L2 architecture and its components, please refer to the following documents:

- [General Overview](./overview.md)
- [Sequencer](./sequencer.md)
- [Contracts](./contracts.md)
- [Prover](./prover.md)
- [State Diffs](./state_diffs.md)
- [Withdrawals](./withdrawals.md)

## Prerequisites

Before you begin, ensure you have the following tools installed:

- [Rust](https://www.rust-lang.org/tools/install)
- [Docker](https://docs.docker.com/engine/install/)

## Quick Start

To get the entire L2 stack up and running with a single command, navigate to the `crates/l2` directory and run:

```bash
make up
```

This command will start both the L1 and L2 services using Docker Compose. To stop all services, use:

```bash
make down
```

## Advanced Usage

For more granular control, you can manage individual components separately. This is useful for debugging or focusing on a specific part of the stack.

### Docker Compose Management

You can start and stop individual services using the following `make` targets:

- **Start L1:** `make up-l1-docker`
- **Stop L1:** `make down-l1-docker`
- **Start L2:** `make up-l2-docker`
- **Stop L2:** `make down-l2-docker`
- **Start Prover:** `make up-prover-docker`
- **Stop Prover:** `make down-prover-docker`

### Host Process Management

If you prefer to run services directly on your host machine, use these targets. The process will run in the foreground of your current terminal. To stop a running process, press `Ctrl-C`.

- **Run L1:** `make run-l1-host`
- **Run L2:** `make run-l2-host`
- **Run Prover:** `make run-prover-host`

## Configuration

The L2 environment can be configured in two ways, allowing for both persistent and temporary settings.

### 1. Using a `.env` File

For persistent configuration, create a `.env` file in the `crates/l2` directory. The `Makefile` will automatically include it and export any variables prefixed with `ETHREX_`.

> **Note:** Your custom variables in the `.env` file are safe. When the contract deployer runs (e.g., as part of `make up`), it will intelligently add or update only the necessary contract addresses, preserving all other values.

**Example `.env` file:**

```env
# L1 RPC URL for the L2 node to connect to
L1_RPC_URL=http://localhost:8545

# Private key for the L2 committer
ETHREX_COMMITTER_L1_PRIVATE_KEY=0x...

# Private key for the proof coordinator
ETHREX_PROOF_COORDINATOR_L1_PRIVATE_KEY=0x...
```

### 2. Passing Variables to `make`

For temporary or one-off configurations, you can pass variables directly to the `make` command. These will override any values set in the `.env` file.

**Example:**

```bash
# Run the L1 on a different port
make up-l1-docker L1_PORT=8546

# Pass a private key directly to the L2 host process
make run-l2-host ETHREX_COMMITTER_L1_PRIVATE_KEY=0x...
```

## Building Binaries

To build the necessary binaries for running host processes, use the following targets:

- **Build all:** `make build` (builds both `ethrex` and `prover`)
- **Build Ethrex:** `make build-ethrex`
- **Build Prover:** `make build-prover PROVER=<risc0|sp1>`
