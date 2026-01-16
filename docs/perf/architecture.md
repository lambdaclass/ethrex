# Performance Architecture Notes

This document contains architecture and code notes relevant to the performance improvement ideas in `perf_ideas_tracking.md`. Use this information to estimate potential gains and prioritize optimizations.

---

## Table of Contents

1. [LEVM Architecture](#levm-architecture)
2. [Block Execution Pipeline](#block-execution-pipeline)
3. [Trie Implementation](#trie-implementation)
4. [Key Performance Bottlenecks](#key-performance-bottlenecks)
5. [Optimization Mapping](#optimization-mapping)

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

### Execution Flow

```
add_block[_pipeline]()
    ↓
execute_block[_pipeline]() [lines 263-391]
    ↓
StoreVmDatabase::new() [vm.rs:27-44]
    ↓
LEVM::execute_block[_pipeline]() [backends/levm/mod.rs:51-194]
    ├─ prepare_block()
    ├─ For each tx: execute_tx_in_block()
    ├─ process_withdrawals()
    └─ extract_requests() [Prague fork]
    ↓
apply_account_updates_batch() [store.rs:1554]
    ├─ apply_account_updates_from_trie_batch()
    ├─ Update tries with state changes
    └─ collect_changes_since_last_hash()
    ↓
store_block() [store.rs:1218]
```

### Transaction Execution Model

**SEQUENTIAL within block**, parallelizable across blocks

**Transaction loop** (`payload.rs:515-596`):
```rust
loop {
    // Get highest-tip tx from plain or blob queues
    let (head_tx, is_blob) = match (plain_txs.peek(), blob_txs.peek()) { ... };

    // Execute single transaction
    let receipt = match self.apply_transaction(&head_tx, context) {
        Ok(receipt) => { txs.shift()?; receipt }
        Err(_) => { txs.pop(); continue; }
    };

    context.payload.body.transactions.push(head_tx.into());
    context.receipts.push(receipt);
}
```

**Performance Notes:**
- Each transaction depends on previous state (sequential)
- Transaction selection: O(log n) insertion into tip-sorted heads
- `Vec::remove(0)` on heads is O(n) - should use VecDeque (TODO line 708)

### State Management During Execution

**Account State Access** (`vm.rs:74-78`):
```rust
fn get_account_state(&self, address: Address) -> Result<Option<AccountState>, EvmError> {
    self.store.get_account_state_by_root(self.state_root, address)
}
```

**Storage Slot Access** (`store.rs:1980-2005`):
1. Check if FlatKeyValue (FKV) available for account
2. If computed: use FKV (fast path, avoids trie traversal)
3. Otherwise: open storage trie → hash slot → lookup

**Each storage access involves:**
- Account lookup: hash(address) → state trie → decode AccountState
- Storage lookup: hash(slot) → storage trie → decode U256
- Multiple trie traversals (2 for account + storage)

### Merkleization

**Location:** `crates/blockchain/blockchain.rs:432-593`

**Parallel Implementation:**
- 16 worker threads spawned per block
- Accounts sharded by first nibble of hashed address (16 shards)
- Each worker processes accounts in its shard
- Lock-free via channels (no global state lock)

**Merkleization Queue** (`blockchain.rs:318`):
- `AtomicUsize queue_length` tracks pending work
- Flush at lines 137-141 if queue empty and 5+ txs executed
- Prevents unbounded queue growth

### Lock Contention Points

| Lock | Location | Frequency | Impact |
|------|----------|-----------|--------|
| `trie_cache.lock()` | store.rs:2328-2330 | Every trie open | HIGH |
| `code_cache.lock()` | store.rs:92-114 | Per code read | MEDIUM |
| `mempool.inner` RwLock | mempool.rs:80 | Per tx add | LOW (write) |

**Trie Cache Lock Pattern:**
```rust
.trie_cache
.lock()
.map_err(|_| StoreError::LockError)?
.clone()  // Clones Arc<TrieLayerCache>
```
- Held briefly for Arc clone
- Could use RwLock (read-heavy workload)

### Payload Building

**Location:** `crates/blockchain/payload.rs:396-434`

**Steps:**
1. Create payload skeleton (gas limit, fees)
2. Apply system operations (beacon root, block hash history)
3. Process withdrawals
4. Fill transactions (main loop)
5. Extract requests (Prague+)
6. Compute roots (transaction, receipt, withdrawal)

**Performance Issues:**
- `payload.clone()` in `build_payload_loop` (lines 373, 375) - clones entire Block
- Block size calculation does RLP encoding per tx (line 553)

---

## Trie Implementation

### Core Structure

**Location:** `crates/common/trie/`

**Trie Struct** (`trie.rs:54-59`):
```rust
pub struct Trie {
    db: Box<dyn TrieDB>,
    root: NodeRef,
    pending_removal: FxHashSet<Nibbles>,
    dirty: FxHashSet<Nibbles>,
}
```

**Node Types** (`node.rs`):
- `Branch(Box<BranchNode>)`: 16 choices + optional value
- `Extension(ExtensionNode)`: Compresses common prefixes
- `Leaf(LeafNode)`: Terminal nodes with values

**NodeRef Enum** (`node.rs:39-49`):
```rust
pub enum NodeRef {
    Node(Arc<Node>, OnceLock<NodeHash>),  // Embedded with memoized hash
    Hash(NodeHash),                        // Reference to DB node
}
```
- `OnceLock` for single-assignment hash caching
- `Arc` enables shared, immutable node references

### Nibbles

**Location:** `crates/common/trie/nibbles.rs:23-28`

```rust
pub struct Nibbles {
    data: Vec<u8>,
    already_consumed: Vec<u8>,
}
```

**Performance Issues:**
- TODO at line 11 suggests replacing with stack-allocated array
- Vec allocations during path operations (line 106, 149):
  ```rust
  self.data = self.data[prefix.len()..].to_vec();  // Allocates
  ret.already_consumed = [&self.already_consumed, &self.data[0..offset]].concat();
  ```

### Caching Layers

**TrieLayerCache** (`crates/storage/layering.rs:14-31`):
```rust
pub struct TrieLayerCache {
    last_id: usize,
    commit_threshold: usize,          // Default: 128 on-disk, 10000 in-memory
    layers: FxHashMap<H256, Arc<TrieLayer>>,
    bloom: Option<qfilter::Filter>,   // Fast negative lookups
}
```

**Caching Strategy:**
- Diff-based: each state root creates new layer
- Bloom filter: returns None quickly if key not in any layer
- Parent chain: walks back through state history
- Commit threshold: 128 layers on-disk, 10000 in-memory

**Code Cache** (`store.rs:73-116`):
- LRU cache, 64MB max
- Keyed by code hash
- Only populated post-execution

### Hash Computation (Merkleization)

**Location:** `crates/common/trie/node.rs:368-400`

**Two-Pass Approach:**
1. `memoize_hashes()`: Post-order traversal, cache hashes bottom-up
2. `compute_hash()`: Encode and hash each node

**NodeRef Hash Caching** (line 167-179):
```rust
pub fn compute_hash_ref(&self) -> &NodeHash {
    match self {
        NodeRef::Node(node, hash) =>
            hash.get_or_init(|| node.compute_hash()),
        NodeRef::Hash(hash) => hash,
    }
}
```

**Performance Notes:**
- `NodeHash::Inline` avoids hashing nodes < 32 bytes
- OnceLock reset on clone loses memoized hashes
- `commit()` does full tree traversal + DB write

### Storage Backend

**Location:** `crates/storage/trie.rs`

**BackendTrieDB:**
- `get()`: Begins read transaction per call (line 96-101)
- `put_batch()`: Single write transaction for N items (line 103-115)

**Tables:**
- ACCOUNT_TRIE_NODES, STORAGE_TRIE_NODES
- ACCOUNT_FLATKEYVALUE, STORAGE_FLATKEYVALUE (optimized leaf access)

---

## Key Performance Bottlenecks

### High Impact

| Issue | Location | Impact |
|-------|----------|--------|
| Nibbles Vec allocations | nibbles.rs:106, 149 | Allocation per path operation |
| Trie cache lock contention | store.rs:2328-2330 | Every state/storage access |
| Sequential tx execution | payload.rs:515-596 | Cannot parallelize within block |
| OnceLock reset on clone | node.rs:209, 221 | Lose memoized hashes |
| TransactionQueue Vec::remove(0) | payload.rs:795-819 | O(n) per tx removed |

### Medium Impact

| Issue | Location | Impact |
|-------|----------|--------|
| Repeated hash computations | rlp.rs, error paths | Extra keccak calls |
| Per-node DB transactions | trie.rs:115 | Many small DB ops |
| Block cloning in payload loop | payload.rs:373, 375 | Full block copy per retry |
| Code not cached during execution | store.rs:163-169 | Extra DB lookups |
| Lock held during Arc clone | store.rs:2328-2330 | Brief but frequent |

### Low Impact

| Issue | Location | Impact |
|-------|----------|--------|
| Pending removal Vec allocs | trie.rs:243 | Per deleted key |
| RefCell borrow checking | memory.rs | Runtime overhead (minimal) |
| Gas i64 conversion | call_frame.rs:379 | Already optimized |

---

## Optimization Mapping

This section maps each performance idea to relevant architecture considerations.

### Execution Optimizations

| Idea | Architecture Notes | Estimated Complexity |
|------|-------------------|---------------------|
| **Nibbles 1-byte 2-nibble** | Currently uses Vec<u8> with 1 nibble per byte. Can pack 2 nibbles per byte to halve memory and improve cache locality. | Low - data structure change |
| **Use FxHashSet for access lists** | Already using FxHashMap in some places. Access lists use std collections. | Low - swap implementation |
| **Skip memory zero-init** | Memory does `resize(new_size, 0)`. Could use uninitialized memory with careful tracking. | Low - requires unsafe |
| **Replace BTreeMap with FxHashMap** | BTreeMap used in some places for ordering. FxHashMap ~2x faster for lookups. | Low - identify locations |
| **Remove RefCell from Memory** | Memory uses `Rc<RefCell<Vec<u8>>>`. Could use raw pointer with safety invariants. | Low - risky change |
| **Inline Hot Opcodes** | Already have hybrid dispatch. Could inline more into fast path match. | Low - compiler hints |
| **Avoid Clone on Account Load** | Account cloning happens in backup system. Could use CoW or persistent data structures. | Low-Mid |
| **SSTORE double lookup** | Cache has original+current separation. Two hashmap lookups per SSTORE. | Low |
| **Hook Cloning Per Opcode** | Hooks cloned once per tx. Could clone individual Rcs. | Low |
| **keccak caching** | No keccak cache exists. Top 10k hashes are often constants. | Low - add LRU cache |
| **Buffer reuse** | Allocations throughout. Free-list pattern could help. | Low |
| **Object Pooling** | Stack already pooled. CallFrame, Memory could benefit. | Mid |
| **SIMD everywhere** | U256 operations use ruint. SIMD could accelerate batch ops. | Mid - platform specific |
| **Stackalloc for Small Buffers** | Many small Vec allocations. Could use stack for <64 bytes. | Mid |
| **Arena Allocator for Substate** | Substate has many small allocations for backups. Arena could batch. | Mid |
| **Arkworks EC Pairing** | Currently using other EC library. Arkworks may be faster. | Mid - dependency change |
| **Jumptable vs Calltable** | Already have jump table. Verifying optimal. | Mid - investigation |
| **Mempool Lock Contention** | Uses RwLock. Pruning is O(n²). DashMap could help. | Mid |
| **Precompile caching** | EC recover is expensive. Per-address LRU could help. | Mid |
| **Cross-block cache reuse** | Caches reset per block. Could persist hot entries. | Mid |
| **Hierarchical storage cache** | Separate caches for code, storage, accounts exist but not unified. | Mid |
| **Parallel proof workers** | Fixed 16 workers. Could be dynamic based on load. | Mid |
| **LEVM simplify stack/results** | Stack is already optimized. Results could be simplified. | High - major refactor |
| **Parallel Transaction Execution** | Requires dependency analysis. Complex with state conflicts. | High |
| **PGO** | Profile-guided optimization. Requires build pipeline changes. | High - tooling |
| **ruint** | Direct replacement doesn't work. Evaluate for specific ops. | High |

### I/O Optimizations

| Idea | Architecture Notes | Estimated Complexity |
|------|-------------------|---------------------|
| **Transaction pre-warming** | No pre-warming exists. Would pre-execute to populate caches. | High - architecture change |
| **Sparse trie** | Already only recompute changed paths via `dirty` set. Verify. | Low - verification |
| **Bloom filter revision** | Bloom filter exists in TrieLayerCache. May need tuning. | Low |
| **LRU cache for states/accounts** | TrieLayerCache exists. Could add account-level LRU. | Mid |
| **Increase CodeCache Size** | Currently 64MB. Could benchmark larger sizes. | Low - config change |
| **Cache trie top levels** | Top 2 levels are frequently accessed. Could pin in memory. | Mid |
| **RocksDB tuning** | Default settings. Block cache, compaction tuning possible. | Mid - experimentation |
| **Multiget everywhere** | Currently single gets. RocksDB multiget is faster for batches. | Mid |
| **Increase TrieLayerCache threshold** | Currently 128 on-disk. Higher = more memory, fewer commits. | Low - config change |
| **State Prefetching** | No prefetching. Could predict next accounts/slots. | High |
| **State pruning** | No pruning implemented. Would reduce DB size. | High |
| **Implementing Ethrex-DB** | Custom DB for Ethereum workload. Major undertaking. | High |
| **Parallel merkelization of storages** | Currently parallel by account shard. Could parallelize within account. | Mid |
| **Deeper pipeline** | Current: execute → merkleize. Could add more stages. | High |

---

## Hot Path Summary

**Most executed code paths during block execution:**

1. **Opcode dispatch** (`vm.rs:554-629`) - every instruction
2. **Stack push/pop** (`call_frame.rs:35-100`) - most instructions
3. **Gas metering** (`call_frame.rs:379`) - every instruction
4. **Memory access** (`memory.rs:147-196`) - MLOAD/MSTORE heavy workloads
5. **Storage access** (`gen_db.rs:471-526`) - SLOAD/SSTORE
6. **Trie lookup** (`trie.rs:101-127`) - every state access
7. **Hash computation** (`node.rs:167-179`) - merkleization

**Optimization priority should focus on these paths first.**
