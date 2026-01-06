# ethrex-blockchain

Core blockchain logic for the ethrex Ethereum client.

## Overview

This crate implements the blockchain layer, responsible for:

- **Block Validation**: Header validation, transaction execution, state root verification
- **State Management**: Account updates, storage changes, code deployments
- **Fork Choice**: Implements the LMD-GHOST fork choice rule
- **Mempool**: Transaction pool management with fee-based prioritization
- **Payload Building**: Block construction for validators

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                      Blockchain                              │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────┐ │
│  │   Mempool   │  │ Fork Choice │  │   Payload Builder   │ │
│  └─────────────┘  └─────────────┘  └─────────────────────┘ │
└─────────────────────────────────────────────────────────────┘
                            │
                            ▼
┌─────────────────────────────────────────────────────────────┐
│                         EVM                                  │
│              (ethrex-vm / LEVM)                             │
└─────────────────────────────────────────────────────────────┘
                            │
                            ▼
┌─────────────────────────────────────────────────────────────┐
│                        Storage                               │
│                    (ethrex-storage)                         │
└─────────────────────────────────────────────────────────────┘
```

## Block Execution Flow

1. **Header Validation**
   - Verify parent block exists
   - Check timestamp, gas limit, base fee
   - Validate fork-specific fields (withdrawals, blob gas, etc.)

2. **Transaction Execution**
   - Execute each transaction in the EVM
   - Collect receipts and logs
   - Track gas usage

3. **State Verification**
   - Compute new state root from account updates
   - Verify against header's state root
   - Verify receipts root and other roots

4. **Storage**
   - Store block header and body
   - Update state trie
   - Index transactions

## Usage

### Creating a Blockchain Instance

```rust
use ethrex_blockchain::{Blockchain, BlockchainOptions};
use ethrex_storage::Store;

let store = Store::new("./data", EngineType::RocksDB)?;
let blockchain = Blockchain::new(store, BlockchainOptions::default());
```

### Adding Blocks

```rust
// Add a single block
blockchain.add_block(&block)?;

// Add a batch of blocks (for sync)
blockchain.add_blocks(&blocks)?;
```

### Mempool Operations

```rust
// Add transaction to mempool
blockchain.add_transaction_to_mempool(tx).await?;

// Get pending transactions for block building
let txs = blockchain.mempool.get_transactions_by_tip(limit);

// Remove transactions after block inclusion
blockchain.mempool.remove_transactions(&included_txs);
```

### Payload Building

```rust
// Start building a payload (called by engine API)
let payload_id = blockchain.build_payload(
    parent_hash,
    timestamp,
    prev_randao,
    suggested_fee_recipient,
    withdrawals,
).await?;

// Get the built payload
let payload = blockchain.get_payload(payload_id).await?;
```

## Modules

- **`blockchain`**: Main blockchain implementation
- **`mempool`**: Transaction pool with priority queue
- **`fork_choice`**: Fork choice rule and head selection
- **`payload`**: Payload building for block production
- **`error`**: Error types for blockchain operations
- **`constants`**: Protocol constants
- **`tracing`**: Transaction tracing support
- **`vm`**: EVM integration layer

## Error Handling

The crate uses `ChainError` for blockchain-level errors:

```rust
pub enum ChainError {
    InvalidBlock(InvalidBlockError),  // Block validation failed
    ParentNotFound,                   // Missing parent block
    StoreError(StoreError),           // Database error
    EvmError(EvmError),               // EVM execution error
    // ...
}
```

## Fork Support

The blockchain automatically handles fork transitions based on chain config:

- **Homestead** through **London**: Legacy fee market
- **Paris (The Merge)**: Proof of Stake transition
- **Shanghai**: Withdrawals
- **Cancun**: EIP-4844 blob transactions
- **Prague**: EIP-7702 and additional improvements

## Features

- `metrics`: Enable Prometheus metrics collection
- `c-kzg`: Enable KZG commitment verification for blobs
