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
| Replace BTreeMap with FxHashMap | 0 | In Progress | Replace BTreeMap/BTreeSet with FxHashMap/FxHashSet|
| Skip Zero-Initialization in Memory Resize | 0 | Pending | Use unsafe set_len (EVM spec says expanded memory is zero | 
| Remove RefCell from Memory | 0 | Pending | Consider using UnsafeCell with manual safety guarantees, or restructure to avoid shared ownership. |
| Object pooling | 2 | Pending | Reuse EVM stack frames to reduce allocations and improve performance |
| Try out PEVM | 0 | In Progress | Benchmark again against pevm |
| Inline Hot Opcodes | 0 | In Progress | Opcodes call a function in a jump table when some of the most used ones could perform better being inline instead |
| Avoid clones in hot path | 2 | Pending | Avoid Clone on Account Load and check rest of the hot path |
| Test ECPairing libraries | 0 | Pending | Benchmark arkworks pairing in levm|
|  SIMD Everywhere | 2 | Pending | There are some libraries that can be replace by others that uses SIMD instructions for better performance  |
| PGO/BOLT | 0 | Pending | Try out both [PGO](https://doc.rust-lang.org/beta/rustc/profile-guided-optimization.html) and [BOLT](https://github.com/llvm/llvm-project/tree/main/bolt) to see if we can improve perf |
| Use an arena allocator for substate tracking | 0 | Pending | Substates are currently a linked list allocated through boxing. Consider using an arena allocator (e.g. bumpalo) for them |
| Nibbles | 1 | Pending | Nibbles are currently stored as a byte (`u8`), when they could be stored compactly as actual nibbles in memory and reduce by half their representation size |
| ruint | 0 | Pending | Try out [ruint](https://github.com/recmo/uint) as the `U256` library to see if it improves performance. Part of SIMD initiative |
| RLP Duplication | 1 | Pending | Check whether we are encoding/decoding something twice (clearly unnecesary) |


---

## IO

| Item | Priority | Status | Description |
|-----|----------|--------|-------------|
| RocksDB configs | 1 | Pending |  |
| Remove locks | 1 | Pending | Check if there are still some unnecesary locks. e.g in the VM we have one |
| Spawned | 3 | Pending | [*Spawnify*](https://github.com/lambdaclass/spawned) io intensive components/flows. Mempool and Snapsync are top priorities | 
| Benchmark bloom filter | 1 | Pending | Review trie layer's bloom filter, remove it or tests other libraries/configurations |
| Add Block Cache (RocksDB)| 0 | Pending | Currently you have no explicit block cache, relying on OS page cache. Also try row cache |
| Use Two-Level Index (RocksDB) | 0 | Pending | Use Two-Level Index with Partitioned Filters |
| Enable unordered writes for State (RocksDB) | 0 | Pending | For `ACCOUNT_TRIE_NODES, STORAGE_TRIE_NODES cf_opts.set_unordered_write(true);` Faster writes when we don't need strict ordering|
| Increase Bloom Filter (RocksDB) | 0 | Pending | Change and benchmark higher  bits per key for state tables |
| Consider LZ4 for State Tables (RocksDB) | 0 | Pending | Trades CPU for smaller DB and potentially better cache utilization |
| Add Read-Ahead for Sequential Scans (RocksDB)| 0 | Pending | Use for trie iteration, sync operations |
| Optimize for Point Lookups (RocksDB) | 0 | Pending | Adds hash index inside FlatKeyValue for faster point lookups |
| Modify block size (RocksDB) | 0 | Pending | Benchmark differnt block size configurations|
| Increase layers commit threshold | 0 | Pending | For read-heavy workloads with plenty of RAM |
| Use multiget on trie traversal | 1 | Pending | Use multiget on trie traversal might redure read time |

---

## ZK + L2

| Item | Priority | Status | Description |
|-----|----------|--------|-------------|
| ZK API | 1 | Pending | Improve prover API to unify multiple backends |
| Native Rollups | 1 | Pending | Add EXEC Precompile POC |
| Based Rollups | 1 | Pending | [Based Rollups Roadmap](https://docs.ethrex.xyz/l2/roadmap.html) |
| Zisk | 2 | In Progress | Integrate full Zisk Proving on the L2 |
| zkVMs | 2 | In Progress | Make GuestProgramState more strict when information is missing |



---

## SnapSync

| Item | Priority | Status | Description |
|-----|----------|--------|-------------|
| Download receipts and blocks | 1 | Pending | After snap sync is finished and the node is executing blocks, it should download all historical blocks and receipts in the background |
| Download headers in backgroud (no rewrite) | 1 | Pending | Download headers in backgroud |
| Avoid copying trie leaves when inserting (no rewrite) | 1 | Pending | Avoid copying trie leaves when inserting |
| Rewrite snapsync | 4 | Pending | Use Spawned for snapsync |

---

## UX / DX

| Item | Priority | Status | Description |
|-----|----------|--------|-------------|
| Improve internal documentation | 0 | In Progress | Improve interal docs for developers, add architecture. |
| Add Tests | 1 | In Progress | Improve coverage |
| Add Fuzzing | 1 | In Progress | Add basic fuzzing scenarios |
| Add Prop test | 1 | In Progress | Add basic property testing scenarios |
| Add security runs to CI | 1 | In Progress | Add fuzzing and every security tool we have to the CI |
| CLI Documentation| 1 | Pending | Review CLI docs and flags |
| API Documentation| 1 | Pending | Add API documentation to docs. Add compliance matrix |
| IPv6 support | 1 | Pending | IPv6 is not fully supported |
| P2P rate limiting | 3 | Pending | Improve scoring heuristic and DDoS protection |
| P2P leechers | 1 | Pending |  Improve scoring heuristic and kick leechers |
| Websocket subscriptions | 2 | Pending | Add subscription support for websocket |
| Not allow empty blocks in dev mode | 2 | Pending | For L2 development it's usefull not to have empty blocks |
| No STD | 5 | Pending | Support WASM target for some crates related to proving, and execution. Useful for dApps builders and light clients |
| Migrations | 4 | Pending | Add DB Migration mechanism for ethrex upgrades |
| geth db migration tooling | 0 | In Progress | As we don't support pre-merge blocks we need a tool to migrate other client's BD to ours at a specific block |
| Custom Deterministic Benchmark  | 1 | In Progress | We have a tool to run certain mainnet blocks, intregate that tool into our pipeline for benchmarking (not easy with DB changes) |
| Benchmark contract call & simple transfers | 1 | Pending | Create a new benchmark with contract call & simple transfers |
| Add MIT License | 0 | Pending | Add dual license |
| Improve Error handling | 1 | In Progress | Avoid panic and unwraps |

---

## New Features

| Item | Priority | Status | Description |
|-----|----------|--------|-------------|
| Block-Level Access Lists | 2 | In Progress | mplement [EIP-7928](https://eips.ethereum.org/EIPS/eip-7928) |
| Disc V5 | 2 | In Progress | Add discV5 Support |
| Sparse Blobpool  | — | Pending | Implement [EIP-8070](https://eips.ethereum.org/EIPS/eip-8070) |
| Pre merge blocks | — | Pending | Be able to process pre merge blocks |
| Archive node | — | Pending | Allow archive node mode |

