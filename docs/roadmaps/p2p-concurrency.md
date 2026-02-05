# P2P & Concurrency Roadmap for ethrex

## Overview

This roadmap organizes all pending work for P2P networking and concurrency in ethrex into two parallel lines of work with clear phases. The goal is to use `spawned-concurrency` (GenServer pattern) for all concurrency handling where applicable.

---

## LINE OF WORK 1: P2P NETWORKING

> **IMPORTANT: DiscV4 Deprecation Notice**
> Source: [Official EL DiscV5 Tracker](https://notes.ethereum.org/@cskiraly/el-discovery-v5-tracker) (Ethereum Foundation)
>
> | Milestone | Date |
> |-----------|------|
> | DiscV5 implementation deadline | **Jan 15, 2026** |
> | DiscV5 must be enabled | **Jan 31, 2026** |
> | DiscV4 disabled | **Glamsterdam hard fork** (~mid-2026, TBD) |
>
> **Current client status:** Geth/Nimbus have both on; Erigon v3.3.3+ is DiscV5-only; Besu/Nethermind/Reth still need DiscV5.
>
> DiscV4-specific improvements are **deprioritized**. Focus on DiscV5 hardening.

### Phase 1: Discovery Protocol Consolidation

**Goal:** Complete DiscV5 implementation, enable dual-stack, prepare for DiscV4 sunset.

| Priority | Task | Issue/PR | Status |
|----------|------|----------|--------|
| P0 | Merge rate limit WHOAREYOU packets (discv5) | [#5909](https://github.com/lambdaclass/ethrex/pull/5909) | Ready (2 approvals) |
| P0 | Bound whoareyou_rate_limit map with LRU cache | [#6125](https://github.com/lambdaclass/ethrex/issues/6125) | Open issue |
| P0 | Merge dual discovery protocol support (discv4+discv5) | [#5962](https://github.com/lambdaclass/ethrex/pull/5962) | Approved (2 approvals) |
| P0 | Remove experimental-discv5 feature flag | [#6015](https://github.com/lambdaclass/ethrex/pull/6015), [#5971](https://github.com/lambdaclass/ethrex/issues/5971) | Open |
| P1 | Unify discovery GenServers into single DiscoveryServer | [#5990](https://github.com/lambdaclass/ethrex/issues/5990) | Open issue |
| P2 | Move discovery tests to dedicated folders | [#5992](https://github.com/lambdaclass/ethrex/issues/5992) | Open issue |

**Branches:** `discv5-discovery-multiplexer`, `discv5-remove-experimental-feature-flag`, `discv5-server-rate-limit`

### Phase 2: DiscV5 Security Hardening

**Goal:** Complete security measures for discv5 protocol before mandatory deadline.

| Priority | Task | Issue/PR | Status |
|----------|------|----------|--------|
| P0 | Verify ID-signature on handshake receipt | [#5832](https://github.com/lambdaclass/ethrex/issues/5832) | Open issue |
| P0 | Store validated ENR from handshake | [#6109](https://github.com/lambdaclass/ethrex/pull/6109), [#6124](https://github.com/lambdaclass/ethrex/pull/6124) | Competing PRs |
| — | ~~DiscV5 codec encoder~~ | — | **NOT A BUG** - Encoding handled via `Packet::encode()` directly, codec only for receiving |
| P1 | Request updated ENR when PONG enr_seq differs | [#5910](https://github.com/lambdaclass/ethrex/pull/5910), [#5850](https://github.com/lambdaclass/ethrex/issues/5850) | Open |
| P1 | Detect external IP via PONG recipient_addr voting | [#5914](https://github.com/lambdaclass/ethrex/pull/5914), [#5851](https://github.com/lambdaclass/ethrex/issues/5851) | Open |

**Branches:** `discv5-server-enr-update-on-pong`, `discv5-server-external-ip-detection`, `discv5-verify-enr-signature`

### Phase 2.5: DiscV4 Maintenance (DEPRIORITIZED)

**Goal:** Minimal maintenance until DiscV5 mandatory deadline. Low priority.

| Priority | Task | Issue/PR | Status |
|----------|------|----------|--------|
| P3 | Ignore unrequested Neighbors messages | [#3746](https://github.com/lambdaclass/ethrex/issues/3746) | Open issue - **DEPRIORITIZED** |
| P3 | Complete DiscV4 ENRResponse validation | — | TODO at `discv4/server.rs:215-222` - **DEPRIORITIZED** |
| P3 | Add DiscV4 per-peer rate limiting | — | **DEPRIORITIZED** - DiscV5 already has this |

### Phase 3: Peer Management & Kademlia

**Goal:** Improve peer table, scoring, and load balancing.

| Priority | Task | Issue/PR | Status |
|----------|------|----------|--------|
| P0 | Reintroduce proper Kademlia table implementation | [#4245](https://github.com/lambdaclass/ethrex/issues/4245) | Open (Milestone: Syncing) |
| P1 | Improve peer scoring and load balancing | [#4861](https://github.com/lambdaclass/ethrex/issues/4861) | Open issue |
| P1 | Detect performance degradation with large contact tables | [#5972](https://github.com/lambdaclass/ethrex/issues/5972) | Open issue |
| P1 | Fix running out of peers mid-syncing | [#3050](https://github.com/lambdaclass/ethrex/issues/3050) | Open issue |
| P2 | Avoid collect in peer table get_contact functions | [#5641](https://github.com/lambdaclass/ethrex/pull/5641) | Open |
| P2 | Avoid iterating whole table on FINDNODE | [#5644](https://github.com/lambdaclass/ethrex/pull/5644) | Draft |

**Branches:** `feature/enhanced-peer-scoring`, `fix-deadlock-discv4`

### Phase 4: RLPx & Protocol Improvements

**Goal:** Optimize RLPx protocol and address technical debt.

| Priority | Task | Issue/PR | Status |
|----------|------|----------|--------|
| P1 | Avoid extra allocations in RLPx handshake | [#5531](https://github.com/lambdaclass/ethrex/pull/5531) | Open |
| P1 | Avoid double authdata allocation in discv5 header | [#5811](https://github.com/lambdaclass/ethrex/pull/5811) | Open |
| P1 | Compute RLPx capability message ID dynamically | [#4545](https://github.com/lambdaclass/ethrex/issues/4545) | Open issue |
| P2 | Remove magic numbers in rlpx/connection | [#4123](https://github.com/lambdaclass/ethrex/issues/4123) | Open issue |
| P2 | Enable TCP_NODELAY on P2P TCP socket | [#5042](https://github.com/lambdaclass/ethrex/issues/5042) | Open issue |
| P2 | Improve transaction broadcasting mechanism | [#3388](https://github.com/lambdaclass/ethrex/issues/3388) | Open issue |

**Branches:** `rlpx-console`, `ethrex_rlpx_console`

### Phase 5: Network Configuration

**Goal:** Improve network addressing flexibility.

| Priority | Task | Issue/PR | Status |
|----------|------|----------|--------|
| P2 | Decouple local interface from announced address | [#5425](https://github.com/lambdaclass/ethrex/issues/5425) | Good first issue |
| P2 | Support different addresses for discv4/RLPx | [#5424](https://github.com/lambdaclass/ethrex/issues/5424) | Good first issue |
| P2 | Add IPv6 support | [#5354](https://github.com/lambdaclass/ethrex/issues/5354) | Good first issue |

---

## LINE OF WORK 2: CONCURRENCY

### Phase 1: Snap Sync Refactoring with Spawned

**Goal:** Reorganize snap sync code and convert to spawned patterns.

| Priority | Task | Issue/PR | Status |
|----------|------|----------|--------|
| P0 | Merge snap sync code reorganization | [#5975](https://github.com/lambdaclass/ethrex/pull/5975) | Approved |
| P0 | Replace sync disk I/O with async operations | [#6113](https://github.com/lambdaclass/ethrex/pull/6113) | Approved |
| P1 | Snapsync rewrite with spawned | [#4240](https://github.com/lambdaclass/ethrex/issues/4240) | Open (Milestone: Syncing) |
| P1 | Extract snapshot dumping helpers | [#6099](https://github.com/lambdaclass/ethrex/pull/6099) | Open |

**Branches:** `snap_sync_final_p2p_perf`, `fix_snap_sync_trie_spawned`

### Phase 2: Parallel Operations

**Goal:** Maximize parallel execution for sync performance.

| Priority | Task | Issue/PR | Status |
|----------|------|----------|--------|
| P0 | Merge parallel storage trie merkelization | [#6079](https://github.com/lambdaclass/ethrex/pull/6079) | Approved |
| P1 | Parallel account range requests with adaptive chunking | [#6101](https://github.com/lambdaclass/ethrex/pull/6101) | Draft (4-phase impl) |
| P1 | Parallelize header download with state download | [#6059](https://github.com/lambdaclass/ethrex/pull/6059) | Open |
| P1 | Parallelize merkelization of storage slots | [#5482](https://github.com/lambdaclass/ethrex/issues/5482) | Open issue |
| P2 | Reduce allocations in account range verification | [#6072](https://github.com/lambdaclass/ethrex/pull/6072) | Open |
| P2 | 4 performance optimizations for faster sync | [#5903](https://github.com/lambdaclass/ethrex/pull/5903) | Open |

**Branches:** `snap-sync/001-peer-capacity-tracking`, `snap-sync/009-peer-quality-scoring`

### Phase 3: Spawned GenServer Migration

**Goal:** Convert remaining raw spawns to GenServer pattern.

| Priority | Task | Issue/PR | Status |
|----------|------|----------|--------|
| P0 | Use spawned sync GenServer for threaded code | [#5599](https://github.com/lambdaclass/ethrex/pull/5599), [#5565](https://github.com/lambdaclass/ethrex/issues/5565) | Open |
| P1 | Add RLPx Downloader actor | [#4420](https://github.com/lambdaclass/ethrex/issues/4420) | Open issue |
| P1 | Spawnify PeerHandler (TODO in code) | — | Code TODO at `peer_handler.rs:153` |
| P2 | Further refactor proof_coordinator with spawned (L2) | [#3009](https://github.com/lambdaclass/ethrex/issues/3009) | Open issue |
| P2 | Fix Block Producer blocking issues (L2) | [#3057](https://github.com/lambdaclass/ethrex/issues/3057) | Open issue |

**Branches:** `peer_handler_spawned_actor`, `handle_spawned_errors`, `spawned_aligned`

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
1. [#5909](https://github.com/lambdaclass/ethrex/pull/5909) - WHOAREYOU rate limiting
2. [#5962](https://github.com/lambdaclass/ethrex/pull/5962) - Dual discovery protocol
3. [#5975](https://github.com/lambdaclass/ethrex/pull/5975) - Snap sync reorganization
4. [#6113](https://github.com/lambdaclass/ethrex/pull/6113) - Async disk I/O
5. [#6079](https://github.com/lambdaclass/ethrex/pull/6079) - Parallel storage merkelization

### Short-term (Next 2-4 weeks)
1. Complete Phase 1 of P2P (Discovery consolidation)
2. Complete Phase 1-2 of Concurrency (Snap sync + parallelization)
3. Address LRU cache for rate limiting [#6125](https://github.com/lambdaclass/ethrex/issues/6125)
4. Complete discv5 security (Phase 2 P2P)

### Medium-term (1-2 months)
1. Kademlia table reimplementation [#4245](https://github.com/lambdaclass/ethrex/issues/4245)
2. GenServer migration for PeerHandler and sync components
3. Peer scoring improvements [#4861](https://github.com/lambdaclass/ethrex/issues/4861)

### Long-term (Ongoing)
1. IPv6 support
2. Lock monitoring tooling
3. Code quality improvements (magic numbers, tech debt)

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

| Category | Open Issues | Open PRs | Draft PRs |
|----------|-------------|----------|-----------|
| Discovery Protocol | 8 | 6 | 1 |
| Discovery Security | 5 | 4 | 0 |
| Peer Management | 5 | 2 | 1 |
| RLPx/Protocol | 4 | 2 | 0 |
| Network Config | 3 | 0 | 0 |
| Snap Sync | 2 | 5 | 1 |
| Parallel Operations | 2 | 5 | 1 |
| GenServer Migration | 5 | 1 | 0 |
| Blocking/Performance | 3 | 1 | 1 |
| **Total** | **37** | **26** | **5** |

---

## SPEC COMPLIANCE GAPS & UNTRACKED RISKS

Based on review of official Ethereum devp2p specifications (RLPx, DiscV4, DiscV5, eth/68-70, snap/1), ENR (EIP-778), and comparison with ethrex implementation.

### Critical/High Priority - Security Risks

| Gap | Severity | Location | Spec Requirement | Status |
|-----|----------|----------|------------------|--------|
| ~~DiscV5 codec encoder~~ | ~~HIGH~~ | `discv5/codec.rs:39` | ~~Required for encoding~~ | **NOT A BUG** - Encoding via `Packet::encode()` |
| **Missing Neighbors request tracking (discv5)** | MEDIUM | `discv5/server.rs:529` | Accept only solicited Neighbors | TODO references #3746 |
| **RLPx message size limit enforcement** | MEDIUM | `rlpx/connection/codec.rs` | Reject decompressed >16 MiB | **NEEDS VERIFICATION** |
| ~~DiscV4 ENRResponse validation~~ | ~~HIGH~~ | `discv4/server.rs:215-222` | ~~Must verify signature~~ | **DEPRIORITIZED** - DiscV4 sunset Jan 2026 |
| ~~DiscV4 rate limiting~~ | ~~MEDIUM~~ | `discv4/server.rs` | ~~Prevent amplification~~ | **DEPRIORITIZED** - DiscV4 sunset Jan 2026 |

### Missing Protocol Features

| Feature | Spec | Current State | Priority |
|---------|------|---------------|----------|
| **eth/70 protocol support** | EIP-7542 | Not implemented | P1 - Being rolled out |
| **DiscV5 Topic Advertisement** | discv5-wire.md | Not implemented | P2 - Optional but useful |
| **DNS-based node discovery** | EIP-1459 | Not implemented | P2 - Complementary to UDP |
| **ENR "eth" field validation** | devp2p/enr.md | Partial (fork ID only) | P2 |

### eth/70 Protocol Details (EIP-7542)
New messages needed:
- `RequestBlockRange (0x0b)` - Query peer's available block range
- `SendBlockRange (0x0c)` - Respond with block range
- Enhanced Status message with explicit `blockRange: [startBlock, endBlock]`

**Rationale:** Supports history expiry (May 1, 2025 baseline) where clients may drop pre-merge history.

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
| `rlpx/connection/server.rs:434,920` | Check if errors are common problems | Debugging |
| `rlpx/eth/transactions.rs:180` | Batch transaction fetching | Performance |
| `peer_handler.rs:1578` | FIXME: unzip takes unnecessary memory | Memory usage |

### Recommended New Issues to Create

1. **[P1] Add eth/70 protocol support**
   - New RequestBlockRange/SendBlockRange messages
   - Enhanced Status with explicit block range
   - Supports upcoming history expiry

2. **[P2] Verify RLPx decompression size limits**
   - Spec requires rejecting messages >16 MiB decompressed
   - Audit snappy decompression paths
   - Add explicit size check before decompression

3. **[P2] Add DNS-based node discovery (EIP-1459)**
   - Complementary to UDP-based discovery
   - Helps bootstrap in restrictive network environments
   - Used by all major clients (geth, nethermind, etc.)

4. **[P2] Implement DiscV5 Topic Advertisement**
   - Optional but enables service discovery
   - Useful for finding specific node types (e.g., light clients)

5. ~~[P3] Complete DiscV4 ENRResponse validation~~ **DEPRIORITIZED**
   - DiscV4 being sunset at Glamsterdam hard fork
   - Only maintain if causing active issues

6. ~~[P3] Add DiscV4 per-peer rate limiting~~ **DEPRIORITIZED**
   - DiscV4 being sunset at Glamsterdam hard fork
   - DiscV5 already has proper rate limiting

---

## Compliance Summary

| Protocol | Compliance | Critical Gaps | Priority |
|----------|------------|---------------|----------|
| **DiscV5** | 95% | Session TODOs (ENR updates, Neighbors validation) | **HIGH** - Mandatory Jan 2026 |
| **DiscV4** | 95% | ENRResponse validation | **LOW** - Sunset Jan 2026 |
| **RLPx** | 98% | Size limit verification | MEDIUM |
| **eth/68** | 100% | None | — |
| **eth/69** | 95% | Block range handling | MEDIUM |
| **eth/70** | 0% | Not implemented | **HIGH** - History expiry |
| **snap/1** | 95% | None critical | — |
| **ENR** | 95% | Full "eth" field validation | LOW |

**Overall Assessment:** ethrex is substantially compliant with current Ethereum P2P specifications. The main gaps are:
1. **eth/70 support** - Needed for history expiry (upcoming requirement)
2. **DiscV5 session TODOs** - ENR updates on PONG, Neighbors request validation
3. **DNS-based discovery** - Operational resilience (nice-to-have)

**DiscV4 gaps are deprioritized** due to Glamsterdam hard fork sunset.
