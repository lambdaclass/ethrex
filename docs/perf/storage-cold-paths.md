# Storage: cold paths, accounts, trie, FKV, caches

Audience: perf analyzers. This documents how ethrex reads state, what makes a
read hot or cold, and the knobs that move the boundary. All references are to
`crates/storage/` and `crates/common/trie/` on `main`. Line numbers drift; grep
the named symbols.

## TL;DR

- State lives in RocksDB as a **single-version, path-keyed** trie plus a
  redundant **flat key-value (FKV)** copy of the leaves. Four column families:
  `account_trie_nodes`, `storage_trie_nodes`, `account_flatkeyvalue`,
  `storage_flatkeyvalue`.
- A read is resolved through three tiers, hottest first:
  1. **In-memory diff-layer overlay** (`TrieLayerCache`) holding the newest
     **128 blocks** of trie diffs.
  2. **FKV point lookup** (one RocksDB get at the full path) when the FKV
     watermark covers the key.
  3. **Trie-node walk** (root to leaf, `depth` gets) when FKV is not yet built
     for that key or the key was written in this trie instance.
- Each tier-2/3 get then resolves inside RocksDB: memtable -> shared block cache
  -> SST on disk. **Truly cold** = not in the 128-layer overlay, not in the
  block cache, not in a memtable, so it costs physical disk I/O.
- The 128-layer window bounds how far back state is in RAM. Anything older than
  ~128 blocks is disk-only.

## Data model: how state is keyed on disk

The trie is **path-keyed**, not hash-keyed. On commit, each node is written at
its trie path (nibbles), and each leaf is written **twice**:
`NodeRef::commit` (`crates/common/trie/node.rs`) pushes `(full_path, value)` for
the leaf and `(node_path, encoded_node)` for the node itself. The `(full_path,
value)` pair is the FKV entry. (`tables.rs` still comments "node_hash.as_ref()"
for these CFs; that is stale, the key is the path.)

Routing to a CF is by **key length** (`BackendTrieDB::table_for_key` in
`trie.rs`, mirrored in `commit_trie_layers` in `store.rs`):

| Key length (nibbles) | Meaning | Column family |
|---|---|---|
| `<= 64` | account-side internal/branch/extension node | `account_trie_nodes` |
| `== 65` | account leaf full path (keccak(addr) 64 + leaf term 16) | `account_flatkeyvalue` |
| `66..=130` | storage-side internal node | `storage_trie_nodes` |
| `== 131` | storage leaf full path | `storage_flatkeyvalue` |

`is_leaf = len == 65 || len == 131`, `is_account = len <= 65`.

Storage keys carry an address prefix (`apply_prefix` in `layering.rs`):
`Nibbles::from_bytes(addr) [65] .append_new(17) [66] .concat(slot_path)`. The
`17` is an invalid nibble used as a separator, so a storage leaf path is
`66 + 65 = 131` nibbles.

Other CFs relevant to account access:

| CF | Key | Value | Notes |
|---|---|---|---|
| `account_codes` | code hash | bytecode | RocksDB blob files, `min_blob_size=32`, lz4 |
| `account_code_metadata` | code hash | code length (u64) | tiny, for size-only queries |
| `misc_values` | `"last_written"` | FKV watermark | see FKV section |

## Read path (one account or one storage slot)

Entry points: `get_account_info` / `get_account_state` / `get_account_info_by_hash`
/ `get_storage_at` / `get_storage_at_root` (`store.rs`). They open a trie and
call `Trie::get`, which routes through the tiers below.

```
Trie::get(path)                                  crates/common/trie/trie.rs
  │
  ├─ if path is dirty in this trie ─────────────► trie-node walk (tier 3)
  │
  ├─ if db.flatkeyvalue_computed(path):           TIER 2  (FKV fast path)
  │     db.get(full_path)  ── one point get
  │        │
  │        └─ TrieWrapper::get                    crates/storage/layering.rs
  │              ├─ TrieLayerCache.get  ────────► TIER 1  (in-memory overlay)
  │              └─ BackendTrieDB.get   ────────► RocksDB point get on *_flatkeyvalue
  │
  └─ else:                                        TIER 3  (structural walk)
        root → branch/extension → … → leaf
        each hop = TrieWrapper::get on a node path
        (TIER 1 overlay, then RocksDB point get on *_trie_nodes)
```

### Tier 1: in-memory diff-layer overlay (`TrieLayerCache`)

`crates/storage/layering.rs`. Held on `Store` as
`trie_cache: Arc<RwLock<Arc<TrieLayerCache>>>` and RCU-swapped by the persist
worker. It is a chain of per-block diff layers keyed by state root, linked
newest -> oldest by `parent`:

```
newest_root -> parent_1 -> … -> oldest_root -> (on-disk trie)
```

- Each `TrieLayer` is an `FxHashMap<path_bytes, node_bytes>` of one block's trie
  diffs (one batch of ~1024 blocks in full sync).
- `get(state_root, key)` checks a **global bloom filter** first (1,000,000
  items, 2% FP, `FxBuildHasher`). Bloom miss returns `None` immediately, so the
  caller falls through to RocksDB without walking the chain. Bloom hit walks the
  parent chain and returns the first (newest) layer that has the key.
- `commit_threshold` decides how many layers stay resident:
  - **128** for regular block-by-block execution (`DB_COMMIT_THRESHOLD`).
  - **4** for full sync / batch mode (`BATCH_COMMIT_THRESHOLD`, each layer ≈
    1024 blocks ≈ 1 GB).
  - **10000** for the in-memory backend (`IN_MEMORY_COMMIT_THRESHOLD`, tests
    need deep history).

This is pure RAM (FxHashMap lookups), the hottest tier. An account touched in
the last 128 blocks is served here with no RocksDB access.

The `open_direct_*` trie constructors (`store.rs`) skip the `TrieWrapper`
overlay and read straight from `BackendTrieDB` (used by genesis and the FKV
generator). Reads through those do not consult Tier 1.

### Tier 2: FKV fast path

`Trie::get` (`trie.rs`) takes the flat path when the key is not dirty **and**
`db.flatkeyvalue_computed(path)` is true. It then does a single `db.get` on the
full leaf path, which hits the `*_flatkeyvalue` CF directly. One logical point
lookup, no structural traversal.

`flatkeyvalue_computed` is a **watermark** check
(`BackendTrieDB::flatkeyvalue_computed`, `last_computed_flatkeyvalue >= key`;
`Store::flatkeyvalue_computed_with_last_written` uses `last_written[0..64] >
account_nibbles`). See the FKV section for what advances the watermark.

`get_storage_at_root` (`store.rs`) shows the intent: when FKV covers the
account, it skips the state-trie account lookup entirely and opens the storage
trie against `EMPTY_TRIE_HASH`, letting the FKV point lookup return the slot.
`get_account_states_batch_by_root` batches the FKV-covered addresses into one
RocksDB `multi_get` on `account_flatkeyvalue`.

### Tier 3: trie-node walk (structural, colder)

Taken when FKV is not yet built for the key range (during/after snap sync,
before the FKV generator reaches it) or the key was modified in this trie
instance (`dirty`). The walk descends root -> branch/extension -> leaf, doing
one `TrieWrapper::get` per hop. On mainnet the account trie is ~7-9 nodes deep,
so this is `depth` point lookups instead of one, each independently subject to
the overlay/block-cache/disk hierarchy. When cold, this multiplies disk seeks by
trie depth and is the most expensive read shape.

## What "truly cold" means

Every tier-2/3 get bottoms out in RocksDB (`backend/rocksdb.rs`), which resolves
in order:

1. **Memtable** (in-flight writes, RAM).
2. **Shared block cache** (LRU, default **12 GiB**). Holds data blocks **and**
   index + bloom-filter blocks (`cache_index_and_filter_blocks(true)`), with L0
   filter/index pinned.
3. **SST files on disk**, one probe per LSM level not pruned by a bloom filter.

A read is **truly cold** when the key is:

- not in any of the ≤128 in-memory diff-layers (account not touched in ~128
  blocks), and
- not in a memtable, and
- its data block (and the index/filter blocks needed to find it) are not
  resident in the block cache.

Then it costs real I/O: a random NVMe read per SST level touched (index block +
data block), reduced by bloom filters that skip SSTs which cannot hold the key.
State/trie/FKV CFs are stored **uncompressed** (see tuning), so the OS page
cache also backstops these reads.

Cost ranking of a single account/slot read:

| Situation | Cost |
|---|---|
| Touched in last 128 blocks | 1 FxHashMap lookup (Tier 1), no I/O |
| FKV-covered, block cache warm | 1 point get, cache hit |
| FKV-covered, cold | 1 point get, ~1 disk data-block read (+ filter/index if uncached) |
| FKV not built or dirty, cold | `depth` point gets, up to `depth` cold reads |

## The 128-block window (diff-layers to disk)

The overlay holds the newest `commit_threshold` blocks in RAM; older state is
disk-only. Commit is driven by the persist worker
(`commit_trie_if_due` -> `commit_trie_layers`, `store.rs`):

1. `get_commitable(state_root)` walks the layer chain; once ≥128 layers are
   stacked it returns the 128-deep ancestor root.
2. `TrieLayerCache::commit(root)` removes that layer and all older ancestors,
   merges their diffs oldest-first, prunes orphaned layers, and rebuilds the
   bloom.
3. The merged diffs are written to RocksDB in one write batch (routed to the
   four CFs by key length), then the trimmed cache is RCU-swapped in.

Implications for perf and correctness:

- **Reorg depth is bounded by the window.** Once a layer is folded into the
  single-version on-disk trie the overwritten ancestor node is gone.
- **Shutdown deliberately drops the uncommitted tail** (`Store::shutdown`). The
  on-disk path store cannot reconstruct overwritten ancestors, so the recent
  (< 128) layers are dropped and re-executed from the deep on-disk base on the
  next start. Block data (headers/bodies/receipts) is flushed; trie diffs are
  not.
- Full sync trades window depth for memory: 4 fat layers instead of 128 thin
  ones.

## FKV generation and the watermark

The FKV is not present after snap sync; it is **backfilled** by a background
thread (`flatkeyvalue_generator`, `store.rs`) and gated by a watermark stored in
`misc_values["last_written"]` and mirrored in
`Store::last_computed_flatkeyvalue: Arc<RwLock<Vec<u8>>>`.

Watermark semantics:

- Absent / `vec![0u8; 64]` -> FKV empty, every read falls to the trie-node walk.
- Partial value -> FKV built for keys `<= watermark` (sorted order); reads below
  the watermark use FKV, reads above walk the trie.
- Sentinel `[0xff]` on disk -> expanded to all-`0xff` in memory -> **FKV fully
  built**, every account/slot read takes the one-get FKV fast path. This is the
  steady state of a synced node.

Genesis writes state only to the trie-node CFs (`setup_genesis_state_trie` via
`open_direct_*`), never to FKV. So immediately after genesis the FKV is empty and
every read walks trie nodes until the generator backfills.

Generator behavior:

- On first run it clears both FKV CFs, then iterates the account trie in sorted
  order (and each account's storage trie), writing leaf values to the FKV CFs
  and advancing `last_written`, committing every 10,000 entries.
- It is **paused** (`FKVGeneratorControlMessage::Stop`) around each
  `commit_trie_layers` (the underlying trie is changing) and **resumed**
  (`Continue`) after. It re-reads a fresh view each iteration and restarts from
  the watermark on `PivotChanged`.
- `commit_trie_layers` skips writing FKV leaf entries whose key is beyond the
  watermark (`is_leaf && key > last_written`); the leaf value still persists
  inside its trie node, and the generator backfills the flat entry later.

Perf reading: on a fully synced node FKV is complete, so account/storage reads
are single point lookups. During or right after snap sync, the un-migrated key
range still pays the deeper trie-node walk, so cold-path cost is higher until
the generator finishes.

## Store-level in-memory caches

Fields on `Store` (`store.rs`) that absorb reads before disk:

| Field | Type | Role |
|---|---|---|
| `trie_cache` | `Arc<RwLock<Arc<TrieLayerCache>>>` | 128-layer trie diff overlay (Tier 1) |
| `block_data_buffer` | `Arc<RwLock<Arc<BlockDataBuffer>>>` | headers/bodies/receipts/codes/tx-index for not-yet-flushed blocks |
| `last_computed_flatkeyvalue` | `Arc<RwLock<Vec<u8>>>` | FKV watermark |
| `account_code_cache` | `Arc<Mutex<CodeCache>>` | LRU bytecode cache, 64 MiB (`CODE_CACHE_MAX_SIZE`) |
| `code_metadata_cache` | `Arc<Mutex<FxHashMap<H256, CodeMetadata>>>` | code lengths only |
| `latest_block_header` | `LatestBlockHeaderCache` | cached canonical head header |
| `pending_trie_roots` | `Arc<PendingTrieRoots>` | gate so a reader blocks until a just-added block's layer is installed (`gated_snapshot`) |

`BlockDataBuffer` (`block_data_buffer.rs`) is an RCU overlay: readers clone the
inner `Arc` under a brief read lock then work lock-free; the single persist
worker mutates a clone and swaps it in. It is consulted before disk for
headers, bodies, numbers, receipts, codes, and tx locations, and evicted after
the block data is durably flushed (`evict_flushed`, tracked by `flushed_upto`).

## RocksDB tuning that moves the cold boundary

From `RocksDBBackend::open` (`backend/rocksdb.rs`). What matters for read cost:

- **Shared LRU block cache**, default **12 GiB**
  (`DEFAULT_ROCKSDB_BLOCK_CACHE_SIZE_BYTES`, override with
  `--rocksdb.block-cache-size`). With `cache_index_and_filter_blocks(true)` +
  `pin_l0_filter_and_index_blocks_in_cache(true)`, this cache is the **effective
  ceiling on resident memory**. It holds data + index + filter blocks. Too small
  relative to the filter working set and filter blocks evict data blocks, so EVM
  reads spill to disk. (Rationale in code: under `max_open_files=-1` the default
  pins every SST's index+filter in heap, ~6 GB on a 490 GB mainnet DB.)
- **`max_open_files=-1`**: keep all SSTs open (no open/close churn on reads).
- **Compression off for state**: `account/storage_trie_nodes` and
  `account/storage_flatkeyvalue` use `None` -> uncompressed blocks, so reads are
  CPU-free and the OS page cache backstops them. `lz4` only for
  headers/bodies/receipts/block-numbers/tx-locations/fullsync-headers.
- **Bloom filters, 10 bits/key**, on all four state CFs, plus
  `memtable_prefix_bloom_ratio(0.2)`. These prune SSTs on point lookups, which
  is exactly the cold-read shape. No bloom on `transaction_locations` (it uses a
  merge operator; negative reads are rare).
- **Block size**: 16 KB for state CFs, 32 KB for headers/bodies/codes/receipts.
- **Large state write buffers**: 512 MB memtable x up to 6, target file 256 MB,
  L1 base 2 GB, dynamic level bytes. Fewer, larger levels -> fewer SSTs to probe
  per read, less write amplification.
- **`account_codes` blob files** (`min_blob_size=32`, lz4): large bytecodes go
  to blobs, keeping the LSM small and code point lookups cheap; delegation
  indicators (< 32 B) stay inline.
- **WAL**: PointInTime recovery, `fdatasync` (not fsync), `bytes_per_sync=32MB`.
  `flush()` on shutdown flushes memtables + WAL so the next open is
  recovery-free.

Tuning takeaways for a perf run:

- The block cache size is the single biggest lever on cold-read rate. Measure
  cache hit ratio before assuming disk latency is the problem.
- On a synced node, expect account/slot reads to be one FKV point get. If you
  see multi-node trie walks in a profile, check the FKV watermark
  (`last_written`) and whether the keys are dirty.
- Anything older than ~128 blocks bypasses Tier 1 entirely, so historical-state
  queries (RPC at old blocks) are inherently colder than head-following reads.
