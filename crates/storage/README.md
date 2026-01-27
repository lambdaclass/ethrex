# ethrex-storage

Persistent storage layer for the ethrex Ethereum client.

## Overview

This crate provides a high-level storage API (`Store`) for blockchain operations, abstracting away two pluggable storage backends: InMemory (for testing) and RocksDB (for production). It handles blocks, state, transactions, receipts, and Merkle Patricia Tries.

## Architecture

```
                    ┌─────────────────────────────────────────────────┐
                    │                    Store                        │
                    │  (High-level API for blockchain operations)     │
                    └─────────────────────────────────────────────────┘
                                           │
                              ┌────────────┴────────────┐
                              ▼                         ▼
                    ┌─────────────────-┐       ┌─────────────────┐
                    │  InMemoryBackend │       │  RocksDBBackend │
                    │    (Testing)     │       │  (Production)   │
                    └─────────────────-┘       └─────────────────┘
```

## Quick Start

```rust
use ethrex_storage::{Store, EngineType};
use std::path::Path;

// Create with RocksDB backend (requires `rocksdb` feature)
let store = Store::new("./data", EngineType::RocksDB)?;

// Or from a genesis file
let store = Store::new_from_genesis(
    Path::new("./data"),
    EngineType::RocksDB,
    "genesis.json"
).await?;

// Add a block
store.add_block(block).await?;

// Query account state
let info = store.get_account_info(block_number, address).await?;
let balance = info.map(|a| a.balance);

// Get storage value
let value = store.get_storage_at(block_hash, address, key)?;
```

## Features

| Feature | Description | Default |
|---------|-------------|---------|
| `rocksdb` | Enable RocksDB backend for persistent storage | No |

Without the `rocksdb` feature, only `EngineType::InMemory` is available.

## Storage Backends

### InMemory Backend

- **Use case**: Testing and development
- **Characteristics**: Fast, non-persistent, uses `BTreeMap`
- **Concurrency**: Thread-safe via `RwLock`

### RocksDB Backend

- **Use case**: Production deployment
- **Characteristics**: Persistent, multi-threaded, optimized per table
- **Features**: Atomic writes, checkpoints, bloom filters

**RocksDB Tuning**:
- Trie nodes: 512MB write buffer, bloom filters
- Block data: 128MB write buffer, LZ4 compression
- Contract code: Blob files with LZ4 compression

## Module Structure

| Module | Description |
|--------|-------------|
| `store` | Main `Store` type with blockchain operations |
| `api` | `StorageBackend` trait and table definitions |
| `backend` | Backend implementations (InMemory, RocksDB) |
| `trie` | Trie database adapters for storage backends |
| `layering` | Diff-layer caching for efficient state access |
| `rlp` | RLP encoding wrappers for storage types |
| `error` | `StoreError` type |
| `utils` | Metadata and index types |

## Core Types

### Store

The main interface for all storage operations:

**Block Operations:**
- `add_block(block)` / `add_blocks(blocks)` - Store blocks
- `get_block_by_hash(hash)` / `get_block_by_number(number)` - Retrieve blocks
- `get_block_header(block_number)` - Get header only
- `remove_block(block_number)` - Remove from chain

**Account/State:**
- `get_account_info(block_number, address)` - Balance, nonce, code_hash
- `get_code_by_account_address(block_number, address)` - Contract bytecode
- `get_storage_at(block_hash, address, key)` - Storage slot value
- `apply_account_updates_batch(...)` - Apply state changes

**Transaction/Receipt:**
- `add_receipt(block_hash, index, receipt)` - Store receipt
- `get_transaction_by_hash(tx_hash)` - Lookup transaction
- `get_receipts_for_block(block_hash)` - All receipts in block

**Chain Data:**
- `get_latest_block_number()` / `get_earliest_block_number()`
- `get_canonical_block_hash(block_number)`
- `set_chain_config(config)` / `get_fork_id()`

### EngineType

```rust
pub enum EngineType {
    InMemory,  // Testing
    RocksDB,   // Production (requires feature)
}
```

### UpdateBatch

Batch of changes to apply atomically:

```rust
pub struct UpdateBatch {
    pub account_updates: Vec<TrieNode>,
    pub storage_updates: Vec<(H256, Vec<TrieNode>)>,
    pub blocks: Vec<Block>,
    pub receipts: Vec<(H256, Vec<Receipt>)>,
    pub code_updates: Vec<(H256, Code)>,
}
```

## Database Tables

The storage uses 18 tables (column families in RocksDB):

| Table | Purpose |
|-------|---------|
| `HEADERS` | Block headers by hash |
| `BODIES` | Block bodies (transactions) by hash |
| `RECEIPTS` | Transaction receipts |
| `TRANSACTION_LOCATIONS` | TX hash to block location |
| `ACCOUNT_TRIE_NODES` | Account state trie |
| `STORAGE_TRIE_NODES` | Contract storage tries |
| `ACCOUNT_CODES` | Contract bytecode |
| `CANONICAL_BLOCK_HASHES` | Number to canonical hash |
| `BLOCK_NUMBERS` | Hash to number mapping |
| `CHAIN_DATA` | Chain configuration |
| `PENDING_BLOCKS` | Non-canonical blocks |
| `EXECUTION_WITNESSES` | zkVM witness data |

## Caching

### Code Cache

LRU cache for contract bytecode:
- **Max size**: 64MB
- **Eviction**: Least recently used

### Trie Layer Cache

Diff-layer caching for efficient state access:
- **Mechanism**: Each block creates a layer of trie changes
- **Commit threshold**: 128 layers before disk flush
- **Bloom filter**: Fast negative lookups (1M-100M items)

## State Management

State is stored using Merkle Patricia Tries:

- **State Trie**: Account address -> AccountState
- **Storage Tries**: Storage key -> value (per contract)
- **Code Storage**: Code hash -> bytecode

### FlatKeyValue

Incremental snapshot format for efficient reads:
- Background thread generates flat key-value pairs
- Paused during tier commits
- Enables fast state sync

## Witness Storage

For zkVM proving, execution witnesses are stored:

```rust
store.store_witness(block_hash, block_number, witness)?;
let witness = store.get_witness_by_number_and_hash(block_number, block_hash)?;
```

Maximum 128 witnesses retained.

## Error Handling

```rust
pub enum StoreError {
    DecodeError,
    RocksdbError(rocksdb::Error),
    RLPDecode(RLPDecodeError),
    Trie(TrieError),
    MissingStore,
    MissingLatestBlockNumber,
    IncompatibleChainConfig,
    IncompatibleDBVersion { found: u64, expected: u64 },
    // ...
}
```

## Schema Versioning

```rust
pub const STORE_SCHEMA_VERSION: u64 = 1;
```

Breaking changes require version bump and re-sync from genesis or snapshot.

## Threading Model

- Async operations use `tokio::task::spawn_blocking()` for I/O
- Background threads for FlatKeyValue generation and trie updates
- Channel-based coordination between workers

## Dependencies

- `ethrex-rlp` - RLP encoding
- `ethrex-common` - Core types
- `ethrex-trie` - Merkle Patricia Trie
- `rocksdb` (optional) - Persistent storage
- `lru` - Code cache
- `qfilter` - Bloom-like filter for trie cache
