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
| `postmerge` | ~1.0 TB | **2 TB minimum, 3 TB recommended** | Measured |
| `all` | not measured | not measured | Bounded by the Byzantium block |

### Measured — Mainnet, `postmerge`, complete run

A mainnet node backfilled the entire post-merge range, from its sync pivot down to
the merge block, and stopped there:

| Metric | Value |
|---|---|
| Blocks backfilled | 9,993,456 (25,530,850 → 15,537,394) |
| Wall-clock duration | ~14 days |
| Average cost per block | ~105 KiB |
| History on disk (`bodies` + `receipts_v2` + `transaction_locations` + `headers`) | ~1,019 GB |
| State on disk (unchanged by backfill) | ~459 GB |
| **Database total** | **1,490 GiB** |

Per column family, for sizing a disk:

| Column family | Size | |
|---|---|---|
| `bodies` | 592 GB | history |
| `receipts_v2` | 265 GB | history |
| `transaction_locations` | 148 GB | history |
| `headers` | 12 GB | history |
| `storage_trie_nodes` | 190 GB | state |
| `storage_flatkeyvalue` | 141 GB | state |
| `account_trie_nodes` | 72 GB | state |
| `account_flatkeyvalue` | 56 GB | state |

Backfill inverts where the database spends its space. A headers-only node is
roughly 82% state and 18% history; the same node fully backfilled is 31% state and
69% history.

Two things worth knowing when sizing from these numbers. The per-block cost is not
uniform: recent blocks measured ~125 KiB each, while the full post-merge average
came out ~105 KiB, because older post-merge blocks are smaller. And the figures
above are a completed `postmerge` run, so a node also needs headroom for RocksDB
compaction and for continued head-following growth; 2 TB is the point at which the
run fits, not the point at which it is comfortable.

> `all` has not been measured. It backfills further, down to the Byzantium block
> rather than genesis (pre-EIP-658 receipts use a format ethrex does not
> represent), and how far it actually gets depends on peer availability for
> pre-merge history, which is limited after the 2025 history expiry rollout.

---

## L2

TBD
