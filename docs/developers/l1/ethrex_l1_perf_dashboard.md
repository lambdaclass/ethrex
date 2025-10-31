# Ethrex L1 Performance Dashboard

## How to use it
The dashboard refreshes every five seconds and defaults to the last 30 minutes, which keeps queries light while still showing real-time behavior. Use the templated selectors at the top to pick the Prometheus datasource (`DS_PROMETHEUS`), target job, network, and instances; all panels respect these filters so you can scope the view to a single node or compare several peers.

## Execution & Consensus Summary
**Execution Client** lists `eth_exe_web3_client_version` so you can confirm the binary and build hash running on each instance at a glance. **Consensus Config** surfaces the network name, letting you double-check that the execution node is at the correct network. **Consensus Fork** echoes the fork reported by the consensus layer, which helps spot nodes that lag on upgrades.

## Block Processing
**Gas Used %** shows how much of the configured gas limit recent blocks actually consumed, revealing saturation or empty-slot patterns per instance. **Ggas/s** tracks execution throughput in gigagas per second and carries a hidden block-number series to align spikes with particular heights if needed. **Block Height** plots the reported head from each node, making startup gaps or sync stalls immediately visible. **Ggas/s by Block** joins the throughput data with block numbers and suppresses moments where all peers don't share the same head. **Block Time** divides gas consumed by throughput to estimate per-block execution duration while linking each sample to its block number for quick deep dives.

## Block Execution Breakdown
**Block Execution Breakdown – $instance** repeats a pie chart per selected instance, grouping `function_duration_seconds_sum` into storage reads, account reads, and generic execution time so you can compare where each node spends its block budget.

## Process & Server Info
**Uptime** subtracts `process_start_time_seconds` from wall clock time to highlight how long the node has been running. **Threads** watches `process_threads` for signs of runaway concurrency. **Open FDs** presents the current file-descriptor count, and **Open FDs Historic** extends the same metric over time to catch slow leaks. **Datadir Size** charts `datadir_size_bytes`, which helps you verify db size and plan for capacity. **Node CPU (avg. cores used)** converts `rate(process_cpu_seconds_total)` into effective core usage, exposing compute hotspots. **Node Memory (RSS)** follows `process_resident_memory_bytes`, separating RSS from other memory gauges so you can diagnose growth quickly. **Host CPU Utilization (%)** pulls Node Exporter CPU counters to show whether the box itself is saturated beyond the client process. **Host RAM (GiB) — Used vs Total** overlays used and total system memory in GiB, flagging when the host nears exhaustion.

## Block Building (WIP)
This collapsed row contains experimental payload metrics: **Block building base_fee** captures the base fee the local builder applies (converted to Gwei); **Blockbuilding Ggas/s** mirrors execution throughput but limited to the block-construction phase; **Block building time taken in ms** records how long payload assembly lasts so you can spot proposer delays.
