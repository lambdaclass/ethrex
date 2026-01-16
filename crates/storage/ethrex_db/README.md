# ethrex_db

A Paprika-inspired Ethereum state storage engine written in Rust for [ethrex](https://github.com/lambdaclass/ethrex).

## Overview

ethrex_db is a high-performance, persistent storage solution for Ethereum state and storage tries. It is inspired by [Paprika](https://github.com/NethermindEth/Paprika), a C# implementation by Nethermind.

### Key Features

- **Block-aware persistence**: Understands Ethereum concepts like finality, reorgs, latest/safe/finalized blocks
- **Copy-on-Write concurrency**: Single writer with multiple lock-free readers
- **Memory-mapped storage**: LMDB-inspired page-based persistence
- **Efficient trie operations**: Optimized nibble path handling and in-page storage

## Architecture

The library is split into two major components:

### 1. Blockchain (Hot Storage)

Handles blocks that are not yet finalized (latest, safe):
- Parallel block creation from the same parent
- Fork Choice Update (FCU) handling
- Copy-on-Write state per block

### 2. PagedDb (Cold Storage)

Handles finalized blocks:
- Memory-mapped file storage using `memmap2`
- 4KB page-based addressing
- Copy-on-Write for consistency

## Core Data Structures

### NibblePath

Efficient path representation for trie traversal. A nibble is a half-byte (4 bits), representing values 0-15. Ethereum trie paths are sequences of nibbles derived from keccak hashes.

```rust
use ethrex_db::data::NibblePath;

let path = NibblePath::from_bytes(&[0xAB, 0xCD]);
assert_eq!(path.get(0), 0xA);
assert_eq!(path.get(1), 0xB);
```

### SlottedArray

In-page key-value storage using the PostgreSQL-inspired slot array pattern:
- Slots grow forward from the header
- Data grows backward from the end of the page
- Tombstone-based deletion with defragmentation

```rust
use ethrex_db::data::{SlottedArray, NibblePath};

let mut arr = SlottedArray::new();
let key = NibblePath::from_bytes(&[0xAB, 0xCD]);
arr.try_insert(&key, b"hello world");
```

### Page Types

- **RootPage**: Metadata, fanout array, abandoned page tracking
- **DataPage**: Intermediate nodes with 256-bucket fanout
- **BottomPage**: Leaf pages with SlottedArray
- **AbandonedPage**: Tracks pages for reuse after reorg depth

## Project Structure

```
ethrex_db/
├── src/
│   ├── lib.rs
│   ├── data/           # Core data structures
│   │   ├── nibble_path.rs
│   │   └── slotted_array.rs
│   ├── store/          # Page-based persistent storage
│   │   ├── page.rs
│   │   ├── data_page.rs
│   │   ├── root_page.rs
│   │   └── paged_db.rs
│   ├── chain/          # Block management
│   │   ├── blockchain.rs
│   │   └── block.rs
│   └── merkle/         # State root computation
│       └── compute.rs
├── Cargo.toml
└── tests/
```

## Implementation Status

### Phase 1: Foundation ✅

Core data structures:
- [x] NibblePath - trie path traversal
- [x] SlottedArray - in-page key-value storage
- [x] Property-based tests with proptest

### Phase 2: Page-Based Storage ✅

- [x] Page abstraction (4KB pages)
- [x] DataPage with fanout buckets
- [x] RootPage with metadata
- [x] LeafPage, AbandonedPage
- [x] PagedDb with memory-mapped files
- [x] Copy-on-Write concurrency
- [x] Batch commit support

### Phase 3: Blockchain Layer ✅

- [x] Block abstraction with parent chain
- [x] WorldState trait for state access
- [x] Blockchain for unfinalized state
- [x] Parallel block creation
- [x] FCU handling
- [x] Finalization to PagedDb

### Phase 4: Merkle Computation ✅

- [x] RLP encoding for Ethereum
- [x] Keccak-256 hashing
- [x] Merkle node types (Leaf, Extension, Branch)
- [x] In-memory MerkleTrie
- [x] Deterministic root hash computation

### Phase 5: Integration (Next)

Connect the individual components into a working system:
- [ ] Store trie nodes in DataPage/LeafPage
- [ ] Load trie nodes from PagedDb on demand
- [ ] Integrate MerkleTrie with PagedDb storage
- [ ] Account storage tries (nested tries per account)
- [ ] State root computation in Blockchain finalization
- [ ] Full read/write path: Blockchain → PagedDb → Merkle

### Phase 6: Optimizations (Future)

- [ ] SIMD vectorization for SlottedArray search
- [ ] Lock-free readers
- [ ] Parallel Merkle computation
- [ ] Abandoned page reuse

## Testing

Run the test suite:

```bash
cargo test
```

The project includes:
- **76 tests** (68 unit + 8 integration)
- **Property-based tests** using `proptest` for NibblePath, SlottedArray, and MerkleTrie
- **Integration tests** in `tests/integration_tests.rs`

## Current State

The individual components are implemented and tested:

| Component | Status | Notes |
|-----------|--------|-------|
| NibblePath | ✅ Working | Full nibble operations, slicing, comparison |
| SlottedArray | ✅ Working | Insert, lookup, delete, defragmentation |
| PagedDb | ✅ Working | Memory-mapped COW storage, batch commits |
| Blockchain | ✅ Working | Block creation, FCU, finalization |
| MerkleTrie | ✅ Working | In-memory trie, deterministic root hashes |

**Not yet integrated**: The MerkleTrie operates in-memory independently from PagedDb. A production system needs Phase 5 to connect these components.

## Dependencies

| Crate | Purpose |
|-------|---------|
| `memmap2` | Memory-mapped files |
| `tiny-keccak` | Keccak hashing |
| `rlp` | Ethereum RLP encoding |
| `primitive-types` | H256, U256 |
| `parking_lot` | Fast synchronization primitives |

## References

- [Paprika (C# implementation)](https://github.com/NethermindEth/Paprika)
- [Paprika Design Document](https://github.com/NethermindEth/Paprika/blob/main/docs/design.md)
- [PostgreSQL Page Layout](https://www.postgresql.org/docs/current/storage-page-layout.html)
- [LMDB - Lightning Memory-Mapped Database](https://github.com/LMDB/lmdb)
- [Andy Pavlo - Database Storage Lectures](https://www.youtube.com/watch?v=df-l2PxUidI)

## License

MIT
