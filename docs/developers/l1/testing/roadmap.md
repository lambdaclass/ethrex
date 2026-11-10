# Testing and verification roadmap

Our suites check that ethrex computes the right answers. Our worst recent bugs came from what happens over time, under faults and across actors: leaks, shutdown races, stalls and deadlocks. This page plans how to test for those failures, with methods that database teams have already used in production. Priorities in the [roadmap](../../../roadmap.md#testing--verification) are proposals for the team to review.

**First step, highest return:** one deterministic test per past incident ([188ac1331](https://github.com/lambdaclass/ethrex/commit/188ac1331), [63167661a](https://github.com/lambdaclass/ethrex/commit/63167661a), [fc44a9fb2](https://github.com/lambdaclass/ethrex/commit/fc44a9fb2), [d958dcba4](https://github.com/lambdaclass/ethrex/commit/d958dcba4)) driven by [`fail`](https://github.com/tikv/fail-rs) failpoints, a SIGTERM process test, and a soak run that would have caught [d0b9fecb9](https://github.com/lambdaclass/ethrex/commit/d0b9fecb9). None of it needs spawned work.

**Prerequisite:** ethrex depends on spawned 0.5.0 (`Cargo.toml:147-148`). spawned [v0.7.0](https://github.com/lambdaclass/spawned/commit/02b9400b) is released and brings `ActorSystem`, supervision and the `spawned-test` crate; everything here that needs them waits for ethrex to upgrade. ethlambda depends on ethrex's `ethrex-p2p` crate, so once ethlambda moves to v0.7 its binary links both spawned 0.5 and 0.7 until ethrex upgrades too. The upgrade is itself a testing item. Spawned's side of the plan: [database-grade testing program](https://github.com/lambdaclass/spawned/blob/main/ROADMAP.md#database-grade-testing-program).

## What ethrex has today

| Suite | What it catches | Deterministic? | Where it runs |
|---|---|---|---|
| Rust unit and integration tests (`test/`, crate tests) | Logic errors in EVM, trie, storage, RPC, p2p codecs, sync scheduling; regression tests for past bugs (`p2p/rlpx/duplicate_connection_tests.rs`, `p2p/sync_race_tests.rs`, `storage/fcu_race_tests.rs`, `blockchain/deep_reorg_state_tests.rs`, `rpc/context_teardown_tests.rs` under `test/tests/`) | Mostly; race tests run real multi-thread runtimes and can be flaky | `pr-main_l1.yaml`, every PR |
| Differential fuzzers (`difffuzz_*`) | LEVM opcodes and BLS/BN254/crypto precompiles that disagree with independent reference models | Yes (seeded); long runs use `DIFFFUZZ_SECS`/`DIFFFUZZ_SEED` | Quick mode in every PR (dev and release); long mode manual |
| proptest | Trie iteration, range proofs and the storage journal on generated inputs | Yes (seeded, shrinking) | Every PR, as part of the Rust tests |
| [EF state and blockchain tests](./ef-tests.md), Engine EF tests | Consensus-rule and EVM divergence from the execution specs | Yes | Every PR |
| [Hive](./hive.md) | RPC compatibility, devp2p eth, Engine API, sync, consume-engine/RLP fixtures | Mostly | Subset every PR (the `ethereum/sync` suite is commented out in `pr-main_l1.yaml`, "Investigate this test"); full suite daily (`daily_hive_report.yaml`) |
| [Assertoor](./assertoor.md) on Kurtosis | Transactions, blobs and chain stability on a small multi-client devnet | No | Every PR |
| Reorg simulator (`tooling/reorgs`) | Deep reorgs (beyond 128 blocks) applied through `engine_newPayload` and `forkchoiceUpdated` | Yes (scripted engine calls) | Every PR; the network-partition variant (`deep_reorg.yaml`) is manual and flaky |
| Snap sync to head (`daily_snapsync.yaml`) | Snap sync regressions on public networks with Lighthouse and Prysm | No | Every 6 hours |
| `multisync-loop-auto` (`tooling/sync`) | Repeated syncs on several networks with debug assertions; used to soak fixes | No | Manual, on a dedicated server |
| `history_validator` | Stored tx/receipt roots that no longer match headers (bit rot, bad compaction) | Yes | Manual, on a datadir snapshot |
| `trace_compare` | `debug_traceTransaction` output that differs from geth/besu | Yes per tx | Manual, local Kurtosis |
| [Load](./load-tests.md) and [RPC load](./rpc-load-tests.md) tests, block benchmarks | Throughput and latency regressions | No | Manual, PR benchmarks |

Nothing on this list injects faults into ethrex itself, checks that memory/fds/tasks stay flat over hours, records RPC histories for checking, or controls message ordering between actors. The workspace uses no sanitizers, Loom, Kani, `fail` crate, madsim/turmoil or `tokio::time::pause`.

## Gaps, from real incidents

These come from the [spawned client sweep](https://github.com/lambdaclass/spawned/blob/main/ROADMAP.md#client-sweep-production-pain) of ethrex. Each one either reached running nodes or showed up only as an intermittent CI failure; no suite above caught it deterministically.

| Incident | Fix | Missing test |
|---|---|---|
| Heap grew ~1.5 GB/day: unbounded per-connection actor mailboxes, no handshake timeout, no inbound cap, maps never pruned | [d0b9fecb9](https://github.com/lambdaclass/ethrex/commit/d0b9fecb9) (#6900) | Soak test with a resource budget; slow-peer fault in p2p |
| Teardown dropped the `PeerTable` actor while sync still ran; `ActorStopped` led to `process::exit(2)`, 6 of 25 failed L1 CI runs | [188ac1331](https://github.com/lambdaclass/ethrex/commit/188ac1331) (#7491) | Shutdown-order fault injection; actor ordering exploration |
| A timed-out `get_best_peer` under mailbox pressure exited the node mid-sync | [63167661a](https://github.com/lambdaclass/ethrex/commit/63167661a) (#6823) | Failpoint that delays the peer table; a mock that times out |
| A connection actor cannot be stopped while its handler is wedged in `.await` (`rlpx/connection/server.rs`) | Bounded send timeout in #6900 | Wedged-peer simulation; stall detection |
| `GetCells` served synchronously on connection tasks and could occupy every runtime worker | [d958dcba4](https://github.com/lambdaclass/ethrex/commit/d958dcba4) (#7404) | Adversarial-peer scenario that checks RPC latency stays bounded |
| Per-connection L2 imports split the merkle pool and deadlocked at 0% CPU, freezing RPC and logs | [535647e45](https://github.com/lambdaclass/ethrex/commit/535647e45) (#7429) | Liveness check under concurrent duplicate sessions |
| A refused duplicate connection's teardown removed the live peer entry (`net_peerCount` 0) | [fc44a9fb2](https://github.com/lambdaclass/ethrex/commit/fc44a9fb2) (#7435) | Covered now by `duplicate_connection_tests`; generally, cleanup-vs-replacement races under exploration |
| Data written before a graceful shutdown was not flushed | [38c03606c](https://github.com/lambdaclass/ethrex/commit/38c03606c) (#6944) | SIGTERM process test that reopens the datadir and checks the head |
| A database whose head lagged an interrupted full-sync batch did not recover | [2cb18b0b9](https://github.com/lambdaclass/ethrex/commit/2cb18b0b9) (#7260) | Failpoint that aborts mid-batch, then restart and resume |
| The syncer lock was held while waiting for a forkchoice head (sync wedge) | [baee6e584](https://github.com/lambdaclass/ethrex/commit/baee6e584) (#6924) | Stall detection in subsystem simulation; sync state machine model |
| Snap sync pivoted on a guessed block number instead of the forkchoice head | [214f1e7f5](https://github.com/lambdaclass/ethrex/commit/214f1e7f5) (#7364) | Reference model of pivot selection |
| Full sync cycles wasted on the ePBS FCU/newPayload ordering race | [46ff092a1](https://github.com/lambdaclass/ethrex/commit/46ff092a1) (#7022) | `sync_race_tests` exists on a real runtime; a failpoint that forces each ordering makes it deterministic |
| Risk, not an incident: deferred replies built by hand (`outgoing_request`/`current_requests` with `request_timeout`) | — | Every request gets exactly one reply, including on peer drop and timeout |

The common shape: bugs appear in orderings, timeouts and resource use over hours, not in single calls. They need faults, time and control over scheduling.

## What blocks simulation today

Paths are under `crates/`, as of main `1dfaa09ec`.

| Concern | State in ethrex |
|---|---|
| Clock | No clock trait. `networking/` has 53 `Instant::now`, 44 `SystemTime::now` and 46 `tokio::time` occurrences; wall-clock helpers in `networking/p2p/utils.rs:16,23,30` and `peer_table.rs:718` |
| Randomness | Not injectable: `rand::thread_rng` in `rlpx/connection/handshake.rs:202,222`; `rand::random`/`OsRng` in `peer_table.rs:1368,1433,1681` and `snap/client.rs:500` |
| Network | Concrete sockets: `UdpSocket::bind` and `TcpListener` in `p2p/network.rs:176,250`, `TcpStream` in `handshake.rs:187` and `rlpx/connection/server.rs:194`. The only seam is the RLPx codec, generic over `AsyncRead + AsyncWrite` (`codec.rs:297`) |
| Storage | Already a seam: `StorageBackend` trait (`storage/api/mod.rs:69`) with `InMemoryBackend` and `RocksDBBackend`; `TrieDB` trait (`common/trie/db.rs:31`) |
| Hard exits | `process::exit(2)` in `p2p/sync.rs:286`; `process::exit(1)` in `p2p/sync/snap_sync.rs:1246,1274,1299`. A simulated failure kills the test process |
| Tuning | Sync constants are `lazy_static` and read from env vars under the `sync-test` feature (`p2p/sync.rs:54`, `snap/constants.rs:249`) |

So whole-node DST is large: clock, RNG and transport all need injection first. Subsystem simulation is medium: `PeerTable` plus the sync manager with mocked peer connections, the in-memory store and paused tokio time. That scope would have caught 188ac1331, 63167661a and fc44a9fb2. The `exit` calls must become errors before either runs in-process.

## Techniques

Each technique below lists what it is, who uses it, the evidence, how it applies to ethrex and the effort.

### Failpoints (BUGGIFY)

- **What:** named fault points in production code (`fail_point!("sync::before_commit")`) that tests turn on to return errors, delay, panic or drop. They compile to nothing unless a feature is enabled.
- **Who:** FoundationDB's [BUGGIFY](https://transactional.blog/simulation/buggify) fires only in simulation, enables each site randomly per run and fires it 25% of the time; etcd's [robustness tests](https://github.com/etcd-io/etcd/tree/main/tests/robustness) use gofail failpoints; TiKV uses the Rust [`fail`](https://github.com/tikv/fail-rs) crate.
- **ethrex:** first the sites the incidents point at: peer-table teardown and request handling (#7491, #6823, #7435), snap batch commit, FCU versus newPayload ordering (#7022), store flush on shutdown (#6944), and an interrupted full-sync batch (#7260). Then RLPx send/receive and handshake, healing and trie flush. Use the `fail` crate behind a `failpoints` cargo feature that release builds never enable.
- **Effort:** small. One crate, a feature flag and a dozen sites; each site pays off as soon as one test uses it.

### Soak tests with a resource budget

- **What:** run a node for hours under steady gossip and peer churn and assert that RSS, open fds, tokio tasks, actor count and mailbox depths stay flat after warm-up.
- **Who:** standard practice; the [spawned roadmap](https://github.com/lambdaclass/spawned/blob/main/ROADMAP.md#testing-to-add) lists it with the ethrex leak as the motivating bug.
- **ethrex:** reuse `multisync-loop-auto` or a Kurtosis devnet with a tx spammer; scrape the existing Prometheus metrics; fail on growth slope above a threshold. Nightly, not per PR.
- **Effort:** small to medium; mostly a script and a dashboard threshold.

### Keep every bug as a reproducible test

- **What:** every fixed bug lands with a test that fails before the fix. Flaky reproductions get a seed or a failpoint until they are deterministic.
- **Who:** etcd's [robustness README](https://github.com/etcd-io/etcd/tree/main/tests/robustness) keeps a track record of past correctness bugs with reproduction targets (`make test-robustness-issue14370`) and re-runs them after large changes to the test framework; FoundationDB and Antithesis replay failures from seeds.
- **ethrex:** already done in places (`duplicate_connection_tests`, `fcu_race_tests`, `sync_race_tests`, `deep_reorg_state_tests`). Make it the PR rule for `fix(...)` PRs and note the reproducing test in the PR body.
- **Effort:** process only.

### Differential testing against other clients

- **What:** feed the same input to ethrex and another implementation, compare outputs.
- **Who:** ethrex's own `difffuzz_*` does this against independent models; Hive does it against expected fixtures.
- **ethrex:** extend `trace_compare` into a scripted job on a multi-client Kurtosis enclave: for the same blocks compare state roots, receipts roots, `eth_getProof`, `eth_call`, `eth_getLogs`, `debug_traceTransaction` against geth and reth. Also replay mainnet block ranges and compare post-state roots per block with another client's.
- **Effort:** medium; the enclave and comparison script exist.

### Adopt `spawned-test` (after spawned v0.7)

- **What:** the [`spawned-test`](https://github.com/lambdaclass/spawned/blob/main/ROADMAP.md#testing-crate-spawned-test) crate: `ActorRef` mocks for any protocol, recording probes, bounded waits and fault injection (kill or restart an actor, delay or drop a message, fire a timeout).
- **ethrex:** replace real actors with dummy contexts in p2p and sync tests; mock `PeerTableServer` to time out, stop early or answer slowly; test shutdown orders with the v0.7 `ActorSystem`. Needs the move from spawned 0.5 to 0.7 (v0.7.0 is released; ethrex has not upgraded).
- **Effort:** small per test once on v0.7.

### History and linearizability checking

- **What:** record concurrent client operations with start/end times and results, then check the history against a sequential model. [Porcupine](https://github.com/anishathalye/porcupine) checks linearizability; [Jepsen](https://jepsen.io/)'s [Elle](https://github.com/jepsen-io/elle) checks transactional isolation.
- **Who:** etcd's robustness tests check each run's history with Porcupine while failpoints and network faults fire; running them under Antithesis also [found a flaw in the checker model](https://etcd.io/blog/2025/autonomus_testing_with_antithesis/).
- **ethrex:** the RPC view of the chain should be explainable by one sequence of canonical heads: no `eth_getBlockByNumber("latest")` going backwards except across a reorg reported by the engine, `eth_getTransactionReceipt` consistent with the block that contains it, `eth_getBalance` at a block hash never changing. Record histories during the reorg simulator, load tests and Hive, check them offline. Pair with `history_validator` on the datadir after the run.
- **Effort:** medium; the model is ethrex-specific.

### Property tests against executable reference models

- **What:** a small, obviously-correct model (a `BTreeMap` for the trie, a sorted list for the mempool) runs alongside the real implementation on generated operation sequences; any difference fails and shrinks.
- **Who:** Amazon S3's ShardStore (Rust) [prevented 16 issues from reaching production](https://www.amazon.science/publications/using-lightweight-formal-methods-to-validate-a-key-value-storage-node-in-amazon-s3) with executable reference models, property tests, stateless model checking and crash-consistency checks.
- **ethrex:** peer table (add, refuse duplicate, remove; the #7435 bug) versus a set; storage journal and trie layering versus a `BTreeMap`; trie (insert/remove/commit/proof) versus a map plus a reference root; snap pivot selection versus "the forkchoice head" (#7364); mempool ordering/replacement/eviction; fork choice versus a model of the block tree. Extends the existing proptest use and the "Add Prop test" item.
- **Effort:** medium; each model is a few hundred lines.

### Metamorphic tests for storage

- **What:** run the same random operation sequence through several configurations that must agree, compare the results.
- **Who:** CockroachDB's Pebble runs its [metamorphic test](https://www.cockroachlabs.com/blog/metamorphic-testing-the-database/) nightly for about 6 hours per run; it has found bugs that would have corrupted or lost data.
- **ethrex:** start with `InMemoryBackend` versus `RocksDBBackend` on the same blocks (the `StorageBackend` trait makes this cheap). Then: reorg away and back to the same head gives the same root; full sync and snap sync to the same block give the same state; different cache sizes, layer commit thresholds, flat key-value on or off, and a restart in the middle. State roots and reads must match.
- **Effort:** medium; ties into the RocksDB tuning items in the IO section, which change exactly these settings.

### Chaos on devnets: Attacknet

- **What:** [Attacknet](https://ethpandaops.io/posts/attacknet-introduction) (ethpandaops and Trail of Bits) deploys a Kurtosis network and injects faults with Chaos Mesh: latency, packet loss, splits, clock skew, CPU/RAM/disk stress and crashes. It was used to test edge cases for the Dencun fork.
- **ethrex:** run Attacknet scenarios against ethrex-heavy enclaves (snap sync while peers drop, reorg after a partition heals, restart under load) with assertoor health checks. Fixes the reproducibility problem of the current network-partition harness only partially: runs remain non-deterministic.
- **Effort:** medium; needs a Kubernetes runner.

### Production invariant monitors

- **What:** check invariants from logs or metrics of real runs. AWS's [PObserve](https://p-org.github.io/P/advanced/pobserve/pobserve) checks service logs from tests or production against P specifications; Brooker and Desai describe it alongside TLA+, P, fuzzing and fault injection (*Systems Correctness Practices at Amazon Web Services*, CACM 68(6), 2025).
- **ethrex:** emit structured events for actor start/stop and peer connect/disconnect, sync phase transitions and canonical-head changes; a checker asserts invariants over the stream: each live connection is listed exactly once in the peer table (the #7435 bug), a peer slot is released after disconnect, sync phases follow the allowed order, the head only moves through forkchoice. Run it on CI logs first, on devnet nodes next.
- **Effort:** medium. MongoDB [found trace checking impractical](https://arxiv.org/abs/2006.00915) against an abstract spec retrofitted to an existing system, while generating tests from the spec worked; keep the invariants concrete and close to the code.

### Deterministic simulation (DST)

- **What:** run several nodes in one process with a simulated network, clock and randomness from one seed; inject faults; every failure replays from its seed.
- **Who:** [FoundationDB](https://apple.github.io/foundationdb/testing.html) simulates whole clusters in a single-threaded process and runs tens of thousands of simulations nightly ([Will Wilson's talk](https://thestrangeloop.com/2014/testing-distributed-systems-w-slash-deterministic-simulation.html)). TigerBeetle's [VOPR](https://docs.tigerbeetle.com/concepts/safety/) runs the real code at 1000x speed on 1024 cores around the clock, and its [protocol-aware DST](https://tigerbeetle.com/blog/2026-08-20-protocol-aware-dst) checks per-replica invariants from inside. Dropbox's Nucleus (Rust) [runs the sync engine as a future under a custom executor](https://dropbox.tech/infrastructure/-testing-our-new-sync-engine) with mocked filesystem, network and timers, tens of millions of seeded runs a night. In async Rust, RisingWave built [madsim](https://github.com/madsim-rs/madsim), [S2](https://s2.dev/blog/dst) uses turmoil plus libc overrides for time and entropy and checks determinism by rerunning a seed and diffing logs, and [Polar Signals](https://blog.polarsignals.com/blog/posts/2025/07/08/dst-rust) designed its database for it.
- **ethrex, subsystem first:** `PeerTable` plus the sync manager in one test process, with mocked peer connections (spawned-test mocks, or scripted peers over the RLPx codec), the in-memory store and paused tokio time. Seeds the scheduling and peer behaviour; does not yet control `SystemTime` or `rand`. Medium effort; catches the #7491, #6823 and #7435 class (see [What blocks simulation today](#what-blocks-simulation-today)).
- **ethrex, whole node:** a turmoil-style harness for p2p. Several ethrex nodes plus scripted adversarial peers in one process, simulated TCP/UDP and clock: RLPx handshake and duplicate connections, discv4/discv5 under loss, snap sync with slow, lying and disappearing peers, peer churn while the node shuts down. Invariants: no panics or exits, sync completes or reports, mailbox depth bounded, every request answered. Needs injectable clock, RNG and transport first.
- **Effort:** subsystem medium; whole node large, mostly the injection work.

### Actor-level ordering exploration

- **What:** control which actor message is delivered next and search orderings (Coyote-style), with failures replayed as a delivery list. Planned in spawned as `explore` and crash tapes ([ranks 20-22](https://github.com/lambdaclass/spawned/blob/main/ROADMAP.md#step-6-the-testing-flagship)).
- **ethrex:** the peer table, connection, sync manager and broadcaster actors. The `ActorStopped` teardown race (#7491) and the #7435 cleanup race are the acceptance tests.
- **Effort:** small for ethrex once spawned ships it; depends on spawned phases 4-5.

### Whole-system simulators: ethshadow and Antithesis

- **ethshadow:** [ethshadow](https://ethereum.github.io/ethshadow) runs real clients on the Shadow discrete-event network simulator with regional latencies; its documented support is geth and Lighthouse; whether ethrex runs under it is uncertain. Useful for scale (hundreds of nodes) and gossip bandwidth, after adding ethrex as a supported client.
- **Antithesis:** a deterministic hypervisor that runs containers, injects faults and searches for property violations. etcd ran its robustness tests in it for 830 hours ("4.5 years of usage"), found a watch bug in all stable releases and rediscovered every planted bug ([etcd blog](https://etcd.io/blog/2025/autonomus_testing_with_antithesis/)). WarpStream [runs its entire SaaS](https://warpstream.com/blog/deterministic-simulation-testing-for-our-entire-saas) in it. Antithesis [says](https://antithesis.com/case_studies/ethereum_merge) the Ethereum Foundation used it to test the Merge (vendor claim). It needs no code changes beyond assertions, so it is the fastest route to deterministic whole-node testing, at a licence cost.
- **Effort:** ethshadow medium; Antithesis is an evaluation first (a pilot on one scenario: snap sync plus reorg with faults).

### Process-level shutdown tests

- **What:** start the real `ethrex` binary, let it sync or serve for a while, send SIGTERM and SIGINT, and fail if it does not exit cleanly within a deadline (well under Docker's 10 s stop timeout), or if it exits with a non-zero code.
- **Why ethrex:** the racy Ctrl-C shutdown (#7491) and `exit(2)` on `ActorStopped` were found by intermittent CI failures, not by a test aimed at shutdown. Ethlambda's 14.3 s shutdown was found by operators.
- **Effort:** small; one CI job, can reuse the dev-mode node.

### Single-CPU runs

- **What:** run the test suite and a short sync with one Tokio worker thread and the process pinned to one core (`taskset -c 0`).
- **Why ethrex:** hidden assumptions about ordering and starvation show up immediately; synchronous cell serving occupying every runtime worker (#7404) is this class of bug.
- **Effort:** small; a CI matrix entry.

### Cancellation-safety tests

- **What:** for async p2p, sync and RPC code, drop each future at each of its await points in turn (a poll-counting wrapper) and check that state stays consistent: no leaked permits, no half-updated peer table, no stuck request ids.
- **Why ethrex:** request timeouts and peer disconnects cancel futures mid-flight constantly; dropping halfway is the commonest async Rust bug class and nothing tests it systematically.
- **Effort:** medium; start with the peer table and request/response paths.

### Panic-safety tests

- **What:** inject a panic into each actor callback and handler in turn (and into message `Drop` impls) and check the node keeps its invariants: supervised actors restart, registries and the peer table drop the dead entry, no other actor wedges.
- **Why ethrex:** one panicking handler should never take down sync or RPC; with spawned v0.7 supervision this becomes checkable.
- **Effort:** medium, with `spawned-test` fault injection after spawned v0.7.

### Leak tests and LeakSanitizer

- **What:** after shutdown of a test node, every actor, task, thread and connection is gone and resource counts return to baseline; a LeakSanitizer CI job runs the same tests.
- **Why ethrex:** the 1.5 GB/day heap growth (#6900) was found in production; soak tests catch growth slowly, leak tests catch it on every PR.
- **Effort:** small to medium.

### Clock-jump tests

- **What:** paused time, large forward and backward wall-clock jumps (NTP corrections), and timer-driven paths (slot timing, engine API timeouts, peer scoring decay, the ~14 actors that reschedule with `send_after`).
- **Why ethrex:** consensus-facing timing and timeouts are wall-clock sensitive; these paths have no clock tests today.
- **Effort:** medium; easier once time comes from an injectable clock (Phase 3).

### Fairness and starvation under load

- **What:** under sync and gossip load, no actor's queued requests wait beyond a bound, and control traffic (shutdown, peer table reads) keeps moving behind a data backlog.
- **Why ethrex:** a `get_best_peer` request timed out under mailbox pressure and exited the node (#6823).
- **Effort:** medium; builds on spawned metrics (queue wait) and the soak harness.

### Smaller additions

- **Allocation budgets (dhat):** fail when processing a block, transaction or message allocates more than a set count; keeps hot paths from regressing silently.
- **Kani proofs:** bounded proofs of no overflow and no panic for small critical functions: RLP length prefixes, gas arithmetic, blob fee math.
- **CI coverage already written:** schedule `deep_reorg.yaml` nightly (it is `workflow_dispatch` only) and re-enable the Hive `ethereum/sync` suite in `pr-main_l1.yaml` if it can be made stable.
- **Flaky-test tracking:** record reruns and quarantine flaky tests instead of retrying them green; concurrency flakes are real bugs (#7491 failed 6 of 25 runs).
- **Open-loop load generation:** load tests issue requests on a schedule, not after each reply, so tail latency is not understated (coordinated omission).

### Specs (TLA+/P)

Only for the sync state machine (pivot choice, syncer lock, full/snap transitions; #6924 and #7364), effort medium to large, after subsystem simulation. Not for the rest of ethrex. AWS uses TLA+ and P on core protocols, and the Cosmos DB [TLA+ model](https://arxiv.org/abs/2210.13661) of user-visible consistency fixed documentation and explained an outage. For ethrex the consensus rules are already specified by the execution specs and EF tests, and the actor lifecycle is modelled in spawned under its [verification rule](https://github.com/lambdaclass/spawned/blob/main/ROADMAP.md#verification-rule). Extend to snap-sync healing only if failpoints and simulation keep finding bugs there.

## Shared with ethlambda

ethlambda is drafting its own plan (lambdaclass/ethlambda, branch `docs/testing-roadmap`). Shared: the storage testing pattern (backend trait, in-memory versus RocksDB); the discv5 `DiscoveryServer`, which ethlambda uses from `ethrex-p2p`, so its tests serve both; and the spawned harness (`spawned-test`, lifecycle monitors, failpoint naming, a nightly seed runner), which can be one harness. Not shared: a whole-node simulator, since ethrex speaks devp2p and ethlambda libp2p.

## Phased plan

### Phase 1: now, on spawned 0.5

| Item | Acceptance criteria |
|---|---|
| Reproducible-bug policy | PR template asks `fix(...)` PRs for the test that fails without the fix; the PR body names it. |
| Failpoints and incident tests | `fail` crate behind a `failpoints` feature; a release build contains no failpoint symbols. One deterministic test each for #7491, #6823, #7435, #7404, #6944, #7260 and #7022, each failing when its fix is reverted (#6823: the peer table delays past the timeout during sync and the node exits). |
| Process-level shutdown test | CI starts the node, sends SIGTERM and SIGINT, and fails if exit takes longer than 8 s or returns non-zero; reopening the datadir shows the last head (#6944). |
| Soak test | Nightly 6-hour run with gossip and peer churn; fails if RSS, fds or task count grow by more than a set slope after warm-up. Reverting #6900's bounded mailbox makes it fail within the run. |
| Existing CI coverage | `deep_reorg.yaml` runs nightly; the Hive `ethereum/sync` suite is re-enabled or its failure is filed. |
| Differential job | Scripted `trace_compare` plus state/receipts roots and five RPC methods against geth and reth for N blocks on Kurtosis; runs nightly; any mismatch fails with a diff. |
| Property tests with reference models | Peer table, journal/layering, trie, snap pivot and mempool models in `test/`; reverting #7435 or #7364 fails them. |
| Metamorphic storage, first step | `InMemoryBackend` and `RocksDBBackend` agree on roots and reads for the same blocks, and for reorg-then-re-reorg to the same head. |
| Single-CPU run | Tests and a short sync pass with one worker thread under `taskset -c 0`. |
| Leak test | After a test node shuts down, actor, task, thread and fd counts are back to baseline; LeakSanitizer job runs the same tests. |

### Phase 2: after ethrex upgrades to spawned v0.7

RPC history checking, the rest of the metamorphic test and Attacknet do not depend on spawned and can start earlier; they are here for capacity.

| Item | Acceptance criteria |
|---|---|
| Upgrade to spawned v0.7, shutdown via `ActorSystem` | Ctrl-C stops every actor within a deadline; no `exit(2)` on `ActorStopped` during shutdown. Reintroducing the #7491 teardown order fails a `spawned-test` shutdown test every run, not 5 in 3600. |
| `spawned-test` in p2p and sync tests | Mocked `PeerTableServer` replaces dummy contexts; tests for timeout, early stop and slow replies on every request the sync path makes. |
| Cancellation and panic safety | Every peer-table and request/response future is dropped at each await point, and a panic is injected into each handler; invariants hold in every case. |
| Fairness under load | Under soak load, queued peer-table requests never wait beyond a bound; reverting #6823's handling does not exit the node. |
| Invariant monitor on CI logs | Peer-table and sync-phase invariants checked on every Hive and Assertoor run; reverting #7435 fails it. |
| RPC history checking | Histories recorded during the reorg simulator and load tests are checked; injecting a stale `latest` read after a reorg fails the checker. |
| Metamorphic storage, full | Nightly; full sync versus snap sync to the same block, cache and layer settings, mid-run restart; injecting a dropped write in one backend fails it. |
| Attacknet chaos | Three scenarios (partition and heal, peer crash during snap sync, clock skew) run weekly on Kurtosis with assertoor checks. |

### Phase 3: injection and simulation

| Item | Acceptance criteria |
|---|---|
| Subsystem simulation: `PeerTable` + sync | `exit` calls in sync return errors instead; `PeerTable` and the sync manager run with mocked peers, the in-memory store and paused tokio time. Reverting #7491, #6823 or #7435 fails it within a fixed seed budget; reverting #6924 is reported as a stall. |
| TLA+ model of the sync state machine | Pivot choice, syncer lock and full/snap transitions; the model finds the #6924 wedge when the lock release is removed. |
| Injectable clock, RNG and transport in p2p | p2p code takes time, randomness and sockets from a handle; production behaviour unchanged (benchmarks flat). |
| Clock-jump tests | With the injectable clock, forward/backward jumps of minutes and hours leave slot timing, timeouts and peer scoring correct. |
| Whole-node p2p DST | Same seed gives byte-identical logs (checked in CI). Reintroducing #6900's unbounded mailbox with a slow simulated peer fails within a fixed seed budget. Runs nightly over many seeds. |
| Actor ordering exploration | With spawned `explore`, the #7491 and #7435 races are found every run. |
| Antithesis pilot | One scenario with faults for a fixed period; decide on adoption from bugs found versus cost. |
| ethshadow | ethrex runs in ethshadow at 100+ nodes; gossip bandwidth and peer counts recorded. |

## Sources

- FoundationDB, [Simulation and testing](https://apple.github.io/foundationdb/testing.html); Will Wilson, [Testing Distributed Systems w/ Deterministic Simulation](https://thestrangeloop.com/2014/testing-distributed-systems-w-slash-deterministic-simulation.html), Strange Loop 2014; [BUGGIFY](https://transactional.blog/simulation/buggify)
- TigerBeetle, [Safety and the VOPR](https://docs.tigerbeetle.com/concepts/safety/); [Protocol-aware DST](https://tigerbeetle.com/blog/2026-08-20-protocol-aware-dst)
- Dropbox, [Testing sync at Dropbox](https://dropbox.tech/infrastructure/-testing-our-new-sync-engine)
- [madsim](https://github.com/madsim-rs/madsim); S2, [Deterministic simulation testing for async Rust](https://s2.dev/blog/dst); Polar Signals, [DST in Rust](https://blog.polarsignals.com/blog/posts/2025/07/08/dst-rust); WarpStream, [DST for our entire SaaS](https://warpstream.com/blog/deterministic-simulation-testing-for-our-entire-saas)
- etcd, [Autonomous testing with Antithesis](https://etcd.io/blog/2025/autonomus_testing_with_antithesis/); [robustness tests](https://github.com/etcd-io/etcd/tree/main/tests/robustness); [Porcupine](https://github.com/anishathalye/porcupine); [Elle](https://github.com/jepsen-io/elle); [fail-rs](https://github.com/tikv/fail-rs)
- Antithesis, [Ethereum Merge case study](https://antithesis.com/case_studies/ethereum_merge)
- Brooker and Desai, *Systems Correctness Practices at Amazon Web Services*, Communications of the ACM 68(6), June 2025 (unlinked: the publisher rejects automated link checks); [PObserve](https://p-org.github.io/P/advanced/pobserve/pobserve); Bornholt et al., [ShardStore, SOSP 2021](https://www.amazon.science/publications/using-lightweight-formal-methods-to-validate-a-key-value-storage-node-in-amazon-s3)
- Hackett, Rowe, Kuppe, [Understanding Inconsistency in Azure Cosmos DB with TLA+](https://arxiv.org/abs/2210.13661); Davis et al., [eXtreme Modelling in Practice](https://arxiv.org/abs/2006.00915) (MongoDB)
- CockroachDB, [Metamorphic testing in Pebble](https://www.cockroachlabs.com/blog/metamorphic-testing-the-database/)
- [Attacknet](https://ethpandaops.io/posts/attacknet-introduction); [ethshadow](https://ethereum.github.io/ethshadow); [Hive](https://github.com/ethereum/hive); [Assertoor](https://github.com/ethpandaops/assertoor)
- Spawned roadmap: [database-grade testing program](https://github.com/lambdaclass/spawned/blob/main/ROADMAP.md#database-grade-testing-program), [client sweep](https://github.com/lambdaclass/spawned/blob/main/ROADMAP.md#client-sweep-production-pain), [testing to add](https://github.com/lambdaclass/spawned/blob/main/ROADMAP.md#testing-to-add), [`spawned-test`](https://github.com/lambdaclass/spawned/blob/main/ROADMAP.md#testing-crate-spawned-test), [verification rule](https://github.com/lambdaclass/spawned/blob/main/ROADMAP.md#verification-rule)
