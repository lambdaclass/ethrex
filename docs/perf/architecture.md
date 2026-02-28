# Performance Architecture Notes

This document contains architecture and code notes relevant to the performance improvement ideas in [ideas.md](ideas.md). Use this information to understand the codebase when implementing optimizations.

---

## Table of Contents

1. [LEVM Architecture](#levm-architecture)
2. [Block Execution Pipeline](#block-execution-pipeline)
3. [Trie and Storage Layer](#trie-and-storage-layer)
4. [Trie Library Deep Dive](#trie-library-deep-dive)
5. [Key Performance Bottlenecks](#key-performance-bottlenecks)
6. [Hot Path Summary](#hot-path-summary)

---

## LEVM Architecture

### Main Execution Loop

**Location:** `crates/vm/levm/src/vm.rs:500-636`

The core execution loop follows this pattern:

```rust
loop {
    let opcode = self.current_call_frame.next_opcode();
    self.advance_pc(1)?;

    // Fast path for common opcodes (direct match)
    let op_result = match opcode {
        0x60 => self.op_push::<1>(),
        // ... 71 fast-path opcodes
        _ => self.opcode_table[opcode as usize].call(self)  // Lookup table
    };

    let result = match op_result {
        Ok(OpcodeResult::Continue) => continue,
        Ok(OpcodeResult::Halt) => self.handle_opcode_result()?,
        Err(error) => self.handle_opcode_error(error)?,
    };

    if self.is_initial_call_frame() {
        self.handle_state_backup(&result)?;
        return Ok(result);
    }
    self.handle_return(&result)?;
}
```

### Opcode Dispatch

**Location:** `crates/vm/levm/src/vm.rs:536-613`, `crates/vm/levm/src/opcodes.rs`

**Current Implementation: Hybrid Fast-Path + Lookup Table**

Hot opcodes have a direct match before the lookup table (71 total):
- PUSH1-PUSH32 (32 specialized handlers with const generics)
- DUP1-DUP16 (16 specialized handlers)
- SWAP1-SWAP16 (16 specialized handlers)
- ADD, CODECOPY, MLOAD, JUMP, JUMPI, JUMPDEST, TSTORE (since Cancun)

Cold opcodes use a 256-element function pointer table: `[OpCodeFn; 256]`

**Performance Notes:**
- Hot path bypasses indirection for ~71 opcodes (stack/memory-heavy operations)
- Function pointers have branch prediction overhead
- Fork-specific tables built at VM construction (one-time cost)
- `#[inline(always)]` on all fast-path handlers ensures no call overhead

### Stack Implementation

**Location:** `crates/vm/levm/src/call_frame.rs:17-220`

```rust
pub struct Stack {
    pub values: Box<[U256; 1024]>,  // Fixed 1024-element array (32 KB)
    pub offset: usize,              // Grows downward from 1024
}
```

**Key Operations (all use unsafe pointer ops):**
- `pop<N>()`: Generic, pops N elements with `get_unchecked()` + `first_chunk::<N>()`
- `pop1()`: Specialized single-pop with bounds check
- `push()`: Uses `ptr::copy_nonoverlapping()` for U256 (4 u64s)
- `push_zero()`: Writes zero array directly via `cast()`
- `dup<N>()`: Uses `ptr::copy_nonoverlapping()`, const generic depth
- `swap<N>()`: Direct array `.swap()` with compile-time bounds assertion

**Stack Pool** (`vm.rs:390`):
```rust
pub stack_pool: Vec<Stack>,  // Reused stacks to avoid allocation
```
- When creating a child call frame, a stack is taken from the pool if available
- Returned to pool after call frame completes
- Avoids 32KB allocation per nested call

**Performance Notes:**
- Fixed allocation avoids dynamic resizing
- Grows downward: underflow check doubles as overflow detection
- Unsafe pointer operations avoid bounds checking in hot path
- `#[inline]` and `#[inline(always)]` on all methods

### Memory Implementation

**Location:** `crates/vm/levm/src/memory.rs`

```rust
pub struct Memory {
    pub buffer: Rc<RefCell<Vec<u8>>>,  // Shared across call frames
    pub len: usize,                     // Logical size (high water mark)
    pub current_base: usize,            // Offset for this call frame
}
```

**Key Design:**
- `Rc<RefCell<>>` allows child call frames to share parent memory
- Lazy expansion, padded to 32-byte multiples
- Zero-initialization via `Vec::resize(new_size, 0)`

**Memory Operations:**
- `load()`: Expands if needed, copies 32 bytes to U256
- `store()`: Expands if needed, writes 32 bytes from U256
- `copy_within()`: Uses `Vec::copy_within()` for MCOPY
- `get_slice()`: Returns `&[u8]` view for RETURN/REVERT data

**Expansion Flow:**
```rust
if offset + size > self.len {
    let new_size = offset.checked_add(size)?.checked_next_multiple_of(32)?;
    self.buffer.borrow_mut().resize(new_size, 0);  // Zero-fill
    self.len = new_size;
}
```

**Performance Notes:**
- RefCell has runtime borrow checking overhead (minimal)
- Memory expansion gas: `floor(words²/512) + 3*words`
- Single growing Vec per transaction
- Expansion cost quadratic - discourages huge memory

### State Access from VM

**Location:** `crates/vm/levm/src/db/gen_db.rs`

**GeneralizedDatabase Structure:**
```rust
pub struct GeneralizedDatabase {
    pub store: Arc<dyn Database>,                           // Backend storage
    pub current_accounts_state: FxHashMap<Address, LevmAccount>,  // Hot cache
    pub initial_accounts_state: FxHashMap<Address, LevmAccount>,  // Tx-start snapshot
    pub codes: FxHashMap<H256, Code>,                        // Bytecode cache
    pub tx_backup: Option<CallFrameBackup>,                  // For undo_last_transaction()
}
```

**LevmAccount Structure:**
```rust
pub struct LevmAccount {
    pub info: AccountInfo,                  // balance, nonce, code_hash
    pub storage: HashMap<H256, U256>,       // Cached storage slots
    pub status: AccountStatus,              // Unmodified/Modified/Created/Deleted
    pub has_storage: bool,                  // Optimization flag
}
```

**Storage Read Path (`get_storage_value()` line 486):**
1. Check `current_accounts_state[address].storage[key]`
2. Fallback to `initial_accounts_state[address].storage[key]`
3. Load from database via `get_value_from_database()` → inserts into cache

**Storage Write Path (`write_account_storage()` line 405):**
1. Backup original value in `CallFrameBackup` (first-write-wins)
2. Update `current_accounts_state[address].storage[key]`
3. Set `account.status = AccountStatus::Modified`

**Account Access:**
- `get_account()`: Same 3-tier lookup as storage
- Creates default empty account if not found (lazy creation)

**Performance Notes:**
- FxHashMap (rustc_hash) is faster than std HashMap for small keys
- Every modification backed up in `CallFrameBackup` for reversion
- `initial_accounts_state` enables cheap reversion to tx-start state
- Double-lookup (current → initial) on every read

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

### Substate: Hierarchical Checkpoint System

**Location:** `crates/vm/levm/src/vm.rs:45-331`

The Substate tracks all changes during execution that may need reverting on failure:

```rust
pub struct Substate {
    parent: Option<Box<Self>>,              // Linked list of checkpoints
    selfdestruct_set: HashSet<Address>,     // Scheduled destructions
    accessed_addresses: HashSet<Address>,   // EIP-2929 warm addresses
    accessed_storage_slots: BTreeMap<Address, BTreeSet<H256>>,  // EIP-2929 warm slots
    created_accounts: HashSet<Address>,     // Newly created accounts
    pub refunded_gas: u64,                  // Gas refund accumulator
    transient_storage: TransientStorage,    // EIP-1153 TSTORE/TLOAD
    logs: Vec<Log>,                         // Emitted events
}
```

**Checkpoint Operations:**
- `push_backup()`: Creates checkpoint by moving current state to parent link
- `commit_backup()`: Merges child changes into parent (call succeeded)
- `revert_backup()`: Discards current and restores parent (call failed)

**Lookup Pattern** (walks parent chain):
```rust
pub fn is_address_accessed(&self, address: &Address) -> bool {
    self.accessed_addresses.contains(address)
        || self.parent.as_ref()
            .map(|parent| parent.is_address_accessed(address))
            .unwrap_or_default()
}
```

**Performance Notes:**
- Append-only design allows simple merge/revert
- Parent chain walk is O(call depth), typically shallow
- HashSet/BTreeSet insertions are fast but allocate

### CallFrameBackup: Account State Reversion

**Location:** `crates/vm/levm/src/call_frame.rs:273-308`

Separate from Substate, each CallFrame tracks the original state of modified accounts:

```rust
pub struct CallFrameBackup {
    pub original_accounts_info: HashMap<Address, LevmAccount>,
    pub original_account_storage_slots: HashMap<Address, HashMap<H256, U256>>,
}
```

**Backup Flow:**
1. Before modifying account/storage, check if already backed up
2. If not, store original value (first-write-wins semantics)
3. On revert: restore from backup via `restore_cache_state()`
4. On success: merge child backup into parent via `merge_call_frame_backup_with_parent()`

**Key Functions:**
- `backup_account_info()`: Saves original account state before modification
- `restore_cache_state()` (`utils.rs:72-97`): Reverts cache to backup state
- `merge_call_frame_backup_with_parent()` (`call_frame.rs:432-462`): Propagates backup up call stack

**Performance Notes:**
- Separate from Substate because it backs up GeneralizedDatabase cache, not Substate fields
- HashMap allocations on first write to each account/slot
- Clone on revert for nested calls

### Transaction-Level Backup (BackupHook)

**Location:** `crates/vm/levm/src/hooks/backup_hook.rs`

For stateless execution (estimateGas, eth_call), the BackupHook preserves pre-execution state:

```rust
pub struct BackupHook {
    pub pre_execution_backup: CallFrameBackup,  // State before tx
}
```

**Workflow:**
1. `prepare_execution()`: Store current `call_frame_backup`
2. `finalize_execution()`: Merge pre-execution + execution backups into `db.tx_backup`
3. `undo_last_transaction()`: Restore from `tx_backup`

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
    data: Vec<u8>,              // Current path (max 65 nibbles for account, 131 for storage)
    already_consumed: Vec<u8>,  // Consumed during traversal (for path tracking)
}
```

See [Trie Library Deep Dive](#trie-library-deep-dive) below for detailed allocation analysis.

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

## Trie Library Deep Dive

This section provides detailed analysis of the core trie implementation in `crates/common/trie/`, focusing on allocation patterns, repeated computations, data structure issues, and API design problems.

### Nibbles: Allocation Problems

**Location:** `crates/common/trie/nibbles.rs`

The `Nibbles` struct uses two `Vec<u8>` fields, causing allocations on nearly every operation.

**Structure:**
```rust
pub struct Nibbles {
    data: Vec<u8>,              // Current path (max 65 nibbles)
    already_consumed: Vec<u8>,  // Path tracking during traversal
}
```

**Allocation-Heavy Operations:**

| Method | Line | Issue | Frequency |
|--------|------|-------|-----------|
| `skip_prefix()` | 106 | `self.data[prefix.len()..].to_vec()` | Every extension/leaf match |
| `offset()` | 147-151 | Two allocations: slice + concat | Every branch child traversal |
| `slice()` | 154-156 | `self.data[start..end].to_vec()` | Node restructuring |
| `next()` | 137 | `self.data.remove(0)` - O(n) shift | Every branch choice |
| `prepend()` | 170 | `self.data.insert(0, nibble)` - O(n) shift | Node restructuring |
| `concat()` | 242-246 | `[...].concat()` creates new Vec | Extension prefix handling |
| `append_new()` | 250-255 | Two clones + vec![nibble] | Commit path building |
| `current()` | 258-263 | `already_consumed.clone()` | Error reporting, path tracking |
| `from_raw()` | 73-86 | `flat_map` allocates intermediates | Every key conversion |
| `encode_compact()` | 180-208 | New Vec per encoding | Every node serialization |

**Recommended Fix:** Replace with stack-allocated array (noted in TODO at line 11):
```rust
pub struct Nibbles {
    data: [u8; 68],    // Max 65 nibbles + 3 padding
    len: u8,
    consumed: u8,      // Index into data for cursor position
}
```

### NodeRef and Hash Memoization

**Location:** `crates/common/trie/node.rs:39-49`

```rust
pub enum NodeRef {
    Node(Arc<Node>, OnceLock<NodeHash>),  // Embedded node + memoized hash
    Hash(NodeHash),                        // Reference by hash
}
```

**Hash Memoization Issues:**

1. **Lost on Clone** (`node.rs:219-223`):
   ```rust
   impl From<Arc<Node>> for NodeRef {
       fn from(value: Arc<Node>) -> Self {
           Self::Node(value, OnceLock::new())  // Hash not preserved!
       }
   }
   ```
   Every conversion to `NodeRef` loses the memoized hash.

2. **Cleared on Mutation** (`node.rs:194-198`):
   ```rust
   pub(crate) fn clear_hash(&mut self) {
       if let NodeRef::Node(_, hash) = self {
           hash.take();  // Must recompute next time
       }
   }
   ```
   Called after every insert/remove, even if hash unchanged.

3. **PartialEq Allocates** (`node.rs:225-230`):
   ```rust
   impl PartialEq for NodeRef {
       fn eq(&self, other: &Self) -> bool {
           let mut buf = Vec::new();  // Allocates on EVERY comparison
           self.compute_hash_no_alloc(&mut buf) == other.compute_hash_no_alloc(&mut buf)
       }
   }
   ```

### RLP Encoding: Two-Pass Design (Not a Problem)

**Location:** `crates/common/trie/rlp.rs`

**BranchNode::encode** calls `compute_hash_ref()` twice per child, but this is **NOT double computation**:

```rust
impl RLPEncode for BranchNode {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        // First pass: compute payload length
        let payload_len = self.choices.iter().fold(value_len, |acc, child| {
            acc + RLPEncode::length(child.compute_hash_ref())  // Computes & memoizes
        });

        encode_length(payload_len, buf);

        // Second pass: encode children
        for child in self.choices.iter() {
            match child.compute_hash_ref() {  // Returns cached value (OnceLock fast path)
                // ...
            }
        }
    }
}
```

**Why it's not a problem:**

1. `compute_hash_ref()` uses `OnceLock::get_or_init()` for memoization
2. First call: computes hash and stores in OnceLock
3. Second call: returns cached reference (just a pointer read)
4. `NodeHash::length()` doesn't compute anything - just matches on enum variant:
   ```rust
   fn length(&self) -> usize {
       match self {
           NodeHash::Hashed(_) => 33,
           NodeHash::Inline((_, len)) => *len as usize,
           // ...
       }
   }
   ```

**Why not use Encoder (single pass)?**

The `Encoder` struct buffers all fields into `temp_buf`, then copies to output. The two-pass approach:
- Pre-allocates exact buffer size (avoids reallocation)
- Writes directly to output (avoids copy)
- Second `compute_hash_ref()` is very cheap (OnceLock fast path)

This trade-off is intentional - see comment at line 34:
```rust
// Duplicated to prealloc the buffer and avoid calculating the payload length twice
```

**Actual hash computation concern - error paths:**

Error handlers call `compute_hash().finalize()` for error messages:
- `branch.rs:62-70` - BranchNode::get
- `branch.rs:99-107` - BranchNode::insert
- `extension.rs:42-51` - ExtensionNode::get

If hashes aren't already memoized when an error occurs, this triggers computation. The fix would be lazy error message construction.

### Node Structure: Memory Layout

**Location:** `crates/common/trie/node.rs`, `node/*.rs`

```rust
pub enum Node {
    Branch(Box<BranchNode>),   // Boxed to keep enum small
    Extension(ExtensionNode),
    Leaf(LeafNode),
}

pub struct BranchNode {
    pub choices: [NodeRef; 16],  // 16 * ~48 bytes = 768 bytes
    pub value: ValueRLP,         // Vec<u8>
}

pub struct ExtensionNode {
    pub prefix: Nibbles,    // Two Vecs
    pub child: NodeRef,     // ~48 bytes
}

pub struct LeafNode {
    pub partial: Nibbles,   // Two Vecs
    pub value: ValueRLP,    // Vec<u8>
}
```

**Memory Issues:**

1. **BranchNode is large** (~800 bytes):
   - 16 NodeRefs @ 48 bytes each = 768 bytes
   - Plus value Vec overhead
   - Must be boxed to keep Node enum reasonable

2. **Each NodeRef contains OnceLock<NodeHash>**:
   - OnceLock is 40 bytes on 64-bit
   - Arc<Node> is 8 bytes
   - Total ~48 bytes per child reference

3. **Nibbles duplication**:
   - Both `data` and `already_consumed` Vecs
   - `already_consumed` only used for path tracking during traversal

### Trie Operations: Allocation Patterns

**Insert Path** (`trie.rs:130-150`, node methods):

```
insert(path, value)
  └── Nibbles::from_bytes(&path)           // Allocates 2 Vecs
        └── path.skip_prefix()              // Allocates new Vec
              └── path.offset()             // Allocates 2 Vecs
                    └── path.next_choice()  // O(n) remove(0)
                          └── ... recursive
```

Each level of trie traversal allocates multiple times.

**Get Path** (`trie.rs:101-127`):

```
get(pathrlp)
  └── Nibbles::from_bytes(pathrlp)         // Allocates
        └── node.get(db, path)
              └── path.skip_prefix()        // Allocates
                    └── path.current()      // Allocates (for error handling)
```

**Commit/Hash Path** (`node.rs:132-161`):

```
commit(path, acc)
  └── path.append_new(choice)              // Allocates 2 Vecs per child
        └── path.concat(&prefix)           // Allocates new Vec
              └── Vec::new() for encoding  // Allocates buffer
```

### TrieDB Interface: Clone on Read

**Location:** `crates/common/trie/db.rs`

```rust
impl TrieDB for InMemoryTrieDB {
    fn get(&self, key: Nibbles) -> Result<Option<Vec<u8>>, TrieError> {
        Ok(self.inner.lock()?.get(key.as_ref()).cloned())  // Always clones
    }
}
```

**Issue:** Every read clones the entire node value, even for cache hits.

**Possible fix:** Return `Arc<Vec<u8>>` or use a borrow-based API.

### Compact Encoding: Allocation on Every Node

**Location:** `nibbles.rs:180-208`, `rlp.rs:61-65`

Every node serialization calls `encode_compact()`:
```rust
impl RLPEncode for ExtensionNode {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        let mut encoder = Encoder::new(buf)
            .encode_bytes(&self.prefix.encode_compact());  // Allocates Vec
        // ...
    }
}
```

`encode_compact()` allocates a new `Vec<u8>` every time. With thousands of nodes per block, this adds up.

### API Design Issues Summary

| Issue | Location | Recommendation |
|-------|----------|----------------|
| Nibbles by-value everywhere | All node methods | Use cursor/iterator pattern |
| Clone-on-read TrieDB | db.rs:102-108 | Return Arc or borrow |
| No buffer reuse for encode_compact | nibbles.rs:180-208 | Accept `&mut Vec<u8>` parameter |
| OnceLock lost on conversions | node.rs:207-223 | Preserve hash through conversions |
| Nibbles two-Vec design | nibbles.rs:23-28 | Single array with cursor index |
| O(n) operations | nibbles.rs:137,170 | Use deque or cursor |
| Hash computation in error paths | branch.rs, extension.rs | Lazy error message construction |
| NodeRef::PartialEq allocates | node.rs:225-230 | Reuse thread-local buffer |

### Trie-Specific Optimization Opportunities

| Opportunity | Description | Potential Gain |
|-------------|-------------|----------------|
| Stack-allocated Nibbles | Replace Vec with [u8; 68] + cursor | Eliminate ~10 allocs per traversal |
| Pooled encoding buffers | Thread-local Vec<u8> pool for encode | Reduce allocations in hot path |
| Hash preservation on clone | Preserve OnceLock through NodeRef conversions | Avoid redundant keccak calls |
| Copy-on-write Nibbles | Only allocate on mutation | Reduce allocs for read operations |
| Cursor-based traversal | Nibbles cursor instead of mutation | Zero allocation for reads |
| Arc<[u8]> for values | Share encoded nodes in TrieDB | Reduce clone overhead |
| Lazy error messages | Defer hash computation in error paths | Avoid work on success path |
| encode_compact buffer reuse | Pass buffer to encode_compact() | Avoid allocation per node |

---

## Key Performance Bottlenecks

### High Impact

| Issue | Location | Impact |
|-------|----------|--------|
| Nibbles Vec allocations | nibbles.rs:106, 137, 149 | ~10 allocations per trie traversal |
| Trie cache lock contention | store.rs:2328-2330 | Every state/storage access |
| Per-node DB transactions | trie.rs:96-101 | New transaction per get() |
| Sequential tx execution | payload.rs:515-596 | Cannot parallelize within block |
| OnceLock reset on clone | node.rs:207-223 | Lose memoized hashes on NodeRef conversion |
| Double HashMap lookup | gen_db.rs:486-505 | current → initial on every SLOAD |
| CallFrameBackup cloning | call_frame.rs:424 | Clone HashMap on revert |
| TrieDB clones on read | db.rs:107 | Full Vec clone per cache hit |

### Medium Impact

| Issue | Location | Impact |
|-------|----------|--------|
| Hash computation in error paths | branch.rs:62-70, extension.rs:42 | compute_hash().finalize() for error msgs |
| encode_compact allocates | nibbles.rs:180-208 | New Vec per node serialization |
| Nibbles::next() is O(n) | nibbles.rs:137 | Vec::remove(0) shifts elements |
| Nibbles::prepend() is O(n) | nibbles.rs:170 | Vec::insert(0) shifts elements |
| NodeRef::PartialEq allocates | node.rs:225-230 | Vec::new() on every comparison |
| Block cloning in payload loop | payload.rs:373, 375 | Full block copy per retry |
| Code not cached during execution | store.rs:163-169 | Extra DB lookups |
| TransactionQueue Vec::remove(0) | payload.rs:795-819 | O(n) per tx removed |
| RCU deep copy | store.rs:2600 | Full TrieLayerCache clone per block |
| Substate parent chain walk | vm.rs:263-269 | O(depth) per is_address_accessed() |
| Per-slot backup HashMap | call_frame.rs:82 | Allocation on first write |
| compact_to_hex multiple allocs | nibbles.rs:301-313 | .to_vec() called multiple times |

### Low Impact

| Issue | Location | Impact |
|-------|----------|--------|
| Pending removal FxHashSet | trie.rs:57 | Nibbles allocation per removal |
| Dirty tracking FxHashSet | trie.rs:58 | Nibbles allocation per insert |
| RefCell borrow checking | memory.rs | Runtime overhead (minimal) |
| Gas i64 conversion | call_frame.rs:379 | Already optimized |
| Hook Vec cloning | vm.rs:671-683 | Rc pointer clone (cheap) |
| BranchNode size | node/branch.rs:25-28 | 768+ bytes per branch node |

### LEVM-Specific Optimization Opportunities

| Opportunity | Description | Potential Gain |
|-------------|-------------|----------------|
| Flatten Substate | Replace linked list with flat structure + index markers | Avoid parent chain walks |
| Batch backup writes | Coalesce multiple slot writes in CallFrameBackup | Reduce HashMap operations |
| Arena allocator for Substate | Pre-allocate HashSet/BTreeSet capacity | Reduce allocations |
| Speculative execution cache | Pre-warm likely storage slots | Hide DB latency |
| Inline critical opcodes | More opcodes in fast path match | Reduce function pointer overhead |

---

## Hot Path Summary

**Most executed code paths during block execution:**

1. **Opcode dispatch** (`vm.rs:527-613`) - every instruction
   - Fast path match for 71 common opcodes
   - Function pointer table lookup for remainder

2. **Stack push/pop** (`call_frame.rs:35-192`) - most instructions
   - `push()`: unsafe ptr::copy_nonoverlapping, overflow via underflow check
   - `pop<N>()`: get_unchecked + first_chunk, const generic
   - `dup<N>()`/`swap<N>()`: unsafe, const generic

3. **Gas metering** (`call_frame.rs:376-387`) - every instruction
   - Single i64 subtraction + sign check
   - No branch on success path

4. **Memory access** (`memory.rs`) - MLOAD/MSTORE heavy workloads
   - RefCell borrow on each access
   - Expansion triggers resize + zero-fill

5. **Storage access** (`gen_db.rs:400-530`) - SLOAD/SSTORE
   - 2-tier HashMap lookup (current → initial)
   - Backup on first write per slot
   - Database fallback on cache miss

6. **Substate lookups** (`vm.rs:252-270`) - warm/cold gas calculation
   - HashSet contains + parent chain walk
   - Called on every CALL/SLOAD/SSTORE

7. **Trie layer lookup** (`layering.rs:63-91`) - every state access not in VM cache
   - Bloom filter fast-path for negatives
   - Chain walk through layer diffs

8. **Hash computation** (`node.rs:167-179`) - merkleization
   - Keccak-256 for every trie node
   - OnceLock memoization (lost on clone)

**Optimization priority should focus on these paths first.**

---

## Appendix: Key Data Flow

### LEVM Call Stack and Backup Flow

```
Transaction Start
      │
      ▼
┌─────────────────────────────────────────────────────────────┐
│  VM::execute()                                               │
│    │                                                         │
│    ├── prepare_execution() ──► Hooks run, nonce increment    │
│    │                                                         │
│    ├── clear call_frame_backup ──► Changes now permanent     │
│    │                                                         │
│    └── substate.push_backup() ──► Create checkpoint          │
│          │                                                   │
│          ▼                                                   │
│    ┌─────────────────────────────────────────────────────┐   │
│    │  run_execution() - Main opcode loop                 │   │
│    │                                                     │   │
│    │  On CALL/CREATE:                                    │   │
│    │    substate.push_backup()                           │   │
│    │    add_callframe(new_call_frame)                    │   │
│    │         │                                           │   │
│    │         ▼                                           │   │
│    │    [Child execution loop]                           │   │
│    │         │                                           │   │
│    │    On Success: ◄────────────────────────────────┐   │   │
│    │      substate.commit_backup()                   │   │   │
│    │      merge_call_frame_backup_with_parent()      │   │   │
│    │                                                 │   │   │
│    │    On Failure: ◄────────────────────────────────┤   │   │
│    │      substate.revert_backup()                   │   │   │
│    │      restore_cache_state() ──► Reverts db cache │   │   │
│    │                                                 │   │   │
│    └─────────────────────────────────────────────────┼───┘   │
│                                                      │       │
│    finalize_execution() ──► Hooks finalize, extract logs     │
│                                                              │
└──────────────────────────────────────────────────────────────┘

Backup Data Structures:
┌────────────────────────────────────────────────────────────┐
│  Substate (linked list via parent pointer)                 │
│  ├── accessed_addresses: HashSet<Address>                  │
│  ├── accessed_storage_slots: BTreeMap<Address, BTreeSet>   │
│  ├── logs: Vec<Log>                                        │
│  └── parent: Option<Box<Substate>> ──► Previous checkpoint │
└────────────────────────────────────────────────────────────┘
                           │
                           │ (separate from)
                           ▼
┌────────────────────────────────────────────────────────────┐
│  CallFrameBackup (per call frame)                          │
│  ├── original_accounts_info: HashMap<Address, LevmAccount> │
│  └── original_account_storage_slots: HashMap<...>          │
│      └── Stores first-seen values for modified slots       │
└────────────────────────────────────────────────────────────┘
```

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
