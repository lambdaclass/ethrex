# ethrex-blockchain

Core blockchain logic for the ethrex Ethereum client.

## Overview

This crate implements the blockchain layer responsible for block validation, execution, state management, transaction mempool, and block building for validators. It is designed as a post-merge Proof-of-Stake client.

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                         Blockchain                              │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────────┐ │
│  │   Mempool   │  │  Payloads   │  │    Block Validation     │ │
│  └─────────────┘  └─────────────┘  └─────────────────────────┘ │
└─────────────────────────────────────────────────────────────────┘
                              │
              ┌───────────────┼───────────────┐
              ▼               ▼               ▼
        ┌──────────┐   ┌──────────┐   ┌──────────────┐
        │  Store   │   │   EVM    │   │  Fork Choice │
        └──────────┘   └──────────┘   └──────────────┘
```

## Quick Start

```rust
use ethrex_blockchain::{Blockchain, BlockchainOptions};

// Create blockchain instance
let blockchain = Blockchain::new(store, BlockchainOptions::default());

// Add a block
blockchain.add_block(&block).await?;

// Add transaction to mempool
blockchain.mempool.add_transaction(hash, sender, tx);

// Build a block for validators
let payload = blockchain.build_payload(block)?;
```

## Core Types

### Blockchain

Main interface for blockchain operations:

```rust
pub struct Blockchain {
    pub mempool: Mempool,
    pub options: BlockchainOptions,
    pub payloads: Arc<TokioMutex<Vec<(u64, PayloadOrTask)>>>,
    storage: Store,
    is_synced: AtomicBool,
}
```

**Key methods:**
- `add_block(block)` - Validate and execute a single block
- `add_block_pipeline(block)` - Execute with concurrent merkleization
- `add_blocks_in_batch(blocks)` - Batch processing for sync
- `build_payload(block)` - Build block for validators
- `validate_transaction(tx, header)` - Validate mempool transaction

### Mempool

Transaction pool with sender-nonce indexing:

```rust
pub struct Mempool {
    inner: RwLock<MempoolInner>,
}
```

**Methods:**
- `add_transaction(hash, sender, tx)` - Add transaction
- `filter_transactions(filter)` - Get sorted transactions by sender
- `get_nonce(address)` - Get highest nonce for sender
- `remove_transaction(hash)` - Remove from pool

### BlockchainOptions

```rust
pub struct BlockchainOptions {
    pub max_mempool_size: usize,       // Default: 10,000
    pub perf_logs_enabled: bool,       // Emit performance metrics
    pub r#type: BlockchainType,        // L1 or L2
    pub max_blobs_per_block: Option<u32>,
    pub precompute_witnesses: bool,    // Generate zkVM witnesses
}
```

### BlockchainType

```rust
pub enum BlockchainType {
    L1,              // Ethereum mainnet/testnet
    L2(L2Config),    // Rollup with fee configuration
}
```

## Module Structure

| Module | Description |
|--------|-------------|
| `error` | `ChainError`, `MempoolError`, `InvalidBlockError` |
| `fork_choice` | Fork choice rule implementation |
| `mempool` | Transaction pool management |
| `payload` | Block building for validators |
| `constants` | Yellow Paper gas constants |
| `vm` | `StoreVmDatabase` adapter |
| `tracing` | Transaction call tracing |
| `metrics/` | Prometheus metrics (11 submodules) |

## Block Execution Flow

```
1. Receive block from consensus/P2P
2. Validate block header
   - Parent hash, timestamp, gas limit
   - Base fee calculation (EIP-1559)
   - Blob gas (EIP-4844)
3. Execute transactions in EVM
4. Validate post-execution state
   - Gas used matches header
   - Receipts root matches
   - Requests hash matches (Prague)
   - State root matches
5. Apply state updates (merkleization)
6. Store block and update canonical chain
```

## Mempool Transaction Validation

Transactions are validated against:

1. **Static checks** - Init code size, data size, gas limit cap
2. **Block header** - Gas limit, fee requirements
3. **Account state** - Nonce, balance, chain ID
4. **Mempool conflicts** - Replacement fee requirements

## Payload Building

For block validators:

```rust
// Initiate async payload build
blockchain.initiate_payload_build(block, payload_id).await;

// Later, retrieve the built payload
let result = blockchain.get_payload(payload_id).await?;

// Result contains:
// - Block with transactions
// - Blobs bundle
// - Block value (tips + MEV)
// - Receipts
// - State updates
```

**Build process:**
1. Apply system operations (beacon root, block hash contracts)
2. Apply withdrawals (post-Shanghai)
3. Fill transactions from mempool (by fee priority)
4. Extract requests (Prague+)
5. Compute roots and finalize header

## Fork Choice

```rust
use ethrex_blockchain::fork_choice::apply_fork_choice;

apply_fork_choice(&store, head_hash, safe_hash, finalized_hash).await?;
```

**Validates:**
- Blocks exist and are connected
- Ordering: finalized ≤ safe ≤ head
- State is reachable for potential reorg

## Feature Flags

| Feature | Description | Default |
|---------|-------------|---------|
| `secp256k1` | ECDSA signature verification | Yes |
| `c-kzg` | KZG commitments (EIP-4844) | No |
| `metrics` | Prometheus metrics collection | No |
| `sp1` | Succinct SP1 zkVM support | No |
| `risc0` | RISC Zero zkVM support | No |
| `zisk` | Polygon ZisK zkVM support | No |

## Execution Modes

| Method | Use Case | Performance |
|--------|----------|-------------|
| `add_block()` | Single blocks, RPC | Baseline |
| `add_block_pipeline()` | New blocks from consensus | Concurrent merkleization |
| `add_blocks_in_batch()` | Sync batches | 2-3x throughput |

## Witness Generation

For zkVM proving:

```rust
let options = BlockchainOptions {
    precompute_witnesses: true,
    ..Default::default()
};

// Witnesses stored in database after block execution
let witness = store.get_witness_by_number_and_hash(number, hash)?;
```

## Constants

### Gas Costs (Yellow Paper)

| Constant | Value |
|----------|-------|
| `TX_GAS_COST` | 21,000 |
| `TX_CREATE_GAS_COST` | 53,000 |
| `TX_DATA_ZERO_GAS_COST` | 4 |
| `TX_DATA_NON_ZERO_GAS_EIP2028` | 16 |
| `MAX_CODE_SIZE` | 24,576 bytes |
| `MAX_INITCODE_SIZE` | 49,152 bytes |

## Supported Forks

Post-merge forks only:
- Paris (The Merge)
- Shanghai (Withdrawals)
- Cancun (EIP-4844 Blobs)
- Prague (Deposit/Exit requests)
- Osaka (Gas limit increase)

## Error Handling

```rust
pub enum ChainError {
    ParentNotFound,
    ParentStateNotFound,
    InvalidBlock(InvalidBlockError),
    EvmError(EvmError),
    StoreError(StoreError),
    // ...
}

pub enum MempoolError {
    NoBlockHeader,
    TxGasLimitExceeded,
    InsufficientFunds,
    NonceTooLow,
    // ...
}
```

## Dependencies

- `ethrex-storage` - Persistent storage
- `ethrex-vm` - EVM execution
- `ethrex-trie` - Merkle Patricia Trie
- `ethrex-common` - Core types
- `ethrex-metrics` - Prometheus integration
