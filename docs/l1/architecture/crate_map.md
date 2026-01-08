# Crate Map

This document provides an overview of all crates in the ethrex monorepo and their responsibilities.

## Crate Dependency Graph

```
                              ┌─────────────────────────────────────┐
                              │           cmd/ethrex                │
                              │      (Main binary entry point)      │
                              └───────────────┬─────────────────────┘
                                              │
                    ┌─────────────────────────┼─────────────────────────┐
                    │                         │                         │
                    ▼                         ▼                         ▼
        ┌───────────────────┐     ┌───────────────────┐     ┌───────────────────┐
        │  networking/rpc   │     │  networking/p2p   │     │    blockchain     │
        │   (JSON-RPC API)  │     │  (P2P networking) │     │ (Chain management)│
        └─────────┬─────────┘     └─────────┬─────────┘     └─────────┬─────────┘
                  │                         │                         │
                  │                         │                         │
                  └─────────────────────────┼─────────────────────────┘
                                            │
                                            ▼
                              ┌─────────────────────────────┐
                              │           vm/levm           │
                              │    (EVM implementation)     │
                              └─────────────┬───────────────┘
                                            │
                                            ▼
                              ┌─────────────────────────────┐
                              │          storage            │
                              │     (Data persistence)      │
                              └─────────────┬───────────────┘
                                            │
                    ┌───────────────────────┼───────────────────────┐
                    │                       │                       │
                    ▼                       ▼                       ▼
        ┌───────────────────┐   ┌───────────────────┐   ┌───────────────────┐
        │    common/trie    │   │    common/rlp     │   │   common/types    │
        │ (Merkle Patricia) │   │ (RLP encoding)    │   │ (Core data types) │
        └───────────────────┘   └───────────────────┘   └───────────────────┘
```

## Core Crates

### `ethrex-common`

**Purpose:** Core data types and utilities shared across all crates.

**Key Modules:**
- `types/` - Block, Transaction, Receipt, Account types
- `trie/` - Merkle Patricia Trie implementation
- `rlp/` - RLP encoding/decoding
- `crypto/` - Keccak hashing, signature recovery

**Notable Types:**
```rust
pub struct Block { header: BlockHeader, body: BlockBody }
pub struct Transaction { /* variants for Legacy, EIP-2930, EIP-1559, EIP-4844, EIP-7702 */ }
pub struct AccountState { nonce: u64, balance: U256, storage_root: H256, code_hash: H256 }
```

---

### `ethrex-storage`

**Purpose:** Persistent storage layer with multiple backend support.

**Key Components:**
- `Store` - High-level API for all blockchain data
- `StoreEngine` trait - Backend abstraction
- `InMemoryStore` - Testing backend
- `RocksDBStore` - Production backend

**Stored Data:**
| Table | Contents |
|-------|----------|
| `block_numbers` | Block hash → block number |
| `canonical_block_hashes` | Block number → canonical hash |
| `headers` | Block hash → BlockHeader |
| `bodies` | Block hash → BlockBody |
| `receipts` | Block hash + index → Receipt |
| `account_trie_nodes` | Node hash → trie node data |
| `storage_trie_nodes` | Node hash → trie node data |
| `account_codes` | Code hash → bytecode |
| `account_flatkeyvalue` | Account flat key-value store |
| `storage_flatkeyvalue` | Storage flat key-value store |

---

### `ethrex-blockchain`

**Purpose:** Chain management, block validation, and mempool.

**Key Components:**
- `Blockchain` - Main orchestrator for chain operations
- `Mempool` - Pending transaction pool
- `fork_choice` - Fork choice rule implementation
- `payload` - Block building for validators
- `validate` - Block and transaction validation

**Public API:**
```rust
impl Blockchain {
    pub fn add_block(&self, block: Block) -> Result<(), ChainError>
    pub fn add_block_pipeline(&self, block: Block) -> Result<(), ChainError>
    pub fn validate_transaction(&self, tx: &Transaction) -> Result<(), MempoolError>
    pub fn build_payload(&self, template: Block) -> Result<PayloadBuildResult, ChainError>
    pub fn get_payload(&self, id: u64) -> Result<PayloadBuildResult, ChainError>
}
```

---

### `ethrex-vm` / `levm`

**Purpose:** Ethereum Virtual Machine implementation.

**Key Components:**
- `VM` - Main EVM execution engine
- `Evm` trait - VM interface for different contexts
- Opcode handlers (one per EVM opcode)
- Precompiled contracts
- Gas metering

**Execution Flow:**
```rust
impl VM {
    pub fn execute(&mut self) -> Result<ExecutionReport, VMError>
    fn execute_opcode(&mut self, opcode: u8) -> Result<(), VMError>
    fn call(&mut self, ...) -> Result<CallOutcome, VMError>
    fn create(&mut self, ...) -> Result<CreateOutcome, VMError>
}
```

---

### `ethrex-networking/rpc`

**Purpose:** JSON-RPC API server.

**Supported Namespaces:**
- `eth_*` - Standard Ethereum methods
- `debug_*` - Debugging and tracing
- `txpool_*` - Mempool inspection
- `admin_*` - Node administration
- `engine_*` - Consensus client communication
- `web3_*` - Web3 utilities

**Architecture:**
```rust
pub trait RpcHandler: Send + Sync {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr>;
    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr>;
}
```

---

### `ethrex-networking/p2p`

**Purpose:** Peer-to-peer networking stack.

**Protocol Layers:**
1. **DiscV4** - Node discovery
2. **RLPx** - Encrypted transport
3. **eth/68** - Ethereum wire protocol
4. **snap/1** - Snap sync protocol

**Key Components:**
- `PeerHandler` - Manages peer connections
- `PeerTable` - Tracks known peers and their scores
- `Syncer` - Synchronization state machine
- `SyncManager` - Coordinates sync operations

---

## Supporting Crates

### `ethrex-common/trie`

**Purpose:** Merkle Patricia Trie implementation.

**Features:**
- Standard MPT operations (get, insert, delete)
- Proof generation and verification
- Sorted insertion for snap sync
- Flat key-value store integration

---

### `ethrex-common/rlp`

**Purpose:** Recursive Length Prefix encoding.

**Traits:**
```rust
pub trait RLPEncode {
    fn encode(&self, buf: &mut dyn BufMut);
    fn encode_to_vec(&self) -> Vec<u8>;
}

pub trait RLPDecode: Sized {
    fn decode(rlp: &[u8]) -> Result<Self, RLPDecodeError>;
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError>;
}
```

---

### `ethrex-metrics`

**Purpose:** Prometheus metrics collection.

**Metric Categories:**
- Block metrics (height, gas, execution time)
- Transaction metrics (types, counts, errors)
- P2P metrics (peers, messages, sync progress)
- RPC metrics (requests, latency)

---

### `ethrex-crypto`

**Purpose:** Cryptographic primitives.

**Features:**
- Keccak-256 hashing
- ECDSA signature recovery
- BLS signatures (for beacon chain)

---

## L2-Specific Crates

### `ethrex-l2`

**Purpose:** L2 sequencer and prover integration.

**Components:**
- Sequencer logic
- State diff computation
- Prover interface
- L1 interaction (deposits, withdrawals)

---

### `ethrex-prover`

**Purpose:** Zero-knowledge proof generation.

**Supported Provers:**
- SP1 (Succinct)
- RISC0
- TDX (Trusted Execution)

---

## Test and Development Crates

### `ef-tests`

**Purpose:** Ethereum Foundation test runner.

Runs official Ethereum tests to verify protocol compliance.

---

### `ethrex-dev`

**Purpose:** Development mode utilities.

Features:
- Local development network
- Block import from files
- Test fixtures

---

## Crate Features

Many crates support feature flags:

| Crate | Feature | Effect |
|-------|---------|--------|
| `ethrex-storage` | `rocksdb` | Enable RocksDB backend |
| `ethrex-blockchain` | `metrics` | Enable Prometheus metrics |
| `ethrex-networking/p2p` | `sync-test` | Testing utilities for sync |
| `ethrex-networking/p2p` | `experimental-discv5` | Enable discv5 node discovery (experimental) |

## Adding New Functionality

When adding new features, consider:

1. **Where does it belong?**
   - Pure data types → `ethrex-common`
   - Database operations → `ethrex-storage`
   - EVM changes → `ethrex-vm`
   - Chain logic → `ethrex-blockchain`
   - API endpoints → `ethrex-networking/rpc`
   - P2P messages → `ethrex-networking/p2p`

2. **Dependency direction**
   - Lower-level crates should not depend on higher-level ones
   - Common types flow down, behaviors flow up

3. **Testing**
   - Unit tests in the crate
   - Integration tests in `tests/` directory
   - EF tests for protocol compliance

## Related Documentation

- [System Overview](./overview.md) - How crates work together
- [Block Execution](./block_execution.md) - Execution flow across crates
- [Sync State Machine](./sync_state_machine.md) - Sync implementation details
