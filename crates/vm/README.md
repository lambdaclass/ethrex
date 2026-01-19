# ethrex-vm

High-level EVM execution layer for the ethrex Ethereum client.

## Overview

This crate provides a high-level abstraction over the LEVM (Lambda EVM) execution engine. It wraps LEVM with additional functionality for block execution, state management, witness generation for zkVM proving, and system contract handling.

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                      ethrex-vm (this crate)                     │
├─────────────────────────────────────────────────────────────────┤
│  ┌──────────────────────────────────────────────────────────┐   │
│  │              Evm (public API)                            │   │
│  │  - execute_block() → BlockExecutionResult               │   │
│  │  - execute_tx() → (Receipt, gas_used)                   │   │
│  │  - simulate_tx_from_generic() → ExecutionResult         │   │
│  └──────────────────────────────────────────────────────────┘   │
│                              │                                   │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │     Database Adapters (VmDatabase trait)                 │   │
│  │  - StoreVmDatabase (from persistent storage)            │   │
│  │  - GuestProgramStateWrapper (for zkVM witness)          │   │
│  └──────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                    ethrex-levm (embedded EVM)                   │
│  - 179 opcodes, 19 precompiles                                  │
│  - Gas accounting, hooks system                                 │
│  - L1/L2 execution modes                                        │
└─────────────────────────────────────────────────────────────────┘
```

## Quick Start

```rust
use ethrex_vm::{Evm, VmDatabase, BlockExecutionResult, ExecutionResult};
use ethrex_levm::VMType;

// Create EVM with database
let evm = Evm::new_for_l1(db)?;

// Execute a full block
let result: BlockExecutionResult = evm.execute_block(&block, &header)?;
println!("Receipts: {:?}", result.receipts);

// Or execute a single transaction
let (receipt, gas_used) = evm.execute_tx(&tx, &sender, &header)?;

// Or simulate without state changes
let result: ExecutionResult = evm.simulate_tx_from_generic(&tx, &header)?;
```

## Core Types

### Evm

The main execution engine wrapping LEVM:

```rust
pub struct Evm {
    pub db: GeneralizedDatabase,
    pub vm_type: VMType,
}
```

**Methods:**
- `new_for_l1(db)` / `new_for_l2(db, fee_config)` - Create EVM instance
- `execute_block(block, header)` - Execute full block
- `execute_tx(tx, sender, header)` - Execute single transaction
- `simulate_tx_from_generic(tx, header)` - Simulate without state changes
- `trace_tx_calls(tx, sender, header)` - Get call trace

### VmDatabase

Trait for state access:

```rust
pub trait VmDatabase: Send + Sync + DynClone {
    fn get_account_state(&self, address: Address) -> Result<Option<AccountState>, EvmError>;
    fn get_storage_slot(&self, address: Address, key: H256) -> Result<Option<U256>, EvmError>;
    fn get_block_hash(&self, block_number: u64) -> Result<H256, EvmError>;
    fn get_chain_config(&self) -> Result<ChainConfig, EvmError>;
    fn get_account_code(&self, code_hash: H256) -> Result<Code, EvmError>;
}
```

### ExecutionResult

Transaction execution outcome:

```rust
pub enum ExecutionResult {
    Success { gas_used: u64, gas_refunded: u64, logs: Vec<Log>, output: Bytes },
    Revert { gas_used: u64, output: Bytes },
    Halt { reason: String, gas_used: u64 },
}
```

### BlockExecutionResult

Block execution outcome:

```rust
pub struct BlockExecutionResult {
    pub receipts: Vec<Receipt>,
    pub requests: Vec<Requests>,  // EIP-7002, EIP-7251
}
```

## Module Structure

| Module | Description |
|--------|-------------|
| `backends` | EVM backend implementations (LEVM wrapper) |
| `db` | `VmDatabase` trait and `DynVmDatabase` boxed type |
| `errors` | `EvmError` type |
| `execution_result` | `ExecutionResult` enum |
| `witness_db` | `GuestProgramStateWrapper` for zkVM |
| `system_contracts` | System contract addresses by fork |
| `tracing` | Call tracing support |

## System Contracts

Prague-era system contracts (from `system_contracts.rs`):

| Contract | Address | Fork |
|----------|---------|------|
| Beacon Roots | `0x000F3df6D732807Ef1319fB7B8bB8522d0Beac02` | Cancun |
| History Storage | `0x0000F90827F1C53a10cb7A02335B175320002935` | Prague |
| Deposit Contract | `0x00000000219ab540356cBB839Cbe05303d7705Fa` | Prague |
| Withdrawal Request | `0x0c15F14308530b7CDB8460094BbB9cC28b9AaaAA` | Prague |
| Consolidation Request | `0x00431F263cE400f4455c2dCf564e53007Ca4bbBb` | Prague |

## Feature Flags

| Feature | Description | Default |
|---------|-------------|---------|
| `secp256k1` | Production ECDSA library | Yes |
| `c-kzg` | C KZG implementation for EIP-4844 | No |
| `sp1` | Succinct SP1 zkVM support | No |
| `risc0` | RISC Zero zkVM support | No |
| `zisk` | Polygon ZisK zkVM support | No |
| `openvm` | OpenVM zkVM support | No |
| `perf_opcode_timings` | Per-opcode timing metrics | No |
| `debug` | LEVM debug mode | No |

## Witness Generation

For zkVM proving, use `GuestProgramStateWrapper`:

```rust
use ethrex_vm::GuestProgramStateWrapper;

// Create wrapper with witness state
let wrapper = GuestProgramStateWrapper::new(witness_state);

// Use as VmDatabase
let evm = Evm::new_for_l1(Box::new(wrapper.clone()))?;

// Execute block - state changes recorded in wrapper
let result = evm.execute_block(&block, &header)?;

// Extract state transitions for proof generation
let root = wrapper.state_trie_root();
```

## Error Handling

```rust
pub enum EvmError {
    Transaction(String),              // Invalid transaction
    Header(String),                   // Invalid block header
    DB(String),                       // Database error
    Precompile(String),               // Precompile execution error
    InvalidEVM(String),               // Wrong VM type (L1 vs L2)
    Custom(String),                   // Generic error
    InvalidDepositRequest,            // EIP-6110 deposit parsing
    SystemContractCallFailed(String), // EIP-7002/7251 failure
}
```

## L1 vs L2 Execution

```rust
use ethrex_levm::{VMType, FeeConfig};

// L1 execution
let evm = Evm::new_for_l1(db)?;

// L2 execution with fee configuration
let fee_config = FeeConfig { /* ... */ };
let evm = Evm::new_for_l2(db, fee_config)?;
```

The `VMType` determines:
- Hook selection (L1 vs L2 hooks)
- Fee calculation logic
- System contract calls (L1 only)

## Dependencies

- `ethrex-levm` - Embedded EVM implementation
- `ethrex-common` - Core types
- `ethrex-trie` - Merkle Patricia Trie
- `ethrex-rlp` - RLP encoding
- `dyn-clone` - Clonable trait objects

## Re-exports

```rust
pub use ethrex_levm::precompiles::precompiles_for_fork;
```

Get the set of precompile addresses available for a given fork.
