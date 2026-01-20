# ethrex-p2p

Peer-to-peer networking layer for the ethrex Ethereum client.

## Overview

This crate implements the complete Ethereum P2P networking stack, including node discovery, encrypted transport, and protocol message handling. It supports both full sync and snap sync modes for blockchain synchronization.

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                      Network Layer                               │
│  ┌─────────────────┐  ┌─────────────────┐  ┌─────────────────┐ │
│  │    Discovery    │  │     RLPx        │  │  Peer Handler   │ │
│  │  (discv4/v5)    │  │   (Transport)   │  │   (Messages)    │ │
│  └─────────────────┘  └─────────────────┘  └─────────────────┘ │
└─────────────────────────────────────────────────────────────────┘
                              │
          ┌───────────────────┼───────────────────┐
          ▼                   ▼                   ▼
┌─────────────────┐ ┌─────────────────┐ ┌─────────────────────────┐
│   Sync Manager  │ │ TX Broadcaster  │ │    Snap/Full Sync       │
│                 │ │                 │ │  ┌─────┐ ┌──────────┐  │
│                 │ │                 │ │  │State│ │ Storage  │  │
│                 │ │                 │ │  │Heal │ │ Healing  │  │
│                 │ │                 │ │  └─────┘ └──────────┘  │
└─────────────────┘ └─────────────────┘ └─────────────────────────┘
```

## Quick Start

```rust
use ethrex_p2p::{start_network, SyncManager};

// Start the P2P network
let (sync_manager, peer_handler) = start_network(
    udp_addr,
    tcp_addr,
    bootnodes,
    signer,
    storage,
    blockchain,
).await?;

// Start synchronization
sync_manager.start_sync().await?;
```

## Core Types

### P2PContext

Network context holding all shared state:

```rust
pub struct P2PContext {
    pub tracker: TaskTracker,
    pub signer: SecretKey,
    pub table: PeerTable,
    pub storage: Store,
    pub blockchain: Arc<Blockchain>,
    pub local_node: Node,
    pub client_version: String,
    pub tx_broadcaster: GenServerHandle<TxBroadcaster>,
}
```

### SyncManager

Abstraction for interacting with the active sync process:

```rust
pub struct SyncManager {
    snap_enabled: Arc<AtomicBool>,
    syncer: Arc<Mutex<Syncer>>,
    last_fcu_head: Arc<Mutex<H256>>,
    store: Store,
}
```

**Methods:**
- `sync_to_head(hash)` - Start sync to a specific block hash
- `sync_mode()` - Get current sync mode (Full or Snap)
- `disable_snap()` - Switch from snap to full sync

### SyncMode

```rust
pub enum SyncMode {
    Full,  // Download and execute all blocks
    Snap,  // Download state snapshots, then full sync recent blocks
}
```

## Module Structure

| Module | Description |
|--------|-------------|
| `network` | Network initialization and peer management |
| `peer_handler` | Message handling for connected peers |
| `sync_manager` | Block synchronization coordination |
| `sync` | Full and snap sync implementations |
| `tx_broadcaster` | Transaction pool broadcasting |
| `discv4` | Node discovery protocol v4 |
| `discv5` | Node discovery protocol v5 (experimental) |
| `rlpx` | RLPx encrypted transport layer |
| `types` | P2P-specific types (Node, endpoint info) |
| `metrics` | Prometheus metrics for networking |

## Protocols

### Discovery Protocols

| Protocol | Status | Description |
|----------|--------|-------------|
| discv4 | Stable | Kademlia-based node discovery using UDP |
| discv5 | Experimental | Enhanced discovery with topic advertisement |

### Wire Protocols

| Protocol | Description |
|----------|-------------|
| eth/68 | Block and transaction propagation |
| eth/69 | Enhanced eth protocol |
| snap/1 | State snapshot synchronization |

## Sync Modes

### Full Sync

Downloads and executes all blocks sequentially:
1. Fetch block headers from peers
2. Download block bodies
3. Execute blocks in batches (default: 1024 blocks)
4. Validate state root after execution

### Snap Sync

Fast initial sync using state snapshots:
1. Download state trie leaves (accounts)
2. Download storage tries for contracts
3. Collect bytecode for contracts
4. Heal state trie (fill missing nodes)
5. Heal storage tries
6. Full sync recent blocks (last ~10,000)

**Snap sync submodules:**
- `code_collector` - Bytecode downloading
- `state_healing` - State trie reconstruction
- `storage_healing` - Storage trie reconstruction

## RLPx Protocol

The `rlpx` module implements the encrypted transport layer:

| Submodule | Description |
|-----------|-------------|
| `connection` | TCP connection management and handshake |
| `eth` | eth protocol message handling |
| `snap` | snap protocol message handling |
| `l2` | L2-specific protocol extensions |
| `p2p` | Base P2P capability negotiation |

### Connection Flow

1. **TCP Connect** - Establish TCP connection to peer
2. **ECIES Handshake** - Exchange encryption keys
3. **Hello Exchange** - Negotiate protocol capabilities
4. **Status Exchange** - Verify chain compatibility
5. **Message Loop** - Handle protocol messages

## Feature Flags

| Feature | Description | Default |
|---------|-------------|---------|
| `c-kzg` | KZG commitment support (EIP-4844) | Yes |
| `sync-test` | Testing utilities for sync operations | No |
| `l2` | L2 rollup support with additional protocols | No |
| `metrics` | Prometheus metrics collection | No |
| `experimental-discv5` | Enable discv5 discovery (development only) | No |
| `test-utils` | Testing utilities | No |

## Configuration

Key constants (from `sync.rs`):

| Constant | Value | Description |
|----------|-------|-------------|
| `MIN_FULL_BLOCKS` | 10,000 | Blocks to full sync during snap sync |
| `EXECUTE_BATCH_SIZE` | 1,024 | Blocks per execution batch |
| `SECONDS_PER_BLOCK` | 12 | Expected block time |
| `BYTECODE_CHUNK_SIZE` | 50,000 | Bytecodes per download batch |

## Error Types

```rust
pub enum NetworkError {
    DiscoveryServer(DiscoveryServerError),
    TxBroadcaster(TxBroadcasterError),
    IO(io::Error),
    // ...
}

pub enum PeerHandlerError {
    Timeout,
    PeerTable(PeerTableError),
    Send(SendError),
    // ...
}
```

## Metrics

When the `metrics` feature is enabled, the following metrics are exported:
- Peer count and connection states
- Messages sent/received by type
- Sync progress and speed
- Discovery statistics

## Dependencies

- `ethrex-blockchain` - Block validation and execution
- `ethrex-storage` - Persistent storage
- `ethrex-trie` - Merkle Patricia Trie operations
- `ethrex-common` - Core Ethereum types
- `ethrex-crypto` - Cryptographic operations
- `secp256k1` - ECDSA signatures
- `tokio` - Async runtime

## Usage Examples

### Connecting to Bootnodes

```rust
use ethrex_p2p::types::Node;

let bootnodes = vec![
    Node::from_enode_url("enode://...@1.2.3.4:30303")?,
];

let (sync_manager, _) = start_network(
    "0.0.0.0:30303".parse()?,  // UDP for discovery
    "0.0.0.0:30303".parse()?,  // TCP for RLPx
    bootnodes,
    signer,
    storage,
    blockchain,
).await?;
```

### Manual Sync Control

```rust
// Get current sync mode
let mode = sync_manager.sync_mode();

// Trigger sync to specific head
sync_manager.sync_to_head(block_hash);

// Switch from snap to full sync
sync_manager.disable_snap();
```

For detailed API documentation:
```bash
cargo doc --package ethrex-p2p --open
```
