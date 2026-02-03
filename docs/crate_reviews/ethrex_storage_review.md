# ethrex-storage Complexity & Concurrency Review

Date: 2025-10-02
Commit: 25ee6a95a6ccf329be87aecf903483fbc34796d0
Target crate: `crates/storage`

## 1. Quantitative Snapshot

| Type | Code | Blank | Doc comments | Comments | Total |
| --- | --- | --- | --- | --- | --- |
| Main | 4619 | 729 | 236 | 151 | 5735 |
| Tests | 912 | 169 | 10 | 56 | 1147 |
| Total | 5531 | 898 | 246 | 207 | 6882 |

- Files analyzed: 19 Rust sources (no exclusions)
- Functions: 391 total with 15 flagged as complex (see heuristics)
- Longest routine(s): [crates/storage/store_db/rocksdb.rs:109](https://github.com/lambdaclass/ethrex/blob/25ee6a95a6ccf329be87aecf903483fbc34796d0/crates/storage/store_db/rocksdb.rs#L109) (`Store::new`, 194 lines/12 branches); [crates/storage/store_db/rocksdb.rs:439](https://github.com/lambdaclass/ethrex/blob/25ee6a95a6ccf329be87aecf903483fbc34796d0/crates/storage/store_db/rocksdb.rs#L439) (`Store::apply_updates`, 91 lines/9 branches); [crates/storage/store_db/libmdbx.rs:129](https://github.com/lambdaclass/ethrex/blob/25ee6a95a6ccf329be87aecf903483fbc34796d0/crates/storage/store_db/libmdbx.rs#L129) (`Store::apply_updates`, 83 lines/10 branches)
- Async/concurrency signals (crate-wide):
  - `async fn`: 0
  - `.await`: 0
  - `tokio::spawn`: 0
  - `spawn_blocking`: 18
  - `Arc<...>`: 33
  - Mutexes: none; `Store` relies on `Arc<RwLock<...>>` for cached headers/config
  - Atomics: 0
  - Other noteworthy primitives: hand-rolled `Box::leak`/`Drop` pairs to provide `'static` RocksDB snapshots; libmdbx dupsort tables

## 2. High-Risk Components
- [crates/storage/store_db/rocksdb.rs:439](https://github.com/lambdaclass/ethrex/blob/25ee6a95a6ccf329be87aecf903483fbc34796d0/crates/storage/store_db/rocksdb.rs#L439) — `Store::apply_updates` funnels all trie, block, and receipt writes through one 90+ line `spawn_blocking` closure; error handling is coarse and the composite-key assembly is duplicated across backends.
- [crates/storage/store.rs:416](https://github.com/lambdaclass/ethrex/blob/25ee6a95a6ccf329be87aecf903483fbc34796d0/crates/storage/store.rs#L416) — `Store::apply_account_updates_from_trie_batch` mutates state and storage tries sequentially, opens storage tries per account, and mixes code writes with trie hashing; failure to propagate storage updates atomically risks state/code divergence.
- [crates/storage/trie_db/rocksdb_locked.rs:19](https://github.com/lambdaclass/ethrex/blob/25ee6a95a6ccf329be87aecf903483fbc34796d0/crates/storage/trie_db/rocksdb_locked.rs#L19) — `RocksDBLockedTrieDB::new` leaks an `Arc` to force `'static` lifetimes and reclaims it in `Drop`; the unsafe `Box::from_raw` dance is fragile if constructors fail mid-way or clone counts drift.

## 3. Concurrency Observations
- Heavy reliance on `tokio::task::spawn_blocking` (18 call sites) to wrap synchronous RocksDB/libmdbx transactions; frequent per-operation thread handoffs can saturate Tokio’s blocking pool under load.
- The public `Store` facade exposes async APIs backed by synchronous locks (`Arc<RwLock<_>>`) and snapshot tries; no actor-based isolation yet, so concurrent callers share DB handles directly.
- Locked trie variants obtain `'static` snapshots by leaking database Arcs; while Drop repairs the leak, panics before Drop or future refactors could strand snapshots and exhaust file handles.

## 4. Engineering Complexity Score
- **Score: 4 / 5** — Dual backends, large batch-write closures, and custom snapshot lifetimes introduce significant surface area for subtle data corruption or performance regressions despite modest file counts.

## 5. Recommendations
1. Split `Store::apply_updates` for both backends into smaller helpers (per column family group) and converge shared key encoding to reduce duplication and branch count.
2. Introduce a dedicated blocking executor or batching queue for high-frequency reads/writes instead of spawning ad-hoc blocking tasks per call.
3. Replace the `Box::leak`/`Drop` pattern with a lifetime-safe wrapper (e.g., owning struct with explicit snapshot handle) or document invariants and add debug assertions to detect mismatched Arc counts.

## 6. Follow-Ups / Tooling Ideas
- Add targeted integration tests that replay concurrent block/receipt writes against both backends to detect divergence in composite-key layout.
- Instrument blocking task latency (Tokio Console or tracing spans) to monitor `spawn_blocking` pool pressure during sync.
- Run Clippy’s `await_holding_lock` (even though counts are zero today) and `cargo +nightly -Ztimings` periodically to catch regressions as actor migration progresses.
