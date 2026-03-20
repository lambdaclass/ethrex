# Binary Trie: Historical State via Persisted Diff Layers

## Background

Content-addressed tries (like the MPT) naturally preserve history. Each node's identity
is its hash, so modifying a node produces a new node with a new hash. Old nodes remain on
disk. Every state root points into a different tree, and traversing that tree recovers the
exact state at any past block. History is a free byproduct of the data structure.

The binary trie uses stable NodeIds. Nodes are mutable -- the same NodeId is overwritten
when state changes. This makes the trie compact and fast for latest-state access, but it
means historical state is not inherently preserved. An explicit mechanism is needed.

## Design: Persisted Diff Layers

The binary trie separates structure from state. The trie is an index: a lookup structure
mapping 32-byte keys to values. The actual state (account balances, storage slot values,
contract code) lives in the leaf values. This separation is fundamental to the design and
does not exist in the MPT, where the trie nodes ARE the state.

The historical state mechanism leverages this separation. The trie remains a single-version
index optimized for block execution. History is stored as a flat journal of value-level
diffs, one per block. This is analogous to how databases work: a B-tree index for fast
lookups, plus a write-ahead log for history.

Each block produces a `StateDiff`: the set of accounts, storage slots, and code that
changed. These diffs serve two roles:

1. **In-memory diff tree** (~128 most recent blocks): fast access for block execution and
   recent RPC queries. Supports branching for reorgs.
2. **On-disk diff archive** (all blocks): permanent record in RocksDB, keyed by block hash.
   Each record includes the parent block hash to enable backward traversal.

### Query paths

**Latest state** (block execution, RPC `latest`): read from the in-memory diff tree, fall
back to the base trie on disk. This is the hot path and is unaffected by the historical
state mechanism.

**Recent state** (within the ~128-block diff window): served entirely from the in-memory
diff tree. Same performance as latest state.

**Historical state** (older than the diff window): load the target block's `StateDiff` from
RocksDB. If the queried value (account, storage slot) appears in that diff, return it.
Otherwise, follow the parent hash to the previous block's diff and repeat. Continue walking
backward until the value is found or the base checkpoint is reached, at which point the
base trie provides the answer.

```
Query: "balance of 0xABC at block 5000"

  Block 5000 diff (disk) -- 0xABC not here
       |
       v  (parent_hash)
  Block 4999 diff (disk) -- 0xABC not here
       |
       v
  Block 4998 diff (disk) -- 0xABC balance = 42  --> return 42
```

Each step is a RocksDB point lookup (~10us with block cache). For a value last modified K
blocks before the queried block, the cost is O(K) lookups. Most account state is modified
frequently enough that K is small.

### Periodic snapshots

For queries deep in history where K is large (millions of blocks), backward walking becomes
slow. This is bounded by periodic snapshots: a frozen full-state checkpoint stored every N
blocks (e.g., every 10,000). To query block 15,234, find the nearest snapshot at or before
that block (block 10,000), then forward-apply diffs from 10,001 to 15,234. Worst-case walk
length is capped at the snapshot interval.

Snapshots are an optimization, not a requirement. Without them, all historical queries are
correct -- just slower for very old, infrequently-modified state.

### Diff lookup semantics

The in-memory diff tree distinguishes two cases when a value is not found:

- **NotModified**: the backward walk reached the base checkpoint without finding the value.
  The value was not modified in any block after the checkpoint. The base trie on disk holds
  the correct answer.
- **NotInMemory**: the queried block hash is not in the in-memory diff tree. The on-disk
  diff archive must be consulted.

This distinction prevents the silent-wrong-answer failure mode where an unknown block hash
falls through to the base trie and returns the latest flushed state instead of the state at
the requested block.

### Storage format

Each persisted diff record contains:

- **Parent hash** (32 bytes): enables backward traversal without coupling the trie crate to
  the block header storage layer.
- **Block number** (8 bytes): enables future pruning by block height without scanning keys.
- **StateDiff**: the set of modified accounts, storage slots, deployed code, and
  storage-cleared addresses for that block.

Records are keyed by `prefix_byte || block_hash` in the existing RocksDB instance, using
prefix-based namespacing consistent with the trie's code and storage key stores.

### Disk usage

A typical Ethereum mainnet block modifies ~200 accounts and ~500 storage slots. The
resulting diff is ~50-100 KB (addresses, keys, values -- no trie node overhead). Over 20M
blocks this is ~1-2 TB uncompressed, reduced 3-5x by RocksDB compression.

For comparison, the MPT stores ~10-15 new trie nodes per modification (path duplication
from root to leaf), totaling ~200-500 KB per block. Versioned node storage (storing each
modified binary trie node as a separate version) would produce ~150-350 KB per block.
Persisted diffs are the most space-efficient option because they store only raw values with
no structural overhead.

| Approach | Disk per block | Structural overhead |
|----------|---------------|-------------------|
| Persisted diffs | ~50-100 KB | None (values only) |
| Versioned nodes | ~150-350 KB | Path nodes duplicated per version |
| MPT (content-addressed) | ~200-500 KB | Full path duplication, larger nodes |

### Why not version the trie nodes

An alternative design stores multiple versions of each trie node keyed by
`(NodeId, block_number)`. This gives O(tree_depth) reads at any historical block --
consistent performance regardless of age. However:

- It versions the trie structure itself, discarding the binary trie's advantage of stable,
  overwritable NodeIds. The trie becomes append-only, similar to the MPT.
- Every node write must include a block number. Every node read must perform a versioned
  seek. The NodeStore's cache system must become version-aware. This is a deep refactor of
  the trie core with regression risk on the block execution hot path.
- Disk usage is 2-5x higher than persisted diffs because intermediate trie nodes along the
  modification path are stored, not just the changed values.

Persisted diffs keep the trie simple and fast for its primary job (block execution) while
adding history as a separate, orthogonal concern. The tradeoff -- slower ancient queries --
is bounded by periodic snapshots and acceptable for the RPC workload where recent queries
dominate.

## Binary Trie vs MPT: Historical State Comparison

| Dimension | Binary Trie (persisted diffs) | MPT (content-addressed) |
|-----------|-------------------------------|-------------------------|
| **How history is preserved** | Explicit journal of value-level diffs per block | Implicit -- old nodes remain on disk because modifying a node creates a new hash/identity |
| **Node identity** | Stable NodeIds, mutable in place | Content-addressed hashes, immutable once written |
| **Latest-state reads** | Direct lookup by NodeId, single-version trie | Traverse from latest state root through hash-linked nodes |
| **Historical reads** | Walk backward through diff chain, then fall back to base trie | Traverse from that block's state root -- same cost as latest |
| **Historical read cost** | O(K) where K = blocks since value last changed. Bounded by snapshots | O(depth) always, regardless of age |
| **Disk per block** | ~50-100 KB (changed values only) | ~200-500 KB (all path nodes from root to leaf duplicated) |
| **Disk growth model** | Proportional to state changes per block | Proportional to state changes * trie depth |
| **Structural overhead in history** | None -- diffs store raw values, no trie nodes | Full -- every intermediate node along the path is a new entry |
| **Proof generation** | Requires latest trie structure; historical proofs need state reconstruction | Any state root can generate proofs directly from its nodes on disk |
| **Pruning** | Delete diffs older than cutoff (simple key range delete) | Complex -- must identify unreferenced nodes across all state roots |
| **Branching/reorgs** | Diff tree supports multiple branches; on-disk diffs are per-block-hash | Each branch has its own state root pointing to shared subtrees |
| **Crash recovery** | Replay from last flush checkpoint; diffs persisted per block | State roots are always consistent on disk |
| **Concurrency** | Single-writer trie + concurrent readers via RwLock | Requires careful locking around trie cache layers |
| **ZK-friendliness** | Binary structure + Poseidon-friendly hashing | 16-way branching + keccak -- expensive in circuits |

**Key strengths of the binary trie approach:**
- 2-5x less disk for historical data due to value-only diffs (no structural duplication)
- Simpler pruning -- deleting old diffs is a key range delete, not a graph reachability problem
- The trie's hot path (block execution) is completely unaffected by the history mechanism
- Clean separation of concerns: the trie handles fast lookups, the diff journal handles history

**Key strengths of the MPT approach:**
- Consistent O(depth) historical reads at any block age with no additional mechanism
- Historical Merkle proofs are available directly from disk without state reconstruction
- History is an inherent property of the data structure, not a bolt-on

## Data flow

```
Block execution
    |
    v
StateDiff produced (accounts, storage slots, code)
    |
    +---> In-memory DiffTree (recent ~128 blocks)
    |
    +---> RocksDB diff archive (permanent, keyed by block hash)
    |
    v
Periodic flush (every ~128 blocks):
    - Base trie on disk updated to latest state
    - In-memory diffs for flushed blocks pruned
    - On-disk diffs retained as permanent historical record
```
