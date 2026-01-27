# On-Demand Ethereum L1 Block Builder - Implementation Plan

## Overview

This document outlines the implementation plan for an on-demand Ethereum L1 block builder. Unlike the existing interval-based block producer (`crates/blockchain/dev`), this builder creates blocks immediately when transactions arrive via JSON-RPC, without using a mempool.

## Key Differences from Current Block Producer

| Aspect | Current Block Producer | On-Demand Block Builder |
|--------|------------------------|-------------------------|
| Trigger | Time interval (e.g., every 12s) | Transaction arrival (default) or configurable interval |
| Mempool | Required | Not needed |
| Empty blocks | Built if no transactions | Never built (on-demand mode) |
| RPC logging | No | Yes, all calls logged |
| Architecture | Async loop with Engine API client | GenServer (spawned) |
| RPC response | N/A | Async (returns tx hash immediately) |

## Design Decisions (Confirmed)

1. **Block building mode**: On-demand by default, with optional `--block-time` flag for interval-based building
2. **Failed transactions**: Follow Ethereum behavior - include failed transactions in blocks with failure receipts
3. **RPC response**: Async like Ethereum - return tx hash immediately, don't wait for block inclusion
4. **Binary**: Separate binary first (`ethrex-dev`), designed as a library for later integration with ethrex `dev` subcommand
5. **Genesis**: Use LocalDevnet genesis with rich wallets

## Startup Banner

When the binary starts, it should display:

```
                _____ _____ _   _ ____  _______  __  ____  ______     __
               | ____|_   _| | | |  _ \| ____\ \/ / |  _ \| ____\ \   / /
               |  _|   | | | |_| | |_) |  _|  \  /  | | | |  _|  \ \ / /
               | |___  | | |  _  |  _ <| |___ /  \  | |_| | |___  \ V /
               |_____| |_| |_| |_|_| \_\_____/_/\_\ |____/|_____|  \_/


    https://github.com/lambdaclass/ethrex

Available Accounts
==================

(0) 0x00000a8d3f37af8def18832962ee008d8dca4f7b (1000000.000000000000000000 ETH)
(1) 0x00002132ce94eefb06eb15898c1aabd94feb0ac2 (1000000.000000000000000000 ETH)
(2) 0x000029bd811d292e7f1cf36c0fa08fd753c45074 (1000000.000000000000000000 ETH)
(3) 0x000036e0f87f8cd3e97f9cfdb2e4e5ff193c217a (1000000.000000000000000000 ETH)
(4) 0x000055acf237931902cebf4b905bf59813180555 (1000000.000000000000000000 ETH)
(5) 0x0000638374f7db166990bdc6abee884ee01a8920 (1000000.000000000000000000 ETH)
(6) 0x000086eeea461ca48e4d319f9789f3efd134e574 (1000000.000000000000000000 ETH)
(7) 0x00009074d8fc5eeb25f1548df05ad955e21fb08d (1000000.000000000000000000 ETH)
(8) 0x0000bbd19f707ca481886244bdd20bd6b8a81bd3e (1000000.000000000000000000 ETH)
(9) 0x0000e101815a78ebb9fbba34f4871ad32d5eb6cd (1000000.000000000000000000 ETH)

Private Keys
==================

(0) 0x941e103320615d394a55708be13e45994c7d93b932b064dbcb2b511fe3254e2e
(1) 0xbcdf20249abf0ed6d944c0288fad489e33f66b3960d9e6229c1cd214ed3bbe31
(2) 0x39725efee3fb28614de3bacaffe4cc4bd8c436257e2c8bb887c4b5c4be45e76d
(3) 0x53321db7c1e331d93a11a41d16f004d7ff63972ec8ec7c25db329728ceeb1710
(4) 0xab63b23eb7941c1251757e24b3d2350d2bc05c3c388d06f8fe6feafefb1e8c70
(5) 0x5d2344259f42259f82d2c140aa66102ba89b57b4883ee441a8b312622bd42491
(6) 0x27515f805127bebad2fb9b183508bdacb8c763da16f54e0678b16e8f28ef3fff
(7) 0x7ff1a4c1d57e5e784d327c4c7651e952350bc271f156afb3d00d20f5ef924856
(8) 0x3a91003acaf4c21b3953d94fa4a6db694fa69e5242b2e37be05dd82761058899
(9) 0xbb1d0f125b4fb2bb173c318cdead45468474ca71474e2247776b2b4c0fa2d3f5

Chain ID
==================

9

Base Fee
==================

1000000000

Gas Limit
==================

25000000

Genesis Timestamp
==================

1718040081

Listening on 127.0.0.1:8545
```

## Architecture

### High-Level Design

```
┌─────────────────────────────────────────────────────────────────┐
│                      JSON-RPC Server (Axum)                     │
│   - Logs all incoming RPC calls to console                      │
│   - Routes eth_sendRawTransaction to BlockBuilder               │
│   - Returns tx hash immediately (async)                         │
└───────────────────────────────┬─────────────────────────────────┘
                                │ Transaction (cast message)
                                ▼
┌─────────────────────────────────────────────────────────────────┐
│                  BlockBuilder (GenServer)                       │
│   - Receives transactions via cast messages (async)             │
│   - On-demand mode: builds block immediately per transaction    │
│   - Interval mode: collects txs, builds block on timer          │
│   - Executes block and updates state                            │
│   - Updates canonical chain head                                │
└───────────────────────────────┬─────────────────────────────────┘
                                │
                                ▼
┌─────────────────────────────────────────────────────────────────┐
│                         Store + Blockchain                      │
│   - In-memory storage (default) or persistent                   │
│   - Transaction execution via EVM                               │
└─────────────────────────────────────────────────────────────────┘
```

### Component Details

#### 1. BlockBuilder GenServer

The core component implemented as a `spawned` GenServer:

```rust
pub struct BlockBuilder {
    store: Store,
    blockchain: Arc<Blockchain>,
    coinbase: Address,
    chain_config: ChainConfig,
    block_time: Option<Duration>,  // None = on-demand, Some = interval mode
    pending_txs: Vec<(Transaction, Option<BlobsBundle>)>,  // For interval mode
}

pub enum CallMsg {
    /// Get the current block number
    GetBlockNumber,
    /// Get the current head block hash
    GetHeadBlockHash,
}

pub enum CastMsg {
    /// Submit a transaction (async, fire-and-forget)
    SubmitTransaction { tx: Transaction, blobs_bundle: Option<BlobsBundle> },
    /// Timer tick for interval mode
    BuildBlock,
}

pub enum OutMsg {
    /// Block number response
    BlockNumber(u64),
    /// Block hash response
    HeadBlockHash(H256),
    /// Error occurred
    Error(BlockBuilderError),
}
```

#### 2. Block Building Flow (On-Demand Mode)

When a transaction arrives:

1. **Validate Transaction**: Basic validation (signature, nonce, balance, gas)
2. **Create Payload**: Build block header with parent, timestamp, coinbase
3. **Execute Transaction**: Run transaction through EVM
4. **Handle Result**:
   - Success: Include transaction with success receipt
   - Failure: Include transaction with failure receipt (Ethereum behavior)
5. **Finalize Block**: Compute state root, receipts root, logs bloom
6. **Store Block**: Persist block and update canonical chain
7. **Apply Fork Choice**: Update head, safe, and finalized block references
8. **Log**: Log block creation to console

#### 3. Block Building Flow (Interval Mode)

When `--block-time` is specified:

1. **Collect Transactions**: Store incoming transactions in `pending_txs`
2. **Timer Fires**: Every `block_time` milliseconds
3. **Build Block**: If `pending_txs` is not empty:
   - Create payload
   - Execute all pending transactions
   - Handle success/failure per transaction
   - Finalize and store block
4. **Clear Pending**: Clear `pending_txs` after block is built
5. **Log**: Log block creation to console

#### 4. RPC Logging

All incoming RPC calls are logged to console:

```rust
// In RPC handler, before processing
tracing::info!(target: "rpc", ">> {}", method);
```

Example output:
```
>> eth_chainId
>> eth_getBalance
>> eth_sendRawTransaction
>> eth_getTransactionReceipt
```

## File Structure

```
crates/blockchain/block_builder/
├── Cargo.toml
├── lib.rs              # Library exports (for later integration)
├── builder.rs          # BlockBuilder GenServer implementation
├── error.rs            # Error types
├── banner.rs           # ASCII art and startup display
└── PLAN.md             # This document

cmd/ethrex_dev/
├── Cargo.toml
├── main.rs             # Binary entry point
└── cli.rs              # CLI argument parsing
```

## Implementation Steps

### Phase 1: Create Crate Structure

1. Create `crates/blockchain/block_builder/` directory
2. Create `Cargo.toml` with dependencies
3. Add crate to workspace `Cargo.toml`
4. Create basic module structure with lib.rs

### Phase 2: Implement BlockBuilder GenServer

1. Define state struct `BlockBuilder`
2. Define message types (`CallMsg`, `CastMsg`, `OutMsg`)
3. Define error types in `error.rs`
4. Implement `GenServer` trait:
   - `handle_call` for synchronous queries (block number, head hash)
   - `handle_cast` for async transaction submission
5. Implement helper methods:
   - `build_block(txs) -> Result<Block, Error>`
   - `validate_transaction(tx) -> Result<(), Error>`
   - `create_block_header(parent) -> BlockHeader`
   - `execute_transaction(block, tx) -> Receipt` (always returns receipt, success or failure)
   - `finalize_block(block, receipts) -> Block`
6. Implement interval mode timer logic

### Phase 3: Implement Startup Banner

1. Create `banner.rs` with ASCII art
2. Implement `display_banner()` function that shows:
   - ASCII art "ETHREX DEV"
   - Available accounts with balances (first 10)
   - Private keys (first 10)
   - Chain ID
   - Base fee
   - Gas limit
   - Genesis timestamp
   - Listening address

### Phase 4: Create Binary

1. Create `cmd/ethrex_dev/` directory
2. Implement CLI parsing with clap:
   ```
   ethrex-dev [OPTIONS]

   OPTIONS:
       --port <PORT>           RPC server port [default: 8545]
       --host <HOST>           RPC server host [default: 127.0.0.1]
       --block-time <MS>       Block time in milliseconds (enables interval mode)
       --coinbase <ADDRESS>    Coinbase address for block rewards
   ```
3. Initialize:
   - Store (in-memory)
   - Genesis from LocalDevnet
   - BlockBuilder GenServer
   - RPC server with logging enabled

### Phase 5: Integrate RPC Logging

1. Add logging middleware to RPC server
2. Log all method calls before processing
3. Create custom RPC handler for `eth_sendRawTransaction` that:
   - Logs the call
   - Sends transaction to BlockBuilder via cast (async)
   - Returns transaction hash immediately

### Phase 6: Testing

1. Unit tests for BlockBuilder logic
2. Integration tests using rex CLI (https://github.com/lambdaclass/rex):
   - Submit transaction via RPC
   - Verify block is created
   - Verify state is updated
   - Verify transaction receipt is available
3. Test both on-demand and interval modes

## Key Functions to Reuse

From `ethrex-blockchain`:
- `create_payload()` - Creates initial block from parent
- `Blockchain::add_block()` - Executes and stores block
- `apply_fork_choice()` - Updates canonical chain head

From `ethrex-vm`:
- `Evm::transact()` - Single transaction execution

From `ethrex-common/config/networks.rs`:
- `LOCAL_DEVNET_GENESIS_CONTENTS` - Genesis JSON
- `LOCAL_DEVNET_PRIVATE_KEYS` - Private keys file
- `Network::LocalDevnet` - Network configuration

## Dependencies

### Library crate (block_builder)

```toml
[dependencies]
ethrex-blockchain.workspace = true
ethrex-common.workspace = true
ethrex-storage.workspace = true
ethrex-vm.workspace = true
ethrex-crypto.workspace = true

spawned-concurrency.workspace = true
tokio = { workspace = true, features = ["rt", "time", "sync"] }
tracing.workspace = true
thiserror.workspace = true
ethereum-types.workspace = true
bytes.workspace = true
```

### Binary crate (ethrex_dev)

```toml
[dependencies]
ethrex-block-builder = { path = "../../crates/blockchain/block_builder" }
ethrex-rpc.workspace = true
ethrex-common.workspace = true
ethrex-storage.workspace = true

spawned-rt.workspace = true
clap = { version = "4", features = ["derive"] }
tokio = { workspace = true, features = ["full"] }
tracing.workspace = true
tracing-subscriber.workspace = true
```

## Success Criteria

1. Running `ethrex-dev` displays the ASCII banner with accounts and keys
2. All RPC calls are logged to console (e.g., `>> eth_sendRawTransaction`)
3. In on-demand mode: transactions submitted via `eth_sendRawTransaction` result in immediate block creation
4. In interval mode (with `--block-time`): blocks are created at specified intervals
5. Failed transactions are included in blocks with failure receipts (Ethereum behavior)
6. RPC returns transaction hash immediately (async, doesn't wait for block)
7. State is correctly updated after each block
8. Transaction receipts are available after block is created
9. The builder is implemented as a proper GenServer using spawned best practices
10. Library design allows easy integration into ethrex `dev` subcommand later
11. Tests pass using rex CLI

## Notes

- Do NOT use any Paradigm tools (anvil, forge, cast, reth) for code reference
- Use rex CLI (https://github.com/lambdaclass/rex) for testing
- Genesis chain ID is 9 (LocalDevnet)
- Rich wallets have balance `0x33b2e3c9fd0803ce8000000` (1,000,000 ETH in wei)
