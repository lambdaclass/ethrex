# ethrex-blockchain

Core blockchain logic for the ethrex Ethereum client.

For detailed API documentation, see the rustdocs:
```bash
cargo doc --package ethrex-blockchain --open
```

## Quick Start

```rust
use ethrex_blockchain::Blockchain;

let blockchain = Blockchain::new(store, BlockchainOptions::default());

// Add a block
blockchain.add_block(&block)?;

// Add transaction to mempool
blockchain.add_transaction_to_mempool(tx).await?;
```

## Features

- `metrics`: Enable Prometheus metrics collection

## Notes

ethrex is a post-merge client and does not support pre-merge (PoW) forks.
