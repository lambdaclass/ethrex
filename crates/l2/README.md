# ethrex-l2

Layer 2 rollup implementation for the ethrex Ethereum client.

## Overview

This crate implements ethrex's L2 rollup, a zkEVM-based optimistic/validity rollup that settles on Ethereum L1. It includes the sequencer, prover infrastructure, based sequencing support, and a terminal-based monitor.

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                         L2 Sequencer                             │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────────┐ │
│  │   Block     │  │     L1      │  │    Proof Coordinator    │ │
│  │  Producer   │  │  Committer  │  │  (SP1/RISC0/ZisK/...)   │ │
│  └─────────────┘  └─────────────┘  └─────────────────────────┘ │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────────┐ │
│  │     L1      │  │     L1      │  │      State Updater      │ │
│  │   Watcher   │  │ Proof Sender│  │                         │ │
│  └─────────────┘  └─────────────┘  └─────────────────────────┘ │
└─────────────────────────────────────────────────────────────────┘
         │                  │                       │
         ▼                  ▼                       ▼
┌─────────────────┐ ┌─────────────────┐ ┌─────────────────────────┐
│  Ethereum L1    │ │    Prover       │ │    Rollup Storage       │
│  (Bridge/OCP)   │ │   (zkVM)        │ │    (Batches/Proofs)     │
└─────────────────┘ └─────────────────┘ └─────────────────────────┘
```

## Quick Start

```rust
use ethrex_l2::{start_l2, SequencerConfig};

// Start the L2 sequencer
let (l1_committer, block_producer, driver) = start_l2(
    store,
    rollup_store,
    blockchain,
    config,
    cancellation_token,
    l2_url,
    genesis,
    checkpoints_dir,
).await?;
```

## Core Components

### Sequencer

The main L2 sequencer consists of several cooperating services:

| Component | Description |
|-----------|-------------|
| `BlockProducer` | Builds L2 blocks from mempool transactions |
| `L1Committer` | Commits transaction batches to L1 |
| `L1ProofSender` | Sends validity proofs to L1 |
| `L1Watcher` | Monitors L1 for deposits and L2 state roots |
| `ProofCoordinator` | Coordinates proof generation across backends |
| `StateUpdater` | Updates L2 state based on verified batches |

### Block Producer

Creates L2 blocks with configurable gas limits:

```rust
pub struct BlockProducerConfig {
    pub block_gas_limit: u64,
    pub max_blobs_per_batch: Option<u32>,
    // ...
}
```

### L1 Committer

Commits batches to the OnChainProposer contract:

```rust
pub struct CommitterConfig {
    pub on_chain_proposer_address: Address,
    pub batch_gas_limit: Option<u64>,
    pub commit_interval_seconds: u64,
    // ...
}
```

### Proof Coordinator

Coordinates proof generation across multiple zkVM backends:

```rust
pub struct ProofCoordinatorConfig {
    pub prover_url: Option<String>,
    pub tdx_private_key: Option<SecretKey>,
    pub proving_interval_seconds: u64,
}
```

## Module Structure

| Module | Description |
|--------|-------------|
| `sequencer` | L2 sequencer services and configurations |
| `based` | Based sequencing support (block fetcher) |
| `monitor` | TUI dashboard for monitoring rollup state |
| `utils` | State reconstruction and utilities |
| `errors` | Error types for L2 operations |

## Related Crates

| Crate | Description |
|-------|-------------|
| `ethrex-prover` | zkVM proving backends (SP1, RISC0, ZisK, OpenVM) |
| `ethrex-sdk` | Developer SDK for L2 interactions |
| `ethrex-l2-common` | Shared types (calldata, messages, prover types) |
| `ethrex-l2-rpc` | L2-specific RPC endpoints |
| `ethrex-storage-rollup` | L2 storage layer for batches and proofs |

## Sequencer Configuration

```rust
pub struct SequencerConfig {
    pub eth: EthConfig,                    // L1 connection settings
    pub block_producer: BlockProducerConfig,
    pub l1_committer: CommitterConfig,
    pub l1_watcher: L1WatcherConfig,
    pub proof_coordinator: ProofCoordinatorConfig,
    pub state_updater: StateUpdaterConfig,
    pub based: BasedConfig,                // Based sequencing config
    pub monitor: MonitorConfig,            // TUI monitor config
    pub admin_server: AdminServerConfig,   // Admin API config
    pub aligned: AlignedConfig,            // Aligned layer config
}
```

## Operating Modes

### Standard Sequencing

The default mode where the sequencer:
1. Collects transactions from mempool
2. Builds blocks with the BlockProducer
3. Commits batches to L1 via L1Committer
4. Generates proofs via ProofCoordinator
5. Submits proofs to L1 via L1ProofSender

### Based Sequencing

When `based.enabled = true`:
1. BlockFetcher monitors L1 for sequenced transactions
2. Blocks are built from L1 data
3. Sequencer validates and proves these blocks

### Syncing Mode

When `state_updater.start_at > 0`:
1. StateUpdater syncs from a specific batch number
2. Useful for node recovery or new node bootstrap

## Prover Backends

The prover supports multiple zkVM backends:

| Backend | Feature Flag | Description |
|---------|--------------|-------------|
| SP1 | `sp1` | Succinct SP1 prover |
| RISC0 | `risc0` | RISC Zero prover |
| ZisK | `zisk` | Polygon ZisK prover |
| OpenVM | `openvm` | Axiom OpenVM prover |
| Exec | (default) | Execute-only (no proof) |

## L2 System Contracts

| Contract | Address | Description |
|----------|---------|-------------|
| CommonBridge | `0x...ffff` | L2 bridge for deposits/withdrawals |
| L2ToL1Messenger | `0x...fffe` | L2 to L1 message passing |
| FeeTokenRegistry | `0x...fffc` | Custom fee token registry |
| FeeTokenPricer | `0x...fffb` | Fee token pricing oracle |

## Monitor

The TUI monitor (`monitor` module) provides real-time dashboard:

- Chain status and sync progress
- Block and batch information
- L1 ↔ L2 message status
- Mempool state
- Rich account balances

Enable with `monitor.enabled = true` in config.

## Feature Flags

| Feature | Description | Default |
|---------|-------------|---------|
| `l2` | Enable L2 functionality | Yes |
| `sp1` | SP1 prover backend | No |
| `risc0` | RISC0 prover backend | No |
| `rocksdb` | RocksDB storage | No |
| `metrics` | Prometheus metrics | No |

## Admin API

The sequencer exposes an admin HTTP API for:
- Pausing/resuming components
- Querying sequencer state
- Manual batch commits
- Health checks

## Error Types

```rust
pub enum SequencerError {
    GasLimitError,
    ProverError(String),
    L1Error(String),
    StorageError(String),
    // ...
}
```

## Dependencies

- `ethrex-blockchain` - Block execution
- `ethrex-storage` - Base storage layer
- `ethrex-storage-rollup` - Rollup-specific storage
- `ethrex-vm` - EVM execution
- `ethrex-rpc` - RPC infrastructure
- `aligned-sdk` - Aligned layer integration
- `ratatui` - TUI framework for monitor

## Usage Examples

### Starting the Sequencer

```rust
use ethrex_l2::{SequencerConfig, start_l2};

let config = SequencerConfig::from_env()?;
let (committer, producer, driver) = start_l2(
    store,
    rollup_store,
    blockchain.into(),
    config,
    CancellationToken::new(),
    "http://localhost:8545".parse()?,
    genesis,
    PathBuf::from("./checkpoints"),
).await?;

// Run the driver future to completion
driver.await?;
```

### Checking Batch Status

```rust
use ethrex_sdk::{get_last_committed_batch, get_last_verified_batch};

let committed = get_last_committed_batch(&client, proposer_address).await?;
let verified = get_last_verified_batch(&client, proposer_address).await?;
```

For detailed API documentation:
```bash
cargo doc --package ethrex-l2 --open
```
