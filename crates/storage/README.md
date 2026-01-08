# ethrex-storage

Persistent storage layer for the ethrex Ethereum client.

For detailed API documentation, see the rustdocs:
```bash
cargo doc --package ethrex-storage --open
```

## Quick Start

```rust
use ethrex_storage::{Store, EngineType};

// Create with RocksDB backend
let store = Store::new("./data", EngineType::RocksDB)?;

// Add a block
store.add_block(block).await?;

// Query account
let info = store.get_account_info(block_number, address)?;
```

## Features

- `rocksdb`: Enable RocksDB backend for persistent storage (default is in-memory)
