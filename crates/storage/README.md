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

## Deep-reorg cap and metrics

By default the maximum reorg depth is finality-bounded: ethrex accepts any reorg
up to the distance between the current head and the last finalized block (or the
lowest journal entry when no finalized block is known). To restrict this further,
pass `--max-reorg-depth <N>` on the CLI. `--max-reorg-depth 0` disables deep reorgs
entirely and restores the pre-4-PR-stack behavior.

The following Prometheus metrics are exposed under `ethrex_reorg_*` /
`ethrex_deep_reorg_*`:

| Metric | Type | Description |
|--------|------|-------------|
| `ethrex_reorg_overlay_entries` | Gauge | Entries in the installed overlay (0 when idle). |
| `ethrex_reorg_overlay_bytes` | Gauge | Byte size of overlay key+value data (0 when idle). |
| `ethrex_reorg_journal_length` | Gauge | Span of the `STATE_HISTORY` column family. |
| `ethrex_reorg_depth` | Histogram | Distribution of attempted reorg depths. |
| `ethrex_reorg_reconcile_duration_seconds` | Histogram | First-commit reconciliation latency. |
| `ethrex_deep_reorg_attempts_total` | Counter | Deep reorgs initiated. |
| `ethrex_deep_reorg_success_total` | Counter | Deep reorgs completed successfully. |
| `ethrex_deep_reorg_aborts_total` | Counter | Deep reorgs that aborted. |
