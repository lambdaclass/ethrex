# ethrex-p2p

Peer-to-peer networking layer for the ethrex Ethereum client.

## Overview

This crate implements the complete Ethereum P2P networking stack:

- **Node Discovery**: Finding and connecting to other Ethereum nodes
- **RLPx Transport**: Encrypted communication with peers
- **eth Protocol**: Block and transaction propagation
- **snap Protocol**: Fast state synchronization

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                      Network Layer                           │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────┐ │
│  │   discv4    │  │    RLPx     │  │   Peer Handler      │ │
│  │ (Discovery) │  │ (Transport) │  │   (Messages)        │ │
│  └─────────────┘  └─────────────┘  └─────────────────────┘ │
└─────────────────────────────────────────────────────────────┘
                             │
          ┌──────────────────┼──────────────────┐
          ▼                  ▼                  ▼
┌─────────────────┐ ┌─────────────────┐ ┌─────────────────┐
│   Sync Manager  │ │ TX Broadcaster  │ │  Snap Sync      │
└─────────────────┘ └─────────────────┘ └─────────────────┘
```

## Modules

### Discovery (`discv4`)
Node discovery protocol v4 for finding peers on the network.
- UDP-based peer discovery
- Kademlia-like DHT for peer routing
- ENR (Ethereum Node Record) support

### Transport (`rlpx`)
RLPx encrypted transport protocol.
- ECIES encryption handshake
- Multiplexed message framing
- Capability negotiation

### Peer Handler (`peer_handler`)
Message handling for connected peers.
- Request/response handling
- Block and transaction propagation
- State sync support

### Sync Manager (`sync_manager`)
Coordinates block synchronization.
- Full sync (downloading all blocks)
- Snap sync (state snapshots + recent blocks)
- Handles chain reorganizations

### TX Broadcaster (`tx_broadcaster`)
Transaction pool broadcasting.
- Announces new transactions to peers
- Handles transaction requests

## Protocols

### eth/68
The main Ethereum wire protocol for:
- Block headers and bodies
- Transaction announcements and requests
- Status messages and chain info

### snap/1
State snapshot protocol for fast sync:
- Account range requests
- Storage range requests
- Bytecode requests
- Trie node requests

## Usage

### Starting the Network

```rust
use ethrex_p2p::{start_network, SyncManager};

let (sync_manager, peer_handler) = start_network(
    udp_addr,      // UDP address for discovery
    tcp_addr,      // TCP address for RLPx
    bootnodes,     // Initial peers to connect to
    signer,        // Node identity
    storage,       // Database
    blockchain,    // Blockchain instance
).await?;
```

### Synchronization

```rust
// Start full sync
sync_manager.start_full_sync().await?;

// Or snap sync
sync_manager.start_snap_sync().await?;

// Check sync status
if sync_manager.is_synced() {
    // Node is synchronized
}
```

### Peer Management

```rust
// Get connected peers
let peers = peer_handler.get_peers().await;

// Add a specific peer
peer_handler.add_peer(enode_url).await?;

// Get peer count
let count = peer_handler.peer_count();
```

## Configuration

The P2P layer is configured through command-line options:

- `--discovery.addr`: UDP address for node discovery
- `--p2p.addr`: TCP address for peer connections
- `--bootnodes`: Comma-separated list of bootnode ENRs
- `--max-peers`: Maximum number of connected peers

## Features

- `experimental-discv5`: Enable discv5 node discovery (experimental)

## Message Types

### eth Protocol Messages
- `Status`: Chain status exchange
- `NewBlockHashes`: Announce new blocks
- `Transactions`: Broadcast transactions
- `GetBlockHeaders`/`BlockHeaders`: Request/response headers
- `GetBlockBodies`/`BlockBodies`: Request/response bodies
- `NewBlock`: Propagate new blocks
- `NewPooledTransactionHashes`: Announce pooled transactions
- `GetPooledTransactions`/`PooledTransactions`: Request/response transactions

### snap Protocol Messages
- `GetAccountRange`/`AccountRange`: Account data
- `GetStorageRanges`/`StorageRanges`: Storage data
- `GetByteCodes`/`ByteCodes`: Contract code
- `GetTrieNodes`/`TrieNodes`: Merkle trie nodes
