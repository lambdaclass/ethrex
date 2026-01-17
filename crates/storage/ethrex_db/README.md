# ethrex_db

A Paprika-inspired Ethereum state storage engine written in Rust for [ethrex](https://github.com/lambdaclass/ethrex).

## Overview

ethrex_db is a high-performance, persistent storage solution for Ethereum state and storage tries. It is inspired by [Paprika](https://github.com/NethermindEth/Paprika), a C# implementation by Nethermind.

### Key Features

- **Block-aware persistence**: Understands Ethereum concepts like finality, reorgs, latest/safe/finalized blocks
- **Copy-on-Write concurrency**: Single writer with multiple lock-free readers
- **Memory-mapped storage**: LMDB-inspired page-based persistence
- **Efficient trie operations**: Optimized nibble path handling and in-page storage

## Performance

Benchmark comparison against ethrex-trie and cita_trie (the baseline used by ethrex):

### Insert Performance

| Items | ethrex_db | ethrex-trie | cita_trie | Speedup vs ethrex-trie |
|-------|-----------|-------------|-----------|------------------------|
| 100   | **68 µs** | 110 µs      | 131 µs    | **1.6x faster**        |
| 1,000 | **762 µs** | 1.35 ms    | 1.39 ms   | **1.8x faster**        |
| 10,000| **7.8 ms** | 14.7 ms    | 14.8 ms   | **1.9x faster**        |

### Get Performance (single lookup in 1000-entry trie)

| Implementation | Time | Speedup |
|----------------|------|---------|
| ethrex_db      | **13 ns** | - |
| ethrex-trie    | 144 ns | **11x faster** |
| cita_trie      | 208 ns | **16x faster** |

### Root Hash Computation

| Items | ethrex_db | ethrex-trie | Speedup |
|-------|-----------|-------------|---------|
| 100   | **3.6 ns** | 39 ns      | **11x faster** |
| 1,000 | **3.6 ns** | 40 ns      | **11x faster** |

Run benchmarks yourself:
```bash
cargo bench --bench trie_comparison
```

## Design: Why Not MPT on RocksDB?

Traditional Ethereum clients store the Merkle Patricia Trie (MPT) on top of general-purpose key-value stores like RocksDB or LevelDB. This approach has fundamental inefficiencies:

### Problems with MPT + RocksDB

1. **Write Amplification**: RocksDB uses LSM-trees which amplify writes 10-30x. Every trie node update triggers multiple compactions.

2. **Read Amplification**: Reading a single value requires traversing ~8 trie nodes (for 32-byte keys), each requiring a separate RocksDB lookup through multiple SST file levels.

3. **Space Amplification**: Trie nodes are stored as separate KV pairs with overhead per entry. LSM compaction creates multiple copies of data.

4. **No Block Awareness**: RocksDB doesn't understand Ethereum's finality model. Reorgs require expensive tombstone writes and compactions.

5. **Cache Inefficiency**: The LSM-tree's multi-level structure has poor cache locality for trie traversals.

### How ethrex_db Solves This

**1. Flat Key-Value with Computed Merkle**

Instead of persisting trie structure, we store flat key-value pairs and compute the Merkle root on demand:
- Inserts are O(1) hash table operations
- Root computation happens once at block commit
- No tree rebalancing on updates

**2. Page-Based Storage (Like LMDB)**

```
┌─────────────────────────────────────────┐
│ Page (4KB)                              │
├─────────┬───────────────────────────────┤
│ Header  │ Payload (SlottedArray)        │
│ 8 bytes │ ◄── slots │ free │ data ──►  │
└─────────┴───────────────────────────────┘
```

- Fixed 4KB pages with O(1) allocation
- No write amplification - pages are written in-place
- Memory-mapped for zero-copy reads
- Copy-on-Write for concurrent readers

**3. Block-Aware Architecture**

```
                    ┌──────────┐
                    │ Genesis  │
                    └────┬─────┘
                         │
              ┌──────────┼──────────┐
              ▼          ▼          ▼
         ┌────────┐ ┌────────┐ ┌────────┐
         │Block 1A│ │Block 1B│ │Block 1C│  ◄── Hot (Blockchain)
         └───┬────┘ └────────┘ └────────┘
             │
             ▼
        ┌─────────┐
        │Finalized│  ◄── Cold (PagedDb)
        └─────────┘
```

- **Hot storage (Blockchain)**: Uncommitted blocks use Copy-on-Write diffs
- **Cold storage (PagedDb)**: Finalized state in memory-mapped pages
- Reorgs only affect hot storage - no disk writes needed

**4. SlottedArray for Dense Packing**

PostgreSQL-inspired in-page storage:
- Slots grow forward, data grows backward
- Variable-length values with minimal overhead
- Binary search for O(log n) lookups within page

**5. 256-Way Fanout**

DataPages use 256-bucket fanout (2 nibbles):
- Reduces tree depth from ~64 to ~16
- Better cache utilization
- Fewer page accesses per lookup

### Comparison Summary

| Aspect | MPT + RocksDB | ethrex_db |
|--------|---------------|-----------|
| Write amplification | 10-30x (LSM) | 1x (in-place pages) |
| Read amplification | High (multi-level) | Low (mmap + fanout) |
| Reorg handling | Expensive (tombstones) | Cheap (COW diffs) |
| Concurrency | Lock-based | Lock-free readers |
| Memory efficiency | Poor (LSM buffers) | Excellent (mmap) |

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

## API

### MerkleTrie
```rust
use ethrex_db::merkle::{MerkleTrie, keccak256, EMPTY_ROOT};

let mut trie = MerkleTrie::new();
trie.insert(&key, value);              // Insert/update
trie.get(&key) -> Option<&[u8]>        // Lookup
trie.remove(&key);                     // Delete
trie.root_hash() -> [u8; 32]           // Compute state root
trie.len() -> usize                    // Entry count
trie.iter() -> impl Iterator           // Iterate all entries
```

### Blockchain
```rust
use ethrex_db::chain::{Blockchain, Account, WorldState};
use ethrex_db::store::PagedDb;

let db = PagedDb::in_memory(1000)?;
let blockchain = Blockchain::new(db);

let mut block = blockchain.start_new(parent_hash, block_hash, number)?;
block.set_account(addr, account);       // Modify account
block.set_storage(addr, slot, value);   // Modify storage
blockchain.commit(block)?;              // Commit block
blockchain.finalize(hash)?;             // Finalize to cold storage
```

### Account
```rust
use ethrex_db::chain::Account;

let account = Account::with_balance(U256::from(1000));
account.nonce;                          // Transaction count
account.balance;                        // ETH balance
account.encode() -> Vec<u8>             // RLP encode
Account::decode(bytes)?                 // RLP decode
```

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

## Testing

Run the test suite:

```bash
cargo test
```

The project includes:
- **117 tests** (97 unit + 20 integration)
- **Property-based tests** using `proptest` for NibblePath, SlottedArray, and MerkleTrie
- **End-to-end tests** covering multi-block chains, forks, storage operations, state persistence

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

### Phase 5: Integration ✅

- [x] SlottedArray iterator for loading entries from pages
- [x] PagedStateTrie for PagedDb integration
- [x] Fanout-based storage for large tries (256 buckets)
- [x] Account storage tries (nested tries per account)
- [x] State root computation in Blockchain finalization
- [x] Full read/write path: Blockchain → PagedDb → Merkle
- [x] State persistence across database reopens

### Phase 6: Optimizations ✅

**SIMD Vectorization Investigation:**
- [x] Benchmarked explicit SIMD (`wide` crate) vs scalar comparison
- [x] **Finding**: LLVM auto-vectorization already optimizes `starts_with`
- [x] Scalar comparison: 1.25-3.26 ns, SIMD: 2.00-4.81 ns
- [x] **Decision**: Keep standard library functions - they're already SIMD-optimized

**Lock-Free Readers:**
- [x] Replaced `std::sync::RwLock` with `parking_lot::RwLock` (faster, no poisoning)
- [x] Added atomic variables for frequently-read metadata:
  - `batch_id`: Current batch ID (AtomicU32)
  - `block_number`: Current block number (AtomicU32)
  - `state_root`: State root address (AtomicU32)
  - `block_hash`: Block hash (4x AtomicU64)
- [x] `begin_read_only()` now lock-free for common metadata
- [x] `batch_id()`, `block_number()`, `block_hash()` all lock-free

**Parallel Merkle Computation:**
- [x] Added Rayon for parallel computation
- [x] Implemented `parallel_root_hash()` with automatic threshold
- [x] Branch nodes with >64 entries computed in parallel
- [x] **Benchmarks**:
  - 100 entries: sequential faster (parallelization overhead)
  - 1,000 entries: parallel 2.2x faster (1.44ms → 650µs)
  - 10,000 entries: parallel 4.2x faster (9.8ms → 2.3ms)

**Abandoned Page Reuse:**
- [x] Added inline abandoned page storage in RootPage
- [x] Batch ID tracking for reorg safety
- [x] Configurable reorg depth (default: 64 batches)
- [x] Automatic page reuse after reorg depth passes
- [x] AbandonedPage linked list for overflow (structure in place)

## Dependencies

| Crate | Purpose |
|-------|---------|
| `memmap2` | Memory-mapped files |
| `tiny-keccak` | Keccak hashing |
| `rlp` | Ethereum RLP encoding |
| `primitive-types` | H256, U256 |
| `parking_lot` | Fast synchronization primitives |
| `rayon` | Parallel computation |

## References

- [Paprika (C# implementation)](https://github.com/NethermindEth/Paprika)
- [Paprika Design Document](https://github.com/NethermindEth/Paprika/blob/main/docs/design.md)
- [PostgreSQL Page Layout](https://www.postgresql.org/docs/current/storage-page-layout.html)
- [LMDB - Lightning Memory-Mapped Database](https://github.com/LMDB/lmdb)
- [Andy Pavlo - Database Storage Lectures](https://www.youtube.com/watch?v=df-l2PxUidI)

## License

MIT
