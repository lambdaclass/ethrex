# Short– to Mid-Term Roadmap

This document represents the **short- to mid-term roadmap**.  
Items listed here are actionable, concrete, and intended to be worked on in the coming weeks.  
Long-term research directions and second-order ideas are intentionally out of scope.

**Priority reflects relative urgency, not effort.**

This is a WIP document and it requires better descriptions, it's supposed to be used internally 


---

## Priority Legend

| Priority | Meaning |
|---------:|---------|
| 0 | Critical. Blocking correctness, stability, or operations |
| 1 | High. Should be addressed soon |
| 2 | Medium. Important but not blocking |
| 3 | Low. Useful improvement |
| 4 | Very low. Nice to have |
| 5 | Deprioritized for now |
| 6 | Long tail / hygiene |
| — | Not yet prioritized |


---

## Execution

| Item | Priority | Status | Description |
|-----|----------|--------|-------------|
| Replace BTreeMap with FxHashMap | 0 | Pending | Replace BTreeMap/BTreeSet with FxHashMap/FxHashSet|
| Remove RefCell from Memory | 0 | Pending | Consider using UnsafeCell with manual safety guarantees, or restructure to avoid shared ownership. |
| Object pooling | 2 | Pending | Reuse EVM stack frames to reduce allocations and improve performance |
| Try out PEVM | 0 | Pending | Benchmark again against pevm |
| Inline Hot Opcodes | 0 | Pending | |
| Avoid clones in hot path | 2 | Pending | Avoid Clone on Account Load |
| Pairing | 0 | Pending | benchmark arkworks pairing|
|  SIMD Everywhere | 2 | Pending | |
| PGO/BOLT | 0 | Pending | |
| Arena substrate | 1 | Pending | |
| Nibbles | 1 | Pending | |
| ruint | 0 | Pending | |

---

## IO

| Item | Priority | Status | Description |
|-----|----------|--------|-------------|
| RocksDB configs | 1 | Pending | |
| Lock | 1 | Pending | |
| Spawned | 3 | Pending | |
| Remove Bloom | 1 | Pending | |
| Remove cache (code + trie) | 0 | Pending | |
| Modify rocks db block size | 0 | Pending | |
| Multiget | 2 | Pending | |
| Btreemap to fxhashmap | 0 | Pending | |

---

## ZK + L2

| Item | Priority | Status | Description |
|-----|----------|--------|-------------|
| ZK API | — | Pending | |

---

## SnapSync

| Item | Priority | Status | Description |
|-----|----------|--------|-------------|
| Download receipts | 1 | Pending | |
| Improve performance | 1 | Pending | |

---

## UX / DX

| Item | Priority | Status | Description |
|-----|----------|--------|-------------|
| Add Docs | 0 | Pending | |
| Add Tests | 1 | Pending | |
| Add Fuzzing | 1 | Pending | |
| Add Prop test | 1 | Pending | |
| Add security runs to CI | 1 | Pending | |
| Remove SnapSync flag from CLI | 1 | Pending | |
| ipv6 support | 1 | Pending | |
| P2P rate limiting | 3 | Pending | |
| P2P leechers | 1 | Pending | |
| Webhook subscriptions | 2 | Pending | |
| Not allow empty blocks in dev mode | 2 | Pending | |
| No STD | 5 | Pending | |
| Migrations | 4 | Pending | |
| geth db migration tooling | 0 | Pending | |
| Custom Deterministic Benchmark  | — | Pending | we built a tool, its time to use it |
| Benchmark contract call & simple transfers | 1 | Pending | |
| RLP Duplication | 1 | Pending | |
| Add MIT License | 0 | Pending | |
| Improve Error handling | 1 | Pending | |

---

## New Features

| Item | Priority | Status | Description |
|-----|----------|--------|-------------|
| BALs | — | Pending | |
| Disc V5 | — | Pending | |
| Blobs | — | Pending | |

