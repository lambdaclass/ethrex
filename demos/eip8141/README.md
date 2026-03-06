# EIP-8141 Frame Transaction Demo -- WebAuthn Passkey Authentication

This demo showcases EIP-8141 frame transactions using WebAuthn passkey-based authentication on a local ethrex node. Users authenticate with biometrics (Touch ID, Face ID, etc.) to sign Ethereum transactions -- no seed phrases or browser extensions required.

Frame transactions bundle multiple execution frames (VERIFY, SENDER, etc.) into a single transaction, enabling patterns like on-chain signature verification, gas sponsorship, batched operations, and atomic deploy-and-execute.

## Architecture

```
Browser (React + WebAuthn)
    |
    |-- reads --> ethrex node (port 8545)
    |
    +-- writes --> TypeScript Backend (port 3000)
                    |
                    +---> ethrex node (eth_sendRawTransaction)
```

- **Frontend** creates a WebAuthn challenge derived from the transaction's signature hash, prompts the user for biometric authentication, and sends the resulting credential to the backend.
- **Backend** constructs the Frame TX (VERIFY frame with WebAuthn signature + SENDER frame with the operation), signs it with a dev key, and submits it to ethrex.
- **ethrex** executes the frames sequentially: the VERIFY frame checks the P256 signature on-chain via the WebAuthnP256Account contract, then the SENDER frame executes the intended operation.

## Prerequisites

- **Rust toolchain** (for building ethrex)
- **Bun** (latest -- for backend and frontend)
- **solc** with `--via-ir` support (for contract compilation)
- **Rex CLI** (`cargo install --path /path/to/rex/cli`)

## Quick Start

```bash
# 1. Install JS dependencies
make install

# 2. Compile contracts (optional -- pre-deployed via genesis)
make compile-contracts

# 3. Start ethrex node (terminal 1)
make start-node

# 4. Start backend (terminal 2)
make start-backend

# 5. Start frontend (terminal 3)
make start-frontend

# 6. Open http://localhost:5173
```

## Demo Walkthroughs

| Tab | Description |
|-----|-------------|
| **Simple Send** | Create a passkey account and send ETH to another address |
| **Sponsored TX** | Send a transaction with gas paid by a GasSponsor contract |
| **Batched Ops** | Execute multiple operations atomically in one transaction |
| **Deploy + Execute** | Deploy a contract and interact with it in a single transaction |

## How It Works

1. The frontend generates a WebAuthn challenge from the EIP-8141 frame transaction's signature hash.
2. The user authenticates with their device's biometric (Touch ID, Face ID, security key).
3. The backend receives the WebAuthn credential and builds a Frame TX with:
   - **VERIFY frame** -- calls the WebAuthnP256Account contract to verify the P256 signature on-chain
   - **SENDER frame** -- executes the intended operation (transfer, contract call, etc.)
4. ethrex executes the frames sequentially. If the VERIFY frame fails, the entire transaction reverts.

## Contract Addresses

| Contract | Address | Notes |
|----------|---------|-------|
| GasSponsor | `0x1000000000000000000000000000000000000001` | Pre-funded with 100 ETH |
| MockERC20 | `0x1000000000000000000000000000000000000002` | ERC-20 token for testing |
| WebAuthnP256Account | Deployed at runtime | Created per-user via CREATE2 |

> **Note:** The GasSponsor and MockERC20 contracts are pre-deployed in the genesis with placeholder bytecode (`0x00`). Replace with compiled bytecode from `contracts/solc_out/` for a functional demo.

## Project Structure

```
demos/eip8141/
|-- contracts/     -- Solidity smart contracts
|-- backend/       -- TypeScript backend (Hono + Bun)
|-- frontend/      -- React frontend (Vite + Tailwind)
|-- genesis.json   -- ethrex genesis configuration
|-- Makefile       -- Build and run commands
+-- README.md      -- This file
```

## Block Explorer (Blockscout)

Run a patched Blockscout instance alongside the demo to explore blocks, transactions, and verified contracts.

### Setup

```bash
# Clone the patched Blockscout (with EIP-8141 frame tx support)
git clone -b eip-8141-support git@github.com:lambdaclass/ethrex-blockscout.git \
  ~/Repositories/lambdaclass/ethrex-blockscout

# Start all Blockscout services (requires Docker)
cd ~/Repositories/lambdaclass/ethrex-blockscout
docker compose -f docker-compose/docker-compose.yml up -d --build

# Open http://localhost:8082
```

### What's patched

The `eip-8141-support` branch adds:

- **Type 6 frame transaction parsing** — Blockscout's indexer crashes on unknown tx types. The patch adds a `do_elixir_to_params` clause in `transaction.ex` that handles type 6 transactions (no `gas`, `input`, `value`, or `to` fields; no ECDSA signature).
- **Local smart-contract-verifier** — Enables `solc --via-ir` verification for the demo contracts.
- **Demo configuration** — Chain ID 1729, RPC port 8545, no ads.

### Contract verification

MockERC20 (`0x...0002`) and WebAuthnVerifier (`0x...0004`) are verifiable via the Blockscout UI or API. The Yul contracts (GasSponsor, WebAuthnP256Account) use `verbatim` custom opcodes and can't be auto-verified.

### Services

| Service | Port | Description |
|---------|------|-------------|
| Frontend (nginx proxy) | 8082 | Blockscout web UI |
| Backend API | 4000 (internal) | Blockscout indexer + API |
| Smart Contract Verifier | 8050 (internal) | Solidity/Vyper verification |
| PostgreSQL | 7432 (internal) | Blockscout database |

## Dev Account

The genesis pre-funds a dev account for deploying contracts and funding passkey accounts:

- **Address:** `0x3f1Eae7D46d88F08fc2F8ed27FCb2AB183EB2d0E`
- **Private key:** `0xb6b15c8cb491557369f3c7d2c287b053eb229daa9c22138887752191c9520659`

This is a local-only test key. Never use it on mainnet or public testnets.
