# Optimization Plan: ethrex Steady-State Performance

Based on a 10-minute CPU profile of ethrex syncing Hoodi (blocks 2177933-2177996) in steady state after FlatKeyValue generation, with 3-4 connected peers.

**Profile summary:** 600s duration, 56.82s total samples (9.47% CPU utilization)

---

## CPU Budget Breakdown

| Category | Flat % | Cumulative % | Samples |
|----------|--------|-------------|---------|
| P2P Discovery | 24.0% | 31.0% | 17.6s |
| KZG Blob Validation | 15.7% | 15.0% | 8.5s |
| RocksDB (compaction + reads) | 15.2% | 20.0% | 11.4s |
| EVM Execution | 3.9% | 10.2% | 5.8s |
| Synchronization primitives | 5.0% | 7.6% | 4.3s |
| Memory allocation | 3.4% | 10.9% | 6.2s |
| Merkleization / Trie | — | 3.4% | 1.9s |
| P2P RLPx | 3.1% | 16.7% | 9.5s |

---

## Optimization 1: Discovery Server Throttling

**Impact: HIGH (~20% CPU reduction)**
**Effort: LOW**
**Risk: LOW**

### Evidence

- `UdpSocket::send_to` is the **#1 flat-time consumer** at 16.0% (9.09s)
- `UdpSocket::poll_recv_from` adds another 5.84% (3.32s)
- `DiscoveryServer::handle_cast` totals 19.39% cumulative (11.02s)
- Two independent lookup timers (`Lookup` and `EnrLookup`) each start at 100ms intervals
- `get_contact_for_lookup` and `get_contact_for_enr_lookup` perform O(n) linear scans with Vec allocation over the entire contact table on every invocation

### Root Causes

1. **Aggressive initial interval**: `INITIAL_LOOKUP_INTERVAL_MS = 100ms` means 10 lookups/sec at startup, with a cubic easing function that reaches 600ms only at 100% peer completion
2. **Two independent timers**: Both `Lookup` and `EnrLookup` are scheduled independently, doubling the effective rate
3. **No hard backoff**: Even with sufficient peers, lookups never fully stop — the minimum rate is 100/minute
4. **Linear scans per lookup**: `get_contact_for_lookup` collects all valid contacts into a Vec to randomly pick one

### Proposed Changes

#### 1a. Stop lookups when peer target is met

```rust
// In handle_cast for Lookup and EnrLookup:
if self.peer_table.target_peers_completion().await >= 1.0 {
    // Reschedule at a very slow maintenance rate (e.g., 30s)
    send_after(Duration::from_secs(30), handle.clone(), Self::CastMsg::Lookup);
    return;
}
```

#### 1b. Use reservoir sampling instead of collect + choose

```rust
fn get_contact_for_lookup(&self) -> Option<Contact> {
    let mut rng = rand::rngs::OsRng;
    let mut result: Option<&Contact> = None;
    let mut count = 0u64;
    for c in self.contacts.values() {
        if c.n_find_node_sent < MAX_FIND_NODE_PER_PEER && !c.disposable {
            count += 1;
            if rng.gen_range(0..count) == 0 {
                result = Some(c);
            }
        }
    }
    result.cloned()
}
```

This eliminates the Vec allocation entirely while maintaining uniform random selection.

#### 1c. Merge Lookup and EnrLookup into a single timer

Instead of two independent timers both firing at the same rate, alternate between lookup types on each tick:

```rust
Self::CastMsg::Lookup => {
    if self.next_lookup_is_enr {
        self.enr_lookup().await;
    } else {
        self.lookup().await;
    }
    self.next_lookup_is_enr = !self.next_lookup_is_enr;
    let interval = self.get_lookup_interval().await;
    send_after(interval, handle.clone(), Self::CastMsg::Lookup);
}
```

#### 1d. Increase the base interval

Change `INITIAL_LOOKUP_INTERVAL_MS` from 100ms to 500ms and `LOOKUP_INTERVAL_MS` from 600ms to 5000ms. The discovery server doesn't need to be this aggressive — devp2p nodes tolerate much slower discovery.

### Expected Impact

- 16% flat UDP send overhead → ~2-4% (8-12% reduction)
- 5.84% poll_recv_from → proportional reduction
- Peer table scan overhead → near zero with reservoir sampling

---

## Optimization 2: KZG Blob Validation Deduplication

**Impact: HIGH (~10-15% CPU reduction)**
**Effort: MEDIUM**
**Risk: LOW**

### Evidence

- `BlobsBundle::validate` = 14.99% cumulative (8.52s)
- `verify_cell_kzg_proof_batch` = 14.97% cumulative
- Called from `PooledTransactions::validate_requested` (8.62% cum) AND again in `add_blob_transaction_to_pool`
- BLS curve operations (`POINTonE1_double` 2.66%, `_compute_commitment` 3.99%, `pippenger` 3.40%) dominate inside

### Root Causes

1. **Duplicate validation**: The same blob is validated in `validate_requested()` (P2P layer, line 1068 of server.rs) AND again in `add_blob_transaction_to_pool()` (blockchain layer). The exact same KZG proof is verified twice.
2. **No cache**: There is no cache of already-validated blob versioned hashes or commitment proofs.
3. **Synchronous on hot path**: KZG validation blocks the peer connection handler thread.

### Proposed Changes

#### 2a. Eliminate duplicate validation

The simplest fix: remove the KZG validation from `add_blob_transaction_to_pool()` since blobs are already validated in `validate_requested()`. Add a comment documenting that blobs are pre-validated at the P2P layer.

Alternatively, add a `validated: bool` flag to the blob bundle that is set after the first successful validation, and check it before re-validating.

#### 2b. Cache validated blob versioned hashes

Add a bounded LRU cache (e.g., 1024 entries) of recently validated `(versioned_hash, commitment, proof)` tuples:

```rust
use lru::LruCache;
use std::sync::Mutex;

static VALIDATED_BLOBS: Lazy<Mutex<LruCache<H256, ()>>> =
    Lazy::new(|| Mutex::new(LruCache::new(NonZeroUsize::new(1024).unwrap())));

pub fn validate(&self, tx: &EIP4844Transaction, fork: Fork) -> Result<(), MempoolError> {
    // Check cache first
    let cache_key = self.compute_cache_key();
    if VALIDATED_BLOBS.lock().unwrap().contains(&cache_key) {
        return Ok(());
    }
    // ... expensive validation ...
    VALIDATED_BLOBS.lock().unwrap().put(cache_key, ());
    Ok(())
}
```

#### 2c. Offload KZG validation to a background task

Move the expensive `verify_cell_kzg_proof_batch` call off the peer connection handler thread:

```rust
// Instead of blocking on validation:
tokio::spawn(async move {
    if let Err(e) = blobs_bundle.validate(&tx, fork) {
        // Handle invalid blob
    }
});
```

This unblocks the peer connection loop for other messages.

### Expected Impact

- Eliminating duplicate validation alone halves the KZG cost: 15% → ~7.5%
- With caching, repeated blobs from multiple peers are validated once
- Combined: 15% → ~3-5% (10-12% reduction)

---

## Optimization 3: Peer Table Data Structure Improvements

**Impact: MEDIUM (~2-4% CPU reduction)**
**Effort: LOW**
**Risk: LOW**

### Evidence

- `PeerTableServer::get_contact_for_enr_lookup` = 2.20% flat (1.25s)
- `PeerTableServer::get_contact_for_lookup` = 1.83% cumulative (1.04s)
- `get_closest_nodes` uses O(n) scan with nested loop
- All lookup functions allocate a Vec on every call

### Root Causes

1. **Full contact table scan** on every lookup (potentially 100K+ entries)
2. **Vec allocation** per call to collect filtered results
3. **`get_closest_nodes`** uses O(n * k) insertion sort instead of a proper k-nearest structure

### Proposed Changes

#### 3a. Reservoir sampling (covered in Optimization 1b)

#### 3b. Use a BinaryHeap for `get_closest_nodes`

```rust
use std::collections::BinaryHeap;

fn get_closest_nodes(&self, node_id: H256) -> Vec<Node> {
    let mut heap = BinaryHeap::with_capacity(MAX_NODES_IN_NEIGHBORS_PACKET + 1);
    for (contact_id, contact) in &self.contacts {
        let distance = Self::distance(&node_id, contact_id);
        heap.push((distance, contact.node.clone()));
        if heap.len() > MAX_NODES_IN_NEIGHBORS_PACKET {
            heap.pop(); // Remove farthest
        }
    }
    heap.into_sorted_vec().into_iter().map(|(_, node)| node).collect()
}
```

#### 3c. Maintain a pre-filtered candidate set

Keep a separate `Vec<NodeId>` of non-disposable contacts updated on insert/remove, avoiding the need to filter the full table on every lookup.

### Expected Impact

- 2.20% + 1.83% → ~0.5% (3.5% reduction)

---

## Optimization 4: RocksDB Tuning

**Impact: MEDIUM (~2-5% CPU reduction)**
**Effort: LOW**
**Risk: LOW**

### Evidence

- `RandomAccessFileReader::Read` = 15.23% flat (8.65s), split between foreground (5.03% via storage path) and background compaction (14.75% cum)
- `PosixClock::CPUMicros` = 0.96% flat — RocksDB internal timing overhead
- `WritableFileWriter::WriteBuffered` = 0.96% flat (compaction output)
- `FullFilterBlockReader::MayMatch` = 0.84% flat (bloom filter checks)

### Root Causes

1. **Background compaction** dominates RocksDB CPU at 14.75% cumulative — this is expected but tunable
2. **PosixClock::CPUMicros** at 0.96% is pure overhead from RocksDB's internal perf timing
3. **Block cache misses** trigger disk reads (8.65s in RandomAccessFileReader::Read)

### Proposed Changes

#### 4a. Disable RocksDB internal timing

```rust
let mut opts = rocksdb::Options::default();
opts.set_report_bg_io_stats(false);
opts.set_statistics_level(rocksdb::statistics::StatsLevel::DisableAll);
```

This eliminates the 0.96% `PosixClock::CPUMicros` overhead.

#### 4b. Increase block cache size

If block cache is undersized, more reads hit disk. Verify current cache size and consider increasing it to reduce the 15.23% read overhead.

#### 4c. Tune compaction

Consider adjusting compaction parameters to reduce background CPU:
- Increase `max_bytes_for_level_base`
- Use `level_compaction_dynamic_level_bytes = true`
- Reduce `max_background_compactions` if currently too high

### Expected Impact

- PosixClock elimination: 0.96% reduction
- Block cache tuning: 1-3% reduction in read path
- Compaction tuning: 1-2% reduction in background CPU

---

## Optimization 5: Synchronization Overhead Reduction

**Impact: LOW-MEDIUM (~2-3% CPU reduction)**
**Effort: MEDIUM**
**Risk: MEDIUM**

### Evidence

- `parking_lot::condvar::Condvar::notify_one_slow` = 3.05% flat (1.73s)
- `parking_lot::condvar::Condvar::wait_until_internal` = 1.96% flat (1.12s)
- `std::sync::mpmc::waker::SyncWaker::notify` = 0.91% flat
- `tokio::scheduler::schedule_task` = 1.69% flat
- Total synchronization overhead: ~7.6% cumulative

### Root Causes

1. **GenServer pattern**: The `spawned_concurrency` framework uses condvar-based messaging, which shows up as `notify_one_slow` and `wait_until_internal`
2. **tokio task scheduling overhead**: 1.69% flat in `schedule_task` — high frequency of small async tasks
3. **MPMC channel overhead**: Used between discovery server and peer table

### Proposed Changes

#### 5a. Batch messages to peer table

Instead of one `cast()` message per discovery event, batch updates:

```rust
// Instead of:
self.peer_table.record_ping_sent(&node_id, hash).await?;
// Consider:
self.pending_peer_updates.push(PeerUpdate::PingSent(node_id, hash));
if self.pending_peer_updates.len() >= BATCH_SIZE || timer_expired {
    self.peer_table.batch_update(std::mem::take(&mut self.pending_peer_updates)).await?;
}
```

#### 5b. Reduce cross-task communication frequency

The discovery server's 100ms timer generates many small messages to the peer table and tokio scheduler. Reducing timer frequency (Optimization 1) will proportionally reduce synchronization overhead.

### Expected Impact

- Reducing discovery timer frequency alone reduces sync overhead proportionally
- Message batching: ~1% additional reduction

---

## Optimization 6: EVM Storage Access Path

**Impact: LOW-MEDIUM (~1-2% CPU reduction)**
**Effort: MEDIUM**
**Risk: LOW**

### Evidence

- `access_storage_slot` = 3.35% cumulative
- `CachingDatabase` uses `RwLock<FxHashMap>` — contention on parallel prewarming
- `Trie::get` = 3.42% cumulative (some overlap with merkleization)
- `NodeRef::get_node_mut` = 1.91% cumulative (Arc::make_mut clones)

### Root Causes

1. **RwLock contention** in CachingDatabase on cache misses
2. **Trie node lazy loading** from RocksDB on first access

### Proposed Changes

#### 6a. Replace RwLock with DashMap in CachingDatabase

Already being explored in PR #5999. DashMap provides lock-free reads and sharded writes.

#### 6b. Prefetch trie nodes

The prewarmer already warms storage, but if cache miss rate is high, consider prefetching trie nodes along the expected access path before EVM execution starts.

### Expected Impact

- DashMap replacement: ~0.5-1% reduction in contention
- Better prefetching: ~0.5-1% reduction in cold trie reads

---

## Implementation Priority

| Priority | Optimization | Impact | Effort | First Step |
|----------|-------------|--------|--------|------------|
| **P0** | 1. Discovery throttling | ~20% CPU | Low | Add backoff at target peers |
| **P0** | 2. KZG dedup | ~10-15% CPU | Medium | Remove duplicate validation call |
| **P1** | 3. Peer table data structures | ~2-4% CPU | Low | Reservoir sampling |
| **P1** | 4. RocksDB tuning | ~2-5% CPU | Low | Disable internal timing |
| **P2** | 5. Sync overhead | ~2-3% CPU | Medium | Reduce timer frequency |
| **P2** | 6. EVM storage path | ~1-2% CPU | Medium | DashMap replacement |

**P0 alone (items 1+2) would reduce CPU usage by ~30-35%** for steady-state syncing.

---

## Methodology

- **Profile tool**: pprof-rs (1000 Hz frame-pointer sampling)
- **Build**: `cargo build --release --features cpu_profiling` (ethrex v9.0.0)
- **Workload**: 10 minutes Hoodi full sync, blocks 2177933-2177996
- **Environment**: macOS aarch64 (Apple Silicon), 3-4 peers connected
- **FKV status**: Already generated (no bias from FlatKeyValue indexing)
- **Analysis**: `go tool pprof` with subsystem-focused views (discovery, KZG, EVM, trie, RocksDB, sync)
