# Ethrex L1 Metrics Coverage Gap Analysis

## Scope
This note tracks the current state of metrics and dashboard observability for the L1, highlights the gaps against a cross-client baseline. It covers runtime metrics exposed through our crates, the existing Grafana "Ethrex L1 - Perf" dashboard, and supporting exporters already wired in provisioning.

### At a glance
- **Covered today**: Block execution timings, detailed execution breakdown, Engine API and JSON-RPC method telemetry, and host/process health are exported and graphed through `metrics/provisioning/grafana/dashboards/common_dashboards/ethrex_l1_perf.json`. The refreshed [L1 Dashboard doc](./dashboards.md) has screenshots and panel descriptions.
- **Missing**: Sync/peer awareness, txpool depth, storage IO metrics, and richer error taxonomy are absent or only logged.
- **Near-term focus**: Ship sync & peer gauges, surface txpool counters we already emit, extend storage instrumentation, and harden alerting before widening coverage further.

## Baseline We Compare Against
The gap analysis below uses a cross-client checklist we gathered after looking at Geth and Nethermind metrics and dashboard setups; this works as a baseline of "must-have" coverage for execution clients. The key categories are:
- **Chain sync & finality**: head vs peer lag, stage progress, finalized/safe head distance, sync ETA.
- **Peer health**: active peers, connected peer roles, snap-capable availability, ingress/egress traffic.
- **Block & payload pipeline**: gas throughput, execution breakdown timings, block import failures, payload build latency.
- **Transaction pool**: pending depth per type, drop/evict counters, gossip ingress/egress rate, TPS trend.
- **Engine API & RPC**: call success ratios, latency histograms for Engine and JSON-RPC methods, error taxonomy.
- **State & storage**: db size, read/write bytes, cache hit/miss, heal backlog, pruning.
- **Process & host health**: CPU, memory, FDs, uptime, disk headroom (usually covered by node_exporter but treated as must-have).
- **Error & anomaly counters**: explicit counters for reorgs, failed imports, sync retries, bad peer events.

Snapshot: November 2025.


| Client | Dashboard snapshot |
| --- | --- |
| Geth | ![Geth dashboard](img/geth-dashboard.png) |
| Nethermind | ![Nethermind dashboard](img/nethermind-dashboard.png) |

Some good resources for reference:
- [Understanding Geth's dashboard](https://geth.ethereum.org/docs/monitoring/understanding-dashboards)
- [Nethermind's metrics](https://docs.nethermind.io/monitoring/metrics/)

## Current Instrumentation
Ethrex exposes the metrics API by default when the CLI `--metrics` flag is enabled, and Prometheus/Grafana wiring is part of the provisioning stack. The table below stacks our current coverage against the reference clients for each essential bucket.

| Bucket | Geth | Nethermind | Ethrex |
| --- | --- | --- | --- |
| Chain sync & finality | Yes | Yes | Partial (head height only) |
| Peer health | Yes | Yes | Partial (peer count, client distribution, disconnections) |
| Block & payload pipeline | Yes | Yes | Yes (latency + throughput) |
| Transaction pool | Yes (basic) | Yes | Partial (counters, no panels) |
| Engine API & RPC | Partial (metrics exist, limited panels) | Yes | Partial (per-method rate/latency) |
| State & storage | Yes | Yes | Partial (datadir size; no pruning) |
| Process & host health | Yes | Yes | Yes (node exporter + process) |
| Error & anomaly counters | Yes | Yes | Partial (Engine/RPC error rates) |

- **Block execution pipeline**
  - Gauges exposed in `ethrex_metrics::metrics_blocks`: `gas_limit`, `gas_used`, `gigagas`, `block_number`, `head_height`, `execution_ms`, `merkle_ms`, `store_ms`, `transaction_count`, plus block-building focused gauges that need to be reviewed first (`gigagas_block_building`, `block_building_ms`, `block_building_base_fee`).
  - Updated on the hot path in `crates/blockchain/blockchain.rs`, `crates/blockchain/payload.rs`, and `crates/blockchain/fork_choice.rs`; block-building throughput updates live in `crates/blockchain/payload.rs`.
  - Exposed via `/metrics` when the `metrics` feature or CLI flag is enabled and visualised in Grafana panels "Gas Used %", "Ggas/s", "Ggas/s by Block", "Block Height", and the expanded "Block Execution Breakdown" row (pie, diff %, deaggregated by block) inside `metrics/provisioning/grafana/dashboards/common_dashboards/ethrex_l1_perf.json`.
- **Transaction pipeline**
  - `crates/blockchain/metrics/metrics_transactions.rs` defines counters and gauges: `transactions_tracker{tx_type}`, `transaction_errors_count{tx_error}`, `transactions_total`, `mempool_tx_count{type}`, `transactions_per_second`.
  - L1 currently uses the per-type success/error counters via `metrics!(METRICS_TX...)` in `crates/blockchain/payload.rs`. Aggregate setters (`set_tx_count`, `set_mempool_tx_count`, `set_transactions_per_second`) are only invoked from the L2 sequencer (`crates/l2/sequencer/metrics.rs` and `crates/l2/sequencer/block_producer.rs`), so there is no TPS gauge driven by the execution client today.
  - No Grafana panels surface these metrics yet, despite being scraped.
- **Process & storage footprint**
  - `crates/blockchain/metrics/metrics_process.rs` registers Prometheus' process collector (available on Linux) and provides `datadir_size_bytes` when the CLI passes the datadir path.
  - Grafana reuses the emitted `datadir_size_bytes` for "Datadir Size" and relies on node_exporter panels for CPU, RSS, open FDs, and host resource graphs in the "Process & Server Info" row.
- **Tracing-driven profiling**
  - `crates/blockchain/metrics/profiling.rs` installs a `FunctionProfilingLayer` whenever the CLI `--metrics` flag is set. Histograms (`function_duration_seconds{function_name}`) capture tracing span durations across block processing.
  - The current "Block Execution Breakdown" pie panel pulls straight from the gauges in `METRICS_BLOCKS` (`execution_ms`, `merkle_ms`, `store_ms`). The profiling histograms are scraped by Prometheus but are not charted in Grafana yet.
- **Engine & RPC telemetry**
  - `function_duration_seconds_*{namespace="engine"|"rpc"}` histograms are emitted by the same profiling layer.
  - Grafana now charts per-method request rates and range-based latency averages for both Engine API and JSON-RPC namespaces via the "Engine API" and "RPC API" rows.
- **Metrics API**
  - `crates/blockchain/metrics/api.rs` exposes `/metrics` and `/health`; orchestration defined in `cmd/ethrex/initializers.rs` ensures the Axum server starts alongside the node when metrics are enabled.
  - The provisioning stack (docker-compose, Makefile targets) ships Prometheus and Grafana wiring, so any new metric family automatically appears in the scrape.

## General improvements
Before addressing the gaps listed below, we should also consider some general improvements in our current metrics setup:

- **Namespace standardisation**: Metric names and labels should follow a consistent naming convention (e.g., `ethrex_l1_` prefix) to avoid collisions and improve clarity. Right now we are not using prefixes.
- **Panels dependent on `ethereum-metrics-exporter`**: Some metrics are only visible through the external `ethereum-metrics-exporter` (e.g., network, client version, consensus fork), we are already pulling those in our dashboard but this is not ideal. We should consider integrating these key metrics directly into Ethrex.
- **Label consistency**: We are not using labels consistently, especially in l1. We might need to take a pass to ensure similar metrics use uniform label names and values to facilitate querying and aggregation if needed or decide to not use labels when appropriate.
- **Exemplars addition**: For histograms, adding exemplars can help trace high-latency events back to specific traces/logs. This is especially useful for latency-sensitive metrics like block execution time or RPC call durations where we could add block hashes as exemplars. This needs to be evaluated on a case-by-case basis and tested.

## Coverage vs Baseline Must-Haves

| Bucket | Have today | Missing / next steps |
| --- | --- | --- |
| Chain sync & finality | `METRICS_BLOCKS.head_height` surfaced in `crates/blockchain/fork_choice.rs`; Grafana charts head height. | Need best-peer lag, sync stage progress, ETA, and finalized/safe head distance. Current counters live only in logs via `periodically_show_peer_stats_during_syncing` (`crates/networking/p2p/network.rs`). |
| Peer health | `ethrex_p2p_peer_count`, `ethrex_p2p_peer_clients`, and `ethrex_p2p_disconnections` (with reason and client labels) exposed via Prometheus; Grafana "Peer Info" row charts peer count, client distribution pie/timeseries, disconnection events, and a detailed disconnections table. | Still missing peer limits/targets, snap-capable availability, handshake failure counters, and ingress/egress traffic metrics. |
| Block & payload pipeline | `METRICS_BLOCKS` tracks gas throughput and execution stage timings; `transaction_count` is exported but not visualised yet. | Add p50/p95 histograms for execution stages, block import success/failure counters, and an L1-driven TPS gauge so operators can read execution throughput without relying on L2 metrics. |
| Transaction pool | Success/error counters per tx type emitted from `crates/blockchain/payload.rs`. | No exported pending depth, blob/regular split, drop reasons, or gossip throughput; aggregates exist only in L2 (`crates/l2/sequencer/metrics.rs`). |
| Engine API & RPC | Per-method request rate, latency (range-based + 18 s lookback) covering `namespace="engine"` and `namespace="rpc"` metrics. | Deepen error taxonomy ( error/rates and distinguish failure reasons), add payload build latency distributions, and baseline alert thresholds. |
| State & storage | Only `datadir_size_bytes` today. | Export healing/download progress, snapshot sync %, DB read/write throughput, pruning/backfill counters (we need to check what makes sense here), and cache hit/miss ratios. |
| Process & host health | Process collector + `datadir_size_bytes`; node_exporter covers CPU/RSS/disk. | Add cache pressure indicators (fd saturation, async task backlog) and ensure dashboards surface alert thresholds. |
| Error & anomaly counters | "Engine and RPC Error rates" row charts success/error rates and error % by method/kind for both Engine API and JSON-RPC. Peer disconnection reasons tracked in "Peer Info" row. | Add counters for failed block imports, reorg depth, sync failures, and wire alerting thresholds. |

### Next steps
1. Tackle general improvements around naming conventions and label consistency.
2. Implement sync & peer metrics (best-peer lag, stage progress) and add corresponding Grafana row.
3. Surface txpool metrics by wiring existing counters and charting them.
4. Add the metrics relying on `ethereum-metrics-exporter` into the existing metrics, and avoid our dashboard dependence on it.
5. Extend Engine API / JSON-RPC metrics with richer error taxonomy and payload construction latency distributions.
6. State and Storage metrics, especially related to snapsync, pruning, db and cache.
7. Process health improvements, especially related to read/write latencies and probably tokio tasks.
8. Review block building metrics.
9. Revisit histogram buckets and naming conventions once new metrics are merged, then define alert thresholds.
10. Investigate exemplar usage where appropriate.