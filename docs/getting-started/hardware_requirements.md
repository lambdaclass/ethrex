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

---

## L2

TBD
