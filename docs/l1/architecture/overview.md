# System Overview

This document provides a high-level overview of ethrex's L1 architecture as an Ethereum execution client.

## Introduction

ethrex is a Rust implementation of an Ethereum execution client. It implements the Ethereum protocol specification, including:

- Block validation and execution
- State management via Merkle Patricia Tries
- P2P networking (devp2p stack)
- JSON-RPC API for external interaction
- Engine API for consensus client communication

## High-Level Architecture

```
                                    ┌─────────────────────┐
                                    │   Consensus Client  │
                                    │  (Lighthouse, etc)  │
                                    └──────────┬──────────┘
                                               │ Engine API
                                               │ (JWT auth)
                                               ▼
┌──────────────────────────────────────────────────────────────────────────────┐
│                              ethrex Execution Client                          │
│                                                                               │
│  ┌─────────────┐     ┌──────────────┐     ┌────────────────────────────────┐ │
│  │   JSON-RPC  │     │  Engine API  │     │           P2P Network          │ │
│  │    Server   │     │   Handler    │     │  ┌────────┐  ┌──────────────┐  │ │
│  │             │     │              │     │  │DiscV4  │  │    RLPx      │  │ │
│  │ eth_*       │     │ engine_*     │     │  │        │  │  ┌────────┐  │  │ │
│  │ debug_*     │     │ forkchoice   │     │  │        │  │  │ eth/68 │  │  │ │
│  │ txpool_*    │     │ newPayload   │     │  │        │  │  │ snap/1 │  │  │ │
│  │ admin_*     │     │ getPayload   │     │  │        │  │  └────────┘  │  │ │
│  └──────┬──────┘     └──────┬───────┘     │  └────────┘  └──────────────┘  │ │
│         │                   │             └────────────────┬───────────────┘ │
│         │                   │                              │                 │
│         └───────────────────┼──────────────────────────────┘                 │
│                             │                                                 │
│                             ▼                                                 │
│  ┌───────────────────────────────────────────────────────────────────────┐   │
│  │                           Blockchain                                   │   │
│  │  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐   │   │
│  │  │   Mempool   │  │  Payload    │  │ Fork Choice │  │   Block     │   │   │
│  │  │             │  │  Builder    │  │   Update    │  │  Pipeline   │   │   │
│  │  └─────────────┘  └─────────────┘  └─────────────┘  └─────────────┘   │   │
│  └───────────────────────────────────────────────────────────────────────┘   │
│                             │                                                 │
│                             ▼                                                 │
│  ┌───────────────────────────────────────────────────────────────────────┐   │
│  │                              EVM (LEVM)                                │   │
│  │  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐   │   │
│  │  │Transaction  │  │   Opcode    │  │  Precompiled│  │    State    │   │   │
│  │  │ Execution   │  │   Handler   │  │  Contracts  │  │ Transitions │   │   │
│  │  └─────────────┘  └─────────────┘  └─────────────┘  └─────────────┘   │   │
│  └───────────────────────────────────────────────────────────────────────┘   │
│                             │                                                 │
│                             ▼                                                 │
│  ┌───────────────────────────────────────────────────────────────────────┐   │
│  │                             Storage                                    │   │
│  │  ┌───────────────────────────────────────────────────────────────┐    │   │
│  │  │                     Store (High-level API)                     │    │   │
│  │  └───────────────────────────────────────────────────────────────┘    │   │
│  │                    │                              │                    │   │
│  │         ┌──────────┴──────────┐        ┌─────────┴────────┐           │   │
│  │         ▼                     ▼        ▼                  ▼           │   │
│  │  ┌─────────────┐       ┌─────────────────┐       ┌───────────────┐    │   │
│  │  │  InMemory   │       │    RocksDB      │       │  State Trie   │    │   │
│  │  │  (Testing)  │       │  (Production)   │       │ (MPT + Flat)  │    │   │
│  │  └─────────────┘       └─────────────────┘       └───────────────┘    │   │
│  └───────────────────────────────────────────────────────────────────────┘   │
└──────────────────────────────────────────────────────────────────────────────┘
```

## Core Components

### 1. Network Layer

The network layer handles all external communication:

**JSON-RPC Server** (`crates/networking/rpc`)
- Implements the Ethereum JSON-RPC specification
- Namespaces: `eth_*`, `debug_*`, `txpool_*`, `admin_*`, `web3_*`
- Validates and broadcasts incoming transactions

**Engine API** (`crates/networking/rpc/engine`)
- Communication channel with the consensus client
- Handles `engine_forkchoiceUpdatedV{1,2,3}`, `engine_newPayloadV{1,2,3}`, `engine_getPayloadV{1,2,3}`
- JWT authentication for security
- Triggers sync when receiving unknown block hashes

**P2P Network** (`crates/networking/p2p`)
- **DiscV4**: Node discovery protocol for finding peers
- **RLPx**: Encrypted transport layer for peer communication
- **eth/68**: Block and transaction propagation protocol
- **snap/1**: Snap sync protocol for fast state download

### 2. Blockchain Layer

The blockchain layer manages chain state and block processing:

**Blockchain** (`crates/blockchain`)
- Orchestrates block validation and execution
- Manages the mempool for pending transactions
- Handles fork choice updates from the consensus layer
- Coordinates payload building for block production

**Mempool**
- Stores pending transactions awaiting inclusion
- Filters transactions by gas price, nonce, and validity
- Supports transaction replacement (EIP-1559 and EIP-4844)
- Broadcasts new transactions to peers

**Fork Choice**
- Implements Ethereum's fork choice rule
- Updates the canonical chain based on consensus client signals
- Handles chain reorganizations

### 3. Execution Layer

**LEVM (Lambda EVM)** (`crates/vm/levm`)
- Custom EVM implementation in Rust
- Executes smart contract bytecode
- Implements all EVM opcodes up to the latest hard fork
- Handles precompiled contracts

**Block Execution Pipeline**
1. Validate block header
2. Apply system-level operations (beacon root, block hash storage)
3. Execute transactions in order
4. Process withdrawals (post-Merge)
5. Extract requests (post-Prague)
6. Compute state root and verify against header

### 4. Storage Layer

**Store** (`crates/storage`)
- High-level API for all blockchain data
- Supports multiple backends: InMemory (testing), RocksDB (production)
- Manages block headers, bodies, receipts, and state

**State Trie** (`crates/common/trie`)
- Merkle Patricia Trie implementation
- Stores account states and contract storage
- Supports flat key-value storage for performance
- Handles trie node caching and persistence

## Data Flow

### Block Import (from P2P)

```
P2P Peer → Block Headers/Bodies → Syncer → Blockchain.add_block() → EVM.execute() → Store
```

1. Syncer requests headers from peers
2. Headers are validated (parent exists, timestamps, gas limits, etc.)
3. Bodies are requested and matched to headers
4. Blocks are executed in batches
5. State is committed to storage

### Block Import (from Consensus Client)

```
Consensus Client → engine_newPayloadV3 → Blockchain.add_block_pipeline() → EVM.execute() → Store
                 → engine_forkchoiceUpdated → Fork Choice Update → Canonical Chain Update
```

1. Consensus client sends new payload via Engine API
2. Block is validated and executed
3. Fork choice update makes the block canonical
4. Sync is triggered if the block's parent is unknown

### Transaction Lifecycle

```
User → JSON-RPC (eth_sendRawTransaction) → Mempool → Broadcast to Peers
                                                   → Include in Block
```

1. Transaction arrives via JSON-RPC or P2P
2. Validated for signature, nonce, balance, gas
3. Added to mempool if valid
4. Broadcast to connected peers
5. Eventually included in a block by the payload builder

## Sync Modes

### Full Sync

Downloads and executes every block from genesis (or a known checkpoint):

1. Request block headers from peers
2. Request block bodies for each header
3. Execute blocks in batches (1024 blocks per batch)
4. Commit state after each batch
5. Update fork choice when sync head is reached

### Snap Sync

Downloads state directly instead of executing all historical blocks:

1. Download block headers to find a recent "pivot" block
2. Download account state trie leaves via snap protocol
3. Download storage tries for accounts with storage
4. Heal any missing trie nodes (state may have changed during download)
5. Download bytecode for contract accounts
6. Execute recent blocks (post-pivot) to catch up

See [Sync State Machine](./sync_state_machine.md) for detailed documentation.

## Concurrency Model

ethrex uses Tokio for async I/O with the following patterns:

- **Async tasks** for network I/O (RPC, P2P)
- **Blocking tasks** for CPU-intensive work (block execution, trie operations)
- **Channels** for inter-component communication (sync signals, mempool updates)
- **RwLock/Mutex** for shared state (mempool, peer table)

## Configuration

Key configuration options:

| Option | Description | Default |
|--------|-------------|---------|
| `--network` | Network to connect to | `mainnet` |
| `--datadir` | Data directory for DB and keys | `~/.ethrex` |
| `--syncmode` | Sync mode (`full` or `snap`) | `snap` |
| `--authrpc.port` | Engine API port | `8551` |
| `--http.port` | JSON-RPC HTTP port | `8545` |
| `--discovery.port` | P2P discovery port | `30303` |

See [Configuration](../running/configuration.md) for the complete reference.

## Next Steps

- [Block Execution Pipeline](./block_execution.md) - Deep dive into block processing
- [Sync State Machine](./sync_state_machine.md) - Detailed sync algorithm documentation
- [Crate Map](./crate_map.md) - Overview of all crates and dependencies
