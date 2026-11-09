# Short– to Mid-Term Roadmap

This document represents the **short- to mid-term roadmap**.
Items listed here are actionable, concrete, and intended to be worked on in the coming weeks.
Long-term research directions and second-order ideas are intentionally out of scope.

**Priority reflects relative urgency, not effort.**

This is a WIP document and it requires better descriptions; it's supposed to be used internally.


---

## Priority Legend

| Priority | Meaning |
|---------:|---------|
| 0 | Highest priority, low effort with potential win |
| 1 | High. Should be addressed soon |
| 2 | Medium. Important but not blocking |
| 3 | Low. Useful improvement |
| 4 | Very low. Nice to have |
| 5 | Deprioritized for now |
| 6 | Long tail / hygiene |
| — | Not yet prioritized |


---

## Execution

| Item | Issue | Priority | Status | Description |
|------|-------|----------|--------|-------------|
| Replace BTreeMap with FxHashMap | #5757 | 0 | Discarded (small regression) | Replace BTreeMap/BTreeSet with FxHashMap/FxHashSet|
| Use FxHashset for access lists | #5800 | 0 | Done (8% improvement) | Replace HashSet with FxHashset |
| Skip Zero-Initialization in Memory Resize | #5755 | 0 | Measure #5774 | Use unsafe set_len (EVM spec says expanded memory is zero) |
| Remove RefCell from Memory  | #5756 | 0 | Measure #5793 | Consider using UnsafeCell with manual safety guarantees, or restructure to avoid shared ownership. |
| Try out PEVM | | 0 | Done. Simple integration caused regression. | Benchmark again against pevm |
| Inline Hot Opcodes | #5752 | 0 | Done. 0 to 20% speedup depending on the time. | Opcodes call a function in a jump table when some of the most used ones could perform better being inlined instead |
| Test ECPairing libraries | #5758 | 0 | Done (#5792). Used Arkworks. 2x speedup on those specific operations. | Benchmark arkworks pairing in levm|
| PGO/BOLT | #5759 | 0 | In progress (#5775) | Try out both [PGO](https://doc.rust-lang.org/beta/rustc/profile-guided-optimization.html) and [BOLT](https://github.com/llvm/llvm-project/tree/main/bolt) to see if we can improve perf |
| Use an arena allocator for substate tracking | #5754 | 0 | Discarded (#5791). Regression of 10% in mainnet. | Substates are currently a linked list allocated through boxing. Consider using an arena allocator (e.g. bumpalo) for them |
| ruint  | #5760 | 0 | Discarded simple approach. Regression. | Try out [ruint](https://github.com/recmo/uint) as the `U256` library to see if it improves performance. Part of SIMD initiative |
| Nibbles | #5801 | 1 | Measure #5912 and #5932 | Nibbles are currently stored as a byte (`u8`), when they could be stored compactly as actual nibbles in memory and reduce by half their representation size. Also we may stack-allocate their buffers instead of heap-allocated vecs. |
| RLP Duplication | #5949 | 1 | Pending | Check whether we are encoding/decoding something twice (clearly unnecessary) |
| Object pooling | #5934 | 2 | Pending | Reuse EVM stack frames to reduce allocations and improve performance |
| Avoid clones in hot path | #5753 | 2 | Measure #5809 on mainnet | Avoid Clone on Account Load and check rest of the hot path |
| SIMD Everywhere | | 2 | Pending | There are some libraries that can be replaced by others that use SIMD instructions for better performance |
| EXTCODESIZE without full bytecode | | 1 | Done (#6034). Improvement of 25%. | EXTCODESIZE loads entire bytecode just to get length. Add `get_account_code_size()` or store code length alongside code (`crates/vm/levm/src/opcode_handlers/environment.rs:260-274`) |
| TransactionQueue data structure | | 1 | Discarded. It is not significant within the critical path. | `TransactionQueue` uses `Vec` with `remove(0)` which is O(n). Replace with `BinaryHeap`/`BTreeSet` or `VecDeque` for O(log n) or O(1) operations (`crates/blockchain/payload.rs:708-820`) |

---

## IO

| Item | Issue | Priority | Status | Description |
|------|-------|----------|--------|-------------|
| Add Block Cache (RocksDB) | #5935 | 0 | Discarded. Did not improve (#6195). | Currently there is no explicit block cache, relying on OS page cache. Also try row cache |
| Use Two-Level Index (RocksDB) | #5936 | 0 | Discarded. Performance regressed by two orders of magnitude on both throughput and latency (#6196). | Use Two-Level Index with Partitioned Filters |
| Enable unordered writes for State (RocksDB) | #5937 | 0 | Pending | For `ACCOUNT_TRIE_NODES, STORAGE_TRIE_NODES cf_opts.set_unordered_write(true);` Faster writes when we don't need strict ordering|
| Increase Bloom Filter (RocksDB) | #5938 | 0 | Pending | Change and benchmark higher bits per key for state tables |
| Consider LZ4 for State Tables (RocksDB) | #5939 | 0 | Pending | Trades CPU for smaller DB and potentially better cache utilization |
| Page caching + readahead | #5940 | 0 | Pending | Use for trie iteration, sync operations |
| Optimize for Point Lookups (RocksDB) | #5941 | 0 | Pending | Adds hash index inside FlatKeyValue for faster point lookups |
| Modify block size (RocksDB) | #5942 | 0 | Pending | Benchmark different block size configurations |
| Memory-Mapped Reads (RocksDB) | #5943 | 0 | Pending | Can be an improvement on high-RAM systems |
| Increase layers commit threshold | #5944 | 0 | Pending | For read-heavy workloads with plenty of RAM |
| Remove locks | #5945 | 1 | Pending | Check if there are still some unnecessary locks, e.g. in the VM we have one |
| Benchmark bloom filter | #5946 | 1 | Pending | Review trie layer's bloom filter, remove it or test other libraries/configurations |
| Use multiget on trie traversal | #4949 | 1 | Pending | Using multiget on trie traversal might reduce read time |
| Bulk reads for block bodies | | 1 | Pending | Implement `multi_get` for `get_block_bodies` and `get_block_bodies_by_hash` which currently loop over per-key reads (`crates/storage/store.rs:388-454`) |
| Canonical tx index | | 1 | Pending | Transaction location lookup does O(k) prefix scans. Add a canonical-tx index table or DUPSORT layout for O(1) lookups (`crates/storage/store.rs:562-606`) |
| Reduce trie cache Mutex contention | | 1 | Pending | `trie_cache` is behind `Arc<Mutex<Arc<TrieLayerCache>>>`. Use `ArcSwap` or `RwLock` for lock-free reads (`crates/storage/store.rs:159,1360`) |
| Reduce LatestBlockHeaderCache contention | | 1 | Pending | `LatestBlockHeaderCache` uses Mutex for every read. Use `ArcSwap` for atomic pointer swaps (`crates/storage/store.rs:2880-2894`) |
| Use Bytes/Arc in trie layer cache | | 2 | Pending | Trie layer cache clones `Vec<u8>` values on every read. Use `Bytes` or `Arc<[u8]>` to reduce allocations (`crates/storage/layering.rs:57,63`) |
| Split hot vs cold data | | 2 | Pending | Geth "freezer/ancients" pattern: store recent state in fast KV store, push old bodies/receipts to append-only ancient store to reduce compaction pressure |
| Configurable cache budgets | | 2 | Pending | Expose cache split for DB/trie/snapshot as runtime config. Currently hardcoded in ethrex |
| Toggle compaction during sync | | 2 | Pending | Disable RocksDB compaction during snap sync for higher write throughput, then compact after (Nethermind pattern). Wire `disable_compaction/enable_compaction` into sync stages |
| Spawned | #5947 | 3 | Pending | [*Spawnify*](https://github.com/lambdaclass/spawned) io intensive components/flows. Mempool and Snapsync are top priorities |

---

## RPC

| Item | Priority | Status | Description |
|-----|----------|--------|-------------|
| Parallel tx decoding | 0 | Discarded | Use rayon to decode transactions in parallel. Currently sequential at ~5-10μs per tx |
| simd-json | 0 | Discarded | Replace serde_json with simd-json for SIMD-accelerated JSON parsing |
| Remove payload.clone() | 0 | Pending | Avoid cloning `ExecutionPayload` in `get_block_from_payload` (`crates/networking/rpc/engine/payload.rs:674`). Use references or owned values directly |
| Remove params.clone() | 0 | Pending | Avoid cloning params before `serde_json::from_value()`. Use references instead of `params[i].clone()` in RPC handlers (`crates/networking/rpc/engine/payload.rs`) |
| Use Bytes instead of String | 0 | Pending | Change HTTP body extraction from `String` to `Bytes` and use `serde_json::from_slice()` instead of `from_str()` to avoid UTF-8 validation overhead (`crates/networking/rpc/rpc.rs:536,563`) |
| RawValue for params | 1 | Pending | Use `Option<Vec<serde_json::value::RawValue>>` instead of `Option<Vec<Value>>` in `RpcRequest` to defer parsing until needed (`crates/networking/rpc/utils.rs:242`) |
| Parallel tx root | 1 | Pending | Parallelize `compute_transactions_root` which computes ~400 keccak256 hashes for 200 txs (`crates/blockchain/payload.rs:671`) |
| phf method routing | 2 | Pending | Replace match statements with `phf::Map` for O(1) RPC method dispatch instead of O(n) string comparisons (`crates/networking/rpc/rpc.rs:652-765`) |
| Pre-create JWT decoder | 2 | Pending | Cache `DecodingKey` and `Validation` at startup instead of creating them on every auth request (`crates/networking/rpc/authentication.rs:43-46`) |
| HTTP/2 support | 3 | Pending | Add HTTP/2 support for reduced latency through multiplexing |
| Direct response serialization | 3 | Pending | Serialize responses directly to the output buffer instead of intermediate Value |
| TCP tuning | 3 | Pending | Tune TCP settings (nodelay, buffer sizes) for lower latency |


---


## ZK + L2

| Item | Priority | Status | Description |
|-----|----------|--------|-------------|
| ZK API | 1 | Pending | Improve prover API to unify multiple backends |
| Native Rollups | 2 | Pending | Add EXEC Precompile POC |
| Based Rollups | 2 | Pending | [Based Rollups Roadmap](https://docs.ethrex.xyz/l2/roadmap.html) |
| Zisk | 2 | In Progress | Integrate full Zisk Proving on the L2 |
| zkVMs | 2 | In Progress | Make GuestProgramState more strict when information is missing |



---

## SnapSync

| Item | Priority | Status | Description |
|-----|----------|--------|-------------|
| Download receipts and blocks | 1 | Pending | After snap sync is finished and the node is executing blocks, it should download all historical blocks and receipts in the background |
| Download headers in background (no rewrite) | 1 | Pending | Download headers in background |
| Avoid copying trie leaves when inserting (no rewrite) | 1 | Pending | Avoid copying trie leaves when inserting |
| Rewrite snapsync | 4 | Pending | Use Spawned for snapsync |

---

## Testing & Verification

Proposed priorities, for the team to review. Plan, evidence and acceptance criteria: [testing roadmap](https://docs.ethrex.xyz/developers/l1/testing/roadmap.html). The coverage, fuzzing, prop test and CI security items stay under UX / DX.

| Item | Priority | Status | Description |
|-----|----------|--------|-------------|
| [Keep every bug as a reproducible test](https://docs.ethrex.xyz/developers/l1/testing/roadmap.html#keep-every-bug-as-a-reproducible-test) | 0 | In Progress (regression tests exist, not yet a rule) | Every `fix` PR lands with a test that fails without the fix; flaky reproductions get a seed or failpoint |
| [Soak tests with a resource budget](https://docs.ethrex.xyz/developers/l1/testing/roadmap.html#soak-tests-with-a-resource-budget) | 0 | Pending (`multisync-loop-auto` used ad hoc) | Nightly multi-hour run with gossip and peer churn; fail if RSS, fds or tasks grow after warm-up (the 1.5 GB/day P2P leak, #6900) |
| [Failpoints](https://docs.ethrex.xyz/developers/l1/testing/roadmap.html#failpoints-buggify) | 1 | Pending | BUGGIFY-style fault points (error, delay, panic, drop) in storage, sync and p2p, behind a feature that release builds never enable |
| [Differential testing against other clients](https://docs.ethrex.xyz/developers/l1/testing/roadmap.html#differential-testing-against-other-clients) | 1 | Pending (`trace_compare` manual; `difffuzz_*` vs reference models in CI) | Compare state/receipts roots, RPC responses and traces against geth and reth on the same blocks, nightly on Kurtosis |
| [Adopt spawned-test](https://docs.ethrex.xyz/developers/l1/testing/roadmap.html#adopt-spawned-test-after-spawned-v07) | 1 | Blocked on spawned v0.7 | Actor mocks and fault injection (stop, delay, timeout) in p2p/sync tests; shutdown through `ActorSystem` (#7491, #6823) |
| [Property tests against reference models](https://docs.ethrex.xyz/developers/l1/testing/roadmap.html#property-tests-against-executable-reference-models) | 2 | In Progress (proptest in trie and storage journal) | Trie, mempool and fork choice checked against small executable models on generated operation sequences. Extends "Add Prop test" |
| [RPC history checking](https://docs.ethrex.xyz/developers/l1/testing/roadmap.html#history-and-linearizability-checking) | 2 | Pending | Record RPC histories during reorg simulator and load runs, check them Porcupine-style against one sequence of canonical heads; pair with `history_validator` |
| [Metamorphic storage tests](https://docs.ethrex.xyz/developers/l1/testing/roadmap.html#metamorphic-tests-for-storage) | 2 | Pending | Same operations through several store configurations (backend, cache sizes, layer thresholds, restart) must give the same roots and reads |
| [Chaos runs with Attacknet](https://docs.ethrex.xyz/developers/l1/testing/roadmap.html#chaos-on-devnets-attacknet) | 2 | Pending | Chaos Mesh faults (partition/heal, crashes, clock skew) on Kurtosis enclaves with assertoor checks; complements the flaky `deep_reorg` harness |
| [Invariant monitors on logs](https://docs.ethrex.xyz/developers/l1/testing/roadmap.html#production-invariant-monitors) | 3 | Pending | PObserve-style checks of actor lifecycle, peer table (#7435) and sync phase invariants over CI and devnet logs |
| [p2p deterministic simulation](https://docs.ethrex.xyz/developers/l1/testing/roadmap.html#deterministic-simulation-dst) | 3 | Pending | Turmoil/madsim-style harness: several nodes and adversarial peers in one process with simulated network and clock for RLPx, discovery and snap sync. Needs injectable time and RNG |
| [Actor ordering exploration](https://docs.ethrex.xyz/developers/l1/testing/roadmap.html#actor-level-ordering-exploration) | 4 | Blocked on spawned `explore` | Search message orderings among peer table, connection and sync actors; #7491 and #7435 as acceptance tests |
| [ethshadow scale runs](https://docs.ethrex.xyz/developers/l1/testing/roadmap.html#whole-system-simulators-ethshadow-and-antithesis) | 4 | Pending | Run ethrex in ethshadow (Shadow simulator) at 100+ nodes for gossip and peer behaviour |
| [Antithesis evaluation](https://docs.ethrex.xyz/developers/l1/testing/roadmap.html#whole-system-simulators-ethshadow-and-antithesis) | 4 | Pending | Pilot one faulted scenario (snap sync plus reorg) in Antithesis' deterministic hypervisor, then decide |
| [Process-level shutdown tests](https://docs.ethrex.xyz/developers/l1/testing/roadmap.html#process-level-shutdown-tests) | 0 | Pending | Start the binary, send SIGTERM/SIGINT, fail if exit exceeds a deadline under Docker's 10 s or is non-zero |
| [Single-CPU runs](https://docs.ethrex.xyz/developers/l1/testing/roadmap.html#single-cpu-runs) | 1 | Pending | Suite and a short sync with one worker thread pinned to one core; exposes ordering and starvation assumptions |
| [Leak tests and LeakSanitizer](https://docs.ethrex.xyz/developers/l1/testing/roadmap.html#leak-tests-and-leaksanitizer) | 1 | Pending | Resources back to baseline after shutdown, on every PR; LSan job |
| [Cancellation-safety tests](https://docs.ethrex.xyz/developers/l1/testing/roadmap.html#cancellation-safety-tests) | 2 | Pending | Drop each async future at each await point; peer table and request paths stay consistent |
| [Panic-safety tests](https://docs.ethrex.xyz/developers/l1/testing/roadmap.html#panic-safety-tests) | 2 | Pending | Panic injected into each handler; node keeps its invariants (with spawned v0.7 supervision) |
| [Fairness under load](https://docs.ethrex.xyz/developers/l1/testing/roadmap.html#fairness-and-starvation-under-load) | 2 | Pending | Queued requests never wait beyond a bound under sync and gossip load (#6823) |
| [Clock-jump tests](https://docs.ethrex.xyz/developers/l1/testing/roadmap.html#clock-jump-tests) | 2 | Pending | Paused time and wall-clock jumps over slot timing, timeouts and peer scoring |
| [Allocation budgets, Kani, flaky tracking, open-loop load](https://docs.ethrex.xyz/developers/l1/testing/roadmap.html#smaller-additions) | 3 | Pending | Smaller additions: allocation counts on hot paths, bounded proofs for small functions, flake quarantine, coordinated-omission-free load tests |

---

## UX / DX

| Item | Priority | Status | Description |
|-----|----------|--------|-------------|
| Improve internal documentation | 0 | In Progress | Improve internal docs for developers, add architecture |
| geth db migration tooling | 0 | In Progress | As we don't support pre-merge blocks we need a tool to migrate other client's DB to ours at a specific block |
| Add MIT License | 0 | Pending | Add dual license |
| Add Tests | 1 | In Progress | Improve coverage |
| Add Fuzzing | 1 | In Progress | Add basic fuzzing scenarios |
| Add Prop test | 1 | In Progress | Add basic property testing scenarios |
| Add security runs to CI | 1 | In Progress | Add fuzzing and every security tool we have to the CI |
| CLI Documentation| 1 | Pending | Review CLI docs and flags |
| API Documentation| 1 | Pending | Add API documentation to docs. Add compliance matrix |
| IPv6 support | 1 | Pending | IPv6 is not fully supported |
| P2P leechers | 1 | Pending |  Improve scoring heuristic and kick leechers |
| Custom Deterministic Benchmark | 1 | In Progress | We have a tool to run certain mainnet blocks, integrate that tool into our pipeline for benchmarking (not easy with DB changes) |
| Benchmark contract call & simple transfers | 1 | Pending | Create a new benchmark with contract call & simple transfers |
| Improve Error handling | 1 | In Progress | Avoid panic, unwrap and expect |
| Websocket subscriptions | 2 | Pending | Add subscription support for websocket |
| Not allow empty blocks in dev mode | 2 | Pending | For L2 development it's useful not to have empty blocks |
| P2P rate limiting | 3 | Pending | Improve scoring heuristic and DDoS protection |
| Migrations | 4 | Pending | Add DB Migration mechanism for ethrex upgrades |
| No STD | 5 | Pending | Support WASM target for some crates related to proving and execution. Useful for dApp builders and light clients |

---

## New Features

| Item | Priority | Status | Description |
|-----|----------|--------|-------------|
| Block-Level Access Lists | 2 | Done | Implement [EIP-7928](https://eips.ethereum.org/EIPS/eip-7928) |
| Disc V5 | 2 | In Progress | Add discV5 Support |
| Sparse Blobpool  | — | Pending | Implement [EIP-8070](https://eips.ethereum.org/EIPS/eip-8070) |
| Pre merge blocks | — | Pending | Be able to process pre merge blocks |
| Archive node | — | Pending | Allow archive node mode |
