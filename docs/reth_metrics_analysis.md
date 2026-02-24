# The Case of the Phantom Gigagas

## How We Found a Measurement Bug That Inflated Reth's Reported Performance by 2.6x

We build [ethrex](https://github.com/lambdaclass/ethrex), a minimalist, modular, and ZK-native Ethereum execution client written in Rust. Performance is central to what we do — not just our own, but the ecosystem's. We continuously measure block processing throughput across the major execution clients (ethrex, reth, Nethermind, Geth) using Prometheus metrics from mainnet nodes, and we share these results openly in our public channels.

Among the key metrics: **gigagas per second** (Ggas/s) and `engine_newPayload` latency — together they capture how fast execution clients process blocks. Here's what a typical daily report looks like:

```
Daily performance report (24-hour average)
• nethermind: 0.754 Ggas/s (mean)
• ethrex:     0.709 Ggas/s (mean)
• reth:       0.558 Ggas/s (mean)
• geth:       0.496 Ggas/s (p50)

Daily block time report (24-hour average)
• ethrex:     45.693ms (mean)
• reth:       48.629ms (p50) | 61.256ms (p99.9)
• nethermind: 52.218ms (mean)
• geth:       67.321ms (p50) | 67.321ms (p99.9)
```

On Saturday, February 8, 2026, that report showed something remarkable.

---

## The Anomaly

Reth's Ggas/s had been steady at **0.5-0.6 Ggas/s** for weeks. Saturday's report showed **1.442 Ggas/s**. A 2.6x improvement in two days.

That would be extraordinary. Reth had shipped several optimization PRs that week — async state hashing, prewarm improvements, MeteredStateHook removal — but each individually claimed low single-digit percentage improvements. Not impossible, but 2.6x warranted a closer look.

We decided to investigate.

## The Sanity Check

Before diving into code, we checked the one metric that should move in lockstep with Ggas/s: `engine_newPayload` latency. If a client is processing blocks 2.6x faster, each `newPayload` call should take proportionally less time. The two metrics are directly related — Ggas/s is just `gas_used / latency`.

Reth's block time was sitting at **48.6ms (p50)** — the same range as the week before. Steady, unremarkable.

A 2.6x throughput improvement with no change in latency is a contradiction. One of the two numbers had to be wrong.

## The Log Analysis: 26,636 Blocks

To establish ground truth independent of Prometheus, we parsed reth's `Block added to canonical chain` logs directly. Four full days of data (Feb 6-9, 2026), covering the restart window and steady-state sync: **26,636 blocks**.

| Percentile | Throughput |
|-----------|------------|
| p50 (median) | 0.567 Ggas/s |
| p90 | 0.705 Ggas/s |
| p95 | 0.748 Ggas/s |
| p99 | 0.839 Ggas/s |
| Max | 1.88 Ggas/s |
| Mean | 0.564 Ggas/s |
| Std Dev | 0.128 Ggas/s |

**Zero blocks exceeded 2.0 Ggas/s.** No hidden cluster of fast blocks. No "turbo mode." The mean was 0.564 Ggas/s — matching the live Prometheus check almost exactly.

The fastest blocks were invariably built by MEV builders (primarily Titan): storage-warm bundles of arbitrage and liquidation transactions that reuse the same DeFi protocol state. These hit **1.7-1.9 Ggas/s** because the EVM spends its time on computation rather than cold storage reads.

| Block | Gas Used | Full% | Elapsed | Throughput | Txs |
|-------|----------|-------|---------|------------|-----|
| 24417388 | 34.1 Mgas | 56.8% | 19.8ms | 1.72 Ggas/s | 100 |
| 24412647 | 54.2 Mgas | 90.4% | 40.8ms | 1.33 Ggas/s | 333 |
| 24420641 | 59.9 Mgas | 99.9% | 46.6ms | 1.29 Ggas/s | 467 |
| 24416501 | 59.3 Mgas | 98.9% | 46.6ms | 1.27 Ggas/s | 228 |
| 24418962 | 18.2 Mgas | 30.4% | 14.3ms | 1.27 Ggas/s | 33 |

The gap was stark: **no individual block in 4 days of logs ever produced a throughput above 1.88 Ggas/s, yet Prometheus reported a 24-hour average of 1.442 Ggas/s.** For that average to be correct, a large fraction of blocks would need to exceed it — but 99.7% of blocks fell below 1.0 Ggas/s.

The Prometheus number was physically impossible given the actual block distribution. Something was injecting phantom observations into the metric.

## The Breakthrough: _sum and _count at 15-Second Resolution

The metric our dashboard uses — `reth_consensus_engine_beacon_new_payload_gas_per_second` — is a Prometheus Summary. Our CI computes the daily average as `rate(_sum[24h]) / rate(_count[24h])`. Critically, Summaries expose these raw `_sum` and `_count` counters, so we could inspect the individual contributions at Prometheus's native 15-second scrape resolution. No smoothing, no windowing — just the counters, looking for the exact moment the numbers diverged from reality.

We found it in the first query.

```
Time (UTC)    _count delta    _sum delta         Avg per observation
01:19:00      +1 obs/15s      ~585M              0.585 Ggas/s  (normal)
01:19:15      +8 obs/15s      ~303 BILLION       37.9 Ggas/s   <<<
01:20:00      +7 obs/15s      ~249 BILLION       35.5 Ggas/s   <<<
01:21:00      +9 obs/15s      ~353 BILLION       39.3 Ggas/s   <<<
  ... (continues for ~8 minutes) ...
01:27:15      +1 obs/15s      ~605M              0.605 Ggas/s  (normal)
```

Between **01:19 and 01:27 UTC on February 8**, in an 8-minute window:

- The `_count` jumped from +1 observation per 15 seconds (one block per scrape, as expected) to **+5 to +14 observations per scrape**
- The `_sum` deltas jumped from ~600 million to **~300 billion** — a 500x increase
- The per-observation average during this window was **31.7 Ggas/s**

During this same window, the block execution logs showed completely normal behavior: 12-second block cadence, normal throughput (100-850 Mgas/s), no catch-up blocks, no reorgs. The phantom observations weren't coming from actual block execution.

## The Smoking Gun: `AlreadySeen(Valid)`

With the anomaly window precisely identified (01:19-01:27 UTC, Feb 8), we traced the code path.

The `newPayload` handler begins in `crates/engine/tree/src/tree/mod.rs`:

```rust
BeaconEngineMessage::NewPayload { payload, tx } => {
    let start = Instant::now();
    let gas_used = payload.gas_used();
    let mut output = self.on_new_payload(payload);
    self.metrics.engine.new_payload.update_response_metrics(
        start, ..., &output, gas_used,
    );
```

The timer starts before `on_new_payload()` and `gas_used` is captured from the payload. After the handler returns, `update_response_metrics` records the metric — regardless of what happened inside.

Inside `on_new_payload`, when a block has already been processed, the fast-path is a simple HashMap lookup in `try_insert_payload` (`mod.rs:~2592`):

```rust
if self.state.tree_state.sealed_header_by_hash(&block_num_hash.hash).is_some() {
    convert_to_block(self, input)?;
    return Ok(InsertPayloadOk::AlreadySeen(BlockStatus::Valid));
}
```

This takes ~1ms. But crucially, `try_insert_payload` maps both `AlreadySeen(Valid)` and `Inserted(Valid)` to the same `PayloadStatusEnum::Valid`:

```rust
InsertPayloadOk::Inserted(BlockStatus::Valid) => {
    latest_valid_hash = Some(block_hash);
    self.try_connect_buffered_blocks(num_hash)?;
    PayloadStatusEnum::Valid
}
InsertPayloadOk::AlreadySeen(BlockStatus::Valid) => {
    latest_valid_hash = Some(block_hash);
    PayloadStatusEnum::Valid  // ← Same variant, no distinction
}
```

The distinction between "just executed" and "already seen" is erased. The metrics code in `update_response_metrics` (`crates/engine/tree/src/tree/metrics.rs:284-291`) records for **all** `Valid` responses:

```rust
PayloadStatusEnum::Valid => {
    self.new_payload_valid.increment(1);
    self.new_payload_total_gas.record(gas_used as f64);
    self.new_payload_total_gas_last.set(gas_used as f64);
    let gas_per_second = gas_used as f64 / elapsed.as_secs_f64();
    self.new_payload_gas_per_second.record(gas_per_second);
    self.new_payload_gas_per_second_last.set(gas_per_second);
}
```

There is no guard for `AlreadySeen`. The code records `gas_used / elapsed` whether the block was actually executed or simply looked up from cache.

For an already-seen block, `elapsed` is approximately **1 millisecond** — just a tree lookup. But `gas_used` is the full block gas: 30-60 million. So the recorded value is:

```
30,000,000 gas / 0.001 seconds = 30,000,000,000 gas/second = 30 Ggas/s
```

Thirty gigagas per second. A physically meaningless number, quietly injected into the Summary alongside legitimate execution measurements.

**Why were duplicate payloads being sent?** During the anomaly window, the consensus client sent `newPayload` for the same block multiple times. The exact reason is orthogonal to this post — consensus clients routinely re-send payloads during fork choice resolution, attestation processing, or normal head-tracking, and reth handles this correctly by returning `Valid` from cache without re-executing. The bug isn't in the handling. It's in the accounting.

## Quantifying the Damage

The 205 phantom observations in 8 minutes represented **2.8% of the day's total observation count** but contributed **63.5% of the daily `_sum`**:

| Source | Observations | Contribution to _sum | Avg per observation |
|--------|-------------|---------------------|---------------------|
| Normal blocks (~24h) | ~7,200 | 3.74 trillion (36.5%) | 0.52 Ggas/s |
| Duplicate `newPayload` fast-path (8 min) | 205 | 6.50 trillion (63.5%) | 31.7 Ggas/s |

Computing the daily average:

```
rate(_sum[24h]) / rate(_count[24h]) = 10.24 trillion / 7,405 = 1.38 Ggas/s
```

**1.38 Ggas/s** — almost exactly the reported 1.442 Ggas/s. The small difference is attributable to Prometheus rate extrapolation and the exact window boundaries.

The entire 2.6x "improvement" was caused by 8 minutes of duplicate `newPayload` calls from the consensus client that poisoned the Summary metric. Reth's actual block processing performance hadn't changed meaningfully.

The CKMS algorithm used by the Summary can't distinguish between real measurements and phantom ones. The p50 quantile — normally the most stable metric — spiked from **0.392 to 39.293 Ggas/s** during the anomaly window before recovering to 0.455 Ggas/s afterward. And because `_sum` and `_count` are monotonic counters that never decrease, the inflated values were permanent. Every subsequent `rate(_sum) / rate(_count)` query over a window containing the anomaly would produce an inflated result.

## Recommendations

1. **Guard `AlreadySeen` from metric recording.** The fix is straightforward: check whether the `Valid` response came from actual execution or a cache hit before recording `gas_per_second`. A single `if` in `update_response_metrics` would eliminate the entire class of phantom observations.

2. **Switch from Summaries to Histograms.** Prometheus Histograms with server-side `histogram_quantile()` are aggregatable across instances and don't suffer from irreversible CKMS corruption. A single phantom observation can permanently skew a Summary's quantile estimates for the process lifetime; with Histograms, it only affects the bucket it lands in.

---

## Conclusion

The 2.6x Ggas/s jump was a ghost. Not a breakthrough — a measurement artifact. 205 duplicate `newPayload` calls in an 8-minute window, each recording 30+ Ggas/s because they hit a cache fast-path that took 1ms instead of 50ms, contributed 63.5% of the daily metric sum while representing just 2.8% of observations.

Reth's actual execution performance — measured by parsing 26,636 blocks of raw logs — was a steady 0.564 Ggas/s mean, with the fastest MEV-builder blocks reaching 1.88 Ggas/s. Good performance, improving legitimately with each optimization PR. But not 1.442 Ggas/s, and certainly not 2.6x in two days.

The lesson is about metrics hygiene. Any system that records throughput as `work / elapsed` must guard against cache-hit pollution — cases where the work is attributed but never actually performed, and the elapsed time collapses to near-zero. The result is a physically meaningless number that silently corrupts every aggregate it touches. In reth's case, 8 minutes of unguarded cache hits permanently inflated 24 hours of counters. The fix is a single `if` statement. The cost of not having it was a 2.6x phantom that made headlines.

We plan to write a separate post exploring how different clients measure Ggas/s and what it would take to make cross-client comparisons meaningful. In the meantime, we hope this investigation is useful to the reth team and to anyone building systems where performance metrics matter.
