# Ethrex L1 Metrics Coverage

## Scope
This note tracks the current state of metrics and dashboard observability for the L1, highlights the gaps against a cross-client baseline. It covers runtime metrics exposed through our crates, the existing Grafana "Ethrex L1 - Perf" dashboard, and supporting exporters already wired in provisioning.

## Baseline We Compare Against
The gap analysis below uses a cross-client checklist we gathered after looking at Geth, Nethermind, and Reth metrics and dashboard setups; this works as a baseline of "must-have" coverage for execution clients. The key categories are:
- **Chain sync & finality**: head vs peer lag, stage progress, finalized/safe head distance, sync ETA.
- **Peer health**: active peers, connected peer roles, snap-capable availability, ingress/egress traffic.
- **Block & payload pipeline**: gas throughput, execution breakdown timings, block import failures, payload build latency.
- **Transaction pool**: pending depth per type, drop/evict counters, gossip ingress/egress rate, TPS trend.
- **Engine API & RPC**: call success ratios, latency histograms for Engine and JSON-RPC methods, error taxonomy.
- **State & storage**: db size, read/write bytes, cache hit/miss, heal backlog, pruning progress.
- **Process & host health**: CPU, memory, FDs, uptime, disk headroom (usually covered by node_exporter but treated as must-have).
- **Error & anomaly counters**: explicit counters for reorgs, failed imports, sync retries, bad peer events.

Some good resources for reference:
- [Understanding Geth's dashboard](https://geth.ethereum.org/docs/monitoring/understanding-dashboards)
- [Nethermind's metrics](https://docs.nethermind.io/monitoring/metrics/)
- [Reth's observability](https://reth.rs/run/monitoring/#observability-with-prometheus--grafana)

## Current Instrumentation
Ethrex exposes the metrics API by default when the CLI `--metrics` flag is enabled, and Prometheus/Grafana wiring is part of the provisioning stack. The table below stacks our current coverage against the reference clients for each essential bucket.

| Bucket | Geth | Nethermind | Reth | Ethrex |
| --- | --- | --- | --- | --- |
| Sync & head tracking | Yes | Yes | Yes | Partial (head height only) |
| Peer connectivity | Yes | Yes | Yes | No |
| Txpool visibility | Yes (basic) | Yes | Yes | Partial (counters, no panels) |
| Block execution latency | Yes | Yes | Yes | Yes |
| Throughput (MGas/s) | Partial (derived) | Yes | Yes | Yes |
| State / DB IO | Yes | Yes | Yes | Partial (datadir size only) |
| Engine API telemetry | Partial (metrics exist, limited panels) | Yes | Yes | No |
| Reorg & error counters | Yes | Yes | Partial (basic) | No |
| Pruning metrics | No | Yes | Yes | No |
| System resources | Yes | Yes | Yes | Yes (node exporter + process) |

- **Block execution pipeline**
  - Gauges exposed in `crates/blockchain/metrics/metrics_blocks.rs`: `gas_limit`, `gas_used`, `gigagas`, `block_number`, `head_height`, `execution_ms`, `merkle_ms`, `store_ms`, `transaction_count`.
  - We have some related to block building also in `crates/blockchain/metrics/metrics_blocks.rs`: `gigagas_block_building`, `block_building_ms`, `block_building_base_fee`. These need a further look to be sure they are correct and useful.
  - Updated on the hot path in `crates/blockchain/blockchain.rs`, `crates/blockchain/payload.rs`, and `crates/blockchain/fork_choice.rs`; exposed via `/metrics` when the `metrics` feature or CLI flag is enabled.
  - Visualised in Grafana panels "Gas Used %", "Ggas/s", "Ggas/s by Block" "Block Height", and "Block Execution Breakdown" inside `metrics/provisioning/grafana/dashboards/common_dashboards/ethrex_l1_perf.json`.
- **Transaction pipeline**
  - `crates/blockchain/metrics/metrics_transactions.rs` defines counters and gauges: `transactions_tracker{tx_type}`, `transaction_errors_count{tx_error}`, `transactions_total`, `mempool_tx_count{type}`, `transactions_per_second`.
  - L1 currently uses the per-type success/error counters via `metrics!(METRICS_TX...)` in `crates/blockchain/payload.rs`. The aggregate gauges (`transactions_total`, `mempool_tx_count`, `transactions_per_second`) are still only fed by L2 sequencer code (`crates/l2/sequencer/...`).
  - No Grafana panels surface these metrics yet, despite being scraped.
- **Process & storage footprint**
  - `crates/blockchain/metrics/metrics_process.rs` registers Prometheus' process collector (Linux only) and provides `datadir_size_bytes` when the CLI passes the datadir path.
  - Grafana reuses the emitted `datadir_size_bytes` for "Datadir Size" and relies on node_exporter panels for CPU, RSS, open FDs, and host resource graphs in the "Process & Server Info" row.
- **Tracing-driven profiling**
  - `crates/blockchain/metrics/profiling.rs` installs a `FunctionProfilingLayer` (opt-in via CLI `--metrics`). Histograms (`function_duration_seconds{function_name}`) capture tracing span durations across block processing.
  - Grafana does not yet visualise these histograms, but Prometheus scrapes them for ad-hoc queries.
- **Metrics API**
  - `crates/blockchain/metrics/api.rs` exposes `/metrics` and `/health`; orchestration defined in `cmd/ethrex/initializers.rs` ensures the Axum server starts alongside the node when metrics are enabled.
  - The provisioning stack (docker-compose, Makefile targets) ships Prometheus and Grafana wiring, so any new metric family automatically appears in the scrape.

## General pitfalls
Before addressing the gaps listed below, we should also consider some general pitfalls in our current metrics setup:

- **No namespace standardisation**: Metric names and labels should follow a consistent naming convention (e.g., `ethrex_` prefix) to avoid collisions and improve clarity. We should also probably add l1/l2 prefixes where applicable.
- **No label consistency**: Similar metrics should use uniform label names and values to facilitate querying and aggregation. This is especially important for metrics that track similar concepts across different components.
- **Exemplar where applicable**: For histograms, adding exemplars can help trace high-latency events back to specific traces/logs. This is especially useful for latency-sensitive metrics like block execution time or RPC call durations.

## Coverage vs Baseline Must-Haves
- **Chain sync status**
  - *Have*: `head_height` and `block_number` gauges provide current execution head; dashboard shows block height trend.
  - *Missing*: Distance to best peer, sync stage/status gauges, ETA, and stalled-stage indicators. Metrics exist in logs (`periodically_show_peer_stats_during_syncing`), but none are exported.
- **Peer connectivity / network health**
  - *Have*: RPC exposes `net_peerCount`; no Prometheus gauges.
  - *Missing*: Peer count, peer limit saturation, P2P ingress/egress throughput, Snap capability availability. Dashboard lacks a networking row entirely.
- **Block and transaction throughput**
  - *Have*: Ggas/s, block processing timings, transaction count per block.
  - *Missing*: Distribution histograms (p50/p95) for execution stages, explicit block import failure counters, sustained TPS gauge fed by L1.
- **Resource utilisation**
  - *Have*: Process collector (Linux), datadir size, node exporter CPU/RSS/disk panels.
  - *Missing*: Internal cache pressure metrics (state trie cache hit/miss, RocksDB compaction backlog) and DB IO rates.
- **State and storage health**
  - *Have*: Only `datadir_size_bytes` plus external host metrics.
  - *Missing*: State heal/download progress, snapshot/sync completion %, DB read/write throughput, pruning/backfill counters.
- **Transaction pool health**
  - *Have*: Transaction success/error counters per type.
  - *Missing*: Pending queue depth, blob vs regular split, drop/eviction counters, gossip ingress/egress rates, age distribution — these gauges exist in code but are unused for L1.
- **Error counters & warnings**
  - *Have*: None beyond logs.
  - *Missing*: Counters for failed block imports, reorg depth, RPC failures, Engine API retries, synced stage failures.
- **Engine API (post-Merge) metrics**
  - *Have*: None.
  - *Missing*: Call counts and latency histograms for `newPayload`, `forkChoiceUpdated`, `getPayload`, payload build success/failure counts, blob service metrics.
- **Consensus / sync quality**
  - *Have*: No dedicated telemetry.
  - *Missing*: Finality lag (latest finalized vs safe vs head), healing backlog, stage-by-stage completion.
- **Additional must-have surfaced by review**
  - *JSON-RPC health*: Need per-method call rates, latency, error counters to detect API regressions (currently absent from both metrics and dashboards).