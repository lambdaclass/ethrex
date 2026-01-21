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

| Items | ethrex_db | ethrex-trie | cita_trie | Speedup |
|-------|-----------|-------------|-----------|---------|
| 100   | **68 µs** (1.47 Melem/s) | 109 µs | 129 µs | **1.6-1.9x faster** |
| 1,000 | **761 µs** (1.31 Melem/s) | 1.35 ms | 1.49 ms | **1.8-2.0x faster** |
| 10,000| **7.9 ms** (1.27 Melem/s) | 17.0 ms | 15.1 ms | **1.9-2.2x faster** |

### Get Performance (single lookup in 1000-entry trie)

| Implementation | Time | Speedup |
|----------------|------|---------|
| ethrex_db      | **14 ns** | baseline |
| ethrex-trie    | 143 ns | **10x faster** |
| cita_trie      | 206 ns | **15x faster** |

### Root Hash Computation (cached)

| Items | ethrex_db | ethrex-trie | Speedup |
|-------|-----------|-------------|---------|
| 100   | **3.6 ns** | 42 ns | **12x faster** |
| 1,000 | **3.6 ns** | 48 ns | **13x faster** |

### Parallel Merkle Computation

| Entries | Sequential | Parallel | Speedup |
|---------|------------|----------|---------|
| 100     | 64 µs | 47 µs | 1.4x |
| 1,000   | 678 µs | 298 µs | **2.3x** |
| 10,000  | 7.3 ms | 2.3 ms | **3.2x** |

### Core Operations

| Operation | Performance |
|-----------|-------------|
| NibblePath from_bytes | **14.2 ns** |
| NibblePath get_nibble | **1.0 ns** |
| NibblePath common_prefix_len | **1.3 ns** |
| SlottedArray insert (100 entries) | **27.2 Melem/s** |
| SlottedArray insert (500 entries) | **31.5 Melem/s** |
| SlottedArray lookup (100 entries) | **156 ns** (3.2 Gelem/s) |
| Keccak256 (32 bytes) | **170 ns** (179 MiB/s) |

### Phase 10 Optimizations

| Optimization | Before | After | Improvement |
|--------------|--------|-------|-------------|
| Page read (cached) | 168 ns | 56 ns | **3x faster** |
| Bloom filter membership | - | 173 ns | Fast negative lookups |
| Cache hit rate | - | 70%+ | Typical workload |

Run benchmarks yourself:
```bash
cargo bench --bench trie_comparison  # Compare against other tries
cargo bench --bench benchmarks       # Full benchmark suite
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
- **173 tests** (unit + integration + Ethereum compatibility)
- **Property-based tests** using `proptest` for NibblePath, SlottedArray, and MerkleTrie
- **End-to-end tests** covering multi-block chains, forks, storage operations, state persistence

### Fuzzing

The project includes fuzz targets for critical data structures:

```bash
# Install cargo-fuzz if needed
cargo install cargo-fuzz

# Run fuzzers
cargo fuzz run fuzz_nibble_path
cargo fuzz run fuzz_slotted_array
cargo fuzz run fuzz_merkle_trie
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

### Phase 7: Production Features ✅

- [x] COW integration with `abandon_page()` - automatic tracking during Copy-on-Write
- [x] AbandonedPage overflow handling - linked list for large abandoned sets
- [x] SlottedArray defragmentation - `defragment()`, `wasted_space()`, `needs_defragmentation()`
- [x] Merkle proof generation - `ProofNode`, `MerkleProof`, `generate_proof()`, `verify()`
- [x] Snapshot/checkpoint support - `create_snapshot()`, `restore_snapshot()`, `export()`, `import()`

### Phase 8: Observability & Testing ✅

- [x] Metrics module (`DbMetrics`, `MetricsSnapshot`)
  - Page allocations, reuse, abandonment tracking
  - COW operations, batch commits/aborts
  - Bytes read/written, snapshot operations
- [x] Fuzz testing with `cargo-fuzz`
  - `fuzz_nibble_path` - NibblePath operations
  - `fuzz_slotted_array` - SlottedArray with consistency verification
  - `fuzz_merkle_trie` - Trie operations with expected state tracking

### Phase 9: Ethereum Compatibility ✅

**Option A: Ethereum Compatibility Verification**
- [x] Test state roots against real mainnet blocks (genesis accounts, precompiles)
- [x] Verify RLP encoding matches Ethereum spec exactly (54 test vectors)
- [x] Add test vectors from ethereum/tests repository patterns
- [x] Ensure Merkle proofs are compatible with other clients

Test coverage includes:
- RLP encoding (empty, bytes, strings, lists, integers, HP encoding)
- Keccak-256 hash verification (empty, RLP empty, known values)
- Basic trie operations (insert, delete, branch, extension nodes)
- Secure trie (keccak-hashed keys, as used in Ethereum state trie)
- Account RLP encoding (nonce, balance, storageRoot, codeHash)
- Mainnet verification (genesis structure, precompile accounts, large balances)
- Merkle proof generation and verification (inclusion/exclusion)
- Storage trie operations (slots, deletion)

### Phase 10: Performance Optimizations ✅

**Option D: Performance Optimizations**
- [x] LRU page cache for hot data (256 pages, **3x speedup** for cached reads)
- [x] Bloom filters for non-existence proofs (~173 ns membership check)
- [ ] Contract ID system (like Paprika) for storage efficiency
- [ ] Background compaction of abandoned pages

Performance improvements:
- **Page cache**: 168 ns (miss) → 56 ns (hit) = 3x faster repeated reads
- **Bloom filter**: Fast rejection of non-existent keys without HashMap lookup
- Cache hit rate in typical workloads: 70%+

### ethrex Integration Verification ✅

**State EF Tests (verified 2026-01-21):**
- [x] 100% pass rate on Ethereum Foundation state tests
- [x] All GeneralStateTests passing (Cancun, Shanghai, Prague forks)
- [x] All state_tests passing (EIP-7702, EIP-1153, EIP-6780, etc.)
- [x] All LegacyTests passing
- [x] Storage fix validated through comprehensive test suite

Run the tests:
```bash
cd tooling/ef_tests/state && make test-levm
```

### Phase 11: Planned

**Option B: Durability & Crash Recovery**
- [ ] Write-ahead logging (WAL)
- [ ] Proper fsync strategies for commit durability
- [ ] Recovery from incomplete/corrupted writes
- [ ] Atomic metadata updates

**Option C: Production Hardening**
- [ ] Better error handling and recovery paths
- [ ] Memory pressure handling (graceful degradation)
- [ ] OpenTelemetry/tracing integration
- [ ] Connection pooling for concurrent access

**Option E: Genesis State Verification**
- [ ] Full genesis state root verification (~8800 accounts)
- [ ] Load and verify against complete Ethereum mainnet genesis

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
