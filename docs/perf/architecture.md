# Performance Architecture Notes

This document contains architecture and code notes relevant to the performance improvement ideas in [ideas.md](ideas.md). Use this information to understand the codebase when implementing optimizations.

---

## Table of Contents

1. [LEVM Architecture](#levm-architecture)
2. [Block Execution Pipeline](#block-execution-pipeline)
3. [Trie and Storage Layer](#trie-and-storage-layer)
4. [Key Performance Bottlenecks](#key-performance-bottlenecks)
5. [Hot Path Summary](#hot-path-summary)

---

## LEVM Architecture

### Opcode Dispatch

**Location:** `crates/vm/levm/src/vm.rs:554-629`, `crates/vm/levm/src/opcodes.rs:376-449`

**Current Implementation: Hybrid Fast-Path + Lookup Table**

Hot opcodes have a direct match before the lookup table:
- PUSH1-PUSH32 (32 specialized handlers)
- DUP1-DUP16 (16 specialized handlers)
- SWAP1-SWAP16 (16 specialized handlers)
- ADD, CODECOPY, MLOAD, JUMP, JUMPI, JUMPDEST, TSTORE

Cold opcodes use a 256-element function pointer table: `[OpCodeFn; 256]`

**Performance Notes:**
- Hot path bypasses indirection for ~15% of all executed instructions
- Function pointers have branch prediction overhead
- Fork-specific tables built at VM construction (one-time cost)

### Stack Implementation

**Location:** `crates/vm/levm/src/call_frame.rs:17-220`

```rust
pub struct Stack {
    pub values: Box<[U256; 1024]>,  // Fixed 1024-element array (32 KB)
    pub offset: usize,              // Grows downward
}
```

**Key Operations (all use unsafe pointer ops):**
- `pop()`: Generic, pops N elements with `first_chunk::<N>()`
- `push()`: Uses `ptr::copy_nonoverlapping()` for U256 (4 u64s)
- `dup()`: Uses `ptr::copy_nonoverlapping()`
- `swap()`: Direct array `.swap()`

**Performance Notes:**
- Fixed allocation avoids dynamic resizing
- Unsafe pointer operations are well-justified and fast
- Stack pool reuse via `stack_pool: Vec<Stack>` reduces allocation

### Memory Implementation

**Location:** `crates/vm/levm/src/memory.rs:17-268`

```rust
pub struct Memory {
    pub buffer: Rc<RefCell<Vec<u8>>>,  // Shared across call frames
    pub len: usize,
    pub current_base: usize,
}
```

**Key Design:**
- `Rc<RefCell<>>` allows child call frames to share parent memory
- Lazy expansion, padded to 32-byte multiples
- Zero-initialization via `Vec::resize(new_size, 0)`

**Performance Notes:**
- RefCell has runtime borrow checking overhead (minimal)
- Memory expansion gas calculation: `floor(words²/128) + 3*words`
- Single growing Vec per transaction

### State Access from VM

**Location:** `crates/vm/levm/src/db/gen_db.rs:70-116, 471-526`

**Multi-tier Caching:**
1. `current_accounts_state` (FxHashMap) - hot cache
2. `initial_accounts_state` (FxHashMap) - transaction-start snapshot
3. Database backend - cold storage

**Storage Access Pattern (`get_storage_value()` line 486):**
1. Check `current_accounts_state.storage` HashMap
2. Fallback to `initial_accounts_state`
3. Load from database via `get_value_from_database()`

**Performance Notes:**
- FxHashMap (rustc_hash) is faster than std HashMap
- Every modification backed up in `CallFrameBackup` for reversion
- Storage uses `HashMap<H256, U256>` per account

### Gas Metering

**Location:** `crates/vm/levm/src/gas_cost.rs`, `call_frame.rs:379`

**Per-opcode tracking:**
```rust
pub fn increase_consumed_gas(&mut self, gas: u64) -> Result<(), ExceptionalHalt> {
    self.gas_remaining -= gas as i64;  // Signed for performance
    if self.gas_remaining < 0 {
        return Err(ExceptionalHalt::OutOfGas);
    }
    Ok(())
}
```

**Performance Notes:**
- Uses i64 for single subtraction + sign check (faster than u64 comparison)
- Gas limit bounded by EIP-7825 (2^24), safe in i64

### Hooks System

**Location:** `crates/vm/levm/src/hooks/`

**Invocation Points:**
- `prepare_execution()`: Before transaction execution
- `finalize_execution()`: After execution completes

**Implementations:**
- `DefaultHook`: Standard L1 (nonce, value transfer, self-destructs)
- `L2Hook`: Fee token handling, L2 fees
- `BackupHook`: Transaction state backup/restore

**Performance Notes:**
- Hooks stored as `Vec<Rc<RefCell<dyn Hook>>>`
- Clone per transaction (clones Rc pointers, cheap)
- RefCell borrowing has minimal overhead

---

## Block Execution Pipeline

### Main Entry Points

**Location:** `crates/blockchain/blockchain.rs`

| Function | Line | Description |
|----------|------|-------------|
| `add_block()` | 1449 | Single-threaded execution |
| `add_block_pipeline()` | 1486 | Parallel merkleization |
| `add_blocks_in_batch()` | 1690 | Batch execution (sync benchmark) |

### Pipeline Execution Flow

The pipelined execution (`execute_block_pipeline`) spawns two threads within a scope:

```
┌─────────────────────────────────────────────────────────────────────┐
│                        std::thread::scope                           │
│                                                                     │
│  ┌─────────────────────┐         channel         ┌────────────────┐ │
│  │   Execution Thread  │ ──Vec<AccountUpdate>──► │ Merkleization  │ │
│  │                     │                         │    Thread      │ │
│  │  For each tx:       │                         │                │ │
│  │  1. execute_tx()    │                         │ Spawns 16      │ │
│  │  2. Every 5 txs or  │                         │ shard workers  │ │
│  │     when queue=0:   │                         │ (if root is    │ │
│  │     send updates    │                         │  branch with   │ │
│  │                     │                         │  ≥3 children)  │ │
│  └─────────────────────┘                         └────────────────┘ │
└─────────────────────────────────────────────────────────────────────┘
```

**Key coordination mechanism** (`backends/levm/mod.rs:137-142`):
```rust
if queue_length.load(Ordering::Relaxed) == 0 && tx_since_last_flush > 5 {
    LEVM::send_state_transitions_tx(&merkleizer, db, queue_length)?;
    tx_since_last_flush = 0;
}
```

The `queue_length` AtomicUsize provides backpressure - execution only sends updates when the merkleization queue is empty and at least 5 transactions have been processed.

### Parallel Merkleization Detail

**Location:** `crates/blockchain/blockchain.rs:432-603`

**Sharding Strategy:**
1. Check if state root is a Branch node with ≥3 valid children
2. If not: fall back to sequential processing (`handle_merkleization_sequential`)
3. If yes: spawn 16 worker threads, one per high nibble (bits 4-7 of hashed address)

**Worker distribution** (`blockchain.rs:528-534`):
```rust
hashed_updates.sort_by_key(|(h, _)| h.0[0]);
for sharded_update in hashed_updates.chunk_by(|l, r| l.0.0[0] & 0xf0 == r.0.0[0] & 0xf0) {
    let shard_message = sharded_update.to_vec();
    workers_tx[(shard_message[0].0.0[0] >> 4) as usize]
        .send(shard_message)?;
}
```

Each worker:
1. Receives updates for accounts in its shard (addresses where `keccak(address)[0] >> 4 == shard_id`)
2. Opens its own state trie view
3. Processes account and storage updates independently
4. Returns partial results (state updates, storage updates, code updates)

**Result merging** (`blockchain.rs:538-554`):
- Main thread collects results from all 16 workers
- Merges the branch node choices from each shard's root
- Handles edge case where branch collapses to extension/leaf

### Transaction Execution Model

**SEQUENTIAL within block**, parallelizable across blocks

**Performance Notes:**
- Each transaction depends on previous state (sequential)
- Transaction selection: O(log n) insertion into tip-sorted heads
- `Vec::remove(0)` on heads is O(n) - should use VecDeque (TODO line 708)

### State Management During Execution

**Storage Slot Access** (`store.rs:1980-2005`):
1. Check if FlatKeyValue (FKV) available for account (background-computed optimization)
2. If FKV computed: direct lookup avoids trie traversal
3. Otherwise: open storage trie → hash slot → traverse trie

**Each storage access involves:**
- Account lookup: hash(address) → state trie → decode AccountState
- Storage lookup: hash(slot) → storage trie → decode U256
- Multiple trie traversals (2 for account + storage) unless FKV available

---

## Trie and Storage Layer

### Trie Layer Cache Architecture

**Location:** `crates/storage/layering.rs`

The trie layer cache implements a **diff-based caching system** with **Read-Copy-Update (RCU)** semantics for lock-free reads during block execution.

```rust
pub struct TrieLayerCache {
    last_id: usize,                          // Monotonic layer ID
    commit_threshold: usize,                 // When to flush to DB (128 on-disk, 10000 in-memory)
    layers: FxHashMap<H256, Arc<TrieLayer>>, // state_root -> layer mapping
    bloom: Option<qfilter::Filter>,          // Global bloom for fast negative lookups
}

struct TrieLayer {
    nodes: FxHashMap<Vec<u8>, Vec<u8>>,      // path -> encoded node
    parent: H256,                             // Parent state root (forms chain)
    id: usize,                                // For ordering/pruning
}
```

**Layer Lookup** (`layering.rs:63-91`):
1. Check bloom filter - if key definitely not present, return None immediately
2. Walk the layer chain from current state_root backward via parent links
3. Return first match found, or None if chain exhausted

**Layer Creation** (`put_batch`):
- Creates new layer with parent pointing to previous state root
- Adds all keys to global bloom filter
- Inserts into layers map keyed by new state root

### TrieWrapper: Layered DB Access

**Location:** `crates/storage/layering.rs:197-235`

```rust
pub struct TrieWrapper {
    pub state_root: H256,
    pub inner: Arc<TrieLayerCache>,  // Shared, immutable reference to layer cache
    pub db: Box<dyn TrieDB>,         // Backend for cache misses
    pub prefix: Option<H256>,        // Account hash prefix for storage tries
}
```

**Get operation** (`layering.rs:223-229`):
```rust
fn get(&self, key: Nibbles) -> Result<Option<Vec<u8>>, TrieError> {
    let key = apply_prefix(self.prefix, key);
    if let Some(value) = self.inner.get(self.state_root, key.as_ref()) {
        return Ok(Some(value));  // Found in layer cache
    }
    self.db.get(key)  // Fall back to database
}
```

### Background Workers

**Location:** `crates/storage/store.rs:1369-1429`

Two background threads handle persistence:

#### 1. Trie Update Worker (`store.rs:1389-1428`)

Receives `TrieUpdate` messages and performs three phases:

**Phase 1 - Update in-memory layers (fast, blocks execution briefly):**
```rust
// Read-Copy-Update pattern
let trie = trie_cache.lock()?.clone();        // Clone Arc (cheap)
let mut trie_mut = (*trie).clone();           // Deep copy
trie_mut.put_batch(parent, child, new_layer); // Mutate copy
*trie_cache.lock()? = Arc::new(trie_mut);     // Swap pointer
result_sender.send(Ok(()))?;                  // Unblock execution
```

**Phase 2 - Persist bottom layer to disk (slow, runs in background):**
```rust
let nodes = trie_mut.commit(root);  // Remove bottom layer, get nodes
for (key, value) in nodes {
    // Route to correct table based on key length
    let table = if is_leaf { fkv_table } else { trie_nodes_table };
    write_tx.put(table, &key, &value)?;
}
write_tx.commit()?;
```

**Phase 3 - Remove committed layer from cache:**
```rust
*trie_cache.lock()? = Arc::new(trie_mut);  // trie_mut has layer removed
```

#### 2. FlatKeyValue Generator (`store.rs:2674-2800`)

Background thread that iterates through the entire trie, pre-computing leaf values for fast direct access:

- Iterates account trie leaves → writes to ACCOUNT_FLATKEYVALUE
- For each account, iterates storage trie leaves → writes to STORAGE_FLATKEYVALUE
- Checkpoints progress every 10,000 entries
- Paused during trie layer commits (to avoid reading inconsistent state)

### Database Backend Interaction

**Location:** `crates/storage/trie.rs`

#### BackendTrieDB (Per-call transactions)

```rust
impl TrieDB for BackendTrieDB {
    fn get(&self, key: Nibbles) -> Result<Option<Vec<u8>>, TrieError> {
        let tx = self.db.begin_read()?;  // NEW transaction per get()
        tx.get(table, prefixed_key.as_ref())
    }
}
```

**Performance issue:** Creates new read transaction for every single trie node access.

#### BackendTrieDBLocked (Persistent snapshots)

```rust
pub struct BackendTrieDBLocked {
    account_trie_tx: Box<dyn StorageLockedView>,   // Persistent snapshot
    storage_trie_tx: Box<dyn StorageLockedView>,
    account_fkv_tx: Box<dyn StorageLockedView>,
    storage_fkv_tx: Box<dyn StorageLockedView>,
}
```

Used for batch reads - holds persistent snapshots of all 4 tables, avoiding per-call transaction overhead.

### Database Tables

| Table | Contents | Key Format |
|-------|----------|------------|
| ACCOUNT_TRIE_NODES | Account trie internal nodes | Nibbles path (0-64 nibbles) |
| STORAGE_TRIE_NODES | Storage trie internal nodes | account_hash + separator(17) + path |
| ACCOUNT_FLATKEYVALUE | Account trie leaves (pre-computed) | Full 64-nibble path |
| STORAGE_FLATKEYVALUE | Storage trie leaves (pre-computed) | Full 131-nibble path |

**Key length determines table routing** (`trie.rs:77-84`):
- 65 nibbles = account leaf (ACCOUNT_FLATKEYVALUE)
- 131 nibbles = storage leaf (STORAGE_FLATKEYVALUE)
- Other = internal node (ACCOUNT/STORAGE_TRIE_NODES)

### Nibbles Implementation

**Location:** `crates/common/trie/nibbles.rs:23-28`

```rust
pub struct Nibbles {
    data: Vec<u8>,              // Current path
    already_consumed: Vec<u8>,  // Consumed during traversal
}
```

**Performance Issues:**
- TODO at line 11 suggests replacing with stack-allocated array
- Vec allocations during path operations:
  ```rust
  self.data = self.data[prefix.len()..].to_vec();  // Allocates
  ret.already_consumed = [&self.already_consumed, &self.data[0..offset]].concat();
  ```

### Lock Contention Analysis

| Lock | Location | Held Duration | Frequency | Impact |
|------|----------|---------------|-----------|--------|
| `trie_cache.lock()` | store.rs:2328 | Brief (Arc clone) | Every trie open | HIGH |
| `trie_cache.lock()` | store.rs:2603 | Longer (RCU swap) | Per block | LOW |
| `code_cache.lock()` | store.rs:92-114 | LRU lookup | Per code read | MEDIUM |
| `last_computed_fkv.lock()` | store.rs:2548 | Brief (Vec clone) | Every trie open | MEDIUM |

**The trie_cache lock is the primary contention point** because:
1. Every `open_state_trie()` and `open_storage_trie()` acquires it
2. Multiple storage accesses per transaction
3. Could be RwLock since reads vastly outnumber writes

---

## Key Performance Bottlenecks

### High Impact

| Issue | Location | Impact |
|-------|----------|--------|
| Nibbles Vec allocations | nibbles.rs:106, 149 | Allocation per path operation |
| Trie cache lock contention | store.rs:2328-2330 | Every state/storage access |
| Per-node DB transactions | trie.rs:96-101 | New transaction per get() |
| Sequential tx execution | payload.rs:515-596 | Cannot parallelize within block |
| OnceLock reset on clone | node.rs:209, 221 | Lose memoized hashes |

### Medium Impact

| Issue | Location | Impact |
|-------|----------|--------|
| Repeated hash computations | rlp.rs, error paths | Extra keccak calls |
| Block cloning in payload loop | payload.rs:373, 375 | Full block copy per retry |
| Code not cached during execution | store.rs:163-169 | Extra DB lookups |
| TransactionQueue Vec::remove(0) | payload.rs:795-819 | O(n) per tx removed |
| RCU deep copy | store.rs:2600 | Full TrieLayerCache clone per block |

### Low Impact

| Issue | Location | Impact |
|-------|----------|--------|
| Pending removal Vec allocs | trie.rs:243 | Per deleted key |
| RefCell borrow checking | memory.rs | Runtime overhead (minimal) |
| Gas i64 conversion | call_frame.rs:379 | Already optimized |

---

## Hot Path Summary

**Most executed code paths during block execution:**

1. **Opcode dispatch** (`vm.rs:554-629`) - every instruction
2. **Stack push/pop** (`call_frame.rs:35-100`) - most instructions
3. **Gas metering** (`call_frame.rs:379`) - every instruction
4. **Memory access** (`memory.rs:147-196`) - MLOAD/MSTORE heavy workloads
5. **Storage access** (`gen_db.rs:471-526`) - SLOAD/SSTORE
6. **Trie layer lookup** (`layering.rs:63-91`) - every state access not in VM cache
7. **Hash computation** (`node.rs:167-179`) - merkleization

**Optimization priority should focus on these paths first.**

---

## Appendix: Key Data Flow

### Block Execution to Storage

```
Transaction Execution
        │
        ▼
   AccountUpdate (address, info, storage changes, code)
        │
        ▼
   Channel to Merkleization Thread
        │
        ▼
   Shard by keccak(address)[0] >> 4
        │
        ├──► Worker 0: addresses 0x0*
        ├──► Worker 1: addresses 0x1*
        │    ...
        └──► Worker 15: addresses 0xf*
        │
        ▼
   Merge partial trie updates
        │
        ▼
   TrieUpdate { parent_root, child_root, nodes }
        │
        ▼
   Background Worker
        │
        ├──► Phase 1: Update TrieLayerCache (RCU)
        │              └──► Unblock execution
        │
        ├──► Phase 2: Commit bottom layer to DB
        │              └──► Pause FKV generator
        │
        └──► Phase 3: Remove committed layer from cache
```
