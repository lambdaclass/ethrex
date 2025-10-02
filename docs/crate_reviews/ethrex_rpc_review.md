# ethrex-rpc Complexity & Concurrency Review

Date: 2025-10-02
Commit: 25ee6a95a6ccf329be87aecf903483fbc34796d0
Target crate: `crates/networking/rpc`

## 1. Quantitative Snapshot

| Type | Code | Blank | Doc comments | Comments | Total |
| --- | --- | --- | --- | --- | --- |
| Main | 6827 | 716 | 66 | 253 | 7862 |
| Tests | 1284 | 111 | 0 | 33 | 1428 |
| Total | 8111 | 827 | 66 | 286 | 9290 |

- Files analyzed: 43 Rust sources (tests included; no nested crates)
- Functions: 331 total with 21 flagged as complex (≥60 lines or branch-heavy)
- Longest routine(s): [crates/networking/rpc/engine/fork_choice.rs:166](https://github.com/lambdaclass/ethrex/blob/25ee6a95a6ccf329be87aecf903483fbc34796d0/crates/networking/rpc/engine/fork_choice.rs#L166) (`handle_forkchoice`, 134 lines, 20 branches); [crates/networking/rpc/eth/transaction.rs:436](https://github.com/lambdaclass/ethrex/blob/25ee6a95a6ccf329be87aecf903483fbc34796d0/crates/networking/rpc/eth/transaction.rs#L436) (`EstimateGasRequest::handle`, 110 lines, 11 branches); [crates/networking/rpc/eth/fee_market.rs:87](https://github.com/lambdaclass/ethrex/blob/25ee6a95a6ccf329be87aecf903483fbc34796d0/crates/networking/rpc/eth/fee_market.rs#L87) (`handle`, 97 lines, 11 branches)
- Async/concurrency signals (`rg -o` manual counts; analyzer under-counts multi-line matches):
  - `async fn`: 166
  - `.await`: 347
  - `tokio::spawn`: 2
  - `spawn_blocking`: 0
  - `Arc<…>`: 6 textual occurrences (heavier use via `Arc::new` not captured)
  - Mutexes: `Arc<Mutex<…>>` for filters (`eth/filter.rs`) plus `Arc<TokioMutex<…>>` for gas tip estimator
  - Atomics: 0
  - Other noteworthy primitives: extensive `reqwest::Client` usage for outbound RPC, background maintenance task for filters

## 2. High-Risk Components
- [crates/networking/rpc/engine/fork_choice.rs:166](https://github.com/lambdaclass/ethrex/blob/25ee6a95a6ccf329be87aecf903483fbc34796d0/crates/networking/rpc/engine/fork_choice.rs#L166) — `handle_forkchoice` coordinates storage reads, sync triggers, mempool eviction, and payload preparation with dense branching; failure paths mix async and sync calls, increasing chances of partial updates under load.
- [crates/networking/rpc/eth/transaction.rs:436](https://github.com/lambdaclass/ethrex/blob/25ee6a95a6ccf329be87aecf903483fbc34796d0/crates/networking/rpc/eth/transaction.rs#L436) — `EstimateGasRequest::handle` performs multi-phase transaction simulation, mutating cloned requests across several awaits; the binary-search loop makes repeated synchronous storage calls and blockchain interactions that are hard to reason about and costly to test.
- [crates/networking/rpc/rpc.rs:196](https://github.com/lambdaclass/ethrex/blob/25ee6a95a6ccf329be87aecf903483fbc34796d0/crates/networking/rpc/rpc.rs#L196) — `start_api` wires global state (`Arc<Blockchain>`, `SyncManager`, filter registry) and spawns the filter janitor; the shared `Arc<Mutex<HashMap<…>>>` becomes the choke point for every filter operation and background clean-up.

## 3. Concurrency Observations
- [crates/networking/rpc/eth/gas_price.rs:31](https://github.com/lambdaclass/ethrex/blob/25ee6a95a6ccf329be87aecf903483fbc34796d0/crates/networking/rpc/eth/gas_price.rs#L31) holds a `TokioMutex` guard on the gas-tip estimator while awaiting extensive storage I/O, serialising every `eth_gasPrice` caller behind the estimator task.
- [crates/networking/rpc/eth/filter.rs:33](https://github.com/lambdaclass/ethrex/blob/25ee6a95a6ccf329be87aecf903483fbc34796d0/crates/networking/rpc/eth/filter.rs#L33) relies on a synchronous `Arc<Mutex<HashMap<…>>>` from async handlers and a background `tokio::spawn`; whenever the mutex contends the runtime thread is blocked, and the poison-recovery path replaces the whole map, risking dropped filters.
- Storage-heavy RPC handlers (e.g., [crates/networking/rpc/eth/block.rs:36](https://github.com/lambdaclass/ethrex/blob/25ee6a95a6ccf329be87aecf903483fbc34796d0/crates/networking/rpc/eth/block.rs#L36), [eth/transaction.rs:436](https://github.com/lambdaclass/ethrex/blob/25ee6a95a6ccf329be87aecf903483fbc34796d0/crates/networking/rpc/eth/transaction.rs#L436)) perform multiple synchronous database lookups in async contexts without `spawn_blocking`, so handlers may occupy core executor threads when RocksDB calls stall.

## 4. Engineering Complexity Score
- **Score: 3 / 5** — Medium-high complexity: broad surface area and many deep handlers, but logic is still compartmentalised per namespace; key risks stem from synchronous locking patterns and heavy storage work inside async flows.

## 5. Recommendations
1. Refactor `ActiveFilters` to an async-friendly primitive (e.g., sharded `tokio::sync::RwLock` or an actor) and give the cleanup loop a cooperative shutdown path.
2. Split `handle_forkchoice` into smaller helpers (separate validation, mempool updates, sync decisions) and cache repeated storage lookups to simplify reasoning and testing.
3. Rework `EstimateGasRequest::handle` to isolate simulation retries into a pure helper so storage and blockchain calls can be mocked and benchmarked independently.

## 6. Follow-Ups / Tooling Ideas
- Enable `cargo clippy -W clippy::await_holding_lock` and audit the `TokioMutex` + `await` patterns surfaced (gas tip estimator is the first hit).
- Add targeted load tests or tracing spans around storage-heavy RPCs to catch blocking behaviour before production (tokio-console or tracing-based histograms would help).
- Extend `docs/crate_reviews/toolkit/analyze_crate.py` to use multiline regex so async keyword counts stay accurate for this crate and future reviews.
