# Snap Sync Module Roadmap

**Author:** Pablo Deymonnaz
**Date:** February 2026
**Status:** Draft for Review

---

## Executive Summary

This roadmap outlines a strategic plan to improve the ethrex snap sync module in two phases:

1. **Phase 1: Performance Optimization** - Make snap sync as fast as possible
2. **Phase 2: Code Quality & Maintainability** - Make the code clear, readable, and easier to understand

The snap sync module currently comprises ~4,650 lines across 12 files. Our goal is to achieve sync times competitive with geth while maintaining code quality standards.

---

## Table of Contents

1. [Current State Analysis](#current-state-analysis)
2. [Phase 1: Performance Optimization](#phase-1-performance-optimization)
3. [Phase 2: Code Quality & Maintainability](#phase-2-code-quality--maintainability)
4. [Success Metrics](#success-metrics)
5. [Risk Assessment](#risk-assessment)
6. [Timeline](#timeline)
7. [Dependencies](#dependencies)

---

## Current State Analysis

### Module Structure

| File | Lines | Purpose |
|------|-------|---------|
| `sync/snap_sync.rs` | 1,139 | Main snap sync orchestration |
| `snap/client.rs` | 1,416 | Client-side snap protocol requests |
| `sync/healing/storage.rs` | 728 | Storage trie healing |
| `sync/healing/state.rs` | 460 | State trie healing |
| `sync/full.rs` | 297 | Full sync implementation |
| `snap/server.rs` | 173 | Server-side snap protocol responses |
| `snap/error.rs` | 158 | Unified error types |
| `snap/constants.rs` | 118 | Protocol constants |
| `sync/code_collector.rs` | 100 | Bytecode collection |
| Other modules | ~61 | Supporting code |
| **Total** | **~4,650** | |

### Snap Sync Phases

The snap sync process consists of 6 sequential phases:

```
┌─────────────────────────────────────────────────────────────────────────┐
│                         SNAP SYNC PIPELINE                               │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                          │
│  1. Header Download ──► 2. Pivot Selection ──► 3. Account Range Download │
│                                                          │               │
│                                                          ▼               │
│  6. Full Sync ◄── 5. Bytecode Download ◄── 4. Storage Range Download    │
│       │                                                                  │
│       ▼                                                                  │
│  [State Healing & Storage Healing run in parallel with phases 4-5]      │
│                                                                          │
└─────────────────────────────────────────────────────────────────────────┘
```

### Current Performance Bottlenecks

Based on code analysis and profiling data:

| Bottleneck | Location | Impact | Priority |
|------------|----------|--------|----------|
| Sequential header download | `sync_cycle_snap()` | Blocks state download start | Critical |
| Single-threaded account range processing | `request_account_range()` | Underutilizes peers | High |
| Inefficient trie node batching | `heal_state_trie()`, `heal_storage_trie()` | Excessive DB writes | High |
| Busy-wait loops | Multiple locations | CPU waste | Medium |
| Unbounded memory structures | `accounts_by_root_hash` | Memory pressure | Medium |
| Synchronous disk I/O | Snapshot dumping | Blocks network operations | Medium |

### Existing Code Quality Issues

| Issue | Location | Description |
|-------|----------|-------------|
| `#[allow(clippy::too_many_arguments)]` | `heal_state_trie()` | 8 parameters - needs context struct |
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
- Individual DB writes with `put_batch()`

**Proposed Changes:**

#### 1.3.1 Use `put_batch_no_alloc()` for Healing
```rust
// Current (healing/state.rs:304)
// PERF: use put_batch_no_alloc (note that it needs to remove nodes too)

// Proposed: Pre-allocate buffers, reuse across batches
struct HealingBatchWriter {
    node_buffer: Vec<(Nibbles, Node)>,
    capacity: usize,
}
```

#### 1.3.2 Dynamic Batch Sizing
Adjust batch sizes based on:
- Available memory
- Peer response latency
- Current healing progress

**Expected Impact:** 30-50% reduction in healing phase duration

**Effort:** Medium (2 weeks)

---

### 1.4 Reduce Busy-Wait Loops (Issue #6140 — Step 9)

**Current State:** Multiple locations use `tokio::time::sleep()` in loops:
- `sync_cycle_snap()`: 100ms sleep waiting for headers
- `request_account_range()`: 10ms sleep waiting for peers
- Healing loops: Various polling intervals

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

### 1.6 Async Disk I/O

**Current State:** Snapshot dumping uses synchronous `std::fs` operations.

**Proposed Change:** Use `tokio::fs` for non-blocking I/O:

```rust
// Current
std::fs::create_dir_all(dir)?;
dump_accounts_to_file(&path, chunk)?;

// Proposed
tokio::fs::create_dir_all(dir).await?;
tokio::task::spawn_blocking(move || {
    dump_accounts_to_file(&path, chunk)
}).await??;
```

**Expected Impact:** Network operations not blocked by disk I/O

**Effort:** Low (1 week)

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

**Expected Impact:** 20-30% improvement in peer utilization

**Effort:** Medium (2 weeks)

---

### 1.8 ~~Parallel Storage Healing~~ (Discarded)

> Discarded — DB write contention makes this impractical without a larger storage refactor.

---

## Phase 2: Code Quality & Maintainability

### Goal
Make the codebase clear, well-documented, and easy for new contributors to understand.

---

### 2.1 Extract Context Structs (Issue #6140 — Steps 5, 6)

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
- `sync/healing/state.rs`
- `sync/healing/storage.rs`
- `sync/snap_sync.rs`

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

### 2.5 State Machine Refactor

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

**Current State:** Two locations crash the node on recoverable errors:
- `panic!("Should have found the account hash")` (line 735)
- `.expect()` calls on store lookups (lines 554-558)

**Proposed Change:** Replace with proper error propagation using `SnapError::InternalError` and `?` operator.

**Effort:** Very low (< 1 day)

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

---

## Risk Assessment

| Risk | Probability | Impact | Mitigation |
|------|-------------|--------|------------|
| Performance regression | Medium | High | Comprehensive benchmarking before/after each change |
| Data corruption during sync | Low | Critical | Extensive integration testing; checksums; recovery mechanisms |
| Breaking changes to peer protocol | Low | Medium | Hive test suite validation |
| Increased complexity | Medium | Medium | Code review; documentation requirements |
| Schedule overrun | Medium | Medium | Prioritize high-impact items; iterative delivery |

---

## Timeline

### Phase 1: Performance (12 weeks)

```
Week 1-2:   1.1 Parallel Header Download (complete PR #6059)
Week 2-3:   1.4 Reduce Busy-Wait Loops (Issue #6140)
Week 3-4:   1.6 Async Disk I/O
Week 4-6:   1.3 Optimize Trie Node Batching
Week 6-8:   1.7 Peer Connection Optimization
```

### Phase 2: Code Quality (10 weeks)

```
Week 1:     2.8 Fix Correctness Bugs (Issue #6140)
Week 1:     2.1 Extract Context Structs (Issue #6140)
Week 1-2:   2.4 Extract Helper Functions (Issue #6140)
Week 2-3:   2.3 Consolidate Error Handling (Merged — PR #5975)
Week 3-5:   2.2 Comprehensive Documentation
Week 5-7:   2.7 Configuration Externalization
Week 7-10:  2.5 State Machine Refactor
Week 8-12:  2.6 Test Coverage Improvement (parallel)
```

**Total Duration:** ~14 weeks (phases overlap)

---

## Dependencies

### External Dependencies

| Dependency | Version | Purpose |
|------------|---------|---------|
| tokio | 1.x | Async runtime |
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
| `healing/storage.rs:156` | Better data receiver design | Medium |
| `healing/storage.rs:230` | Use `put_batch_no_alloc` | High |
| `healing/storage.rs:298` | Store error handling | High |
| `healing/state.rs:149` | Peer scoring for responses | Medium |
| `healing/state.rs:194` | Optimize trie leaf reaching | Low |
| `snap/client.rs:567` | Stable sort for binary search | Low |
| `snap/client.rs:599` | Replace with removable structure | Medium |
| `snap/client.rs:983` | Unnecessary unzip/memory | Low |

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

*Document Version: 1.0*
*Last Updated: February 2026*
