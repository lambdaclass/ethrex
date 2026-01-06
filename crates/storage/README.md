# ethrex-storage

Persistent storage layer for the ethrex Ethereum client.

## Overview

This crate provides all data persistence functionality for ethrex, including:

- **Block Storage**: Headers, bodies, receipts, and transaction indexing
- **State Storage**: Account balances, nonces, code, and storage slots
- **Trie Management**: Merkle Patricia Trie for state verification
- **Chain Configuration**: Fork schedules, chain ID, and genesis state

## Architecture

```
┌─────────────────────────────────────────────────┐
│                    Store                        │
│  (High-level API for blockchain operations)     │
└─────────────────────────────────────────────────┘
                       │
          ┌────────────┴────────────┐
          ▼                         ▼
┌─────────────────┐       ┌─────────────────┐
│  InMemoryBackend │       │  RocksDBBackend │
│    (Testing)     │       │  (Production)   │
└─────────────────┘       └─────────────────┘
```

## Storage Backends

### InMemory
- Fast, non-persistent storage
- Used for testing and development
- No external dependencies

### RocksDB (requires `rocksdb` feature)
- Production-grade persistent storage
- Optimized for blockchain workloads
- Supports snapshots and atomic batches

## Usage

### Creating a Store

```rust
use ethrex_storage::{Store, EngineType};
use std::path::Path;

// Create with RocksDB backend
let store = Store::new("./data", EngineType::RocksDB)?;

// Or create from genesis file
let store = Store::new_from_genesis(
    Path::new("./data"),
    EngineType::RocksDB,
    "genesis.json"
).await?;
```

### Block Operations

```rust
// Add a block
store.add_block(block).await?;

// Get block by number
let header = store.get_block_header(block_number)?;
let body = store.get_block_body(block_number)?;

// Get block by hash
let header = store.get_block_header_by_hash(block_hash)?;

// Get receipts
let receipts = store.get_receipts(block_number)?;
```

### State Queries

```rust
// Get account info at a specific block
let info = store.get_account_info(block_number, address)?;
let balance = info.map(|a| a.balance).unwrap_or_default();

// Get contract code
let code = store.get_code(block_number, address)?;

// Get storage value
let value = store.get_storage_at(block_number, address, key)?;

// Get account proof (for eth_getProof)
let proof = store.get_account_proof(block_number, address)?;
```

### State Updates

```rust
// Apply account updates after block execution
store.apply_account_updates_batch(
    block_hash,
    &account_updates,
)?;

// Batch update for multiple blocks
store.apply_updates(UpdateBatch {
    account_updates: trie_nodes,
    storage_updates: storage_nodes,
    blocks: vec![block],
    receipts: vec![(block_hash, receipts)],
    code_updates: vec![(code_hash, code)],
}).await?;
```

## Database Schema

### Tables

| Table | Key | Value | Description |
|-------|-----|-------|-------------|
| `HEADERS` | block_hash | BlockHeader (RLP) | Block headers by hash |
| `BODIES` | block_hash | BlockBody (RLP) | Block bodies by hash |
| `CANONICAL_BLOCK_HASHES` | block_number | block_hash | Canonical chain mapping |
| `BLOCK_NUMBERS` | block_hash | block_number | Reverse block lookup |
| `RECEIPTS` | block_hash | Vec<Receipt> (RLP) | Transaction receipts |
| `TRANSACTION_LOCATIONS` | tx_hash + block_hash | tx_index | Transaction indexing |
| `ACCOUNT_TRIE_NODES` | node_hash | TrieNode (RLP) | State trie nodes |
| `STORAGE_TRIE_NODES` | account_hash + node_hash | TrieNode (RLP) | Storage trie nodes |
| `ACCOUNT_CODES` | code_hash | bytecode | Contract bytecode |
| `CHAIN_DATA` | index | value | Chain config, latest block, etc. |

## Caching

The store maintains several caches for performance:

- **Trie Layer Cache**: Recent trie nodes for fast state access without full trie traversal
- **Code Cache**: LRU cache for contract bytecode (64MB default)
- **Latest Block Cache**: Cached latest block header for RPC queries

## Features

- `rocksdb`: Enable RocksDB backend for persistent storage

## Schema Versioning

The store uses schema versioning (`STORE_SCHEMA_VERSION`) to detect incompatible database formats. When the schema version changes, a full resync is required.
