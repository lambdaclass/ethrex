# ethrex-common

Core types, constants, and utilities for the ethrex Ethereum client.

## Overview

This crate serves as the foundational library for ethrex, providing all the core Ethereum data structures that other crates depend on. It defines blocks, transactions, accounts, receipts, and chain configuration types with full support for all Ethereum forks through Prague and beyond.

## Core Types

### Account

```rust
use ethrex_common::types::{Account, AccountInfo, AccountState};

// Account with full state (code + storage)
let account = Account {
    info: AccountInfo {
        code_hash: EMPTY_KECCACK_HASH,
        balance: U256::from(1000),
        nonce: 0,
    },
    code: Code::default(),
    storage: FxHashMap::default(),
};

// Slim account state for RLP encoding
let state = AccountState {
    nonce: 0,
    balance: U256::from(1000),
    storage_root: EMPTY_TRIE_HASH,
    code_hash: EMPTY_KECCACK_HASH,
};
```

### Block

```rust
use ethrex_common::types::{Block, BlockHeader, BlockBody};

let block = Block {
    header: BlockHeader {
        parent_hash: H256::zero(),
        number: 1,
        gas_limit: 30_000_000,
        timestamp: 1234567890,
        // ... other fields
    },
    body: BlockBody {
        transactions: vec![],
        ommers: vec![],
        withdrawals: Some(vec![]),
    },
};

// Block hash is computed lazily
let hash = block.hash();
```

### Transaction

All Ethereum transaction types are supported:

| Type | EIP | Description |
|------|-----|-------------|
| `LegacyTransaction` | Pre-EIP-155 | Original transaction format |
| `EIP2930Transaction` | EIP-2930 | Access list transactions |
| `EIP1559Transaction` | EIP-1559 | Fee market transactions |
| `EIP4844Transaction` | EIP-4844 | Blob transactions |
| `EIP7702Transaction` | EIP-7702 | Set EOA account code |
| `PrivilegedL2Transaction` | L2 | Deposit transactions |

```rust
use ethrex_common::types::Transaction;

// Transaction is an enum of all types
let tx = Transaction::EIP1559Transaction(EIP1559Transaction {
    chain_id: 1,
    nonce: 0,
    max_priority_fee_per_gas: 1_000_000_000,
    max_fee_per_gas: 100_000_000_000,
    gas_limit: 21000,
    to: TxKind::Call(address),
    value: U256::from(1_000_000_000_000_000_000u64),
    data: Bytes::new(),
    access_list: vec![],
    // signature fields...
});
```

### Receipt

```rust
use ethrex_common::types::{Receipt, Log, TxType};

let receipt = Receipt {
    tx_type: TxType::EIP1559,
    succeeded: true,
    cumulative_gas_used: 21000,
    logs: vec![
        Log {
            address: contract_address,
            topics: vec![event_signature],
            data: Bytes::from(event_data),
        }
    ],
};
```

### Genesis & Chain Configuration

```rust
use ethrex_common::types::{Genesis, ChainConfig};

// Load from JSON
let genesis: Genesis = serde_json::from_str(genesis_json)?;

// Check fork activation
if genesis.config.is_cancun_activated(block_timestamp) {
    // Apply Cancun rules
}
```

## Module Structure

| Module | Description |
|--------|-------------|
| `types` | Core Ethereum data structures (Block, Transaction, Account, etc.) |
| `constants` | Protocol constants (gas limits, blob sizes, hash values) |
| `serde_utils` | JSON serialization helpers for hex/decimal encoding |
| `evm` | EVM utilities (CREATE address calculation) |
| `utils` | General utilities (keccak, U256 conversions) |
| `rkyv_utils` | Zero-copy serialization wrappers for zkVM |
| `genesis_utils` | Genesis JSON file utilities |
| `errors` | Error types (EcdsaError) |
| `base64` | RFC 4648 URL-safe base64 encoding |
| `fd_limit` | File descriptor limit management |
| `tracing` | Logging/tracing support |

## Re-exports

The crate re-exports commonly used types:

```rust
// From ethereum-types
pub use ethereum_types::*;  // Address, H256, U256, Bloom, etc.

// From bytes
pub use bytes::Bytes;

// From ethrex-trie
pub use ethrex_trie::{TrieLogger, TrieWitness};
```

## Constants

### Protocol Constants

```rust
use ethrex_common::constants::*;

// Gas parameters
const GAS_LIMIT_MINIMUM: u64 = 5000;
const INITIAL_BASE_FEE: u64 = 1_000_000_000;

// Blob parameters (EIP-4844)
const BYTES_PER_BLOB: usize = 131_072;
const GAS_PER_BLOB: u64 = 131_072;
const VERSIONED_HASH_VERSION_KZG: u8 = 0x01;

// Block limits (EIP-7934)
const MAX_BLOCK_SIZE: u64 = 10_485_760;
```

### Hash Constants

```rust
use ethrex_common::types::EMPTY_KECCACK_HASH;
use ethrex_common::types::EMPTY_TRIE_HASH;

// Keccak256 of empty byte array
let empty_code_hash = EMPTY_KECCACK_HASH;

// Root hash of empty Merkle Patricia Trie
let empty_state_root = EMPTY_TRIE_HASH;
```

## Feature Flags

| Feature | Description | Default |
|---------|-------------|---------|
| `secp256k1` | Production ECDSA library | Yes |
| `c-kzg` | Fast KZG via C bindings to BLST | No |
| `risc0` | RISC0 zkVM compatibility | No |
| `sp1` | Succinct SP1 zkVM compatibility | No |
| `zisk` | Polygon ZisK zkVM compatibility | No |
| `openvm` | OpenVM zkVM compatibility | No |

### ECDSA Backend Selection

The crate supports two ECDSA implementations:

- **secp256k1** (default): Fast C library for production use
- **k256**: Pure Rust implementation for zkVM compatibility

When any zkVM feature is enabled, `k256` is used automatically.

## Serialization

### RLP Encoding

All core types implement `RLPEncode` and `RLPDecode`:

```rust
use ethrex_rlp::encode::RLPEncode;
use ethrex_rlp::decode::RLPDecode;

let encoded = block_header.encode_to_vec();
let decoded = BlockHeader::decode(&encoded)?;
```

### JSON Serialization

Custom serde modules handle Ethereum's hex encoding conventions:

```rust
#[derive(Serialize, Deserialize)]
struct Example {
    #[serde(with = "serde_utils::u64::hex_str")]
    gas_limit: u64,

    #[serde(with = "serde_utils::u256::hex_str")]
    value: U256,
}
```

### Rkyv (Zero-Copy)

For zkVM proving, types can be serialized with `rkyv`:

```rust
use ethrex_common::rkyv_utils::*;

#[derive(Archive, Serialize)]
struct WitnessData {
    #[rkyv(with = H256Wrapper)]
    block_hash: H256,

    #[rkyv(with = U256Wrapper)]
    balance: U256,
}
```

## Network Configuration

Built-in genesis configurations for public networks:

```rust
use ethrex_common::types::{Network, PublicNetwork};

let network = Network::PublicNetwork(PublicNetwork::Mainnet);
let genesis_json = network.genesis_file()?;
```

Supported networks:
- Mainnet (chain ID: 1)
- Sepolia (chain ID: 11155111)
- Holesky (chain ID: 17000)
- Hoodi (chain ID: 560496)

## Utilities

### Keccak Hashing

```rust
use ethrex_common::utils::keccak;

let hash: H256 = keccak(b"hello");
```

### U256 Conversions

```rust
use ethrex_common::utils::{u256_from_big_endian, u256_to_big_endian};

let value = u256_from_big_endian(&bytes);
let bytes = u256_to_big_endian(value);
```

### CREATE Address

```rust
use ethrex_common::evm::calculate_create_address;

let contract_address = calculate_create_address(sender, nonce);
```

## Fork Support

The crate supports all Ethereum forks:

**Block-number activated:**
- Homestead, DAO Fork, Tangerine Whistle (EIP-150), Spurious Dragon (EIP-155/158)
- Byzantium, Constantinople, Petersburg, Istanbul, Muir Glacier
- Berlin, London, Arrow Glacier, Gray Glacier

**Timestamp-activated:**
- Paris (The Merge), Shanghai, Cancun, Prague, Osaka
- BPO1-5 (Blob Pipeline Optimizations)

```rust
let config = ChainConfig::default();

// Check if a fork is active
if config.is_prague_activated(timestamp) {
    // Apply Prague rules
}

if config.is_london_activated(block_number) {
    // Apply London rules (EIP-1559)
}
```
