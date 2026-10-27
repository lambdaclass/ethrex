# ethrex-vm

High-level EVM execution layer for the ethrex Ethereum client.

## Overview

This crate provides a high-level abstraction over LEVM (Lambda EVM), wrapping the low-level EVM execution engine with additional functionality for:

- Block and transaction execution
- State management via the `VmDatabase` trait
- Witness generation for zkVM proving
- System contract handling (EIP-7002, EIP-7251)

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                         Evm                                  │
│            (High-level execution interface)                  │
└─────────────────────────────────────────────────────────────┘
                             │
                             ▼
┌─────────────────────────────────────────────────────────────┐
│                    LEVM (ethrex-levm)                        │
│              (Low-level EVM execution engine)                │
└─────────────────────────────────────────────────────────────┘
                             │
                             ▼
┌─────────────────────────────────────────────────────────────┐
│                     VmDatabase                               │
│        (Account state, storage, code access)                 │
└─────────────────────────────────────────────────────────────┘
```

## Quick Start

```rust
use ethrex_vm::{Evm, BlockExecutionResult, ExecutionResult};
use ethrex_common::types::{Block, BlockHeader, Transaction};

// Create EVM with database
let evm = Evm::new_for_l1(db)?;

// Execute a full block
let result: BlockExecutionResult = evm.execute_block(&block, &header)?;
println!("Receipts: {:?}", result.receipts);
println!("Requests: {:?}", result.requests);

// Or simulate a single transaction
let result: ExecutionResult = evm.simulate_tx_from_generic(&tx, &header)?;
match result {
    ExecutionResult::Success { output, gas_used, .. } => {
        println!("Success! Gas used: {}", gas_used);
    }
    ExecutionResult::Revert { output, gas_used } => {
        println!("Reverted: {:?}", output);
    }
    ExecutionResult::Halt { reason, gas_used } => {
        println!("Halted: {:?}", reason);
    }
}
```

## Core Types

### Evm

The main execution interface:

```rust
pub struct Evm {
    // Contains state database and configuration
}

impl Evm {
    // Create for L1 (standard Ethereum)
    pub fn new_for_l1(db: impl VmDatabase) -> Result<Self, EvmError>;

    // Create for L2 with custom hooks
    pub fn new_for_l2(db: impl VmDatabase) -> Result<Self, EvmError>;

    // Execute an entire block
    pub fn execute_block(&self, block: &Block, header: &BlockHeader)
        -> Result<BlockExecutionResult, EvmError>;

    // Simulate a single transaction
    pub fn simulate_tx_from_generic(&self, tx: &Transaction, header: &BlockHeader)
        -> Result<ExecutionResult, EvmError>;
}
```

### VmDatabase

Trait for state access (implemented by `Store`):

```rust
pub trait VmDatabase: Send + Sync {
    fn get_account_state(&self, address: Address) -> Result<AccountState, DatabaseError>;
    fn get_storage_value(&self, address: Address, key: H256) -> Result<U256, DatabaseError>;
    fn get_block_hash(&self, block_number: u64) -> Result<H256, DatabaseError>;
    fn get_chain_config(&self) -> Result<ChainConfig, DatabaseError>;
    fn get_account_code(&self, code_hash: H256) -> Result<Code, DatabaseError>;
}
```

### ExecutionResult

Transaction execution outcome:

```rust
pub enum ExecutionResult {
    Success {
        reason: SuccessReason,
        gas_used: u64,
        gas_refunded: u64,
        output: Bytes,
        logs: Vec<Log>,
    },
    Revert {
        gas_used: u64,
        output: Bytes,
    },
    Halt {
        reason: ExceptionalHalt,
        gas_used: u64,
    },
}
```

### BlockExecutionResult

Result of executing an entire block:

```rust
pub struct BlockExecutionResult {
    pub receipts: Vec<Receipt>,
    pub requests: Vec<Request>,  // EIP-7685 requests
    pub gas_used: u64,
}
```

### GuestProgramStateWrapper

Thread-safe wrapper for zkVM witness state:

```rust
pub struct GuestProgramStateWrapper(Arc<Mutex<GuestProgramState>>);

impl GuestProgramStateWrapper {
    pub fn new() -> Self;
    pub fn get_state(&self) -> MutexGuard<GuestProgramState>;
}
```

Used to collect execution witnesses during proving.

## Module Structure

| Module | Description |
|--------|-------------|
| `backends` | EVM backend implementations (LEVM wrapper) |
| `db` | `VmDatabase` trait and `DynVmDatabase` wrapper |
| `errors` | `EvmError` type |
| `execution_result` | `ExecutionResult` and `BlockExecutionResult` |
| `witness_db` | `GuestProgramStateWrapper` for zkVM |
| `system_contracts` | System contract addresses by fork |
| `tracing` | Call tracing support |

## System Contracts

### Prague Fork

| Address | Name | Purpose |
|---------|------|---------|
| `0x00000000219ab540356cBB839Cbe05303d7705Fa` | Beacon Deposit | ETH deposits to beacon chain |
| `0x0c15F14308530b7CDB8460094BbB9cC28b9AaaAA` | EIP-7002 | Validator withdrawal requests |
| `0x0d92049a23a29193cf4DB30305A2d93E91FFE9F0` | EIP-7251 | Consolidation requests |

### Osaka Fork

Additional system contracts for future upgrades.

## Feature Flags

| Feature | Description | Default |
|---------|-------------|---------|
| `secp256k1` | Production ECDSA library | Yes |
| `c-kzg` | C KZG implementation for EIP-4844 | No |
| `sp1` | Succinct SP1 zkVM support | No |
| `risc0` | RISC Zero zkVM support | No |
| `zisk` | Polygon ZisK zkVM support | No |
| `openvm` | OpenVM zkVM support | No |

## Execution Flow

### Block Execution

1. Validate block header
2. For each transaction:
   - Validate transaction (signature, nonce, gas)
   - Execute transaction via LEVM
   - Generate receipt
   - Apply state changes
3. Process system contract requests (EIP-7685)
4. Return receipts and requests

### Transaction Simulation

1. Create environment from header
2. Initialize VM state from database
3. Execute transaction without committing
4. Return execution result

## Error Handling

```rust
pub enum EvmError {
    Database(DatabaseError),
    Execution(VMError),
    Block(BlockValidationError),
    Transaction(TxValidationError),
}
```

## Integration with ethrex

This crate is used by:
- **ethrex-blockchain**: Block validation and execution
- **ethrex-rpc**: Transaction simulation (`eth_call`, `eth_estimateGas`)
- **ethrex-prover**: Generating execution witnesses for zkVM

## Dependencies

- `ethrex-levm` - Low-level EVM implementation
- `ethrex-common` - Core types
- `ethrex-rlp` - RLP encoding
