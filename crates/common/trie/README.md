# ethrex-trie

Merkle Patricia Trie implementation for the ethrex Ethereum client.

## Overview

This crate provides a production-grade implementation of the Ethereum Merkle Patricia Trie (MPT), the fundamental data structure used for storing account state, contract storage, and transaction/receipt trees. It supports all standard trie operations, proof generation, and range verification for state synchronization.

## Features

- **Full MPT support**: Branch, Extension, and Leaf nodes with proper path compression
- **Lazy hash computation**: Hashes cached and only computed when needed
- **Flexible storage**: Trait-based backend supporting in-memory and persistent storage
- **Proof generation**: Single-path and multi-path Merkle proofs
- **Range verification**: Verify key-value ranges for state sync
- **Witness logging**: Track accessed nodes for zkVM proving

## Quick Start

```rust
use ethrex_trie::{Trie, EMPTY_TRIE_HASH};
use ethrex_rlp::encode::RLPEncode;

// Create a new in-memory trie
let mut trie = Trie::new_temp();

// Insert key-value pairs (RLP-encoded)
let key = b"account".encode_to_vec();
let value = b"data".encode_to_vec();
trie.insert(key, value)?;

// Compute root hash (commits changes)
let root_hash = trie.hash()?;

// Retrieve value
let retrieved = trie.get(&key)?;

// Generate Merkle proof
let proof = trie.get_proof(&key)?;
```

## Core Types

### Trie

The main data structure for Merkle Patricia Trie operations:

```rust
let mut trie = Trie::new_temp();           // In-memory trie
let trie = Trie::new(custom_db);           // Custom backend
let trie = Trie::open(db, root_hash)?;     // Open existing trie
let trie = Trie::stateless();              // No persistence (proofs only)
```

**Operations:**
- `insert(path, value)` - Insert RLP-encoded key-value pair
- `get(path)` - Retrieve value by RLP-encoded key
- `remove(path)` - Remove value, returns the removed value
- `hash()` - Compute root hash and commit changes
- `hash_no_commit()` - Get hash without persisting
- `get_proof(path)` - Generate Merkle proof for path
- `get_proofs(paths)` - Generate proofs for multiple paths
- `commit()` - Flush pending changes to database

### Node Types

| Node | Description | RLP Encoding |
|------|-------------|--------------|
| `BranchNode` | 16-way branching with optional value | `[child_0, ..., child_15, value]` |
| `ExtensionNode` | Path compression (shared prefix) | `[prefix, child]` |
| `LeafNode` | Terminal node with value | `[path, value]` |

### NodeRef

Smart reference to trie nodes with automatic caching:

```rust
pub enum NodeRef {
    Node(Arc<Node>, OnceLock<NodeHash>),  // Embedded node + cached hash
    Hash(NodeHash),                        // Reference by hash (in DB)
}
```

### NodeHash

Efficient hash representation:

```rust
pub enum NodeHash {
    Hashed(H256),           // Nodes >= 32 bytes encoded
    Inline(([u8; 31], u8)), // Nodes < 32 bytes (stored inline)
}
```

Small nodes (< 32 bytes RLP-encoded) are stored inline to avoid unnecessary hashing.

## Module Structure

| Module | Description |
|--------|-------------|
| `db` | `TrieDB` trait and `InMemoryTrieDB` implementation |
| `node` | Node types (Branch, Extension, Leaf) and `NodeRef` |
| `error` | `TrieError` and `InconsistentTreeError` types |
| `logger` | `TrieLogger` and `TrieWitness` for access tracking |
| `verify_range` | Range verification for state synchronization |
| `trie_sorted` | Efficient batch insertion from sorted data |
| `rkyv_utils` | Serialization utilities for zkVM |

## Database Backend

The `TrieDB` trait defines the storage interface:

```rust
pub trait TrieDB: Send + Sync {
    fn get(&self, key: Nibbles) -> Result<Option<Vec<u8>>, TrieError>;
    fn put(&self, key: Nibbles, value: Vec<u8>) -> Result<(), TrieError>;
    fn put_batch(&self, key_values: Vec<(Nibbles, Vec<u8>)>) -> Result<(), TrieError>;
    fn commit(&self) -> Result<(), TrieError>;
}
```

### InMemoryTrieDB

Thread-safe in-memory implementation using `BTreeMap`:

```rust
let db = InMemoryTrieDB::new();
let trie = Trie::new(Box::new(db));
```

### Custom Backends

Implement `TrieDB` for persistent storage (e.g., RocksDB):

```rust
impl TrieDB for RocksDbTrieDB {
    fn get(&self, key: Nibbles) -> Result<Option<Vec<u8>>, TrieError> {
        // Read from RocksDB
    }
    // ... other methods
}
```

## Proof Generation

### Single Path Proof

```rust
let proof = trie.get_proof(&path)?;
// proof: Vec<Vec<u8>> - list of RLP-encoded nodes on path
```

### Multi-Path Proofs

```rust
let proofs = trie.get_proofs(vec![path1, path2, path3])?;
// Automatically deduplicates shared nodes
```

### Range Verification

For state synchronization, verify that a range of key-values belongs to a trie:

```rust
use ethrex_trie::verify_range;

let result = verify_range(
    root_hash,
    &first_key,
    &keys,
    &values,
    &proof,
)?;

// result: (bool, bool) = (verified, more_right)
// - verified: true if range is valid
// - more_right: true if more values exist after this range
```

## Witness Logging

Track all accessed nodes for zkVM witness generation:

```rust
use ethrex_trie::TrieLogger;

// Wrap trie with logger
let (witness, logged_trie) = TrieLogger::open_trie(trie);

// Perform operations...
logged_trie.get(&path)?;

// Extract witness (all accessed nodes)
let witness_nodes = witness.lock().unwrap();
```

## Type Aliases

```rust
pub type PathRLP = Vec<u8>;   // RLP-encoded path
pub type ValueRLP = Vec<u8>;  // RLP-encoded value
pub type NodeRLP = Vec<u8>;   // RLP-encoded node
```

## Constants

```rust
// Hash of empty trie: keccak(RLP_NULL) = keccak(0x80)
pub static EMPTY_TRIE_HASH: H256;
```

## Error Handling

```rust
pub enum TrieError {
    RLPDecode(RLPDecodeError),              // Invalid RLP encoding
    Verify(String),                          // Proof verification failed
    InconsistentTree(InconsistentTreeError), // Tree structure error
    LockError,                               // Mutex acquisition failed
    DbError(anyhow::Error),                  // Backend error
    InvalidInput,                            // Invalid operation input
}

pub enum InconsistentTreeError {
    ExtensionNodeChildDiffers,    // Extension prefix mismatch
    ExtensionNodeChildNotFound,   // Missing extension child
    NodeNotFoundOnBranchNode,     // Missing branch child
    RootNotFound(H256),           // Root not in database
    RootNotFoundNoHash,           // Empty root missing
}
```

## Performance

- **Hash computation**: O(n) where n = modified nodes
- **Get operation**: O(log16(n)) average case
- **Insert operation**: O(log16(n)) + node restructuring
- **Proof generation**: O(log16(n)) path length
- **Memory**: Optimized via extension nodes and inline hashes

### Batch Operations

For bulk insertions, use `TrieSorted` for parallel processing:

```rust
use ethrex_trie::trie_sorted::TrieSorted;

// Efficient insertion from sorted key-value pairs
let trie = TrieSorted::from_sorted_iter(sorted_pairs, db)?;
```

## Trie Structure

The Merkle Patricia Trie uses three node types for efficient storage:

```
Root
 |-- Extension(prefix="acc")
     |-- Branch[choices]
         |-- [0] Leaf(key="ount1", value=...)
         |-- [1] Leaf(key="ount2", value=...)
         |-- [2] Extension(prefix="ess")
                 |-- Leaf(key="1", value=...)
```

- **Extension nodes** compress shared prefixes
- **Branch nodes** provide 16-way branching (hex nibbles)
- **Leaf nodes** store final values

## Benchmarking

To measure performance against [citahub's cita_trie](https://github.com/citahub/cita_trie):

```bash
make bench
```

Benchmarks are in the `benches` folder.

## Useful Links

- [Ethereum.org - Merkle Patricia Trie](https://ethereum.org/en/developers/docs/data-structures-and-encoding/patricia-merkle-trie/)
- [Stack Exchange Discussion](https://ethereum.stackexchange.com/questions/130017/merkle-patricia-trie-in-ethereum)

## Dependencies

- `ethrex-crypto` - Keccak hashing
- `ethrex-rlp` - RLP encoding/decoding
- `ethereum-types` - H256 and other primitives
