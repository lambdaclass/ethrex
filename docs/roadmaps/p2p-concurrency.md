# P2P & Concurrency Roadmap for ethrex

> **Last updated:** 2026-03-27

## Overview

This roadmap organizes all pending work for P2P networking and concurrency in ethrex into two parallel lines of work with clear phases. The goal is to use `spawned-concurrency` (GenServer pattern) for all concurrency handling where applicable.

---

## LINE OF WORK 1: P2P NETWORKING

> **IMPORTANT: DiscV4 Deprecation Notice**
> Source: [Official EL DiscV5 Tracker](https://notes.ethereum.org/@cskiraly/el-discovery-v5-tracker) (Ethereum Foundation)
>
> | Milestone | Date | Status |
> |-----------|------|--------|
> | DiscV5 implementation deadline | **Jan 15, 2026** | PASSED - ethrex has DiscV5 impl |
> | DiscV5 must be enabled | **Jan 31, 2026** | PASSED - dual-stack via [#5962](https://github.com/lambdaclass/ethrex/pull/5962) |
> | DiscV4 disabled | **Glamsterdam hard fork** (~mid-2026, TBD) | Upcoming |
>
> **Current client status:** Geth/Nimbus have both on; Erigon v3.3.3+ is DiscV5-only; Besu/Nethermind/Reth still need DiscV5.
>
> DiscV4-specific improvements are **deprioritized**. Focus on DiscV5 hardening.

### Phase 1: Discovery Protocol Consolidation

**Goal:** Complete DiscV5 implementation, enable dual-stack, prepare for DiscV4 sunset.

| Priority | Task | Issue/PR | Status |
|----------|------|----------|--------|
| P0 | ~~Merge rate limit WHOAREYOU packets (discv5)~~ | [#5909](https://github.com/lambdaclass/ethrex/pull/5909) | **Merged** (Feb 9) |
| P0 | ~~Bound whoareyou_rate_limit map with LRU cache + global rate limit~~ | [#6125](https://github.com/lambdaclass/ethrex/issues/6125), [#6383](https://github.com/lambdaclass/ethrex/pull/6383) | **In Review** (1/3 approvals, CI green) |
| P0 | ~~Merge dual discovery protocol support (discv4+discv5)~~ | [#5962](https://github.com/lambdaclass/ethrex/pull/5962) | **Merged** (Feb 25) |
| P0 | ~~Remove experimental-discv5 feature flag~~ | [#6015](https://github.com/lambdaclass/ethrex/pull/6015), [#5971](https://github.com/lambdaclass/ethrex/issues/5971) | **Merged** (Mar 4) |
| P1 | Unify discovery GenServers into single DiscoveryServer | [#5990](https://github.com/lambdaclass/ethrex/issues/5990) | Open issue |
| P1 | Track unrecognized discovery packets | [#6400](https://github.com/lambdaclass/ethrex/issues/6400), [#6408](https://github.com/lambdaclass/ethrex/pull/6408) | Open PR |
| P1 | Explicitly filter discv4/discv5 packets in multiplexer | [#6398](https://github.com/lambdaclass/ethrex/pull/6398) | Open PR |
| P2 | ~~Move p2p inline tests to test crate~~ | [#5992](https://github.com/lambdaclass/ethrex/issues/5992), [#6354](https://github.com/lambdaclass/ethrex/pull/6354) | **Merged** (Mar 16) |

### Phase 2: DiscV5 Security Hardening

**Goal:** Complete security measures for discv5 protocol.

| Priority | Task | Issue/PR | Status |
|----------|------|----------|--------|
| P0 | ~~Verify ID-signature on handshake receipt~~ | [#5832](https://github.com/lambdaclass/ethrex/issues/5832), [#6055](https://github.com/lambdaclass/ethrex/pull/6055) | **Merged** (Jan 30) |
| P0 | ~~Store validated ENR from handshake~~ | [#6109](https://github.com/lambdaclass/ethrex/pull/6109) | **Merged** (Feb 23) |
| P1 | ~~Request updated ENR when PONG enr_seq differs~~ | [#5910](https://github.com/lambdaclass/ethrex/pull/5910), [#5850](https://github.com/lambdaclass/ethrex/issues/5850) | **Merged** (Feb 11) |
| P1 | ~~Update existing contact ENR on NODES response~~ | [#6172](https://github.com/lambdaclass/ethrex/pull/6172) | **Merged** (Feb 19) |
| P1 | ~~Detect external IP via PONG recipient_addr voting~~ | [#5914](https://github.com/lambdaclass/ethrex/pull/5914), [#5851](https://github.com/lambdaclass/ethrex/issues/5851) | **Merged** (Feb 24) |
| P1 | ~~Add anti-amplification check to discv5 handle_find_node~~ | [#6200](https://github.com/lambdaclass/ethrex/pull/6200) | **Merged** (Feb 23) |
| P1 | ~~P2P sync stall fixes and discovery hardening~~ | [#6394](https://github.com/lambdaclass/ethrex/pull/6394) | **Merged** (Mar 25) |
| P1 | Validate PONG req_id matches a pending ping | [#6167](https://github.com/lambdaclass/ethrex/issues/6167) | Open issue |
| P1 | Fix discv5 Hive test failures | [#6401](https://github.com/lambdaclass/ethrex/pull/6401) | Open PR |
| P2 | Prune session_ips in cleanup_stale_entries | [#6404](https://github.com/lambdaclass/ethrex/issues/6404) | Open issue |
| P2 | FindNode should return local ENR for distance=0 | [#6030](https://github.com/lambdaclass/ethrex/issues/6030) | Open issue |
| P2 | Periodically update local ENR | [#5493](https://github.com/lambdaclass/ethrex/issues/5493) | Open issue |

### Phase 2.1: DiscV5 Protocol Tests

**Goal:** Improve test coverage for discv5 message encoding/decoding.

| Priority | Task | Issue/PR | Status |
|----------|------|----------|--------|
| P2 | Add PONG message encode/decode tests | [#5991](https://github.com/lambdaclass/ethrex/issues/5991), [#6343](https://github.com/lambdaclass/ethrex/pull/6343) | Open PR |
| P2 | Add FindNode message encode/decode tests | [#5993](https://github.com/lambdaclass/ethrex/issues/5993) | Open issue |
| P2 | Add Nodes message encode/decode tests | [#5994](https://github.com/lambdaclass/ethrex/issues/5994) | Open issue |

### Phase 2.5: DiscV4 Maintenance (DEPRIORITIZED)

**Goal:** Minimal maintenance until Glamsterdam hard fork. Low priority.

| Priority | Task | Issue/PR | Status |
|----------|------|----------|--------|
| P3 | Ignore unrequested Neighbors messages | [#3746](https://github.com/lambdaclass/ethrex/issues/3746) | Open issue - **DEPRIORITIZED** |
| P3 | Complete DiscV4 ENRResponse validation | — | TODO at `discv4/server.rs:215-222` - **DEPRIORITIZED** |
| P3 | Add DiscV4 per-peer rate limiting | — | **DEPRIORITIZED** - DiscV5 already has this |
| P3 | Reimplement DiscV4 tests | [#4423](https://github.com/lambdaclass/ethrex/issues/4423) | Open issue - **DEPRIORITIZED** |

### Phase 3: Peer Management & Kademlia

**Goal:** Improve peer table, scoring, and load balancing.

| Priority | Task | Issue/PR | Status |
|----------|------|----------|--------|
| P0 | Reintroduce proper Kademlia table implementation | [#4245](https://github.com/lambdaclass/ethrex/issues/4245) | Open (Milestone: Syncing) |
| P1 | Improve peer scoring and load balancing | [#4861](https://github.com/lambdaclass/ethrex/issues/4861) | Open issue |
| P1 | ~~Allow min-score peers to handle 1 concurrent request~~ | [#6272](https://github.com/lambdaclass/ethrex/pull/6272) | **Merged** (Feb 27) |
| P1 | Detect performance degradation with large contact tables | [#5972](https://github.com/lambdaclass/ethrex/issues/5972) | Open issue |
| P1 | Fix running out of peers mid-syncing | [#3050](https://github.com/lambdaclass/ethrex/issues/3050) | Open issue |
| P1 | Leech detection and rate limiting | [#5522](https://github.com/lambdaclass/ethrex/issues/5522) | Open issue |
| P2 | ~~Avoid collect in peer table get_contact functions~~ | [#5641](https://github.com/lambdaclass/ethrex/pull/5641) | **Closed** |
| P2 | ~~Avoid iterating whole table on FINDNODE~~ | [#5644](https://github.com/lambdaclass/ethrex/pull/5644) | **Closed** |

### Phase 4: RLPx & Protocol Improvements

**Goal:** Optimize RLPx protocol and address technical debt.

| Priority | Task | Issue/PR | Status |
|----------|------|----------|--------|
| P1 | ~~Avoid extra allocations in RLPx handshake~~ | [#5531](https://github.com/lambdaclass/ethrex/pull/5531) | **Merged** (Feb 23) |
| P1 | ~~Avoid double authdata allocation in discv5 header~~ | [#5811](https://github.com/lambdaclass/ethrex/pull/5811) | **Merged** (Mar 2) |
| P1 | ~~Fix consistent encoding for blob tx size in NewPooledTransactionHashes~~ | [#6256](https://github.com/lambdaclass/ethrex/pull/6256) | **Merged** (Feb 24) |
| P1 | ~~Fix broadcast_pool race and offload tx pool insertion~~ | [#6253](https://github.com/lambdaclass/ethrex/pull/6253) | **Merged** (Feb 24) |
| P1 | ~~Implement eth/70 partial receipt fetching~~ | [#6327](https://github.com/lambdaclass/ethrex/pull/6327) (EIP-7542) | **Merged** (Mar 25) |
| P1 | Implement eth/71 Block Access List exchange | [#6306](https://github.com/lambdaclass/ethrex/pull/6306) (EIP-8159) | Open PR |
| P1 | Compute RLPx capability message ID dynamically | [#4545](https://github.com/lambdaclass/ethrex/issues/4545) | Open issue |
| P2 | Remove magic numbers in rlpx/connection | [#4123](https://github.com/lambdaclass/ethrex/issues/4123) | Open issue |
| P2 | Enable TCP_NODELAY on P2P TCP socket | [#5042](https://github.com/lambdaclass/ethrex/issues/5042) | Open issue |
| P2 | Improve transaction broadcasting mechanism | [#3388](https://github.com/lambdaclass/ethrex/issues/3388) | Open issue |
| P2 | TXBroadcaster batching may delay tx propagation | [#5833](https://github.com/lambdaclass/ethrex/issues/5833) | Open issue |

### Phase 5: Network Configuration

**Goal:** Improve network addressing flexibility.

| Priority | Task | Issue/PR | Status |
|----------|------|----------|--------|
| P1 | Dual-stack IPv6 support in discv5 | [#6371](https://github.com/lambdaclass/ethrex/issues/6371), [#6377](https://github.com/lambdaclass/ethrex/pull/6377), [#6376](https://github.com/lambdaclass/ethrex/pull/6376) | Open PRs |
| P2 | Decouple bind address from external address | [#5425](https://github.com/lambdaclass/ethrex/issues/5425), [#6374](https://github.com/lambdaclass/ethrex/pull/6374) | Open PR |
| P2 | Decouple discv4 address from RLPx address | [#5424](https://github.com/lambdaclass/ethrex/issues/5424), [#6375](https://github.com/lambdaclass/ethrex/pull/6375) | Open PR |
| P2 | Add flag to specify P2P/discovery address | [#5290](https://github.com/lambdaclass/ethrex/issues/5290) | Open issue |
| P2 | Add IPv6 support | [#5354](https://github.com/lambdaclass/ethrex/issues/5354) | Open issue |

---

## LINE OF WORK 2: CONCURRENCY

> **See Also: Comprehensive Snap Sync Roadmap ([#6112](https://github.com/lambdaclass/ethrex/pull/6112))**
>
> PR #6112 contains a detailed 788-line snap sync roadmap covering:
> - **Phase 1: Performance Optimization** - Parallel headers, adaptive chunking, async I/O, memory-bounded structures
> - **Phase 2: Code Quality & Maintainability** - Context structs, documentation, error handling consistency
>
> The phases below align with and implement specific items from that roadmap.

### Phase 1: Snap Sync Refactoring with Spawned

**Goal:** Reorganize snap sync code and convert to spawned patterns.

| Priority | Task | Issue/PR | Status |
|----------|------|----------|--------|
| P0 | ~~Merge snap sync code reorganization~~ | [#5975](https://github.com/lambdaclass/ethrex/pull/5975) | **Merged** (Feb 6) |
| P0 | Replace sync disk I/O with async operations | [#6113](https://github.com/lambdaclass/ethrex/pull/6113) | Approved |
| P1 | Snapsync rewrite with spawned | [#4240](https://github.com/lambdaclass/ethrex/issues/4240) | Open (Milestone: Syncing) |
| P1 | Extract snapshot dumping helpers | [#6099](https://github.com/lambdaclass/ethrex/pull/6099) | Open |

> **Snap Sync Reorganization Plan ([#5975](https://github.com/lambdaclass/ethrex/pull/5975))**
>
> PR #5975 contains a detailed 5-phase refactoring plan covering ~6,500 lines across 7 files:
> - **Phase 1: Foundation** - Create `snap/` module directory, extract server code, add constants
> - **Phase 2: Protocol Layer** - Split `rlpx/snap.rs` into messages and codec modules
> - **Phase 3: Healing Unification** - Create unified `sync/healing/` directory with shared types
> - **Phase 4: Sync Orchestration** - Split `sync.rs` into focused modules, extract snap client from peer_handler
> - **Phase 5: Error Handling** - Create unified `SnapError` enum, remove redundant error types

### Phase 2: Parallel Operations

**Goal:** Maximize parallel execution for sync performance.

| Priority | Task | Issue/PR | Status |
|----------|------|----------|--------|
| P0 | ~~Merge parallel storage trie merkelization~~ | [#6079](https://github.com/lambdaclass/ethrex/pull/6079) | **Merged** (Feb 5) |
| P1 | ~~Parallel account range requests with adaptive chunking~~ | [#6101](https://github.com/lambdaclass/ethrex/pull/6101) | **Closed** |
| P1 | ~~Speed up snap sync validation with parallelism and deduplication~~ | [#6191](https://github.com/lambdaclass/ethrex/pull/6191) | **Merged** (Feb 25) |
| P1 | Parallelize header download with state download | [#6059](https://github.com/lambdaclass/ethrex/pull/6059) | Open |
| P1 | Parallelize merkelization of storage slots | [#5482](https://github.com/lambdaclass/ethrex/issues/5482) | Open issue |
| P2 | Reduce allocations in account range verification | [#6072](https://github.com/lambdaclass/ethrex/pull/6072) | Open |
| P2 | 4 performance optimizations for faster sync | [#5903](https://github.com/lambdaclass/ethrex/pull/5903) | Open |

### Phase 3: Spawned GenServer Migration

**Goal:** Convert remaining raw spawns to GenServer pattern.

| Priority | Task | Issue/PR | Status |
|----------|------|----------|--------|
| P0 | Use spawned sync GenServer for threaded code | [#5599](https://github.com/lambdaclass/ethrex/pull/5599), [#5565](https://github.com/lambdaclass/ethrex/issues/5565) | Open |
| P0 | Migrate actors to spawned 0.5.0 macro API | [#6295](https://github.com/lambdaclass/ethrex/pull/6295) | Open PR |
| P1 | Add RLPx Downloader actor | [#4420](https://github.com/lambdaclass/ethrex/issues/4420) | Open issue |
| P1 | Spawnify PeerHandler (TODO in code) | — | Code TODO at `peer_handler.rs:153` |
| P2 | Further refactor proof_coordinator with spawned (L2) | [#3009](https://github.com/lambdaclass/ethrex/issues/3009) | Open issue |
| P2 | Fix Block Producer blocking issues (L2) | [#3057](https://github.com/lambdaclass/ethrex/issues/3057) | Open issue |

**Current raw tokio::spawn locations to migrate:**
- `cmd/ethrex/ethrex.rs` - Version check task
- `crates/networking/p2p/peer_handler.rs` (4 instances) - Request workers
- `crates/networking/p2p/sync_manager.rs` - Sync task
- `crates/networking/p2p/sync/state_healing.rs` - Parallel healing requests
- `crates/networking/rpc/rpc.rs` - Timer spawn

### Phase 4: Blocking Code & Performance

**Goal:** Identify and fix blocking operations, add monitoring.

| Priority | Task | Issue/PR | Status |
|----------|------|----------|--------|
| P1 | Check potential blocking code | [#4494](https://github.com/lambdaclass/ethrex/issues/4494) | Open issue (6 comments) |
| P1 | Implement adaptive peer timeouts | [#6117](https://github.com/lambdaclass/ethrex/pull/6117) | Draft |
| P2 | Add lock monitoring tooling | [#4495](https://github.com/lambdaclass/ethrex/issues/4495) | Open issue |

**Known blocking operations to address (from #4494):**
- RLPx handshake: compression/decompression in `send_ack`/`send_auth`
- RLPx connection: DB iteration for `GetReceipts`, blocking for `GetBytecodes`
- Discv4: quadratic search with lock held in `get_closest_nodes`
- L2: `recover_address` call in `should_process_new_block`

---

## Execution Order Recommendation

### Immediate (Merge Ready)
1. [#6383](https://github.com/lambdaclass/ethrex/pull/6383) - WHOAREYOU LRU cache + global rate limit (1/3 approvals)
2. [#6401](https://github.com/lambdaclass/ethrex/pull/6401) - Fix discv5 Hive test failures
3. [#6113](https://github.com/lambdaclass/ethrex/pull/6113) - Async disk I/O (approved)

### Short-term (Next 2-4 weeks)
1. Merge remaining discovery hardening ([#6383](https://github.com/lambdaclass/ethrex/pull/6383), [#6401](https://github.com/lambdaclass/ethrex/pull/6401))
2. Complete IPv6/dual-stack support ([#6377](https://github.com/lambdaclass/ethrex/pull/6377), [#6376](https://github.com/lambdaclass/ethrex/pull/6376))
3. Address decoupling ([#6374](https://github.com/lambdaclass/ethrex/pull/6374), [#6375](https://github.com/lambdaclass/ethrex/pull/6375))
4. Merge eth/71 BAL exchange ([#6306](https://github.com/lambdaclass/ethrex/pull/6306))
5. Validate PONG req_id ([#6167](https://github.com/lambdaclass/ethrex/issues/6167)) and prune session_ips ([#6404](https://github.com/lambdaclass/ethrex/issues/6404))

### Medium-term (1-2 months)
1. Kademlia table reimplementation [#4245](https://github.com/lambdaclass/ethrex/issues/4245)
2. GenServer migration for PeerHandler and sync components ([#6295](https://github.com/lambdaclass/ethrex/pull/6295))
3. Peer scoring improvements [#4861](https://github.com/lambdaclass/ethrex/issues/4861)
4. Leech detection [#5522](https://github.com/lambdaclass/ethrex/issues/5522)

### Long-term (Ongoing)
1. DNS-based node discovery (EIP-1459)
2. Lock monitoring tooling
3. Code quality improvements (magic numbers, tech debt)
4. P2P metrics and observability

---

## Key Files to Monitor

**P2P:**
- `crates/networking/p2p/discv4/server.rs`
- `crates/networking/p2p/discv5/server.rs`
- `crates/networking/p2p/peer_handler.rs`
- `crates/networking/p2p/rlpx/connection/server.rs`
- `crates/networking/p2p/tx_broadcaster.rs`

**Concurrency:**
- `crates/networking/p2p/sync_manager.rs`
- `crates/networking/p2p/sync.rs` (being split)
- `crates/networking/p2p/snap.rs`
- `crates/storage/store.rs` (background threads)

---

## Issue/PR Summary

| Category | Open Issues | Open PRs | Merged |
|----------|-------------|----------|--------|
| Discovery Protocol | 3 | 3 (#6383, #6408, #6398) | 5 (#5909, #5962, #6015, #6354, #6394) |
| Discovery Security | 4 (#6167, #6404, #6030, #5493) | 2 (#6401, #6343) | 7 (#6055, #5910, #6172, #6109, #5914, #6200, #6394) |
| Discovery Tests | 2 (#5993, #5994) | 1 (#6343) | 0 |
| Peer Management | 6 | 0 | 1 (#6272) |
| RLPx/Protocol | 5 | 1 (#6306) | 5 (#5531, #5811, #6256, #6253, #6327) |
| Network Config | 4 | 4 (#6377, #6376, #6374, #6375) | 0 |
| Snap Sync | 2 | 4 | 1 (#5975) |
| Parallel Operations | 2 | 3 | 2 (#6079, #6191) |
| GenServer Migration | 5 | 2 (#5599, #6295) | 0 |
| Blocking/Performance | 3 | 1 | 0 |
| **Total** | **36** | **21** | **21** |

---

## SPEC COMPLIANCE GAPS & UNTRACKED RISKS

Based on review of official Ethereum devp2p specifications (RLPx, DiscV4, DiscV5, eth/68-71, snap/1), ENR (EIP-778), and comparison with ethrex implementation.

### Critical/High Priority - Security Risks

| Gap | Severity | Location | Spec Requirement | Status |
|-----|----------|----------|------------------|--------|
| **Missing Neighbors request tracking (discv5)** | MEDIUM | `discv5/server.rs` | Accept only solicited Neighbors | TODO references #3746 |
| **Unsolicited PONG acceptance** | MEDIUM | `discv5/server.rs` | Validate req_id matches pending ping | Open [#6167](https://github.com/lambdaclass/ethrex/issues/6167) |
| **RLPx message size limit enforcement** | MEDIUM | `rlpx/connection/codec.rs` | Reject decompressed >16 MiB | **NEEDS VERIFICATION** |
| ~~DiscV4 ENRResponse validation~~ | ~~HIGH~~ | `discv4/server.rs:215-222` | ~~Must verify signature~~ | **DEPRIORITIZED** - DiscV4 sunset |
| ~~DiscV4 rate limiting~~ | ~~MEDIUM~~ | `discv4/server.rs` | ~~Prevent amplification~~ | **DEPRIORITIZED** - DiscV4 sunset |

### Missing Protocol Features

| Feature | Spec | Current State | Priority |
|---------|------|---------------|----------|
| **eth/71 Block Access List exchange** | EIP-8159 | In Progress ([#6306](https://github.com/lambdaclass/ethrex/pull/6306)) | P1 |
| **DiscV5 Topic Advertisement** | discv5-wire.md | Not implemented | P2 - Optional but useful |
| **DNS-based node discovery** | EIP-1459 | Not implemented | P2 - Complementary to UDP |
| **ENR "eth" field validation** | devp2p/enr.md | Partial (fork ID only) | P2 |
| **P2P bandwidth/connection metrics** | Best practice | Not implemented | P2 - Observability |
| **Inbound/outbound peer ratio** | Best practice | Not enforced | P3 |

### Security Best Practices - Gaps

| Practice | Spec Reference | Current State | Risk |
|----------|----------------|---------------|------|
| **Eclipse attack mitigations** | devp2p security | Basic peer rotation | Low - needs audit |
| **Diverse peer selection** | Best practice | Not explicit | Low |
| **Connection slot management** | eth protocol | Basic limits only | Low |
| **Inbound/outbound peer ratio** | Best practice | Not enforced | Low |

### Code Quality TODOs (Not in Issues)

| Location | TODO | Impact |
|----------|------|--------|
| `discv4/server.rs:301,325,350` | Parametrize expiration timeouts | Maintainability |
| `discv4/server.rs:809` | Reimplement removed tests | Test coverage |
| `rlpx/connection/server.rs:775` | Match error types to disconnect reasons | Spec compliance |
| `rlpx/connection/server.rs:434` | Check if errors are common problems | Debugging |
| `rlpx/connection/server.rs:920` | Handle disconnection request properly | Spec compliance |
| `rlpx/eth/transactions.rs:180` | Batch transaction fetching | Performance |
| `peer_handler.rs:1578` | FIXME: unzip takes unnecessary memory | Memory usage |

### Recommended New Issues to Create

1. **[P2] Verify RLPx decompression size limits**
   - Spec requires rejecting messages >16 MiB decompressed
   - Audit snappy decompression paths
   - Add explicit size check before decompression

2. **[P2] Add DNS-based node discovery (EIP-1459)**
   - Complementary to UDP-based discovery
   - Helps bootstrap in restrictive network environments
   - Used by all major clients (geth, nethermind, etc.)

3. **[P2] Implement DiscV5 Topic Advertisement**
   - Optional but enables service discovery
   - Useful for finding specific node types (e.g., light clients)

4. **[P2] Add P2P metrics and observability**
   - Bandwidth per peer, message rates, discovery stats
   - Connection diversity metrics (subnet/ASN distribution)
   - Help diagnose sync stalls and peer issues in production

---

## Compliance Summary

| Protocol | Compliance | Critical Gaps | Priority |
|----------|------------|---------------|----------|
| **DiscV5** | 95% | PONG validation, Neighbors tracking, session cleanup | **HIGH** - Hardening ongoing |
| **DiscV4** | 95% | ENRResponse validation | **LOW** - Sunset at Glamsterdam |
| **RLPx** | 98% | Size limit verification | MEDIUM |
| **eth/68** | 100% | None | — |
| **eth/69** | 95% | Block range handling | MEDIUM |
| **eth/70** | 100% | ~~Not implemented~~ **Done** ([#6327](https://github.com/lambdaclass/ethrex/pull/6327)) | — |
| **eth/71** | In Progress | BAL exchange ([#6306](https://github.com/lambdaclass/ethrex/pull/6306)) | HIGH |
| **snap/1** | 95% | None critical | — |
| **ENR** | 95% | Full "eth" field validation | LOW |

**Overall Assessment:** ethrex is substantially compliant with current Ethereum P2P specifications. Since the last update (Feb 5), significant progress has been made:
- **eth/70** went from 0% to 100% complete
- **eth/71** implementation is in progress
- Discovery security hardening continues with 7+ PRs merged
- IPv6/dual-stack support has active PRs
- Test coverage improved with p2p test migration to dedicated crate

**Remaining priorities:**
1. **Discovery hardening** - PONG validation (#6167), session cleanup (#6404), Hive fixes (#6401)
2. **eth/71 support** - BAL exchange for Amsterdam hard fork
3. **IPv6/dual-stack** - Required by external users (#6371)
4. **DNS-based discovery** - Operational resilience (nice-to-have)

**DiscV4 gaps remain deprioritized** due to Glamsterdam hard fork sunset.
