# ethrex_db Integration Plan

## Overview

**Objective**: Integrate [ethrex_db](https://github.com/lambdaclass/ethrex-db) into ethrex as the primary storage backend for state and storage tries, while keeping RocksDB for other blockchain data (blocks, headers, receipts, etc.).

**Why ethrex_db?**
- Paprika-inspired, LMDB-like memory-mapped page-based storage
- 1.6-2.2x faster inserts and 10-15x faster lookups vs. traditional trie implementations
- Avoids LSM-tree write amplification (10-30x in RocksDB)
- Native support for Ethereum concepts: blocks, finality, Fork Choice, reorgs
- Copy-on-Write concurrency: single writer, multiple lock-free readers
- Two-tier storage: hot (unfinalized blocks) + cold (finalized state)

---

## Current Architecture Summary

### ethrex Storage (crates/storage/)

**Backend trait**: `StorageBackend` in `api/mod.rs`
- `begin_read() -> StorageReadView`
- `begin_write() -> StorageWriteBatch`
- `begin_locked() -> StorageLockedView`
- `create_checkpoint()`

**18 Tables**:

| Category | Tables |
|----------|--------|
| State Tries | `ACCOUNT_TRIE_NODES`, `STORAGE_TRIE_NODES`, `ACCOUNT_FLATKEYVALUE`, `STORAGE_FLATKEYVALUE` |
| Account Data | `ACCOUNT_CODES` |
| Blocks | `CANONICAL_BLOCK_HASHES`, `BLOCK_NUMBERS`, `HEADERS`, `BODIES`, `FULLSYNC_HEADERS` |
| Transactions | `RECEIPTS`, `TRANSACTION_LOCATIONS` |
| Chain State | `CHAIN_DATA`, `SNAP_STATE`, `PENDING_BLOCKS`, `INVALID_CHAINS` |
| Misc | `MISC_VALUES`, `EXECUTION_WITNESSES` |

**Caching layers**:
- `TrieLayerCache`: 128 block diff layers in memory
- `CodeCache`: 64 MB LRU for contract bytecode

### ethrex_db Architecture

**Modules**:
- `chain`: Block, Blockchain, WorldState, ReadOnlyWorldState, Account
- `store`: PagedDb (memory-mapped cold storage)
- `data`: NibblePath, SlottedArray
- `merkle`: MerkleTrie (flat KV with on-demand Merkle root computation)

**Key concepts**:
- Hot storage (Blockchain): Unfinalized blocks with Copy-on-Write
- Cold storage (PagedDb): Finalized state, 4KB pages, memory-mapped
- Native Fork Choice support

---

## Integration Strategy

### Approach: Hybrid Backend

Create a new backend that combines:
- **ethrex_db**: State trie + Storage tries (the performance-critical path)
- **RocksDB**: Everything else (blocks, headers, receipts, chain data)

**Rationale**:
1. ethrex_db is optimized for state/storage trie operations (the hottest path)
2. Block/header/receipt access patterns are different (sequential writes, indexed reads)
3. RocksDB's compression is beneficial for block data
4. Minimal changes to existing code - only trie storage changes

---

## Implementation Phases

### Phase 1: Add ethrex_db Dependency and Explore API

**Goal**: Understand ethrex_db's API and ensure it compiles with ethrex.

**Tasks**:
1. Clone ethrex_db locally or add as git dependency
2. Study the public API:
   - `Blockchain::new()`, `add_block()`, `finalize()`
   - `WorldState` and `ReadOnlyWorldState` access patterns
   - `PagedDb` persistence model
3. Write test harness to verify basic operations
4. Document API mapping: ethrex concepts → ethrex_db concepts

**Deliverables**:
- [x] API documentation for ethrex_db integration (see below)
- [x] Test harness demonstrating basic usage (`crates/storage/tests/ethrex_db_integration.rs`)
- [x] Dependency added to workspace (git submodule at `crates/storage/ethrex-db`)

### Phase 1 Completion: API Summary

**ethrex_db Public API**:

```rust
// Core types from chain module
pub use ethrex_db::chain::{
    Block,              // Mutable block for state changes
    BlockId,            // (number, hash) identifier
    Blockchain,         // Main orchestrator for hot/cold storage
    BlockchainError,    // Error type
    Account,            // Account state (nonce, balance, code_hash, storage_root)
    WorldState,         // Trait for mutable state access
    ReadOnlyWorldState, // Trait for read-only state access
};

// Storage layer
pub use ethrex_db::store::{
    PagedDb,            // Memory-mapped cold storage
    PagedStateTrie,     // Persistent state trie
    AccountData,        // Account data for trie storage
    DbError,            // Storage errors
    CommitOptions,      // Flush options (FlushDataOnly, FlushDataAndRoot, DangerNoFlush)
};

// Merkle computation
pub use ethrex_db::merkle::{
    MerkleTrie,         // In-memory trie for root computation
    EMPTY_ROOT,         // Empty trie root constant
};
```

**Key Blockchain Methods**:

| Method | Description |
|--------|-------------|
| `Blockchain::new(db)` | Create blockchain from PagedDb |
| `start_new(parent, hash, num)` | Create new block on parent |
| `commit(block)` | Make block queryable (not finalized) |
| `finalize(hash)` | Persist to cold storage |
| `fork_choice_update(head, safe, finalized)` | Handle FCU |
| `get_account(block_hash, addr)` | Get account from committed block |
| `get_storage(block_hash, addr, key)` | Get storage from committed block |
| `get_finalized_account(addr)` | Get account from finalized state |
| `state_root()` | Get current state root hash |
| `last_finalized_number()` / `last_finalized_hash()` | Finalization metadata |

**Key Block Methods (WorldState trait)**:

| Method | Description |
|--------|-------------|
| `set_account(addr, account)` | Set account in block |
| `get_account(addr)` | Get account from block |
| `set_storage(addr, key, value)` | Set storage slot |
| `get_storage(addr, key)` | Get storage slot |
| `delete_account(addr)` | Delete account |

---

### Phase 2: Design Interface Mapping

**Goal**: Design how ethrex's storage interface maps to ethrex_db.

**Detailed API Mapping**:

| ethrex Concept | ethrex_db Concept | Notes |
|----------------|-------------------|-------|
| `Store` | `Blockchain` | Main orchestrator |
| `StorageBackend` | `PagedDb` | Low-level storage |
| `BackendTrieDB` | `PagedStateTrie` | State trie interface |
| `TrieLayerCache` | `Blockchain` hot storage | ethrex_db handles this internally |
| `TrieDB::get()` | `PagedStateTrie::get_account()` | Account access |
| `TrieDB::write_trie_update()` | `Block::set_account()` + `commit()` | State updates |
| Account trie nodes | `PagedStateTrie` | Merkle Patricia Trie |
| Storage trie nodes | `StorageTrie` (per account) | Via `PagedStateTrie::storage_trie()` |
| State root | `Blockchain::state_root()` | 32-byte hash |
| `forkchoice_update()` | `Blockchain::fork_choice_update()` | Native FCU support |
| Finalization | `Blockchain::finalize()` | Moves to cold storage |
| Snap-sync | `Blockchain::new_with_state_trie()` + `persist_state_trie_checkpoint()` | Supported |

**Address Handling**:
- ethrex uses `Address` (20 bytes) and `H256` (32 bytes for keccak hash)
- ethrex_db uses `H256` for addresses in `WorldState` trait (hashed addresses)
- ethrex_db uses `[u8; 20]` for addresses in `PagedStateTrie` (raw addresses)
- Conversion: `H256::from_low_u64_be()` for test addresses, `keccak256(address)` for real addresses

**Storage Key Handling**:
- Both use `H256` for storage slot keys
- Both use `U256` for storage values
- ethrex_db converts to big-endian bytes internally

**Questions Resolved**:

1. **Storage root per account**: ethrex_db handles this via `PagedStateTrie::storage_trie(&addr)`. Each account has its own storage trie, and the storage_root is computed automatically.

2. **forkchoice_update integration**: ethrex_db has native `Blockchain::fork_choice_update(head, safe, finalized)` that maps directly to Ethereum's FCU.

3. **Block awareness**: Yes, ethrex_db is fully block-aware. Use `start_new()` → modify via `WorldState` → `commit()` → `finalize()`.

4. **Snap-sync**: Supported via:
   - `Blockchain::new_with_state_trie()` - initialize with pre-built trie
   - `persist_state_trie_checkpoint()` - save incremental progress
   - `get_finalized_account_by_hash()` / `get_finalized_storage_by_hash()` - query by hash

**Deliverables**:
- [x] Interface mapping document (above)
- [ ] Design decision document for edge cases (Phase 6)

---

### Phase 3: Implement Hybrid Backend

**Goal**: Create `EthrexDbBackend` that implements `StorageBackend`.

**File structure**:
```
crates/storage/
├── backend/
│   ├── mod.rs                    # Add ethrex_db module
│   ├── in_memory.rs
│   ├── rocksdb.rs
│   └── ethrex_db.rs              # NEW: Hybrid backend
```

**Implementation details**:

```rust
// backend/ethrex_db.rs (conceptual)

pub struct EthrexDbBackend {
    /// State and storage tries - uses ethrex_db
    state_db: ethrex_db::Blockchain,

    /// All other data - uses RocksDB
    auxiliary_db: RocksDB,
}

impl StorageBackend for EthrexDbBackend {
    fn begin_read(&self) -> Box<dyn StorageReadView + '_> {
        Box::new(EthrexDbReadView {
            state_snapshot: self.state_db.snapshot(),
            aux_read: self.auxiliary_db.begin_read(),
        })
    }

    fn begin_write(&self) -> Box<dyn StorageWriteBatch + 'static> {
        Box::new(EthrexDbWriteBatch {
            state_changes: Vec::new(),
            aux_batch: self.auxiliary_db.begin_write(),
        })
    }
    // ...
}
```

**Tables routing**:

| Tables | Backend |
|--------|---------|
| `ACCOUNT_TRIE_NODES`, `STORAGE_TRIE_NODES` | ethrex_db |
| `ACCOUNT_FLATKEYVALUE`, `STORAGE_FLATKEYVALUE` | ethrex_db (if supported) or RocksDB |
| All others (14 tables) | RocksDB |

**Deliverables**:
- [x] `EthrexDbBackend` struct implementing `StorageBackend` (`backend/ethrex_db.rs`)
- [x] `EthrexDbReadView` implementing `StorageReadView`
- [x] `EthrexDbWriteBatch` implementing `StorageWriteBatch`
- [x] `EthrexDbLockedView` implementing `StorageLockedView`
- [x] Feature flag `ethrex-db` in Cargo.toml
- [x] `EngineType::EthrexDb` variant added to store.rs
- [x] 4 unit tests for hybrid backend

**Implementation Notes**:
- Tables routed to ethrex-db: `ACCOUNT_TRIE_NODES`, `STORAGE_TRIE_NODES`, `ACCOUNT_FLATKEYVALUE`, `STORAGE_FLATKEYVALUE`
- Added `pending_trie_writes` cache to bridge write batch model with ethrex-db's block model
- Storage layout: `state.db` (PagedDb file) + `auxiliary/` (RocksDB directory)

---

### Phase 4: Adapt Trie Interface

**Goal**: Connect `BackendTrieDB` to use ethrex_db for state/storage operations.

**Current trie interface** (`trie.rs`):
```rust
impl<'a> TrieDB for BackendTrieDB<'a> {
    fn get(&self, path: &[u8]) -> Option<Vec<u8>>;
    fn write_trie_update(&mut self, updates: &[(Vec<u8>, Option<Vec<u8>>)]);
}
```

**New implementation**:
- When backend is `EthrexDbBackend`:
  - Delegate to `WorldState::get_account()` / `set_account()`
  - Handle address → account mapping
  - Handle storage key → value mapping
- State root: Use ethrex_db's `MerkleTrie::root()`

**Considerations**:
- ethrex_db may have different path encoding (NibblePath)
- Need to adapt nibble format if different
- Storage trie isolation: ethrex uses address prefix, ethrex_db may use separate tries

**Deliverables**:
- [x] Updated `BackendTrieDB` with ethrex_db support
- [x] Adapters for path encoding if needed
- [x] Storage trie routing to correct account

**Implementation Notes**:
- Added `ParsedTrieKey` enum to represent parsed nibble keys (AccountLeaf, StorageLeaf, IntermediateNode)
- Implemented `parse_trie_key()` to convert nibble-based paths to address/slot hashes
- Account leaf keys: 65 nibbles = 64 nibbles (keccak256(address)) + terminator
- Storage leaf keys: 131 nibbles = 64 nibbles (address hash) + separator(17) + 64 nibbles (slot hash) + terminators
- `EthrexDbReadView::get()` and `EthrexDbLockedView::get()` now query ethrex_db's finalized state for leaf data
- Added RLP encoding helpers for account state and storage values
- 6 new unit tests for path translation utilities

---

### Phase 5: Integrate with Block Execution Flow

**Goal**: Wire ethrex_db into block execution and state updates.

**Current flow** (`store.rs`):
```
1. Execute block → state updates
2. Put updates in TrieLayerCache (in-memory)
3. Background thread: persist when threshold reached (128 blocks)
4. forkchoice_update() → finalize blocks
```

**New flow with ethrex_db**:
```
1. Execute block → state updates
2. Push to ethrex_db's Blockchain (hot storage)
3. ethrex_db handles in-memory caching automatically
4. forkchoice_update() → call ethrex_db::finalize()
5. ethrex_db persists finalized state to PagedDb (cold storage)
```

**Key integration points**:
- `Store::add_block()`: Create block in ethrex_db Blockchain
- `Store::store_block_updates()`: Write state to WorldState
- `Store::forkchoice_update()`: Call ethrex_db finalize
- Reorg handling: ethrex_db should handle naturally via Fork Choice

**Deliverables**:
- [ ] Block execution integration
- [ ] forkchoice_update() adaptation
- [ ] Reorg handling verification
- [ ] Commit threshold removal (ethrex_db handles internally)

---

### Phase 6: Handle Edge Cases

**Goal**: Address special scenarios.

**Edge cases**:

1. **Snap-sync**: ethrex uses `BackendTrieDBLocked` for consistent snapshots
   - Verify ethrex_db supports equivalent functionality
   - May need adapter or alternative approach

2. **Execution witnesses**: Currently stored in RocksDB, keep as-is

3. **FlatKeyValue generator**:
   - ethrex_db may not need this (different access pattern)
   - Keep for RocksDB fallback or remove if redundant

4. **Code cache**: Keep as-is (codes in RocksDB)

5. **Checkpoints**: RocksDB checkpoint for auxiliary data, ethrex_db has own mechanism

**Deliverables**:
- [ ] Snap-sync compatibility
- [ ] FlatKeyValue decision (keep/remove)
- [ ] Checkpoint mechanism for hybrid backend

---

### Phase 7: Testing and Benchmarking

**Goal**: Verify correctness and measure performance.

**Test plan**:

1. **Unit tests**: Backend trait implementation
2. **Integration tests**: Full block execution with state verification
3. **State root verification**: Compare roots against RocksDB backend
4. **Reorg tests**: Verify correct state after Fork Choice changes
5. **Concurrent access**: Multiple readers, single writer

**Benchmarks**:

| Metric | Baseline (RocksDB) | Target (ethrex_db) |
|--------|-------------------|-------------------|
| State trie insert | X ops/sec | 1.5-2x faster |
| State trie lookup | Y ops/sec | 10-15x faster |
| Block execution | Z blocks/sec | Improved |
| State root computation | W ms | Comparable or faster |
| Disk usage | A GB | Reduced (no LSM amplification) |

**Test datasets**:
- Mainnet sync (first 1M blocks)
- Hive test suite
- Custom stress tests

**Deliverables**:
- [ ] Full test suite passing
- [ ] Benchmark results documented
- [ ] Performance regression tests

---

### Phase 8: Documentation and Cleanup

**Goal**: Document the new backend and clean up.

**Documentation**:
- Update `crates/storage/README.md`
- Add ethrex_db backend usage guide
- Document configuration options
- Add architecture diagram showing hybrid backend

**Cleanup**:
- Remove dead code paths
- Optimize feature flags
- Ensure backward compatibility with RocksDB-only builds

**Deliverables**:
- [ ] Updated documentation
- [ ] Clean feature flag structure
- [ ] Migration guide for existing databases

---

## Risk Assessment

| Risk | Impact | Mitigation |
|------|--------|------------|
| API incompatibility | High | Phase 1 exploration, early prototyping |
| Performance regression | Medium | Comprehensive benchmarking, keep RocksDB fallback |
| State root mismatch | High | Extensive testing against known-good roots |
| Snap-sync breakage | Medium | Early investigation in Phase 6 |
| ethrex_db instability | Medium | Pin version, fork if needed |

---

## Timeline (Approximate)

| Phase | Effort | Dependencies |
|-------|--------|--------------|
| Phase 1: Explore API | 2-3 days | None |
| Phase 2: Design mapping | 2-3 days | Phase 1 |
| Phase 3: Implement backend | 1-2 weeks | Phase 2 |
| Phase 4: Adapt trie | 1 week | Phase 3 |
| Phase 5: Block execution | 1 week | Phase 4 |
| Phase 6: Edge cases | 1 week | Phase 5 |
| Phase 7: Testing | 1-2 weeks | Phase 6 |
| Phase 8: Documentation | 2-3 days | Phase 7 |

**Total**: ~6-8 weeks

---

## Open Questions for User

1. Should we support running both backends simultaneously (A/B testing)?
2. Priority: full integration or state-trie-only first?
3. Are there specific performance targets to hit?
4. Should we consider contributing improvements back to ethrex_db?
5. Do we need database migration tooling for existing RocksDB databases?

---

## Appendix: Tables by Backend

### ethrex_db (State/Storage Tries)

| Table | Purpose |
|-------|---------|
| `ACCOUNT_TRIE_NODES` | State trie node data |
| `STORAGE_TRIE_NODES` | Storage trie node data |
| `ACCOUNT_FLATKEYVALUE` | Pre-computed account leaf values (maybe) |
| `STORAGE_FLATKEYVALUE` | Pre-computed storage leaf values (maybe) |

### RocksDB (Everything Else)

| Table | Purpose |
|-------|---------|
| `CANONICAL_BLOCK_HASHES` | Block number → hash mapping |
| `BLOCK_NUMBERS` | Hash → block number mapping |
| `HEADERS` | Block headers |
| `BODIES` | Block bodies (transactions, uncles) |
| `RECEIPTS` | Transaction receipts |
| `TRANSACTION_LOCATIONS` | Tx hash → location |
| `ACCOUNT_CODES` | Contract bytecode |
| `CHAIN_DATA` | Chain configuration, latest/safe/finalized |
| `SNAP_STATE` | Snap-sync progress |
| `PENDING_BLOCKS` | Unvalidated blocks |
| `INVALID_CHAINS` | Invalid block tracking |
| `FULLSYNC_HEADERS` | Full-sync headers |
| `MISC_VALUES` | General purpose KV |
| `EXECUTION_WITNESSES` | zkVM execution proofs |
