# Binary Trie Historical State: Design and Rationale

## The Problem

The binary trie stores one version of state on disk: the latest flushed snapshot. An
in-memory diff tree holds the last ~128 blocks of changes on top of that snapshot.

This means historical state queries ("what was account X's balance at block 5,000,000?")
only work correctly for recent blocks within the diff window. For older blocks, the code
silently falls back to the flushed snapshot, returning the **wrong** state for any account
that changed between the queried block and the flush point.

The MPT does not have this problem. Every block's state root points into a
content-addressed tree of nodes on disk. Old nodes are never overwritten because their
identity is their hash -- modifying a node produces a new node with a new hash. History
is preserved as a natural consequence of the data structure.

The binary trie uses stable NodeIds. When a node is modified, the same NodeId is
overwritten in place. Old state is lost. We need an explicit mechanism to preserve history.

## Options Considered

### Option A: Persisted Diff Layers (chosen)

Store each block's state diff (the set of account and storage values that changed) to
disk. The trie on disk remains a single latest-state snapshot. Historical queries walk
backward through persisted diffs until the target value is found or the base is reached.

### Option B: Versioned Node Storage

Store multiple versions of each trie node, keyed by `(NodeId, block_number)`. The trie
itself becomes versioned. Historical queries traverse the trie at a specific block by
reading the correct version of each node along the path.

### Why Option A

The decision comes down to what makes the binary trie different from the MPT, and whether
we should lean into that difference or replicate MPT's approach.

**The binary trie separates structure from state.** The trie is an index -- a lookup
structure that maps 32-byte keys to values. The actual state (account balances, storage
slot values, code) lives in the leaf values. This separation does not exist in the MPT,
where the trie nodes ARE the state. Every MPT state root leads to a different tree of
nodes, and traversing that tree is the only way to recover state at a given block.

Option A leverages this separation. The trie stays as a fast, compact, single-version
index optimized for the hot path (block execution). History is stored separately as a
flat journal of value-level diffs. This is analogous to how databases work: a B-tree
index for fast lookups, plus a write-ahead log for history and recovery.

Option B ignores this separation. It versions the trie structure itself, making the
binary trie behave like the MPT: every modification creates new node versions along the
path, old versions are kept. The binary trie's advantage of stable, overwritable NodeIds
is discarded.

**Disk usage.** Option A stores only the changed values per block. A typical block
modifying 200 accounts and 500 storage slots produces ~50-100 KB of diff data (addresses,
keys, values). Option B stores all modified trie nodes along the path: ~25 InternalNodes
(73 bytes each) plus StemNodes (~450 bytes each) per modification, totaling ~150-350 KB
per block. Option A uses 2-5x less disk. Both use significantly less disk than the MPT,
which duplicates entire node paths (often 1-5 KB per modification due to larger nodes).

**Hot path impact.** Option A does not change the trie's read or write path at all. Block
execution reads from the in-memory diff tree and base trie exactly as it does today.
Historical queries are a separate code path that loads diffs from disk. Option B requires
every node write to include a block number, every node read to do a versioned seek, and
the NodeStore's cache system to become version-aware. This is a significant refactor of
the trie's core with risk of regression on the hot path.

**Implementation complexity.** Option A is roughly 200-400 lines of new code, primarily
in the state layer (serializing diffs, storing/loading them, walking backward). The trie
internals (NodeStore, node_store.rs, trie.rs) are untouched. Option B requires 500-800+
lines touching the core NodeStore, changing the RocksDB key format, adding versioned
reads, and updating every call site that accesses nodes.

**Historical read performance.** This is Option A's weakness. Querying block N requires
walking backward through diffs from block N until the target value is found. For recent
blocks this is fast (a few diffs). For very old blocks it could be slow (thousands of
diffs). Option B gives consistent O(tree_depth) reads at any block.

This weakness is addressed by periodic snapshots (described below).

## Periodic Snapshots (future optimization)

Option A's ancient query performance can be bounded by storing periodic full-state
snapshots. Every N blocks (e.g., 10,000), persist a frozen copy of the base trie. To
query block 15,234:

1. Find the nearest snapshot at or before block 15,234 (snapshot at block 10,000)
2. Forward-apply diffs from block 10,001 to 15,234
3. Return the resulting state

This caps the walk length at the snapshot interval. With 10,000-block intervals, worst
case is applying 10,000 diffs, which takes well under a second.

Snapshots are deferred to a later phase. The initial implementation supports full
historical queries without them -- just slower for very old blocks.

## How It Works

### Data flow

```
Block execution
    |
    v
StateDiff created (accounts changed, storage slots changed, code deployed)
    |
    +--> In-memory DiffTree (last ~128 blocks, fast access)
    |
    +--> RocksDB "diffs" column family (permanent, keyed by block hash)
    |
    v
On flush (every 128 blocks):
    - Base trie on disk updated to latest state
    - In-memory diffs for flushed blocks pruned
    - On-disk diffs are NOT pruned (they are the historical record)
```

### Query paths

**Latest state (block execution, RPC "latest"):**
Same as today. Read from in-memory diff tree, fall back to base trie. No change.

**Recent historical (within diff window, ~128 blocks):**
Same as today. In-memory diff tree lookup. No change.

**Older historical (outside diff window):**
1. Load the StateDiff for the queried block hash from RocksDB
2. If the target value is in this diff, return it
3. If not, load the parent block's diff and check there
4. Continue walking backward through parent diffs
5. If we reach a block at or before the base checkpoint, read from the base trie

The parent hash for each block is stored in the persisted diff (or can be resolved from
block headers in the Store). Walking backward is a chain of RocksDB point lookups.

### DiffLookup fix

The current `DiffLookup` enum conflates two cases into `NotInDiffs`:

1. **Walked to base hash**: the value was not modified in any diff layer, so the base
   trie has the correct value. This is a valid fallback.
2. **Unknown block hash**: the block is not in the in-memory diff tree at all. Falling
   back to the base trie returns the wrong state.

After this change, case 2 triggers the on-disk diff lookup path instead of silently
returning incorrect data.

## Tradeoff Summary

| Dimension | Option A (chosen) | Option B | MPT |
|-----------|-------------------|----------|-----|
| Disk per block | ~50-100 KB | ~150-350 KB | ~200-500 KB |
| Latest reads | Unchanged (fast) | Versioned seek | Trie traversal |
| Recent history | In-memory diffs | Versioned seek | Trie traversal |
| Ancient history | Walk diffs (slow, fixable with snapshots) | Versioned seek (fast) | Trie traversal |
| Hot path changes | None | Major (NodeStore refactor) | N/A |
| Implementation risk | Low | High | N/A |
| Leverages binary trie design | Yes (trie as index, diffs as journal) | No (versions trie like MPT) | N/A |
