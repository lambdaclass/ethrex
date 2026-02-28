# LEVM (Lambda EVM)

A pure Rust implementation of the Ethereum Virtual Machine for the ethrex client.

## Overview

LEVM (Lambda EVM) is ethrex's native EVM implementation, designed for:
- **Correctness**: Full compatibility with Ethereum consensus tests
- **Performance**: Optimized opcode execution and memory management
- **Readability**: Clean, well-documented Rust code
- **Extensibility**: Modular design with hooks for L1/L2 customization

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                           VM                                 │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────┐ │
│  │  CallFrame  │  │   Memory    │  │       Stack         │ │
│  └─────────────┘  └─────────────┘  └─────────────────────┘ │
│                                                             │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────┐ │
│  │  Substate   │  │ Precompiles │  │   Environment       │ │
│  └─────────────┘  └─────────────┘  └─────────────────────┘ │
└─────────────────────────────────────────────────────────────┘
                             │
                             ▼
┌─────────────────────────────────────────────────────────────┐
│                    GeneralizedDatabase                       │
│              (Account state, storage, code)                  │
└─────────────────────────────────────────────────────────────┘
```

## Quick Start

```rust
use ethrex_levm::{VM, Environment, VMType};
use ethrex_levm::db::GeneralizedDatabase;

// Create VM with database and environment
let mut vm = VM::new(env, &mut db, &tx, tracer, debug_mode, vm_type)?;

// Execute the transaction
let report = vm.execute()?;

// Check execution result
if report.result.is_success() {
    println!("Gas used: {}", report.gas_used);
    println!("Output: {:?}", report.output);
}
```

## Supported Forks

| Fork | Status | Key Features |
|------|--------|--------------|
| Osaka | Supported | Future optimizations |
| Prague | Supported | BLS12-381 precompiles |
| Cancun | Supported | KZG, MCOPY, TLOAD/TSTORE |
| Shanghai | Supported | PUSH0 opcode |
| Paris (Merge) | Supported | Post-merge baseline |

Note: ethrex is a post-merge client and does not support pre-merge forks.

## Core Types

### VM

The main execution engine:
```rust
pub struct VM<'a> {
    pub call_frames: Vec<CallFrame>,      // Call stack
    pub current_call_frame: CallFrame,    // Active frame
    pub env: Environment,                 // Block/tx context
    pub substate: Substate,               // Execution state
    pub db: &'a mut GeneralizedDatabase,  // Account state
    // ...
}
```

### CallFrame

Execution context for each call/create:
- Stack (1024 items max)
- Memory (dynamically expanding)
- Program counter and bytecode
- Gas accounting
- Call data and return data

### Environment

Block and transaction context:
- Block number, timestamp, coinbase
- Base fee, blob base fee
- Transaction origin and gas limit
- Fork configuration

### Substate

Tracks execution state for reverting:
- Accessed addresses/storage (EIP-2929)
- Self-destruct set
- Created accounts
- Transient storage (EIP-1153)
- Event logs

## Module Structure

| Module | Description |
|--------|-------------|
| `vm` | Main VM execution engine |
| `call_frame` | CallFrame and Stack types |
| `memory` | EVM memory with expansion tracking |
| `environment` | Block and transaction context |
| `opcodes` | Opcode enum (179 opcodes) |
| `opcode_handlers` | Opcode execution logic by category |
| `precompiles` | Native precompiled contracts |
| `hooks` | L1/L2 execution hooks |
| `db` | Database trait and GeneralizedDatabase |
| `errors` | VMError, ExceptionalHalt, etc. |
| `gas_cost` | Gas cost calculations |
| `tracing` | Geth-compatible call tracer |

## Precompiles

### Pre-Cancun (9 precompiles)
| Address | Name | Description |
|---------|------|-------------|
| 0x01 | ECRECOVER | Recover signer from signature |
| 0x02 | SHA2-256 | SHA256 hashing |
| 0x03 | RIPEMD-160 | RIPEMD160 hashing |
| 0x04 | IDENTITY | Copy input to output |
| 0x05 | MODEXP | Modular exponentiation |
| 0x06 | ECADD | BN254 point addition |
| 0x07 | ECMUL | BN254 point multiplication |
| 0x08 | ECPAIRING | BN254 pairing check |
| 0x09 | BLAKE2F | BLAKE2F compression |

### Cancun (1 new)
| Address | Name | Description |
|---------|------|-------------|
| 0x0A | POINT_EVALUATION | KZG point evaluation (EIP-4844) |

### Prague (7 new - BLS12-381)
| Address | Name | Description |
|---------|------|-------------|
| 0x0B | BLS12_G1ADD | G1 point addition |
| 0x0C | BLS12_G1MSM | G1 multi-scalar multiplication |
| 0x0D | BLS12_G2ADD | G2 point addition |
| 0x0E | BLS12_G2MSM | G2 multi-scalar multiplication |
| 0x0F | BLS12_PAIRING_CHECK | Pairing verification |
| 0x10 | BLS12_MAP_FP_TO_G1 | Map field to G1 |
| 0x11 | BLS12_MAP_FP2_TO_G2 | Map field extension to G2 |

## Feature Flags

| Feature | Description | Default |
|---------|-------------|---------|
| `secp256k1` | Production ECDSA library | Yes |
| `c-kzg` | C KZG implementation | No |
| `sp1` | Succinct SP1 zkVM support | No |
| `risc0` | RISC Zero zkVM support | No |
| `zisk` | Polygon ZisK zkVM support | No |
| `openvm` | OpenVM zkVM support | No |
| `debug` | Debug mode compilation | No |
| `perf_opcode_timings` | Per-opcode timing | No |
| `ethereum_foundation_tests` | EF test compatibility | No |

### zkVM Support

LEVM supports multiple zero-knowledge proving backends:
- **SP1**: Uses `substrate-bn` for BN254 operations
- **RISC0**: Uses `substrate-bn` and `c-kzg`
- **ZisK**: Integrates with Polygon's `ziskos` library
- **OpenVM**: Axiom's OpenVM platform

## Error Handling

```rust
pub enum VMError {
    Internal(InternalError),           // Unexpected errors
    TxValidation(TxValidationError),   // Pre-execution failures
    ExceptionalHalt(ExceptionalHalt),  // EVM exceptions
    RevertOpcode,                       // REVERT opcode
}

pub enum ExceptionalHalt {
    StackUnderflow, StackOverflow,
    InvalidOpcode, InvalidJump,
    OutOfGas, OutOfBounds,
    // ...
}
```

## Hooks System

LEVM uses hooks for L1/L2 customization:

```rust
pub trait Hook {
    fn prepare_execution(&mut self, vm: &mut VM) -> Result<(), VMError>;
    fn finalize_execution(&mut self, vm: &mut VM, report: &mut ContextResult)
        -> Result<(), VMError>;
}
```

- **L1**: Standard Ethereum execution with `DefaultHook`
- **L2**: Custom fee calculation with `L2Hook` and backup support

## Database Interface

```rust
pub trait Database: Send + Sync {
    fn get_account_state(&self, address: Address) -> Result<AccountState, DatabaseError>;
    fn get_storage_value(&self, address: Address, key: H256) -> Result<U256, DatabaseError>;
    fn get_block_hash(&self, block_number: u64) -> Result<H256, DatabaseError>;
    fn get_chain_config(&self) -> Result<ChainConfig, DatabaseError>;
    fn get_account_code(&self, code_hash: H256) -> Result<Code, DatabaseError>;
}
```

`GeneralizedDatabase` wraps this trait with caching and modification tracking.

## Testing

We run Ethereum Foundation tests in state and blockchain form:

```bash
# From crates/vm/levm directory
make download-evm-ef-tests run-evm-ef-tests QUIET=true
```

Test suites:
- EELS (Ethereum Execution Layer Specification)
- ethereum/tests
- legacyTests

## Documentation

- Code has extensive inline documentation
- See [FAQ](../../../docs/vm/levm/faq.rs) for common questions
- State tests: [tooling/ef_tests/state/README.md](../../../tooling/ef_tests/state/README.md)
- Blockchain tests: [tooling/ef_tests/blockchain/README.md](../../../tooling/ef_tests/blockchain/README.md)

## Useful Links

- [Ethereum Yellowpaper](https://ethereum.github.io/yellowpaper/paper.pdf) - Formal protocol definition
- [The EVM Handbook](https://noxx3xxon.notion.site/The-EVM-Handbook-bb38e175cc404111a391907c4975426d) - General EVM resources
- [EVM Codes](https://www.evm.codes/) - Opcode reference
- [EVM Playground](https://www.evm.codes/playground) - Interactive opcode testing
- [EVM Deep Dives](https://noxx.substack.com/p/evm-deep-dives-the-path-to-shadowy) - In-depth EVM analysis

## Dependencies

- `ethrex-common` - Core types
- `ethrex-crypto` - Cryptographic operations
- `ethrex-rlp` - RLP encoding
- `sha2`, `sha3`, `ripemd` - Hash functions
- `k256`, `secp256k1` - ECDSA
- `bls12_381`, `ark-bn254` - Elliptic curves
- `lambdaworks-math`, `malachite` - Math operations
