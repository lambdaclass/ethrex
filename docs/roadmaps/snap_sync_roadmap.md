# Snap Sync Module Roadmap

**Author:** Pablo Deymonnaz (original), ElFantasma (ongoing updates)
**Date:** February 2026 (last updated 2026-04-15)
**Status:** Draft for Review

---

## Executive Summary

This roadmap outlines a strategic plan to improve the ethrex snap sync module in three phases:

1. **Phase 1: Performance Optimization** - Make snap sync as fast as possible
2. **Phase 2: Code Quality & Maintainability** - Make the code clear, readable, and easier to understand
3. **Phase 3: Pipeline Architecture** - Migrate to spawned actors for pipelined concurrent execution

The snap sync module currently comprises ~4,900 lines across 11 files. Our goal is to achieve sync times competitive with geth while maintaining code quality standards.

> **April 2026 update (initial):** Spawned 0.5.0 has been [merged](#6295) — the actor framework blocker for Phase 3 is gone. Several new performance PRs (#6410, #6184, #6177, #6159, #6178) have been opened since the original roadmap. Phase 1 now includes trie building optimizations that represent the largest single improvement opportunity (-31% account insertion time). Phase 3 pipelining has been partially achieved without actors (#6184), validating the incremental approach.

> **2026-04-15 update:** Multiple workstreams advanced significantly:
>
> - **Reliability:** Discovered and diagnosed three compounding bugs causing ~20% of mainnet snap syncs to crash on pivot update (peer rotation, weight function, BlockRangeUpdate filtering — see Issue #6474). Quick fix shipped as PR #6475; proper multi-bug fix tracked as #6474.
> - **Observability:** PR #6470 opened adding `admin_syncStatus` / `admin_peerScores` RPC endpoints, live `peer_top.py` TUI, Grafana dashboards, and monitor improvements. Tooling was used to produce the forensic analysis for the reliability work.
> - **Profiling-driven perf candidates:** Mainnet profiling of `insert_storages` revealed that **~80% of idle thread-seconds** come from small-account dispatcher overhead, not from the monster account. Two new issues opened:
>   - **#6476** small-account batching — amortize dispatcher overhead for the 26M <1ms trie builds
>   - **#6477** big-account parallelization in snap sync — analogous to PR #6410 but for large storage tries (realistic gain ~5-6%, smaller than initially thought)
> - **Pipelining context added** to Issue #4240 (Phase 3 spawned rewrite) with a concrete proposal: ~13% gain from phase overlap (Accounts → Storage + Bytecodes → Healing pipelined via actors).
> - **New roadmap items for reliability** (see §1.18 and §1.19 below).

---

## Table of Contents

1. [Current State Analysis](#current-state-analysis)
2. [Phase 1: Performance Optimization](#phase-1-performance-optimization)
3. [Phase 2: Code Quality & Maintainability](#phase-2-code-quality--maintainability)
4. [Phase 3: Pipeline Architecture](#phase-3-pipeline-architecture)
5. [Success Metrics](#success-metrics)
6. [Risk Assessment](#risk-assessment)
7. [Timeline](#timeline)
8. [Dependencies](#dependencies)

---

## Current State Analysis

### Module Structure

| File | Lines | Purpose |
|------|-------|---------|
| `snap/client.rs` | 1,401 | Client-side snap protocol requests |
| `sync/snap_sync.rs` | 1,181 | Main snap sync orchestration |
| `sync/healing/storage.rs` | 740 | Storage trie healing |
| `sync/healing/state.rs` | 463 | State trie healing |
| `sync/full.rs` | 297 | Full sync implementation |
| `sync.rs` | 290 | Module root: `Syncer`, `AccountStorageRoots`, `SyncError` |
| `snap/server.rs` | 166 | Server-side snap protocol responses |
| `snap/error.rs` | 147 | Unified error types |
| `snap/constants.rs` | 121 | Protocol constants |
| `sync/code_collector.rs` | 100 | Bytecode collection |
| **Total** | **~4,906** | |

### Snap Sync Phases

The snap sync process consists of 6 sequential phases:

```
┌──────────────────────────────────────────────────────────────────────────┐
│                          SNAP SYNC PIPELINE                              │
├──────────────────────────────────────────────────────────────────────────┤
│                                                                          │
│  1. Header Download ──► 2. Pivot Selection ──► 3. Account Range Download │
│                                                          │               │
│                                                          ▼               │
│  6. Full Sync ◄── 5. Bytecode Download ◄── 4. Storage Range Download    │
│       │                                                                  │
│       ▼                                                                  │
│  [State Healing & Storage Healing run in parallel with phases 4-5]       │
│                                                                          │
└──────────────────────────────────────────────────────────────────────────┘
```

### Current Performance Bottlenecks

Based on code analysis and mainnet profiling data (PR #6410; April 2026 profiling run `20260412_172457`):

| Bottleneck | Location | Impact | Priority |
|------------|----------|--------|----------|
| **Pivot update crashes** | `update_pivot` in `snap_sync.rs` | **~20% of mainnet runs crash** with `process::exit(2)` + DB corruption — requires full resync | **Critical (reliability)** |
| **Trie building in insertion** | `insert_accounts`, `insert_storage` | **75-91% of insertion time** (883s/1184s for accounts, 2357s/2587s for storage on mainnet) | **Critical** |
| **Small-account dispatcher overhead** | `insert_storages` dispatcher | **~80% of the 49% idle thread-seconds** (≈14,915 thread-seconds on a 2347s mainnet storage phase). 26.3M accounts × avg <1ms, dispatcher blocked 69% of wall on slot turnover | **Critical** (newly quantified) |
| **Large storage tries run single-threaded** | `insert_storages` per-account task | Monster (Uniswap-class, 159.6M leaves) runs 244.9s on 1 thread; ~20% of idle thread-seconds | **High** |
| Sequential header download | `sync_cycle_snap()` | Blocks state download start | Critical |
| Sequential phase pipeline | `snap_sync.rs` orchestration | Bytecodes/storage wait for all accounts to finish | High |
| Redundant code hash pass | `insert_accounts` | Extra full iteration over temp DB (addressed in #6410) | Medium |
| Trie node batching | `heal_state_trie()`, `heal_storage_trie()` | Writes are batched but could use `put_batch_no_alloc` | Medium |
| Busy-wait loops | Multiple locations | CPU waste (only when no peers available) | Medium |
| SST file intermediate step | Account/storage download | Overlapping key ranges force RocksDB merge during ingestion | Medium |
| Sync `std::fs` calls | Snapshot dumping | Already in `spawn_blocking`, but directory ops should use `tokio::fs` | Low |

### Existing Code Quality Issues

| Issue | Location | Description |
|-------|----------|-------------|
| `#[allow(clippy::too_many_arguments)]` | `heal_state_trie()`, `process_node_responses()` | 8+ parameters - needs context struct |
| Repeated code patterns | `snap/client.rs` | Snapshot dumping logic duplicated |
| Magic numbers | Various | Hardcoded values without constants |
| Missing documentation | Healing modules | Complex algorithms undocumented |
| Inconsistent error handling | Various | Mix of `?`, `.expect()`, silent drops |

---

## Phase 1: Performance Optimization

### Goal
Reduce snap sync time by 50% or more through parallelization, batching optimizations, and I/O improvements.

---

### 1.1 Parallel Header Download (PR #6059 - In Progress)

**Current State:** Headers are downloaded sequentially before state download begins.

**Proposed Change:** Download headers in a background task while state download proceeds in parallel.

**Implementation:**
- Add `header_receiver` channel to `SnapBlockSyncState`
- Spawn `download_headers_background()` task
- Process headers incrementally at strategic points
- Add early abort mechanism when switching to full sync

**Expected Impact:**
- State download starts immediately instead of waiting for millions of headers
- Estimated time savings: 10-20% of total sync time

**Status:** PR #6059 open, addressing review feedback

---

### 1.2 ~~Parallel Account Range Requests~~ (Discarded)

> Discarded — not needed after profiling.

---

### 1.3 Optimize Trie Node Batching

**Current State:**
- `NODE_BATCH_SIZE = 500` nodes per request
- `STORAGE_BATCH_SIZE = 300` accounts per batch
- DB writes use `put_batch()` (already batched)

**Proposed Changes:**

#### 1.3.1 Use `put_batch_no_alloc()` for Healing
```rust
// Current (sync/healing/state.rs:302, sync/healing/storage.rs:231)
// PERF: use put_batch_no_alloc (note that it needs to remove nodes too)

// Proposed: Pre-allocate buffers, reuse across batches
struct HealingBatchWriter {
    node_buffer: Vec<(Nibbles, Node)>,
    capacity: usize,
}
```

#### 1.3.2 Dynamic Batch Sizing (Needs Measurement)
Adjust batch sizes based on:
- Available memory
- Peer response latency
- Current healing progress

**Note:** Impact on healing duration needs empirical measurement — current batching may already be sufficient. Should only pursue if benchmarks show batch sizing is a bottleneck.

**Expected Impact:** Needs measurement

**Effort:** Medium (2 weeks)

---

### 1.4 Reduce Busy-Wait Loops (Issue #6140 — Step 9)

**Current State:** Multiple locations use `try_recv()` + `tokio::time::sleep()` in loops:
- `request_account_range()` (`snap/client.rs:193`): 10ms sleep waiting for peers
- `request_bytecodes()` (`snap/client.rs:383`): 10ms sleep waiting for peers
- `request_storage_ranges()` (`snap/client.rs:646`): 10ms sleep waiting for peers
- `heal_state_trie()` (`sync/healing/state.rs:151`): `try_recv` polling
- `heal_storage_trie()` (`sync/healing/storage.rs:261`): `try_recv` polling

Note: busy-waits only trigger when no peers are available.

**Proposed Change:** Replace with proper async primitives:

```rust
// Current (snap_sync.rs ~line 452)
loop {
    if let Some(headers) = block_sync_state.header_receiver.try_recv() { ... }
    tokio::time::sleep(Duration::from_millis(100)).await;
}

// Proposed: Blocking receive with timeout
match tokio::time::timeout(
    Duration::from_secs(30),
    block_sync_state.header_receiver.recv()
).await {
    Ok(Some(headers)) => { ... }
    Ok(None) => break, // Channel closed
    Err(_) => continue, // Timeout, check staleness
}
```

**Expected Impact:** Reduced CPU usage, faster response to events

**Effort:** Low (1 week)

---

### 1.5 ~~Memory-Bounded Structures~~ (Discarded)

> Discarded — not a real bottleneck in practice.

---

### 1.6 Use `tokio::fs` for Directory Operations

**Current State:** Snapshot dumping is already inside `spawn_blocking`. However, directory creation and existence checks still use synchronous `std::fs` calls.

**Proposed Change:** Replace `std::fs` directory operations with `tokio::fs`:

```rust
// Current
std::fs::create_dir_all(dir)?;

// Proposed
tokio::fs::create_dir_all(dir).await?;
```

**Expected Impact:** Minor — main I/O is already non-blocking via `spawn_blocking`.

**Effort:** Low (< 1 week)

---

### 1.7 Peer Connection Optimization

**Current State:**
- `PEER_REPLY_TIMEOUT = 15 seconds`
- `MAX_IN_FLIGHT_REQUESTS = 77`
- No adaptive timeout based on peer performance

**Proposed Changes:**

#### 1.7.1 Adaptive Timeouts
```rust
struct AdaptivePeerConfig {
    base_timeout: Duration,
    peer_latencies: HashMap<H256, RollingAverage>,

    fn timeout_for_peer(&self, peer_id: &H256) -> Duration {
        self.peer_latencies
            .get(peer_id)
            .map(|avg| avg.mean() * 3.0) // 3x average latency
            .unwrap_or(self.base_timeout)
    }
}
```

#### 1.7.2 Request Pipelining
Increase in-flight requests for high-quality peers:
```rust
fn max_requests_for_peer(&self, peer_id: &H256) -> u32 {
    match self.peer_quality(peer_id) {
        PeerQuality::Excellent => 100,
        PeerQuality::Good => 77,
        PeerQuality::Average => 50,
        PeerQuality::Poor => 20,
    }
}
```

**Expected Impact:** 20-30% improvement in peer utilization (needs empirical measurement)

**Note:** Could introduce excessive complexity for marginal gain. Should only pursue if benchmarks show peer utilization is actually a bottleneck.

**Effort:** Medium (2 weeks)

---

### 1.8 ~~Parallel Storage Healing~~ (Discarded)

> Discarded — storage healing is already parallelized via `JoinSet` with up to `MAX_IN_FLIGHT_REQUESTS` (77) concurrent requests.

### 1.9 ~~Bytes for Trie Values — O(1) Clones~~ (Discarded)

> Discarded — PR #6057 closed without merging.

### 1.10 Snap Sync Benchmark Tool (PR #6108 - In Progress)

Python tool (`tooling/sync/sync_benchmark.py`) to analyze snap sync performance from container logs, identifying bottlenecks per phase.

### 1.11 Per-Phase Timing Breakdown in Slack Notifications (✅ DONE — Merged in #6136)

Surfaces per-phase completion timings (Block Headers, Account Ranges, Storage Ranges, Healing, etc.) directly in Slack notifications from the multisync monitoring script, so performance bottlenecks are visible at a glance.

---

### 1.12 Optimize Trie Building in Snap Sync Insertion (PR #6410 — In Progress)

**Current State:** Trie building is 75-91% of insertion time (profiled on mainnet). Account insertion took ~20 minutes and storage insertion ~43 minutes — together 61% of total snap sync time. The trie build is entirely CPU-bound (0% I/O wait).

**Changes:**
1. **Eliminate redundant code hash iteration pass** — original code iterated all accounts twice (once for code hashes, once for trie). Merged into single pass.
2. **Reuse `nodehash_buffer` across calls** — avoids ~700M allocations on mainnet.
3. **Parallel state trie building across 16 nibble ranges** — splits state trie into 16 independent sub-tries built concurrently.

**Benchmarks (mainnet, release profile, no validation):**

| Phase | Before | After | Delta |
|-------|--------|-------|-------|
| Account Insertion | 1184s (19m 44s) | 818s (13m 40s) | **-31%** |
| Storage Insertion | 2587s (43m 7s) | 2433s (40m 30s) | **-6%** |

| Network | Before | After | Saved |
|---------|--------|-------|-------|
| Hoodi | ~25m | ~13m | ~12m |
| Sepolia | ~90m | ~43m | ~47m |
| Mainnet | ~1h 42m | ~1h 35m | ~7m |

**Note:** This optimization is orthogonal to the concurrency model — it operates at the trie-building level, below the orchestration layer. No dependency on actor migration.

**Status:** PR #6410 open, benchmarked on mainnet

---

### 1.13 Pipeline Bytecode Downloads & Background Storage Healing (PR #6184 — In Progress)

**Current State:** Bytecodes wait for all healing; storage healing must reach 100% before finalization.

**Changes:**
1. **Concurrent bytecode downloads** — stream code hashes via `mpsc` channel to a concurrent download task running alongside healing. Content-addressed (hash = key), safe regardless of pivot changes.
2. **Background storage healing with 99% threshold** — state healing runs to 100%, but storage healing finalizes at 99% and completes the remaining <1% in a background task after finalization.

**Benchmarks (Hoodi):** -13% total sync time (489s vs 564s baseline).

**Note:** Implements Phase 3 pipelining goals (3.3, 3.4) incrementally using `mpsc` channels, without requiring a full actor migration. Validates that the orchestration can be improved without restructuring to actors first.

**Status:** PR #6184 open

---

### 1.14 Eliminate SST File Intermediate Step (PR #6177 — In Progress)

Replace SST file writer + ingest flow with direct `WriteBatch` writes to temp RocksDB during download phase. Removes overlapping key range merge overhead. Also merges the two iterator passes in `insert_accounts` into a single pass.

**Status:** PR #6177 open, needs mainnet testing

---

### 1.15 Optimize Insertion and Healing Write Paths (PR #6159 — In Progress)

Addresses multiple hot-path bottlenecks found via profiling:
- Single-element `Vec` alloc per key in `put_batch` (~20k allocs per flush)
- `put_batch_no_alloc` actually allocates (encodes all nodes, collects into Vec)
- BTreeMap overwrites in healing batch writes
- Hardcoded 12 threads for trie building

**Status:** PR #6159 open

---

### 1.16 Disable WAL and Improve Concurrency (PR #6178 — In Progress)

Disable write-ahead log for snap sync temp DBs (crash recovery not needed) and tune RocksDB concurrency settings.

**Status:** PR #6178 open

---

### 1.17 Fill All Peer Slots per Tick in Healing Dispatch (PR #6175 — In Progress)

Current healing dispatch only fills one peer slot per loop iteration. Fill all available slots per tick to maximize network utilization.

**Status:** PR #6175 open

---

### 1.18 Snap Sync Observability Tooling (PR #6470 — In Progress, added 2026-04-15)

**Current State:** Diagnosing snap sync failures required manual log grep and docker inspection. No runtime visibility into peer scores, sync phase, or request distribution.

**Changes:**
- `admin_syncStatus` RPC endpoint — live sync phase, pivot block, progress metrics, error history
- `admin_peerScores` RPC endpoint — per-peer scores, inflight request counts, capabilities, last BlockRangeUpdate, eligibility
- `admin_setLogLevel` RPC — dynamically raise to TRACE during incidents
- `tooling/sync/peer_top.py` — live TUI showing peer scores, request distribution, and selection patterns in real time
- Grafana dashboard panels: sync progress, peer scoring distribution, request rates, per-phase rate overview
- Docker monitor improvements: rolling snapshots, degradation detection, 5s polling, force-dump on failure
- Header-download diagnostics logging in `snap_sync.rs`

**Impact:** No direct sync-time impact. Enables diagnosis of every other reliability/perf issue. The forensic analysis for §1.19 (pivot-update crashes) was only possible because of this tooling.

**Status:** PR #6470 open

---

### 1.19 Pivot Update Reliability (PRs/Issues #6475, #6474 — added 2026-04-15)

**Current State:** `update_pivot` crashes the node (`process::exit(2)`) on ~20% of mainnet sync runs. The exit leaves the DB in an inconsistent state (`Unknown state found in DB`), requiring a full `removedb` and resync from scratch.

**Root causes** (full forensics in Issue #6474):

- **Bug A — `update_pivot` classified as irrecoverable.** Commit `583795955` changed the retry loop from infinite to `MAX_TOTAL_FAILURES=15` with exponential backoff. 15 failures exhaust in ~2-3 min, then `PeerHandlerError::BlockHeaders` → classified irrecoverable → `process::exit(2)`.
- **Bug B — Deterministic peer selection never rotates.** `get_best_peer` + `.max_by_key(weight_peer)` always returns the same top-scored peer. The weight function `score - inflight_requests` systematically prefers idle/incapable peers (e.g., eth/70-only erigon with 0 snap requests, score 47) over busy healthy peers (Geth with 47 snap requests, weight 3).
- **Bug C — `BlockRangeUpdate.range_to` unused in selection.** Peers advertise their chain tip; we store it but don't filter by it. On hoodi we kept asking a peer for pivot block 2593928 while its last BlockRangeUpdate said range_to=2593886 (42 blocks behind).

**Evidence:** Hoodi failure `run_20260411_033943` — only 2 distinct peers tried across 9 attempts; at least 6 other peers had the block. Mainnet failure `run_20260414_011127` — stuck on erigon/v3.5.0-dev with weight 47 while 11 healthy peers had weight 3.

**Quick fix (PR #6475 — shipped):**
- Reclassify `PeerHandler`/`NoBlockHeaders` errors as recoverable (narrow per-variant via `PeerHandlerError::is_recoverable()`)
- Add `get_best_peer_excluding(caps, excluded)` — rotation-aware peer selection
- Replace `MAX_TOTAL_FAILURES=15` with `MAX_ROTATIONS=5` (scales with peer count)
- Catch recoverable errors inside retry loop so protocol errors advance rotation

**Proper fix (Issue #6474 — deferred):** tackle the deeper peer-selection bugs:
1. Fix `weight_peer` for control-plane requests (pivot update, header resolution) — don't penalize data-plane inflight
2. Filter peers by `BlockRangeUpdate.range_to` before selection
3. Broaden rotation across all eligible peers
4. Don't count passive waits as failures
5. Revert irrecoverable classification of post-pivot-header fetch failures
6. Raise/remove `MAX_TOTAL_FAILURES`
7. Cherry-pick fixes from `fullsync-acceleration` branches (880244afe, efaa344d4)
8. DB cleanup / graceful shutdown on sync failure

**Impact:** Removes a reliability failure that was ~20% of mainnet runs. Currently masks optimization progress (slower runs → more pivot updates → more exposure to the bug).

**Status:** PR #6475 open (AI agent feedback addressed 2026-04-15); Issue #6474 open as follow-up.

---

### 1.20 Within-Trie Parallelization for Large Storage Tries (Issue #6477 — added 2026-04-15)

**Current State:** `insert_storages` is parallel across accounts (16 worker threads, one account per task), but each individual account's trie is built single-threaded. For large storage tries (Uniswap-class, 159M+ leaves), this means a single thread runs for ~245s while the monster is processed.

**Proposed Change:** Apply `trie_from_sorted_parallel` (from PR #6410) *inside* each large storage task, splitting the trie build across 16 storage-slot-nibble ranges.

**Distinction from #5482:** Issue #5482 (existing, open) addresses the same idea but for *block execution* — parallelizing per-tx storage updates during state-root computation. This issue (#6477) is the analogous change for snap sync's initial trie construction.

**Expected Impact:** ~100-150s saved (~5-6% of storage phase). Smaller than originally projected because the other 15 threads aren't actually idle during the monster's solo run (they're working on the long tail).

**Status:** Issue #6477 open

---

### 1.21 Small-Account Batching in insert_storages (Issue #6476 — added 2026-04-15)

**Current State:** `insert_storages` profiling shows only 8.1 of 16 threads used on average (**49% of thread-seconds idle**, 18,589 / 37,554). Decomposition:
- Monster account serialization: ~20% of idle time
- **Small-account dispatcher overhead: ~80% of idle time** — 26.3M accounts with avg <1ms trie build each, dispatcher blocked 69% of wall on slot turnover

**Proposed Change:** Bundle N small accounts per worker job to amortize send/reap/slot-free overhead. Large accounts still run as single tasks.

**Expected Impact:** Potentially 5-15 min off the 39-min storage phase (30-40%). This is the dominant parallelism killer and **higher-value than #6477** (big-account).

**Architectural dependencies:** None. Change is inside the existing dispatcher loop — independent of the spawned migration (Phase 3) and of #6477 (complementary, additive).

**Status:** Issue #6476 open

---

### 1.22 Decoded `TrieLayerCache` (PR #6348 — In Progress, added to roadmap 2026-04-15)

**Current State:** `TrieLayerCache` hits still go through `Node::decode()` even when the node was just cached — decoded representation is discarded and re-derived on every access.

**Proposed Change:** Cache the decoded `Node` value alongside the encoded bytes. Skip `Node::decode()` on cache hits.

**Expected Impact:** TBD — needs benchmark. Hot path in trie traversal; decode is not free.

**Status:** PR #6348 open (author: Arkenan)

---

### 1.23 Bloom Filter for Non-Existent Storage Slots (PR #6288 — In Progress, added to roadmap 2026-04-15)

**Current State:** Storage trie seeks are issued for every slot read, including for accounts that have no storage or slots that don't exist. On large contracts, many lookups miss.

**Proposed Change:** Add a bloom filter to skip trie seeks for slots known not to exist.

**Expected Impact:** TBD — needs benchmark. Could help both sync-time trie seeks and runtime reads.

**Status:** PR #6288 open (author: ilitteri)

---

### 1.24 Adaptive Request Sizing & Storage Bisection (PR #6181 — In Progress, added to roadmap 2026-04-15)

**Current State:** Storage range requests use fixed size per request. Adaptive sizing based on peer response history and bisection on oversized responses could improve throughput.

**Proposed Change:** Adaptive request sizing + storage bisection on oversized responses + parallel trie construction for storage.

**Status:** PR #6181 open (author: ilitteri)

---

### 1.25 Concurrent Bytecode + Storage Download (PR #6205 — In Progress, added to roadmap 2026-04-15)

**Current State:** Bytecodes download as a distinct phase after storage. Pipeline opportunity similar to what PR #6184 did for bytecodes + healing.

**Proposed Change:** Run bytecode downloads concurrently with storage downloads.

**Note:** May overlap partially with PR #6184 (which concurrently runs bytecodes with *healing*). Needs review to reconcile.

**Status:** PR #6205 open (author: ilitteri)

---

### 1.26 Phase Completion Markers for Validation (PR #6189 — In Progress, added to roadmap 2026-04-15)

**Current State:** No persisted markers for phase completion; recovery and validation tooling has to infer progress.

**Proposed Change:** Add phase completion markers to the snap sync validation flow.

**Status:** PR #6189 open (author: ilitteri)

---

## Phase 2: Code Quality & Maintainability

### Goal
Make the codebase clear, well-documented, and easy for new contributors to understand.

---

### 2.1 Extract Context Structs (Issue #6140 — Steps 5, 6)

**Also includes:** `AccountStorageRoots` simplification (defined in `sync.rs:167`) — replace `BTreeMap<H256, (Option<H256>, Vec<(H256, H256)>)>` with SoA approach and named struct instead of tuple. Named structs for channel types in `sync/healing/state.rs` and worker return types in `snap/client.rs`.

**Current State:** Functions with many parameters:
```rust
#[allow(clippy::too_many_arguments)]
async fn heal_state_trie(
    state_root: H256,
    store: Store,
    mut peers: PeerHandler,
    staleness_timestamp: u64,
    global_leafs_healed: &mut u64,
    mut healing_queue: StateHealingQueue,
    storage_accounts: &mut AccountStorageRoots,
    code_hash_collector: &mut CodeHashCollector,
) -> Result<bool, SyncError>
```

**Proposed Change:**
```rust
struct StateHealingContext {
    state_root: H256,
    store: Store,
    staleness_timestamp: u64,
}

struct StateHealingProgress {
    global_leafs_healed: u64,
    healing_queue: StateHealingQueue,
    storage_accounts: AccountStorageRoots,
    code_hash_collector: CodeHashCollector,
}

async fn heal_state_trie(
    ctx: &StateHealingContext,
    peers: &mut PeerHandler,
    progress: &mut StateHealingProgress,
) -> Result<bool, SyncError>
```

**Files Affected:**
- `sync/healing/state.rs` (`#[allow(clippy::too_many_arguments)]` at line 76)
- `sync/healing/storage.rs` (`#[allow(clippy::too_many_arguments)]` at line 519)
- `sync/snap_sync.rs`
- `sync.rs` (`AccountStorageRoots` at line 167)

**Effort:** Low (1 week)

---

### 2.2 Comprehensive Documentation

**Current State:** Module documentation is sparse; healing algorithms are complex and undocumented.

**Proposed Documentation:**

#### 2.2.1 Architecture Documentation
Create `docs/snap_sync_architecture.md`:
- High-level overview with diagrams
- Data flow through the system
- State machine for sync phases
- Interaction with storage layer

#### 2.2.2 Algorithm Documentation
Document healing algorithms inline:
```rust
/// # State Trie Healing Algorithm
///
/// The healing process fixes inconsistencies in the state trie that occur
/// when snap sync spans multiple pivot blocks.
///
/// ## Algorithm
///
/// 1. Start from the state root node
/// 2. For each node, check if all children exist in local storage
/// 3. For missing children:
///    a. Add to download queue
///    b. Request from peers in batches of NODE_BATCH_SIZE
/// 4. When a node's children are all present, flush to DB
/// 5. Repeat until no missing nodes remain
///
/// ## Invariants
///
/// - Parent nodes are only flushed after all children are present
/// - The healing queue tracks `missing_children_count` per node
/// - Staleness checks prevent infinite loops on changing state
///
/// ## Complexity
///
/// - Time: O(n) where n is the number of trie nodes
/// - Space: O(d * b) where d is max depth and b is branching factor
```

#### 2.2.3 Inline Code Comments
Add comments explaining non-obvious logic, especially:
- Hash boundary calculations
- Pivot staleness detection
- Proof verification

**Effort:** Medium (2 weeks)

---

### 2.3 Consolidate Error Handling (Merged — PR #5975)

---

### 2.4 Extract Helper Functions (Issue #6140 — Steps 3, 4)

**Current State:** Duplicated patterns identified:
- Snapshot dumping (4 occurrences) - **Partially addressed**
- Peer selection and retry logic (6+ occurrences)
- Progress reporting (5+ occurrences)

**Proposed Change:** Create shared utilities:

```rust
// snap/utils.rs (new file)

/// Retries an operation with exponential backoff
pub async fn with_retry<T, E, F, Fut>(
    max_attempts: u32,
    operation: F,
) -> Result<T, E>
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<T, E>>,
{ ... }

/// Reports progress at regular intervals
pub struct ProgressReporter {
    interval: Duration,
    last_report: Instant,
    metrics_key: &'static str,
}

impl ProgressReporter {
    pub fn maybe_report(&mut self, current: u64, total: u64) { ... }
}
```

**Effort:** Low (1 week)

---

### 2.5 State Machine Refactor (Consider deferring — subsumed by 3.1)

> **Note:** This would be subsumed by the actor migration (3.1), where each actor naturally represents a phase with explicit state. Consider skipping this as a standalone item if 3.1 is planned soon.

**Current State:** Snap sync phases are implicit in control flow.

**Proposed Change:** Make phases explicit:

```rust
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SnapSyncPhase {
    Initializing,
    DownloadingHeaders,
    DownloadingAccounts,
    DownloadingStorages,
    DownloadingBytecodes,
    HealingState,
    HealingStorage,
    FullSync,
    Complete,
}

pub struct SnapSyncStateMachine {
    phase: SnapSyncPhase,
    progress: SnapSyncProgress,

    pub fn transition(&mut self, event: SnapSyncEvent) -> Result<(), SyncError> {
        match (self.phase, event) {
            (DownloadingHeaders, HeadersComplete) => {
                self.phase = DownloadingAccounts;
                Ok(())
            }
            // ... other transitions
            _ => Err(SyncError::InvalidStateTransition),
        }
    }
}
```

**Benefits:**
- Clear phase boundaries
- Easier to add new phases
- Better logging and metrics
- Simpler testing

**Effort:** High (3 weeks)

---

### 2.6 Test Coverage Improvement

**Current State:**
- 12 snap server tests
- Limited client-side testing
- No integration tests for full sync cycle

**Proposed Testing Strategy:**

#### 2.6.1 Unit Tests
- Test each healing algorithm with mock data
- Test pivot selection logic
- Test proof verification

#### 2.6.2 Integration Tests
- Mock peer network
- Test full sync cycle with small state
- Test pivot updates mid-sync
- Test recovery from interrupted sync

#### 2.6.3 Property-Based Tests
- Random account ranges
- Random trie structures
- Fuzz testing for proof verification

**Target Coverage:** 80%+ for core modules

**Effort:** High (4 weeks)

---

### 2.7 Configuration Externalization

**Current State:** Constants are hardcoded in `snap/constants.rs`.

**Proposed Change:** Make tunable parameters configurable:

```rust
// config/snap_sync.rs
#[derive(Debug, Clone, Deserialize)]
pub struct SnapSyncConfig {
    /// Maximum response size in bytes (default: 512KB)
    pub max_response_bytes: u64,

    /// Number of accounts per request (default: 128)
    pub snap_limit: usize,

    /// In-memory buffer size before disk flush (default: 64MB)
    pub range_file_chunk_size: usize,

    /// Maximum concurrent in-flight requests (default: 77)
    pub max_in_flight_requests: u32,

    // ... other parameters
}

impl Default for SnapSyncConfig {
    fn default() -> Self {
        Self {
            max_response_bytes: 512 * 1024,
            snap_limit: 128,
            // ... defaults from current constants
        }
    }
}
```

**Benefits:**
- Tuning without recompilation
- Environment-specific configurations
- Easier benchmarking

**Effort:** Medium (2 weeks)

---

### 2.8 Fix Correctness Bugs in `request_storage_ranges` (Issue #6140 — Steps 1, 2)

**Current State:** One location crashes the node on a recoverable error:
- `panic!("Should have found the account hash")` (`snap/client.rs:729`)

Other `.expect()` calls (`snap/client.rs:630-631`) are on `JoinSet` results, not store lookups.

**Proposed Change:** Replace with proper error propagation using `SnapError::InternalError` and `?` operator.

**Effort:** Very low (< 1 day)

---

### 2.9 Fix Snap Protocol Capability Bug — ✅ DONE

**Status:** Merged in #5975. All `get_best_peer()` calls now use `SUPPORTED_SNAP_CAPABILITIES`.

---

### 2.10 Add `spawn_blocking` to Bytecodes Handler — ✅ DONE

**Status:** Merged in #5975. Bytecodes handler in `snap/server.rs` uses `spawn_blocking`, matching the pattern of all other handlers.

---

### 2.11 Remove Dead `DumpError.contents` Field — ✅ DONE

**Status:** Merged in #5975. `DumpError` in `snap/error.rs` no longer has the `contents` field and uses `#[derive(Debug, thiserror::Error)]` instead of a custom `Debug` impl.

---

### 2.12 Use `JoinSet` Instead of Channels for Workers (Consider deferring — subsumed by 3.1)

> **Note:** The actor migration (3.1) would replace both channels and JoinSets with actor messages. Consider skipping this intermediate step if 3.1 is planned soon.

**Current State:** Both `request_account_range` (`snap/client.rs:138`) and `request_storage_ranges` (`snap/client.rs:587`) use `mpsc::channel` for worker communication. If a worker panics, the message is lost silently and the main loop may hang waiting for results.

**Proposed Change:** Migrate to `tokio::task::JoinSet` which propagates panics and handles task lifecycle. The bytecodes path already uses `JoinSet` as a reference.

**Effort:** Medium (1-2 weeks)

---

### 2.13 Self-Contained `StorageTask` with Hashes

**Current State:** `StorageTask` in `snap/client.rs:75` references `accounts_by_root_hash` by index (`start_index`, `end_index`). Any mutation of the vector would silently corrupt in-flight tasks. The task is not self-contained.

**Proposed Change:** Include actual account hashes and storage roots in `StorageTask` instead of indices. This makes tasks self-contained and eliminates the implicit coupling to the vector.

**Effort:** Medium (1 week)

---

### 2.14 Move Snap Client Methods Off `PeerHandler` — ✅ DONE

**Status:** Merged in #5975. Snap client methods extracted from `peer_handler.rs` to `snap/client.rs` as standalone functions taking `peers: &mut PeerHandler`.

---

### 2.15 Guard `write_set` in Account Path

**Current State:** `request_account_range` (`snap/client.rs:170`) spawns disk-write tasks without checking if one is already pending. The storage path (`snap/client.rs:625`) already does `!disk_joinset.is_empty()` check. Missing the guard can lead to multiple concurrent writes.

**Proposed Change:** Add the same `!disk_joinset.is_empty()` guard to the account range disk write path, matching the storage path pattern.

**Effort:** Very low (small)

---

### 2.16 Healing Code Unification

**Current State:** `sync/healing/state.rs` (~463 lines) and `sync/healing/storage.rs` (~740 lines) implement the same trie healing algorithm. Differences: path representation (single vs double nibbles) and leaf type (accounts vs U256). Lots of duplicated logic.

**Proposed Change:** Extract a generic healing function parameterized by path and leaf type. Both modules would call into the shared implementation.

**Effort:** High (3+ weeks)

---

### 2.17 Use Existing Constants for Magic Numbers

**Current State:** Most magic numbers have been replaced with named constants:
- `STORAGE_BATCH_SIZE` (used at `snap/client.rs:567`)
- `HASH_MAX` (used at `snap/client.rs:821,856,1275`)
- `ACCOUNT_RANGE_CHUNK_COUNT` (used at `snap/client.rs:109`)

Remaining: channel capacity `1000` appears at lines 138, 370, 587 — could be a named constant.

**Effort:** Very low (trivial)

---

### 2.18 Storage Download Refactor via `StorageTrieTracker` (PR #6171 — In Progress, added 2026-04-15)

**Current State:** `AccountStorageRoots` tracks storage downloads per-account with complex index-based referencing into `accounts_by_root_hash`. Tasks reference accounts by index, results carry index ranges, big-account promotion mutates intervals — hard to follow and brittle.

**Proposed Change:** New `StorageTrieTracker` groups storage tries by root hash from the start, separating small (single-request) from big (multi-request) tries. Moves trie data into tasks and back in results, eliminating index-based coupling.

**Relation:** Complementary to #6140 (same file, orthogonal concerns). This is the data-ownership refactor; #6140 is the readability/correctness cleanup.

**Status:** PR #6171 open (author: fedacking)

---

### Issue #6140 — Refactor `request_storage_ranges` (Steps Summary)

9-step plan to refactor `request_storage_ranges` in `snap/client.rs`. Each step is one independently correct commit. Full details in [Issue #6140](https://github.com/lambdaclass/ethrex/issues/6140).

| Step | Description | Sections | Risk |
|------|-------------|----------|------|
| 1 | Replace `panic!` with proper error return | 2.8 | Very low |
| 2 | Replace `.expect()` with `?` operator | 2.8 | Very low |
| 3 | Extract `ensure_snapshot_dir` helper (4 occurrences) | 2.4 | Very low |
| 4 | Extract big-account chunking helper — DRY (~70 dup lines) | 2.4 | Low |
| 5 | Introduce `TaskTracker` struct for task counting | 2.1 | Very low |
| 6 | Extract result processing into `StorageDownloadState.process_result()` | 2.1 | Medium |
| 7 | Track buffer size incrementally (O(1) instead of O(n)) | — | Low |
| 8 | Remove `accounts_done` HashMap (inline removal) | — | Low |
| 9 | Replace busy-poll (`try_recv` + `sleep`) with `tokio::select!` | 1.4 | Medium |

**Dependencies:**
```
Steps 1, 2, 3, 5 — independent
Step 4 — independent
Step 6 — depends on Steps 4, 5
Steps 7, 8 — depend on Step 6
Step 9 — depends on Step 6
```

**Execution order:** 1 → 2 → 3 → 5 → 4 → 6 → 7 → 8 → 9

---

## Phase 3: Pipeline Architecture

### Goal
Replace the sequential sync phase model with a pipelined actor architecture using Spawned, enabling concurrent execution of phases that are currently blocked on each other.

### Context

The rest of the p2p layer already uses `spawned_concurrency` actors (`ActorRef`, `#[protocol]`, `#[actor]`) for peer table, discovery, RLPx connections, and tx broadcasting. The snap sync module is the main holdout — it uses raw `tokio::spawn` + `mpsc::channel` for concurrency.

**Update (April 2026):** The spawned 0.5.0 migration has been merged (PR #6295, March 31). All existing actors across ethrex now use the new `#[protocol]` + `#[actor]` macro API. The spawned framework blocker is resolved — Phase 3 can proceed whenever the team is ready.

Additionally, PR #6184 has demonstrated that key pipelining goals (concurrent bytecodes, background storage healing) can be achieved incrementally with `mpsc` channels, without a full actor rewrite. This validates an incremental approach: land targeted optimizations first, then migrate to actors for cleaner architecture.

### Motivation

Phases 1 and 2 optimize individual sync stages but don't challenge the fundamental sequential pipeline:

```
Headers → Pivot → Accounts → Storage → Bytecodes → Healing → Full Sync
```

In practice, once an account batch is downloaded, its storage and bytecodes are immediately known — there's no reason to wait for ALL accounts before starting storage/bytecode downloads. Similarly, headers can be fetched in the background while state download proceeds. The current code can't express this easily because each phase is a monolithic function that must complete before the next starts.

With actors, each phase becomes a message-driven pipeline stage:

```
                      ┌──────────────┐
                      │ HeaderActor  │  (background, feeds into FullSync later)
                      └──────────────┘

┌──────────────┐    ┌───────────────┐    ┌───────────────┐
│ AccountActor │───►│ StorageActor  │───►│ HealingActor  │
│              │───►│ BytecodeActor │    │               │
└──────────────┘    └───────────────┘    └───────────────┘
```

Each actor owns its own state, peer management, and timing — no shared mutable state or busy-wait coordination.

---

### 3.1 Migrate Snap Sync to Spawned Actors

**Current State:** `snap_sync.rs` orchestrates everything via sequential function calls. Workers are spawned with `tokio::spawn` and communicate via `mpsc::channel`. Peer acquisition uses `try_recv` + sleep busy-wait loops.

**Proposed Change:** Define a `#[protocol]` for each sync stage and implement them as `Actor`s:

```rust
#[protocol]
pub trait AccountDownloaderProtocol: Send + Sync {
    fn download_range(&self, start: H256, end: H256) -> Result<(), ActorError>;
    fn account_batch_ready(&self, accounts: Vec<AccountRangeUnit>) -> Result<(), ActorError>;
}

#[actor(protocol = AccountDownloaderProtocol)]
impl AccountDownloader {
    #[started]
    async fn started(&mut self, ctx: &Context<Self>) { ... }

    async fn handle_download_range(&mut self, start: H256, end: H256) { ... }
}
```

**Actors:**
- `HeaderActor` — downloads headers in background, notifies orchestrator on completion
- `AccountActor` — downloads account ranges, sends each batch downstream immediately
- `StorageActor` — receives account batches, starts storage downloads per-batch
- `BytecodeActor` — receives code hashes from account batches, downloads in parallel
- `HealingActor` — starts healing as soon as enough state is available
- `SyncOrchestrator` — coordinates state machine transitions and pivot updates

**Subsumes:** 1.1 (parallel headers), 2.12 (JoinSet → actors), 2.5 (state machine refactor)

**Effort:** High (6+ weeks)

---

### 3.2 Peer Scoring

**Current State:** `get_best_peer()` selects peers by capability match and in-flight request count. There's no tracking of peer throughput, latency, or reliability beyond simple success/failure scores.

**Proposed Change:** Extend `PeerTable` with richer metrics per peer:
- Rolling average response latency
- Throughput (bytes/second)
- Reliability score (success rate over recent window)

Actors request peers from `PeerTable` with requirements (e.g., "need a peer for storage range, prefer high-throughput"), and `PeerTable` returns the best match.

**Subsumes:** 1.7 (adaptive timeouts, request pipelining)

**Effort:** Medium (2-3 weeks)

---

### 3.3 Pipelined Account → Storage Download

**Current State:** Storage download starts only after ALL accounts are downloaded. In `snap_sync.rs`, `request_account_range()` must complete before `request_storage_ranges()` is called.

**Proposed Change:** `AccountActor` sends each completed account batch to `StorageActor` immediately via actor messages. `StorageActor` starts downloading storage for those accounts while more accounts are still being fetched.

This requires `StorageActor` to handle dynamically growing task queues (new account batches arrive while existing ones are being processed).

**Expected Impact:** Significant — storage is one of the longest phases and can start much earlier.

**Note:** PR #6184 partially addresses this with background storage healing (99% threshold + background task), but doesn't pipeline the download phase itself. The actor-based approach would go further.

**Effort:** Medium (2-3 weeks, depends on 3.1)

---

### 3.4 Pipelined Bytecode Download

**Current State:** Bytecodes are downloaded in phase 5, after all storage ranges. Code hashes are known as soon as accounts are downloaded (they're part of the account state).

**Proposed Change:** `AccountActor` sends code hashes to `BytecodeActor` immediately as accounts arrive. `BytecodeActor` downloads bytecodes in parallel with storage downloads.

**Expected Impact:** Removes bytecode download as a sequential bottleneck — it becomes fully overlapped with storage.

**Note:** PR #6184 already implements concurrent bytecode downloads via `mpsc` channel (without actors). If #6184 merges first, the actor migration would formalize the existing pattern.

**Effort:** Low (1-2 weeks, depends on 3.1)

---

### 3.5 Compute FKV on Insertion

**Current State:** The flat key-value (FKV) store is populated by a background generator (`flatkeyvalue_generator` in `store.rs`) that runs after sync, iterating the trie to build denormalized lookup entries. This adds post-sync latency before the node is fully operational.

**Proposed Change:** Compute and insert FKV entries as account/storage data arrives during snap sync, eliminating the post-sync generation step. Each actor writes FKV entries alongside trie nodes in the same batch.

**Expected Impact:** Eliminates FKV generation as a post-sync step. Node becomes operational immediately after sync completes.

**Effort:** Medium (2-3 weeks)

---

### Phase 3 Dependency Graph

```
3.1 (Actor migration)
 ├── 3.2 (Peer scoring)     — independent, can start in parallel
 ├── 3.3 (Pipelined storage) — depends on 3.1
 ├── 3.4 (Pipelined bytecodes) — depends on 3.1
 └── 3.5 (FKV on insertion)  — independent of 3.1, can start earlier
```

---

## Success Metrics

### Phase 1: Performance

| Metric | Current | Target | Measurement Method |
|--------|---------|--------|-------------------|
| Mainnet full sync time | TBD | -50% | End-to-end benchmark |
| Account download rate | TBD | 2x | Accounts/second metric |
| Storage healing time | TBD | -60% | Phase duration metric |
| Peak memory usage | TBD | -30% | Process monitoring |
| CPU utilization during sync | TBD | >80% | Process monitoring |

### Phase 2: Code Quality

| Metric | Current | Target | Measurement Method |
|--------|---------|--------|-------------------|
| Test coverage | ~20% | >80% | `cargo tarpaulin` |
| Clippy warnings | 0 | 0 | CI enforcement |
| Documentation coverage | ~30% | >90% | `cargo doc` coverage |
| Cyclomatic complexity | TBD | <15 per function | `cargo clippy` |
| Functions >100 lines | TBD | 0 | Custom lint |

### Phase 3: Pipeline Architecture

| Metric | Current | Target | Measurement Method |
|--------|---------|--------|-------------------|
| Phase overlap | 0% (sequential) | >50% | Phase timing breakdown |
| Time from first account to first storage download | Full account phase | <30s | Per-phase logs |
| Post-sync FKV generation time | TBD | 0 (eliminated) | End-to-end benchmark |
| Total sync time improvement over Phase 1 | Baseline | -30% additional | End-to-end benchmark |

---

## Risk Assessment

| Risk | Probability | Impact | Mitigation |
|------|-------------|--------|------------|
| Performance regression | Medium | High | Comprehensive benchmarking before/after each change |
| Data corruption during sync | Low | Critical | Extensive integration testing; checksums; recovery mechanisms |
| Breaking changes to peer protocol | Low | Medium | Hive test suite validation |
| Increased complexity | Medium | Medium | Code review; documentation requirements |
| Schedule overrun | Medium | Medium | Prioritize high-impact items; iterative delivery |
| ~~Spawned refactor not ready~~ | ~~Medium~~ | ~~High~~ | ✅ Resolved — spawned 0.5.0 merged in #6295 (March 31, 2026) |
| Pipeline ordering bugs | Medium | High | Pipelining introduces data races if actors process out of order; needs careful invariant tracking |
| **Pivot-update crash masks optimization progress** | **High** | **High** | Ship §1.19 quick fix (#6475) before investing in further perf work — otherwise measurements are noisy and users hit the crash before seeing gains. Identified 2026-04-15. |
| **DB corruption requires full resync on any crash** | **High** | **High** | `process::exit(2)` paths leave inconsistent state. Addressed in §1.19 proper fix (#6474 item 8). Until fixed, every sync crash costs a full resync. |

---

## Timeline

### Recommended execution order (updated 2026-04-15)

Two key insights drive priorities:
- **Trie building dominates insertion time** (75-91% from 1.12 profiling)
- **Small-account dispatcher overhead accounts for ~80% of idle thread-seconds** in `insert_storages` (new finding from #6476 profiling)

Recommended priority:

1. **Unblock users first** — ship pivot-update crash fix (#6475) so ~20% of mainnet runs stop crashing. **This is the highest-priority item right now** — every other perf win is masked by the crash.
2. **Land observability tooling** (1.18, #6470) — enables measuring all other changes
3. **Land trie building optimizations** (1.12, #6410) — largest single improvement, orthogonal to concurrency model
4. **Land write path optimizations** (1.14, 1.15, 1.16) — compound on trie improvements
5. **Explore small-account batching** (1.21, #6476) — potentially 30-40% of storage phase (largest unexploited opportunity)
6. **Land pipelining** (1.13, #6184) — concurrent bytecodes + background healing
7. **Big-account within-trie parallelization** (1.20, #6477) — ~5-6%, complementary to #6476
8. **Phase 2 quick wins** (2.8, 2.17, 2.15, 2.1, 2.4) — low-effort correctness/quality
9. **Proper pivot-update fix** (#6474) — deeper peer selection bugs, tackle after quick fix stabilizes
10. **Actor migration** (3.1) — clean architecture, now unblocked, subsumes 2.5/2.12
11. **Remaining Phase 2/3 items** — documentation, testing, peer scoring

### Phase 1: Performance

```
Priority 0 (CRITICAL — user-facing reliability):
  1.19  Pivot update reliability quick fix (PR #6475) — stops ~20% crash rate

Priority 1 (high impact, ready now):
  1.18  Snap sync observability tooling (PR #6470) — prereq for measuring others
  1.12  Optimize trie building (PR #6410) — -31% account insertion
  1.15  Optimize insertion/healing write paths (PR #6159)
  1.14  Eliminate SST file intermediate step (PR #6177)
  1.13  Pipeline bytecodes + background healing (PR #6184) — -13% total
  1.21  Small-account batching (Issue #6476) — potentially 30-40% of storage phase

Priority 2 (medium impact):
  1.16  Disable WAL and improve concurrency (PR #6178)
  1.17  Fill all peer slots per tick in healing (PR #6175)
  1.20  Big-account within-trie parallelization (Issue #6477) — ~5-6%
  1.24  Adaptive request sizing + storage bisection (PR #6181)
  1.1   Parallel Header Download (PR #6059)
  1.6   Async Disk I/O (PR #6113)
  1.4   Reduce Busy-Wait Loops (Issue #6140)

Lower priority / needs measurement:
  1.22  Decoded TrieLayerCache (PR #6348)
  1.23  Bloom filter for non-existent storage slots (PR #6288)
  1.25  Concurrent bytecode + storage (PR #6205) — may overlap with 1.13
  1.26  Phase completion markers (PR #6189)
  1.3   Optimize Trie Node Batching
  1.7   Peer Connection Optimization (PR #6117)
  1.10  Snap sync benchmark tool (PR #6108)

Follow-up / deferred:
  1.19b Pivot update proper fix (Issue #6474) — deeper peer selection bugs

Done:
  1.11  Per-phase timing breakdown (✅ Merged #6136)

Discarded:
  1.2   Parallel Account Range Requests
  1.5   Memory-Bounded Structures
  1.8   Parallel Storage Healing
  1.9   Bytes for Trie Values (PR #6057 closed)
```

### Phase 2: Code Quality

```
Quick wins (do alongside Phase 1):
  2.8   Fix Correctness Bugs (Issue #6140)
  2.17  Use existing constants for magic numbers
  2.15  Guard write_set in account path
  2.1   Extract Context Structs (Issue #6140)
  2.4   Extract Helper Functions (Issue #6140)
  2.13  Self-contained StorageTask with hashes

Defer until after actor migration (subsumed by 3.1):
  2.5   State Machine Refactor
  2.12  Use JoinSet for snap workers

Independent (do when bandwidth available):
  2.2   Comprehensive Documentation
  2.6   Test Coverage Improvement
  2.7   Configuration Externalization
  2.16  Healing Code Unification
  2.18  StorageTrieTracker refactor (PR #6171)

Done:
  2.3   Consolidate Error Handling (✅ Merged #5975)
  2.9   Fix snap protocol capability bug (✅ Merged #5975)
  2.10  Add spawn_blocking to bytecodes handler (✅ Merged #5975)
  2.11  Remove DumpError.contents dead field (✅ Merged #5975)
  2.14  Move snap client methods off PeerHandler (✅ Merged #5975)
```

### Phase 3: Pipeline Architecture

```
Unblocked (spawned 0.5.0 merged in #6295):
  3.1   Migrate snap sync to Spawned actors
  3.2   Peer scoring (independent of 3.1)
  3.5   Compute FKV on insertion (independent of 3.1)

After 3.1:
  3.3   Pipelined account → storage download
  3.4   Pipelined bytecode download (partially done by #6184)
```

**Note:** Spawned 0.5.0 has landed (#6295, March 31). Phase 3 is no longer blocked. However, Phase 1 performance wins should land first since they're orthogonal to the concurrency model and provide immediate measurable impact.

**Total Duration:** ~20+ weeks (all phases overlap)

---

## Dependencies

### External Dependencies

| Dependency | Version | Purpose |
|------------|---------|---------|
| tokio | 1.x | Async runtime |
| spawned-concurrency | 0.5.0 (✅ merged in #6295) | Actor framework (Phase 3) |
| rayon | 1.x | Parallel iterators |
| tracing | 0.1.x | Logging/metrics |

### Internal Dependencies

| Module | Dependency | Notes |
|--------|------------|-------|
| snap sync | ethrex-storage | Trie operations |
| snap sync | ethrex-trie | Merkle Patricia Trie |
| snap sync | peer_handler | Network layer |

### Infrastructure Dependencies

- Benchmarking environment with mainnet-like data
- CI pipeline for performance regression detection
- Test network (Sepolia/Holesky) access

---

## Appendix A: Reference Implementation Comparison

### geth Snap Sync
- Parallel header and state download
- Adaptive peer scoring
- In-memory trie caching

### reth Snap Sync
- Staged sync architecture
- Parallel range downloads
- Memory-mapped storage

### Key Takeaways
1. All major clients parallelize header and state download
2. Adaptive batching is common
3. Memory management is critical for mainnet scale

---

## Appendix B: Existing TODOs/FIXMEs

| Location | Issue | Priority |
|----------|-------|----------|
| `sync/healing/storage.rs:157` | Better data receiver design | Medium |
| `sync/healing/storage.rs:231` | Use `put_batch_no_alloc` | High |
| `sync/healing/storage.rs:299` | Store error handling (`.expect()`) | High |
| `sync/healing/storage.rs:377` | Add error handling | Medium |
| `sync/healing/state.rs:150` | Peer scoring for responses | Medium |
| `sync/healing/state.rs:195` | Optimize trie leaf reaching | Low |
| `sync/healing/state.rs:246` | Check errors for stale block detection | Medium |
| `sync/healing/state.rs:283` | Reuse buffers | Low |
| `sync/healing/state.rs:302` | Use `put_batch_no_alloc` | High |
| `sync/healing/state.rs:346` | Change tuple to struct | Low |
| `snap/client.rs:175` | Check error type and handle properly | Medium |
| `snap/client.rs:281` | Repeated code, consider refactoring | Medium |
| `snap/client.rs:565` | Stable sort for binary search | Low |
| `snap/client.rs:595` | Replace with removable structure | Medium |
| `snap/client.rs:808` | DRY — duplicated big-account logic | Medium |
| `snap/client.rs:976` | Unnecessary unzip/memory | Low |

---

## Appendix C: Glossary

| Term | Definition |
|------|------------|
| **Pivot** | The block whose state we're syncing; updated when stale |
| **Healing** | Process of fixing trie inconsistencies after multi-pivot sync |
| **Staleness** | When the pivot block is too old relative to chain head |
| **Account Range** | A contiguous range of accounts in the state trie |
| **Storage Range** | A contiguous range of storage slots for an account |
| **In-flight Request** | A request sent to a peer awaiting response |

---

*Document Version: 1.3*
*Last Updated: 2026-04-15*
