# Hardware Requirements

> NOTE: The guidance in this document applies to running an L1 (Ethereum) node. L2 deployments (sequencers, provers and related infra) have different hardware profiles and operational requirements — see the "L2" section below for details.

Hardware requirements depend primarily on the **network** you're running — for example, **Hoodi**, **Sepolia**, or **Mainnet**.

## General Recommendations

Across all networks, the following apply:

- **Disk Type:** Use **high-performance NVMe SSDs**. For multi-disk setups, **software RAID 0** is recommended to maximize speed and capacity. **Avoid hardware RAID**, which can limit NVMe performance.
- **RAM:** Sufficient memory minimizes sync bottlenecks and improves stability under load.
- **CPU:** 4-8 Cores.
  - x86-64 bit Processors must be compatible with the instruction set AVX2.

---

## Disk and Memory Requirements by Network

| Network | Disk (Minimum) | Disk (Recommended) | RAM (Minimum) | RAM (Recommended) |
|------|------------------|--------------------|----------------|-------------------|
| **Ethereum Mainnet** | 500 GB | 1 TB | 32 GB | 64 GB |
| **Ethereum Sepolia** | 250 GB | 400 GB| 32 GB | 64 GB |
| **Ethereum Hoodi** | 60 GB | 100 GB | 32 GB | 64 GB |

These figures are for the **default** node profile, which keeps only block headers
below the snap-sync pivot. Enabling historical chain backfill raises the disk
requirement considerably — see below.

---

## With historical chain backfill enabled

[Historical chain backfill](../l1/fundamentals/history_backfill.md)
(`--history.chain`, off by default) additionally stores the block bodies and
receipts for blocks below the sync pivot. This affects **disk only**; RAM and CPU
requirements are unchanged, because backfill is a bounded, rate-limited
background task that yields to chain-head following.

| `--history.chain` | Extra disk (history) | Total disk, Mainnet | Status |
|---|---|---|---|
| `off` (default) | — | see table above | Measured |
| `postmerge` | ~0.7–1.2 TB | ~2 TB minimum, 3–4 TB recommended | **Provisional — run in progress** |
| `all` | TBD | TBD | TBD |

### Measured so far — Mainnet, `postmerge`

Sampled over 4.75 days on a mainnet node backfilling with
`--history.chain postmerge`:

| Metric | Value |
|---|---|
| Blocks backfilled | 2.85 M (frontier 25,530,850 → 22,685,024) |
| History added (`bodies` + `receipts_v2` + `transaction_locations`) | 340 GiB |
| Cost per block | ~125 KiB |
| Fill rate | ~600 k blocks/day (~70 GiB/day) |
| Remaining to the merge block (15,537,394) | ~7.1 M blocks |
| Database total at ~40% filled | 825 GiB (of which ~460 GiB is state) |

Extrapolating that per-block cost over the remaining range gives roughly 1.2 TB
of history for the full post-merge span, but this is an **upper bound**: the
blocks measured so far are the most recent and therefore the largest, and the
fill rate has been accelerating (107 k → 253 k blocks per 6 h) as it reaches
older, smaller blocks. The true total is expected toward the lower end of the
range above.

> These `postmerge` numbers are provisional and will be replaced with final
> measured totals once the reference run reaches the merge block. The `all`
> profile has not been measured yet; it additionally depends on peer
> availability for pre-merge history, which is limited after the 2025 history
> expiry rollout.

---

## L2

TBD
