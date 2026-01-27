# ethrex-storage

Persistent storage layer for the ethrex Ethereum client.

For detailed API documentation, see the rustdocs:
```bash
cargo doc --package ethrex-storage --open
```

## Quick Start

```rust
use ethrex_storage::{Store, EngineType};

// Create with RocksDB backend (default persistent storage)
let store = Store::new("./data", EngineType::RocksDB)?;

// Create with ethrex_db hybrid backend (optimized for state tries)
let store = Store::new("./data", EngineType::EthrexDb)?;

// Create with in-memory backend (for testing)
let store = Store::new("", EngineType::InMemory)?;

// Add a block
store.add_block(block).await?;

// Query account
let info = store.get_account_info(block_number, address)?;
```

## Features

| Feature | Description |
|---------|-------------|
| `rocksdb` | Enable RocksDB backend for persistent storage |
| `ethrex-db` | Enable hybrid ethrex_db + RocksDB backend (optimized state storage) |

Enable features in your `Cargo.toml`:
```toml
[dependencies]
ethrex-storage = { version = "...", features = ["rocksdb"] }
# or for the optimized hybrid backend:
ethrex-storage = { version = "...", features = ["ethrex-db"] }
```

Note: The `ethrex-db` feature automatically enables `rocksdb` as it uses RocksDB for auxiliary data.

## Storage Backends

### In-Memory Backend

Best for testing. All data is lost when the process exits.

```rust
let store = Store::new("", EngineType::InMemory)?;
```

### RocksDB Backend

Traditional LSM-tree based storage. Good general-purpose backend.

```rust
let store = Store::new("./data", EngineType::RocksDB)?;
```

**File structure:**
```
./data/
├── db/              # RocksDB data files
└── metadata.json    # Schema version
```

### ethrex_db Hybrid Backend

Optimized storage using [ethrex_db](https://github.com/lambdaclass/ethrex-db) for state/storage tries and RocksDB for other blockchain data.

```rust
let store = Store::new("./data", EngineType::EthrexDb)?;

// Check if using ethrex_db backend
if store.uses_ethrex_db() {
    // Access the underlying blockchain for direct state operations
    let blockchain_ref = store.ethrex_blockchain();
}
```

**File structure:**
```
./data/
├── state.db         # ethrex_db PagedDb (memory-mapped state storage)
├── auxiliary/       # RocksDB (blocks, headers, receipts, etc.)
└── metadata.json    # Schema version
```

**Performance characteristics:**
- 10-15x faster state lookups (flat key-value vs MPT traversal)
- 1.5-2x faster state inserts (no LSM write amplification)
- 30-50% disk reduction (no LSM overhead for state data)
- Copy-on-Write concurrency (single writer, multiple lock-free readers)

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                           Store                                  │
│  - High-level API for blockchain data operations                │
│  - Manages caches (TrieLayerCache, CodeCache)                   │
│  - Handles forkchoice updates and finalization                  │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                      StorageBackend Trait                        │
│  - begin_read() → StorageReadView                               │
│  - begin_write() → StorageWriteBatch                            │
│  - begin_locked() → StorageLockedView                           │
└─────────────────────────────────────────────────────────────────┘
          │                   │                    │
          ▼                   ▼                    ▼
┌─────────────┐    ┌─────────────────┐    ┌─────────────────────┐
│  InMemory   │    │    RocksDB      │    │   EthrexDb Hybrid   │
│   Backend   │    │    Backend      │    │      Backend        │
│             │    │                 │    │                     │
│ HashMap-    │    │ LSM-tree        │    │ ┌─────────────────┐ │
│ based       │    │ storage         │    │ │   ethrex_db     │ │
│             │    │                 │    │ │  (state tries)  │ │
│             │    │                 │    │ └─────────────────┘ │
│             │    │                 │    │ ┌─────────────────┐ │
│             │    │                 │    │ │    RocksDB      │ │
│             │    │                 │    │ │  (auxiliary)    │ │
│             │    │                 │    │ └─────────────────┘ │
└─────────────┘    └─────────────────┘    └─────────────────────┘
```

### Table Routing (Hybrid Backend)

The hybrid backend routes tables to different storage engines:

| Storage Engine | Tables |
|---------------|--------|
| **ethrex_db** | `ACCOUNT_TRIE_NODES`, `STORAGE_TRIE_NODES`, `ACCOUNT_FLATKEYVALUE`, `STORAGE_FLATKEYVALUE` |
| **RocksDB** | `HEADERS`, `BODIES`, `RECEIPTS`, `CANONICAL_BLOCK_HASHES`, `BLOCK_NUMBERS`, `TRANSACTION_LOCATIONS`, `ACCOUNT_CODES`, `CHAIN_DATA`, `SNAP_STATE`, `PENDING_BLOCKS`, `INVALID_CHAINS`, `FULLSYNC_HEADERS`, `MISC_VALUES`, `EXECUTION_WITNESSES` |

### ethrex_db Architecture

ethrex_db uses a two-tier storage model:

```
┌─────────────────────────────────────────────────────────────────┐
│                        Blockchain                                │
│  - Hot storage for unfinalized blocks                           │
│  - Copy-on-Write concurrency                                    │
│  - Native Fork Choice support                                   │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼ finalize()
┌─────────────────────────────────────────────────────────────────┐
│                         PagedDb                                  │
│  - Cold storage for finalized state                             │
│  - Memory-mapped 4KB pages                                      │
│  - PagedStateTrie for account/storage data                      │
└─────────────────────────────────────────────────────────────────┘
```

## Caching

The Store maintains several caches for performance:

| Cache | Size | Purpose |
|-------|------|---------|
| **TrieLayerCache** | 128 blocks | Recent trie nodes for fast state access |
| **CodeCache** | 64 MB LRU | Contract bytecode by code hash |
| **LatestBlockHeader** | 1 entry | Cached latest block for RPC queries |

## Usage Examples

### Adding Genesis State

```rust
use ethrex_storage::{Store, EngineType};
use ethrex_common::types::Genesis;

let mut store = Store::new("./data", EngineType::EthrexDb)?;
let genesis: Genesis = serde_json::from_str(genesis_json)?;
store.add_initial_state(genesis).await?;
```

### Building a Chain

```rust
// Add blocks to the chain
for block in blocks {
    let block_hash = block.hash();
    let block_number = block.header.number;

    store.add_block(block).await?;

    // Update canonical chain via forkchoice
    store.forkchoice_update(
        vec![(block_number, block_hash)],
        block_number,
        block_hash,
        None,  // safe block
        None,  // finalized block
    ).await?;
}
```

### Querying State

```rust
// Get account info at a specific block
let account = store.get_account_info(block_number, address)?;

// Get storage value
let value = store.get_storage_at(block_number, address, slot)?;

// Get block header
let header = store.get_block_header(block_number)?;

// Get block by hash
let block = store.get_block_by_hash(hash)?;
```

### Concurrent Access

The Store is `Clone` and thread-safe:

```rust
let store = Arc::new(store);

// Spawn reader threads
for _ in 0..4 {
    let store = Arc::clone(&store);
    thread::spawn(move || {
        // Safe concurrent reads
        let header = store.get_block_header(0)?;
    });
}
```

## Testing

Run tests with different backends:

```bash
# Run all tests with RocksDB
cargo test --package ethrex-storage --features rocksdb

# Run all tests with ethrex_db hybrid backend
cargo test --package ethrex-storage --features ethrex-db

# Run specific test file
cargo test --package ethrex-storage --features ethrex-db --test ethrex_db_store_tests
```

## Configuration

### Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `ETHREX_DATA_DIR` | Data directory path | `./data` |

### Feature Combinations

| Build | Features | Use Case |
|-------|----------|----------|
| Testing | (none) | In-memory only |
| Production | `rocksdb` | Standard persistent storage |
| Optimized | `ethrex-db` | High-performance state storage |
