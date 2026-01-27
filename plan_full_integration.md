# ethrex_db Full Integration Plan

## Overview

**Objective**: Complete the ethrex_db integration so that state operations actually flow through ethrex_db's optimized storage engine, delivering the promised performance benefits.

**Current State**: The hybrid backend infrastructure exists, but state operations still use the existing trie path (TrieLayerCache → RocksDB). ethrex_db is essentially unused for state management.

**Target State**: All state reads and writes go through ethrex_db's Blockchain API, leveraging its:
- Copy-on-Write concurrency (single writer, multiple lock-free readers)
- Two-tier storage (hot unfinalized blocks, cold finalized state)
- Flat key-value storage (10-15x faster lookups)
- No LSM write amplification (1.5-2x faster inserts)

---

## Current Architecture

```
Block Execution
      │
      ▼
AccountUpdate[]
      │
      ▼
┌─────────────────────────────────────────────────────────────────┐
│                     Store::store_block_updates()                 │
│  1. Send TrieUpdate to background worker                        │
│  2. Worker applies updates to TrieLayerCache                    │
│  3. TrieLayerCache persists to RocksDB after 128 blocks         │
└─────────────────────────────────────────────────────────────────┘
      │
      ▼
┌─────────────────────────────────────────────────────────────────┐
│                        TrieLayerCache                            │
│  - In-memory diff layers keyed by state_root                    │
│  - Each layer: HashMap<path, node_data>                         │
│  - Bloom filter for fast negative lookups                       │
│  - Persists oldest layers to RocksDB when threshold reached     │
└─────────────────────────────────────────────────────────────────┘
      │
      ▼
┌─────────────────────────────────────────────────────────────────┐
│                    RocksDB (Trie Tables)                         │
│  ACCOUNT_TRIE_NODES, STORAGE_TRIE_NODES                         │
│  ACCOUNT_FLATKEYVALUE, STORAGE_FLATKEYVALUE                     │
└─────────────────────────────────────────────────────────────────┘
```

---

## Target Architecture

```
Block Execution
      │
      ▼
AccountUpdate[]
      │
      ▼
┌─────────────────────────────────────────────────────────────────┐
│                 Store::store_block_updates()                     │
│  1. Get/create ethrex_db Block for this execution               │
│  2. Apply AccountUpdates via WorldState API                     │
│  3. Commit block to ethrex_db Blockchain                        │
│  4. Store block header/body in RocksDB (auxiliary)              │
└─────────────────────────────────────────────────────────────────┘
      │
      ▼
┌─────────────────────────────────────────────────────────────────┐
│                   ethrex_db Blockchain                           │
│  - Hot storage: unfinalized blocks (Copy-on-Write)              │
│  - get_account(block_hash, addr) → Account                      │
│  - get_storage(block_hash, addr, slot) → U256                   │
│  - fork_choice_update(head, safe, finalized)                    │
└─────────────────────────────────────────────────────────────────┘
      │
      ▼ finalize()
┌─────────────────────────────────────────────────────────────────┐
│                     ethrex_db PagedDb                            │
│  - Cold storage: finalized state                                │
│  - Memory-mapped 4KB pages                                      │
│  - Flat key-value for accounts and storage                      │
│  - On-demand Merkle root computation                            │
└─────────────────────────────────────────────────────────────────┘
```

---

## Implementation Phases

### Phase 1: State Write Path Integration

**Goal**: Make `store_block_updates()` write state to ethrex_db.

**Current flow** (`store.rs:1244-1336`):
```rust
pub fn store_block_updates(&self, update_batch: UpdateBatch) -> Result<(), StoreError> {
    // 1. Extract account_updates, storage_updates
    // 2. Send to trie_update_worker (TrieLayerCache)
    // 3. Store headers/bodies/receipts in RocksDB
}
```

**New flow**:
```rust
pub fn store_block_updates(&self, update_batch: UpdateBatch) -> Result<(), StoreError> {
    #[cfg(feature = "ethrex-db")]
    if let Some(blockchain_ref) = &self.ethrex_blockchain {
        return self.store_block_updates_ethrex_db(update_batch, blockchain_ref);
    }

    // Fallback to existing implementation for non-ethrex-db backends
    self.apply_updates(update_batch)
}

#[cfg(feature = "ethrex-db")]
fn store_block_updates_ethrex_db(
    &self,
    update_batch: UpdateBatch,
    blockchain_ref: &BlockchainRef,
) -> Result<(), StoreError> {
    let mut blockchain = blockchain_ref.0.write()?;

    for block in &update_batch.blocks {
        let parent_hash = block.header.parent_hash;
        let block_hash = block.hash();
        let block_number = block.header.number;

        // 1. Create new block in ethrex_db
        let mut ethrex_block = blockchain.start_new(
            H256::from(parent_hash.0),
            H256::from(block_hash.0),
            block_number,
        )?;

        // 2. Apply account updates
        for update in &update_batch.account_updates {
            self.apply_account_update_to_ethrex_db(&mut ethrex_block, update)?;
        }

        // 3. Apply storage updates
        for (address, slots) in &update_batch.storage_updates {
            for (slot, value) in slots {
                let addr_hash = keccak256(address);
                let slot_hash = keccak256(slot);
                ethrex_block.set_storage(
                    H256::from(addr_hash.0),
                    H256::from(slot_hash.0),
                    *value,
                );
            }
        }

        // 4. Commit block to hot storage
        blockchain.commit(ethrex_block)?;
    }

    // 5. Store headers/bodies/receipts in RocksDB auxiliary
    self.store_block_metadata(&update_batch)?;

    Ok(())
}

fn apply_account_update_to_ethrex_db(
    &self,
    block: &mut ethrex_db::chain::Block,
    update: &AccountUpdate,
) -> Result<(), StoreError> {
    let addr_hash = H256::from(keccak256(&update.address).0);

    if update.removed {
        block.delete_account(addr_hash);
        return Ok(());
    }

    if let Some(info) = &update.info {
        // Get existing account or create new one
        let storage_root = block.get_account(&addr_hash)
            .map(|a| a.storage_root)
            .unwrap_or(H256::from(EMPTY_ROOT));

        block.set_account(addr_hash, ethrex_db::chain::Account {
            nonce: info.nonce,
            balance: info.balance,
            storage_root,
            code_hash: H256::from(info.code_hash.0),
        });
    }

    // Apply storage changes
    for (slot, value) in &update.added_storage {
        let slot_hash = H256::from(keccak256(slot).0);
        block.set_storage(addr_hash, slot_hash, *value);
    }

    Ok(())
}
```

**Tasks**:
- [ ] Add `store_block_updates_ethrex_db()` method to Store
- [ ] Add `apply_account_update_to_ethrex_db()` helper
- [ ] Add `store_block_metadata()` helper for RocksDB auxiliary writes
- [ ] Handle code storage (ethrex_db may not store code - keep in RocksDB)
- [ ] Add tests for state write path

**Files to modify**:
- `crates/storage/store.rs`

---

### Phase 2: State Read Path Integration

**Goal**: Make `get_account_info()` and `get_storage_at()` read from ethrex_db.

**Current flow** (`store.rs:1500-1530`):
```rust
pub fn get_account_info_by_hash(&self, block_hash: BlockHash, address: Address) -> ... {
    let state_trie = self.state_trie(block_hash)?;  // Uses TrieLayerCache + RocksDB
    let encoded_state = state_trie.get(hashed_address)?;
    AccountState::decode(&encoded_state)
}
```

**New flow**:
```rust
pub fn get_account_info_by_hash(&self, block_hash: BlockHash, address: Address) -> ... {
    #[cfg(feature = "ethrex-db")]
    if let Some(blockchain_ref) = &self.ethrex_blockchain {
        return self.get_account_info_ethrex_db(block_hash, address, blockchain_ref);
    }

    // Fallback to existing implementation
    let state_trie = self.state_trie(block_hash)?;
    // ...
}

#[cfg(feature = "ethrex-db")]
fn get_account_info_ethrex_db(
    &self,
    block_hash: BlockHash,
    address: Address,
    blockchain_ref: &BlockchainRef,
) -> Result<Option<AccountInfo>, StoreError> {
    let blockchain = blockchain_ref.0.read()?;
    let addr_hash = H256::from(keccak256(&address).0);
    let block_hash = H256::from(block_hash.0);

    // Try committed (hot) blocks first
    if let Some(account) = blockchain.get_account(&block_hash, &addr_hash) {
        return Ok(Some(AccountInfo {
            nonce: account.nonce,
            balance: account.balance,
            code_hash: H256::from(account.code_hash.0),
        }));
    }

    // Fall back to finalized (cold) state
    let addr_bytes: [u8; 20] = address.0;
    if let Some(account) = blockchain.get_finalized_account(&addr_bytes) {
        return Ok(Some(AccountInfo {
            nonce: account.nonce,
            balance: account.balance,
            code_hash: H256::from(account.code_hash.0),
        }));
    }

    Ok(None)
}
```

**Tasks**:
- [ ] Add `get_account_info_ethrex_db()` method
- [ ] Add `get_storage_at_ethrex_db()` method
- [ ] Handle the block_hash → ethrex_db block mapping
- [ ] Handle finalized vs unfinalized state queries
- [ ] Add tests for state read path

**Files to modify**:
- `crates/storage/store.rs`

---

### Phase 3: State Root Computation

**Goal**: Use ethrex_db's state root instead of computing via TrieLayerCache.

**Current flow**:
- Block execution computes state root via `Trie::collect_changes_since_last_hash()`
- State root stored in block header

**Challenge**:
- ethrex_db computes state root lazily via `MerkleTrie::root_hash()`
- Block execution needs the state root before committing

**Options**:

**Option A: Compute root after commit**
```rust
// In store_block_updates_ethrex_db:
blockchain.commit(ethrex_block)?;
let state_root = blockchain.state_root();  // Computed on demand
// Verify it matches block header
assert_eq!(state_root, block.header.state_root);
```

**Option B: Let block execution query ethrex_db for root**
- Modify block execution to call `Store::compute_state_root()`
- Which queries ethrex_db's `Blockchain::state_root()`

**Recommended**: Option A (simpler, verification-based)

**Tasks**:
- [ ] Add state root verification after commit
- [ ] Handle state root mismatch (indicates bug in state application)
- [ ] Add `Store::compute_state_root_ethrex_db()` for block producers
- [ ] Add tests comparing roots against known values

**Files to modify**:
- `crates/storage/store.rs`
- `crates/vm/` (if block execution needs modification)

---

### Phase 4: Finalization Integration

**Goal**: Make `forkchoice_update()` properly finalize state in ethrex_db.

**Current flow** (`store.rs:2116-2175`):
```rust
pub async fn forkchoice_update(...) -> ... {
    self.forkchoice_update_inner(...).await?;

    #[cfg(feature = "ethrex-db")]
    if let Some(blockchain_ref) = &self.ethrex_blockchain {
        // Try to finalize in ethrex_db (currently fails - blocks not in Blockchain)
    }
}
```

**New flow** (after Phase 1 completes):
```rust
pub async fn forkchoice_update(...) -> ... {
    self.forkchoice_update_inner(...).await?;

    #[cfg(feature = "ethrex-db")]
    if let Some(blockchain_ref) = &self.ethrex_blockchain && let Some(finalized_number) = finalized {
        let blockchain = blockchain_ref.0.write()?;

        // Now blocks exist in ethrex_db's Blockchain (from Phase 1)
        let finalized_hash = self.get_canonical_block_hash_sync(finalized_number)?;
        if let Some(hash) = finalized_hash {
            blockchain.finalize(H256::from(hash.0))?;
            // Moves finalized state to PagedDb (cold storage)
        }
    }
}
```

**Tasks**:
- [ ] Verify finalization works after Phase 1 (blocks exist in Blockchain)
- [ ] Handle finalization errors gracefully
- [ ] Add metrics for finalization timing
- [ ] Test chain reorganizations

**Files to modify**:
- `crates/storage/store.rs`

---

### Phase 5: Remove TrieLayerCache for ethrex_db

**Goal**: Disable TrieLayerCache when using ethrex_db (it's now redundant).

**Current state**:
- TrieLayerCache still active even with ethrex_db
- Wastes memory and CPU
- Potential consistency issues

**Tasks**:
- [ ] Skip TrieLayerCache initialization for ethrex_db backend
- [ ] Remove trie_update_worker channel for ethrex_db
- [ ] Update `state_trie()` to skip TrieLayerCache for ethrex_db
- [ ] Keep TrieLayerCache for RocksDB/InMemory backends

**Files to modify**:
- `crates/storage/store.rs`
- `crates/storage/layering.rs` (if needed)

---

### Phase 6: Code Storage Handling

**Goal**: Ensure contract code storage works correctly with hybrid backend.

**Decision**: ethrex_db does NOT support code storage. Keep code in RocksDB `ACCOUNT_CODES` table.

**Current code storage** (no changes needed):
```rust
// In store_block_updates:
for (code_hash, code) in update_batch.code_updates {
    tx.put(ACCOUNT_CODES, code_hash.as_ref(), &buf)?;  // Goes to RocksDB auxiliary
}
```

**Tasks**:
- [ ] Verify `get_account_code()` works correctly with hybrid backend
- [ ] Ensure code is written to RocksDB auxiliary in Phase 1
- [ ] Add test for code storage and retrieval

**Files to modify**:
- `crates/storage/store.rs` (verification only, likely no changes)

---

### Phase 7: Historical State Access

**Goal**: Support reading state at any block within the last 256 blocks.

**Decision**: Maximum history depth is **256 blocks**. This matches Ethereum's BLOCKHASH opcode limit.

**Implementation**:
- Hot storage: Keep last 256 blocks in ethrex_db Blockchain
- Finalize blocks older than 256 blocks behind head
- Queries for blocks older than 256 return error (or require archive node)

**Configuration**:
```rust
const HISTORY_DEPTH: u64 = 256;

// In forkchoice_update:
if head_number > HISTORY_DEPTH {
    let finalize_up_to = head_number - HISTORY_DEPTH;
    // Finalize blocks up to this point
}
```

**Tasks**:
- [ ] Add `HISTORY_DEPTH` constant (256)
- [ ] Implement automatic finalization based on history depth
- [ ] Return appropriate error for queries beyond history depth
- [ ] Add test for boundary cases (block 256, 257, etc.)

---

### Phase 8: Performance Optimization

**Goal**: Ensure we actually get the performance benefits.

**Benchmarks to run**:
1. State lookup throughput (accounts/sec)
2. State write throughput (updates/sec)
3. Block execution speed (blocks/sec)
4. Memory usage during sync
5. Disk I/O during sync

**Tasks**:
- [ ] Create benchmark suite
- [ ] Run before/after comparison
- [ ] Profile hotspots
- [ ] Tune ethrex_db parameters (page size, cache size, etc.)

---

### Phase 9: Testing

**Test categories**:

1. **Unit tests**:
   - State write path
   - State read path
   - State root computation
   - Finalization

2. **Integration tests**:
   - Full block execution flow
   - Chain reorganization
   - Concurrent access
   - Restart/recovery

3. **Hive tests**:
   - Run full Hive suite with ethrex_db backend
   - Compare results with RocksDB backend

4. **Mainnet sync**:
   - Sync first 1M blocks
   - Verify state roots match

**Tasks**:
- [ ] Add unit tests for each phase
- [ ] Run Hive tests
- [ ] Mainnet sync test
- [ ] Add CI job for ethrex_db tests

---

## Type Conversions

**Address handling**:
```rust
// ethrex: Address (20 bytes)
// ethrex_db WorldState: H256 (hashed address)
// ethrex_db PagedStateTrie: [u8; 20] (raw address)

let addr_hash = H256::from(keccak256(&address).0);  // For WorldState
let addr_bytes: [u8; 20] = address.0;               // For PagedStateTrie
```

**Hash handling**:
```rust
// ethrex: ethrex_common::H256
// ethrex_db: primitive_types::H256

let ethrex_hash: ethrex_common::H256 = ...;
let ethrex_db_hash = primitive_types::H256::from(ethrex_hash.0);
```

---

## Risk Assessment

| Risk | Impact | Mitigation |
|------|--------|------------|
| State root mismatch | High | Verify root after every block, fail loudly |
| Performance regression | Medium | Benchmark before deploying |
| Data loss on crash | High | Use ethrex_db's flush options correctly |
| Memory exhaustion | Medium | Configure history depth, monitor usage |
| API incompatibility | Medium | Abstract behind Store interface |

---

## Timeline Estimate

| Phase | Effort | Dependencies |
|-------|--------|--------------|
| Phase 1: Write path | 1 week | None |
| Phase 2: Read path | 3-4 days | Phase 1 |
| Phase 3: State root | 2-3 days | Phase 1, 2 |
| Phase 4: Finalization | 2 days | Phase 1 |
| Phase 5: Remove TrieLayerCache | 1-2 days | Phase 1-4 |
| Phase 6: Code storage | 0.5 day | Phase 1 (verification only) |
| Phase 7: Historical access (256 blocks) | 2-3 days | Phase 1-4 |
| Phase 8: Optimization | 1 week | Phase 1-7 |
| Phase 9: Testing | 1-2 weeks | All phases |

**Total**: ~4-6 weeks

---

## Success Criteria

1. **Correctness**: All Hive tests pass
2. **Performance**:
   - State lookups ≥5x faster than RocksDB
   - State writes ≥1.5x faster than RocksDB
   - Block execution ≥20% faster
3. **Stability**: Mainnet sync completes without errors
4. **Memory**: No memory leaks, bounded memory usage

---

## Decisions (Resolved)

1. **Does ethrex_db support code storage, or should we keep code in RocksDB?**
   - **Answer**: ethrex_db doesn't support code storage. Keep code in RocksDB.
   - **Impact**: Phase 6 simplified - no changes needed for code storage.

2. **What's the maximum history depth we need to support?**
   - **Answer**: 256 blocks.
   - **Impact**: Phase 7 can use bounded hot storage (256 blocks in Blockchain).

3. **Should we support both backends simultaneously for A/B testing?**
   - **Answer**: No.
   - **Impact**: Simplifies implementation - no dual-backend mode needed.

4. **How do we handle snap-sync with ethrex_db?**
   - **Answer**: Will need additional work (separate effort).
   - **Impact**: Snap-sync integration deferred to future work.

5. **What's the migration path for existing RocksDB databases?**
   - **Answer**: No migration path needed.
   - **Impact**: Users start fresh with ethrex_db or continue with RocksDB.
