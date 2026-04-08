# Binary Trie: Code Architecture Walkthrough

This document walks through how the EIP-7864 binary trie is implemented in
ethrex: where the code lives, how the pieces connect, and how data flows
during block execution.

## Crate Layout

The binary trie is a standalone crate at `crates/common/binary_trie/`. It has
no dependency on the rest of ethrex. Integration happens in `crates/storage/`
and `crates/blockchain/`, which wire the trie into block execution.

```
crates/common/binary_trie/
  lib.rs            # Public exports
  node.rs           # Node types: InternalNode, StemNode
  trie.rs           # BinaryTrie: insert, get, remove
  node_store.rs     # NodeStore: in-memory node cache with disk persistence
  merkle.rs         # State root computation (BLAKE3 hashing)
  key_mapping.rs    # Converts Ethereum state (accounts, storage, code) into 32-byte tree keys
  state.rs          # BinaryTrieState: applies account updates, manages flush/genesis
  layer_cache.rs    # Keeps recent per-block diffs in memory for fast reads
  proof.rs          # Generates and verifies inclusion/exclusion proofs
  witness.rs        # Block witness for stateless validation
  hash.rs           # Thin BLAKE3 wrapper
  db.rs             # TrieBackend trait (abstraction over RocksDB / in-memory)
  error.rs          # Error types

crates/storage/
  binary_trie_read.rs  # BinaryTrieWrapper: reads state by checking layers, then trie, then disk
  store.rs             # handle_merkleization(), flush logic, ties trie into the Store

crates/blockchain/
  blockchain.rs        # Block execution pipeline, calls into storage for merkleization
```

## Node Types

**File:** `crates/common/binary_trie/node.rs`

The tree is made of two kinds of nodes:

```rust
enum Node {
    Internal(InternalNode),  // branch: picks left (bit 0) or right (bit 1)
    Stem(StemNode),          // leaf group: holds up to 256 values under a shared prefix
}
```

### InternalNode

A branching point. At each level of the tree, one bit of the key is examined.
Bit 0 means go left, bit 1 means go right.

```rust
struct InternalNode {
    left: Option<NodeId>,
    right: Option<NodeId>,
    cached_hash: Option<[u8; 32]>,  // cleared when a descendant changes, recomputed lazily
}
```

### StemNode

Where actual values are stored. Instead of one value per leaf (like a typical
trie), a StemNode groups up to 256 values under a shared 31-byte prefix called
the "stem". The last byte of the key (the "sub-index") picks which of the 256
slots to read or write.

```rust
struct StemNode {
    stem: [u8; 31],
    values: BTreeMap<u8, [u8; 32]>,                    // only stores non-empty slots
    cached_subtree: Option<Box<[[u8; 32]; 511]>>,      // internal Merkle tree over the 256 slots
    cached_hash: Option<[u8; 32]>,                     // final hash of this node
}
```

**Why `cached_subtree`?** To compute a StemNode's hash, its 256 value slots
are arranged into a small binary Merkle tree (256 leaves, 8 levels deep, 511
nodes total). Without caching, every call to `merkelize()` would rebuild this
entire subtree from scratch: 511 hashes. With the cache, when a single value
changes only the 8 hashes along that value's path through the subtree are
recomputed. The cache is allocated on first use and cleared when the StemNode
is evicted from memory.

**Why a BTreeMap instead of a fixed-size array?** A naive `[Option<[u8;32]>; 256]`
array takes ~8.5KB per StemNode even if only 1 or 2 slots are used, and most
StemNodes in practice hold 1-5 values. The NodeStore (explained later) keeps
up to 2M nodes in its LRU cache, so fixed arrays would need ~8GB of RAM just
for that cache.
Using a sparse BTreeMap brings a typical StemNode down to ~200 bytes, fitting
the same cache in ~260MB. About a 40x memory reduction.

### Key splitting

Every 32-byte key is split into two parts:
- **Stem** (first 31 bytes): determines the path through InternalNodes to
  reach the right StemNode
- **Sub-index** (last byte): picks one of the 256 value slots inside that
  StemNode

The tree can be at most 248 levels deep (31 bytes x 8 bits per byte).

### Node identity

Nodes are not referenced by pointers. Each node gets a `NodeId` (a u64 integer),
and the `NodeStore` (described later) manages the mapping from ID to actual
node data. This makes serialization and persistence straightforward.

## Trie Operations

**File:** `crates/common/binary_trie/trie.rs`

```rust
struct BinaryTrie {
    store: NodeStore,
    root: Option<NodeId>,   // None = empty trie
}
```

### Insert

Walks the tree bit-by-bit from the root, following InternalNodes left or right
based on the stem bits.

- If it reaches a StemNode with the **same stem**: updates the value at the
  given sub-index in place.
- If it reaches a StemNode with a **different stem**: "splits" by inserting
  new InternalNodes for each bit where the two stems agree, then places the
  two StemNodes on opposite sides at the first bit where they differ.
- If it reaches an empty slot: creates a new StemNode.

### Get

Same traversal as insert. Follows InternalNodes down to a StemNode. If the
stem matches, returns the value at the sub-index. Otherwise returns None.

### Remove

Deletes a value from a StemNode's slots. If the StemNode has no remaining
values, it's freed. If an InternalNode is left with only one child (and that
child is a StemNode), the InternalNode is removed and the StemNode is promoted
upward.

InternalNode children are never promoted this way. An InternalNode examines
a specific bit position (determined by its depth in the tree). If you moved
it to a shallower depth, it would look at the wrong bit, routing lookups
to the wrong side. StemNodes don't have this problem because they don't
branch on bits; they just hold values.

## Key Mapping: Ethereum State to Tree Keys

**File:** `crates/common/binary_trie/key_mapping.rs`

In the old MPT world, accounts live in one trie and each account's storage
lives in a separate sub-trie. The binary trie puts everything in a single
tree. This module defines how Ethereum state maps to 32-byte tree keys:

```
tree_key(address, tree_index, sub_index):
    stem = BLAKE3(zero_pad_32(address) || big_endian_32(tree_index))[0..31]
    key  = stem || sub_index
```

The `tree_index` and `sub_index` select what kind of data we're addressing:

| State type                  | tree_index              | sub_index                 |
|-----------------------------|-------------------------|---------------------------|
| Account basic_data          | 0                       | 0                         |
| Account code_hash           | 0                       | 1                         |
| Header storage (slots 0-63) | 0                       | 64 + slot                 |
| Code chunks                 | (128 + chunk_id) / 256  | (128 + chunk_id) % 256    |
| Main storage (slots >= 64)  | (2^248 + slot) / 256    | (2^248 + slot) % 256      |

Notice that when `tree_index` is 0, the stem is the same for all of them.
That means an account's basic data, code hash, first 64 storage slots, and
first 128 code chunks all land in the **same StemNode**. Reading related data
for one account usually hits a single node.

### Account data packing

Account info (nonce, balance, etc.) is packed into a single 32-byte value:

```
[version: 1B] [reserved: 4B] [code_size: 3B] [nonce: 8B] [balance: 16B]
```

### Code chunking

Contract bytecode is split into 31-byte slices. Each slice gets a 1-byte
header that indicates how many bytes at the start are continuation data from
a PUSH instruction in the previous chunk. This is defined by the EIP so that
code analysis tools can tell which bytes are executable vs. data without
scanning from the beginning.

## Merkleization (State Root Computation)

**File:** `crates/common/binary_trie/merkle.rs`

The state root is a single 32-byte hash that commits to the entire trie.
It's computed bottom-up:

- **Empty slot**: `[0x00; 32]` (32 zero bytes, no hashing needed)
- **InternalNode**: `BLAKE3(left_hash || right_hash)` (concatenate the two
  children's hashes and hash the result)
- **StemNode**: `BLAKE3(stem || 0x00 || subtree_root)`
  - The "subtree_root" is itself a small Merkle tree: the 256 value slots
    form a fixed-depth-8 complete binary tree (256 leaves hashed pairwise
    up to a single root). Empty slots use zero bytes, non-empty slots are
    hashed with BLAKE3.

**Special case from the EIP**: when both inputs to a hash are all zeros (i.e.
hashing 64 zero bytes), the result is defined as 32 zero bytes. This is *not*
the actual BLAKE3 output of that input; it's a domain-specific override that
only applies during merkleization (not during key derivation).

### Incremental hashing

Recomputing the full tree hash from scratch every block would be too slow as
the trie grows. Instead, each node caches its hash. When a value changes,
only the nodes on the path from that StemNode up to the root have their
cached hash cleared. The next `merkelize()` call skips nodes with valid
caches and only rehashes the dirty path.

For StemNodes, the 511-entry subtree (the internal Merkle tree over 256 slots)
is also cached. When one slot changes, only the 8 hashes along that slot's
path through the subtree are recomputed, not all 511.

## BinaryTrieState: The State Manager

**File:** `crates/common/binary_trie/state.rs`

This is the main interface between ethrex and the binary trie. It wraps
`BinaryTrie` and adds everything needed for block-level state management:

```rust
struct BinaryTrieState {
    trie: BinaryTrie,
    current_block_diffs: Vec<([u8; 32], Option<[u8; 32]>)>,
    prev_state_root: [u8; 32],
    storage_keys: Mutex<FxHashMap<Address, FxHashSet<H256>>>,
    backend: Option<Arc<dyn TrieBackend>>,
    blocks_since_flush: u64,
    flush_threshold: u64,     // default 128
    // ...
}
```

**`current_block_diffs`** records every leaf-level change made during the
current block: a list of (tree_key, new_value) pairs, where `None` means the
leaf was deleted. After the block is merkleized, these diffs are drained via
`take_block_diffs()` and handed to the `BinaryTrieLayerCache`. The
`BinaryTrieLayerCache` needs them so it can answer "what was the value of key
X at block N?" for recent blocks. Without this, every historical read would
have to go to disk.

**`prev_state_root`** is the state root *before* the current block's updates
were applied (i.e., the parent block's root). The `BinaryTrieLayerCache`
stores diffs as "at state root R, these leaves changed relative to parent root
P". So when handing off diffs, we need to tell it both the new root and the
parent root. `prev_state_root` provides that parent reference.

**`storage_keys`** tracks which storage slots each address has ever written
to. This is a side index that the trie itself can't provide. In the MPT,
each account has its own storage sub-trie, so you can iterate it to find all
slots. The binary trie has no such grouping; an account's storage slots are
scattered throughout the single tree with no way to enumerate them by address.

This index is needed for two things:
1. **SELFDESTRUCT**: the EVM operation that destroys a contract and clears all
   its storage. We need to know which slots to delete.
2. **`has_storage` check**: the VM needs to know whether an account has any
   storage at all (it affects gas costs and account emptiness checks). We
   answer this by checking if the address has any entries in this index.

### Key operations

- **`apply_account_update(update)`**: Takes an `AccountUpdate` (the output of
  EVM execution for one account) and translates it into trie operations. For
  example, a balance change becomes: read the basic_data leaf, unpack it,
  update the balance field, repack it, insert back. Storage writes become
  inserts at the mapped tree keys. Code deploys chunk the bytecode and insert
  each chunk. Each leaf change is recorded in `current_block_diffs`.

- **`state_root()`**: Runs `merkelize()` on the trie and returns the 32-byte
  root hash.

- **`take_block_diffs(new_root)`**: Returns the leaf diffs accumulated this
  block and resets the list. Also advances `prev_state_root` to `new_root`.
  The caller (storage layer) feeds these diffs into the `BinaryTrieLayerCache`.

- **`apply_genesis(accounts)`**: Populates the trie with genesis state
  (pre-funded accounts, system contracts, initial storage and code).

- **`prepare_flush(block, hash)`**: Gathers all dirty trie nodes and storage
  key tracking entries into a list of write operations, ready for an atomic
  RocksDB batch write. Also rotates the NodeStore's memory tiers.

### Storage key tracking

In the MPT, each account has its own storage sub-trie, so you can enumerate
all storage for an address by iterating that sub-trie. The binary trie has no
such structure; storage slots are scattered across the single tree.

`storage_keys` is a side index that tracks which storage slots each address
has written. This is needed for two things:
1. **SELFDESTRUCT**: must delete all of an account's storage slots, so we need
   to know which ones exist.
2. **FKV `storage_root` sentinel**: the VM checks `storage_root != EMPTY` to
   determine if an account has storage. Since the binary trie has no
   per-account storage root, we use this index to synthesize the answer.

## NodeStore: Tiered Node Caching

**File:** `crates/common/binary_trie/node_store.rs`

### NodeStore vs FKV: two representations of the same state

The NodeStore exists solely for **merkleization** (computing state roots). It
stores trie nodes (InternalNode, StemNode) and their tree structure.

FKV (flat key-value) exists solely for **VM reads**. It stores denormalized
key-value pairs (`keccak(address) -> AccountState`, `keccak(address) ||
keccak(slot) -> value`) optimized for O(1) lookups during EVM execution.

Both are written to on every block, but they serve completely different
purposes. The VM never touches the trie, and merkleization never touches FKV.
They are two parallel representations of the same state:

- **FKV**: "what is alice's balance?" (flat lookup, no tree structure)
- **NodeStore + trie**: "what is the cryptographic commitment to the entire
  state?" (tree structure, needed for hashing into a state root)

### The problem

The trie can have millions of nodes. Keeping all of them in memory is not
feasible long-term, but going to RocksDB for every single node read during
insert/get/merkelize would be far too slow. We need a middle ground: keep
the hot working set in memory, persist everything else to disk, and make
the transition between the two efficient.

There's an additional constraint: during merkleization, nodes along the
dirty path are read, hashed, and updated with their cached hash. These reads
happen many times per block and must be fast. After a flush to disk, the
nodes we just wrote are very likely to be read again in the next block
(the upper levels of the trie are touched by almost every block). If we
evicted them from memory immediately after flushing, the next block would
have to reload them from RocksDB, which is wasteful.

### The three-tier design

```rust
struct NodeStore {
    dirty_nodes: FxHashMap<NodeId, Node>,          // tier 1: modified this checkpoint interval
    warm_nodes: FxHashMap<NodeId, Node>,            // tier 2: flushed last interval, read-only
    clean_cache: Mutex<LruCache<NodeId, Node>>,     // tier 3: older nodes, evicted when full
    freed: FxHashSet<NodeId>,                       // nodes to delete on next flush
    next_id: NodeId,                                // monotonically increasing ID allocator
    backend: Option<Arc<dyn TrieBackend>>,          // RocksDB (or None for tests)
}
```

**Dirty** (tier 1): nodes that have been created or modified since the last
flush. These are the nodes that will be written to RocksDB on the next flush.
All trie mutations (`insert`, `remove`, cached hash updates during
merkleization) land here.

**Warm** (tier 2): nodes that were flushed in the *previous* checkpoint
interval. They're read-only (already persisted) but kept in memory because
they're likely to be accessed again. The upper levels of the trie, for
example, are touched by nearly every block. Without this tier, every flush
would cause a "cold start" where the next block has to reload frequently-used
nodes from disk.

**Clean LRU** (tier 3): a bounded cache (up to 2M entries) for older nodes
loaded from RocksDB. When a node is read from disk, it's placed here so
repeated reads don't go to disk again. When the cache is full, the
least-recently-used entries are evicted.

**Reading a node** checks the tiers in order: dirty, then warm, then clean
LRU, then falls back to loading from RocksDB.

**Creating a node** allocates the next ID and places it in dirty.

**Mutating a node** requires `take(id)` (removes it from whatever tier it's
in), then `put(id, node)` to return it as dirty.

### Flush cycle

Every ~128 blocks, the NodeStore flushes to disk:

1. All dirty nodes are written to the `BINARY_TRIE_NODES` column family in
   RocksDB. Freed nodes are deleted. This happens as a single atomic
   `WriteBatch`.
2. Tiers rotate: dirty moves to warm (keeping those nodes hot for the next
   interval), previous warm is pushed into the clean LRU (where it competes
   for space with other old entries), and the clean LRU evicts its oldest
   entries if over capacity.

The 128-block interval is the same as the MPT branch's `DB_COMMIT_THRESHOLD`.
It balances write amplification (flushing too often means more disk I/O)
against memory usage (flushing too rarely means dirty keeps growing).

### Serialization

InternalNode serializes to 17 bytes: a 1-byte tag + two u64 child IDs.
StemNode uses a 32-byte presence bitmap to record which of the 256 slots
have values, followed by only the non-empty values. This keeps storage
compact.

## BinaryTrieLayerCache: Recent Block Diffs

**File:** `crates/common/binary_trie/layer_cache.rs`

This is the binary trie equivalent of `TrieLayerCache` from the MPT branch
(same role, same design). The trie itself only reflects the latest state. But
the node needs to answer queries about recent blocks (e.g., an RPC call asking
for state at block N-5). The `BinaryTrieLayerCache` solves this by keeping the
last ~128 blocks worth of leaf-level diffs in memory. The difference from the
MPT version is what the diffs contain: `TrieLayerCache` stores raw trie node
bytes keyed by nibble paths, while `BinaryTrieLayerCache` stores leaf-level
diffs (32-byte tree keys to 32-byte values).

```rust
struct BinaryTrieLayerCache {
    layers: FxHashMap<[u8; 32], Arc<BinaryTrieLayer>>,  // keyed by state root
    bloom: AtomicBloomFilter<FxBuildHasher>,             // fast "definitely not here" check
    commit_threshold: usize,                             // default 128
}

struct BinaryTrieLayer {
    leaves: FxHashMap<[u8; 32], Option<[u8; 32]>>,  // key -> Some(value) or None (deleted)
    parent: [u8; 32],                                // state root of the previous block
    id: usize,                                       // for ordering
}
```

Each layer records what changed in one block. Layers are chained by their
`parent` field (each layer points to the previous block's state root).

**Reading**: to look up a key at a given block's state root, walk the chain
from that root backward. If any layer has the key, that's the answer (even
if the value is None, meaning it was deleted). If no layer has it, fall
through to the trie or FKV on disk.

**Committing**: once the chain exceeds the threshold (128 layers), the oldest
layers are removed and their diffs are flushed to FKV on disk.

## Block Execution: End-to-End Data Flow

**Files:** `crates/blockchain/blockchain.rs`, `crates/storage/store.rs`

Block execution uses three concurrent threads. This is the same pipeline
structure as the MPT branch; only the merkleizer's internals changed.

```
[warmer]      prefetches account/storage data into cache before execution
[executor]    runs LEVM (the EVM), produces state changes
[merkleizer]  applies state changes to the binary trie, computes state root
```

### Step by step

1. **Block arrives** via p2p or RPC. The block header is validated (parent
   hash, timestamp, gas limit, etc.).

2. **Executor thread** runs each transaction through LEVM. The VM reads state
   through a chain of caching layers:
   ```
   LEVM -> CachingDatabase -> StoreVmDatabase -> Store -> FKV (RocksDB)
   ```
   The VM never interacts with the trie. All reads come from FKV (flat
   key-value tables), which are simple key-value lookups.

3. **Executor sends results** to the merkleizer as `Vec<AccountUpdate>` over
   a channel. Each `AccountUpdate` describes what changed for one account:
   new nonce, new balance, modified storage slots, deployed code, etc.

4. **Merkleizer thread** receives the updates and calls
   `Store::handle_merkleization()`, which:
   - Calls `BinaryTrieState::apply_account_update()` for each update,
     translating account changes into trie key inserts/removes
   - Calls `state_root()` to compute the new root (incrementally, only
     rehashing changed paths)
   - Calls `take_block_diffs()` to collect the leaf-level changes
   - Feeds the diffs into the `BinaryTrieLayerCache`

5. **Block is stored**: the block header, receipts, and FKV updates are
   written to RocksDB. The FKV tables are updated so that future VM reads
   see the new state.

6. **Periodic flush**: every ~128 blocks, dirty trie nodes are written to
   the `BINARY_TRIE_NODES` column family and old `BinaryTrieLayerCache`
   entries are committed to FKV.

### Read path (RPC queries at a specific block)

RPC calls like `eth_getBalance` can ask for state at a specific block. ethrex
is not an archive node: it can only give accurate answers for the last ~128
blocks (the depth of the `BinaryTrieLayerCache`). Beyond that window, there
is no historical state. FKV only stores the latest block's state (it
overwrites values on each block, no history). So for any query older than the
current block, we need the layer cache to know what changed since then.

```
BinaryTrieWrapper::get_leaf(tree_key)
  1. Check BinaryTrieLayerCache (recent ~128 blocks, in memory)
  2. Check trie nodes (committed state in memory/disk)
  3. Fall through to FKV on disk (direct RocksDB lookup)
```

The `BinaryTrieLayerCache` knows what changed in each of the last ~128 blocks.
If the key was modified in a recent block, the matching layer has the answer.
If no layer has touched the key, it means the value hasn't changed recently,
so FKV's latest value is still correct for any of those recent block heights.
That's why falling through to FKV as the last step works.

For the **latest** block specifically, FKV would always have the right answer
and steps 1-2 are unnecessary. But for historical queries, FKV's value may
have been overwritten by newer blocks, so the layer cache is needed.

`BinaryTrieWrapper` (in `crates/storage/binary_trie_read.rs`) coordinates
these layers. It also translates between the binary trie's leaf format and
the `AccountState` struct that the rest of ethrex expects: it unpacks
basic_data into nonce/balance/code_size, reads the code_hash leaf, and
produces a synthetic `storage_root` value so the VM's `is_empty_account()`
check works correctly.

## Persistence: RocksDB Tables

The binary trie adds two RocksDB column families:

| Column Family               | Key                  | Value                       |
|-----------------------------|----------------------|-----------------------------|
| `BINARY_TRIE_NODES`         | NodeId (u64 LE)      | Serialized node bytes       |
| `BINARY_TRIE_STORAGE_KEYS`  | Address (20 bytes)   | Packed list of storage keys |

`BINARY_TRIE_STORAGE_KEYS` is the on-disk backing for the `storage_keys` side
index described in the BinaryTrieState section. It's flushed alongside trie
nodes so that after a node restart, ethrex still knows which storage slots
each address owns. Without it, SELFDESTRUCT wouldn't know which slots to
delete and the `has_storage` check would give wrong answers.

Metadata is stored in `BINARY_TRIE_NODES` under reserved keys:
- `[0xFF, 'R']` -- root NodeId
- `[0xFF, 'N']` -- next_id counter
- `[0xFF, 'B']` -- last flushed block number
- `[0xFF, 'H']` -- last flushed block hash

The existing tables are **unchanged**:
- `ACCOUNT_FLATKEYVALUE` / `STORAGE_FLATKEYVALUE` (FKV, used for VM reads)
- `ACCOUNT_CODES` (contract bytecode by hash)
- Block headers, receipts, transaction indices, etc.

The binary trie replaces only the MPT merkleization tables
(`ACCOUNT_TRIE_NODES`, `STORAGE_TRIE_NODES`).

## What Stayed the Same

| Component              | Notes                                              |
|------------------------|----------------------------------------------------|
| LEVM (the EVM)         | Zero code changes. Completely unaware of the trie. |
| FKV tables             | Same tables, same key format, same O(1) lookups.   |
| Pipeline structure     | Same 3 threads: warmer, executor, merkleizer.      |
| StoreVmDatabase        | Still reads from FKV, not from the trie.           |
| Block/header/receipt DB| Unchanged.                                         |
| P2P, RPC, mempool      | Unchanged.                                         |
| Consensus validation   | Unchanged (except state root check is skipped).    |

## FAQ

**Why do we keep both FKV and the trie? Can't we just use one?**

They serve different purposes. FKV is optimized for fast O(1) reads during EVM
execution (the VM needs to look up balances, storage slots, etc. millions of
times per block). The trie is optimized for computing a cryptographic
commitment to the entire state (the state root). The trie's tree structure
makes it expensive to do random key lookups, and FKV's flat structure can't
produce a Merkle root. Both are persisted to disk, so there is storage
duplication, but eliminating either would mean either slow execution (no FKV)
or no state roots (no trie).

**What happens if the node crashes mid-flush?**

Trie node flushes use RocksDB's atomic `WriteBatch`, so a crash mid-flush
means the entire batch is discarded. On restart, the node detects the last
successful checkpoint (block number + hash stored in `BINARY_TRIE_NODES`
metadata) and resumes from there, re-executing any blocks after the
checkpoint.

**The state root check is skipped. Is that a security concern?**

The block header's `state_root` field is an MPT root, which is meaningless to
the binary trie. The binary trie computes its own root per block. Equivalence
between the two is proven externally by the zkVM proving layer (the EF Privacy
team's scope), not by direct comparison. For the binary trie node itself,
correctness is validated by replaying real chain blocks and checking that gas
usage and receipts match.

**What's the `storage_root` sentinel? Is it fragile?**

The MPT stores a `storage_root` (hash of the account's storage sub-trie) in
each account's FKV entry. The VM uses `storage_root != EMPTY_TRIE_HASH` to
check if an account has storage, which affects gas costs and account emptiness.
The binary trie has no per-account storage sub-trie, so there's no natural
value for this field. We synthesize a sentinel: `EMPTY_TRIE_HASH` for no
storage, `H256(1)` for has storage. It works but it's a known piece of tech debt. The clean fix would be to add an
explicit `has_storage: bool` to the VM's account representation, removing the
dependency on `storage_root` entirely. We opted for the sentinel to avoid
refactoring `AccountState` usage across storage, blockchain, VM, and RPC code.

**How long does a full replay from genesis take?**

This depends on the chain. On Hoodi (test network), 10k+ blocks replay in
minutes. Mainnet would take significantly longer since we re-execute every
block from genesis (there's no bulk state conversion because the MPT stores
`keccak(address)` as keys and keccak is irreversible, so we don't have the
original addresses to compute binary trie keys). The binary trie itself is
fast (~4% of total replay time); LEVM execution dominates at ~65%.

**What happens on a reorg?**

The current implementation has limited reorg support. FKV only stores the
latest state and has no undo log, so a reorg requires reloading the trie from
the last disk checkpoint and re-executing the new fork's blocks. This works for
shallow reorgs (a few blocks) but is expensive for deeper ones. A proper FKV
undo log is planned but not yet implemented.

**How does performance compare to the MPT?**

We don't have direct head-to-head benchmarks yet. What we know from profiling
the binary trie in isolation: it accounts for ~4% of total block processing
time during replay, while LEVM (EVM execution) dominates at ~65%. The
incremental merkleization means per-block cost scales with the number of
modified accounts, not the total trie size. The trie is not the bottleneck.

**Why BLAKE3 and not Poseidon2?**

BLAKE3 was chosen to match the EIP-7864 reference implementation. Poseidon2
would be ~50-200x cheaper inside ZK circuits (SP1, RISC Zero), but BLAKE3 is
much faster in native execution. For this prototype, native performance matters
more. If ZK proving cost becomes a bottleneck, the hash function can be swapped
later (it's isolated in `hash.rs` and `merkle.rs`).

**What's the proof size improvement over MPT?**

An MPT proof is ~8-10 RLP-encoded nodes (1-4KB). A binary trie proof is ~25
sibling hashes (~800 bytes). The binary structure (2 children per node instead
of 16) means more levels but each level only needs one sibling hash (32 bytes)
instead of up to 15.

**Can two different accounts end up on the same stem?**

The stem is `BLAKE3(zero_padded_address || tree_index)[0..31]`. For different
addresses with the same `tree_index`, a collision would require a BLAKE3
collision on the first 31 bytes (248 bits). This is astronomically unlikely
and treated as impossible. If it did happen, the second account's insert would
trigger a stem split, creating InternalNodes to distinguish them, but with
248-bit stems this would mean diverging at a very deep level.
