# Mainnet Storage Access Analysis - Overnight Run

**Date**: 2026-01-20
**Duration**: ~12 hours overnight
**Block range**: 24,270,198 - 24,275,016 (4,819 blocks)

---

## Summary

| Metric | Value |
|--------|-------|
| Total operations | 44,156,685 |
| SLOAD operations | 37,308,917 (84.5%) |
| SSTORE operations | 6,847,768 (15.5%) |
| Unique addresses | 43,569 |
| Avg operations/block | 9,163 |

---

## Top 30 Contracts by Storage Operations

| # | Address | Protocol | SLOADs | SSTOREs | Total | % | Unique Slots |
|---|---------|----------|--------|---------|-------|---|--------------|
| 1 | `0xdac17f958d2ee523a2206206994597c13d831ec7` | **USDT** | 7,110,128 | 1,153,098 | 8,263,226 | **18.71%** | 435,626 |
| 2 | `0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48` | **USDC** | 6,600,330 | 850,891 | 7,451,221 | **16.87%** | 151,404 |
| 3 | `0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2` | **WETH** | 2,074,323 | 561,264 | 2,635,587 | **5.97%** | 17,473 |
| 4 | `0x06450dee7fd2fb8e39061434babcfc05599a6fb8` | **XEN Crypto** | 1,409,766 | 445,500 | 1,855,266 | **4.20%** | 364,972 |
| 5 | `0x000000000004444c5dc75cb358380d2e3de08a90` | Uniswap Permit2 | 633,590 | 119,459 | 753,049 | 1.71% | 18,500 |
| 6 | `0x62de5ea210b8a24697383a5936ceb1ebbc1d25c6` | ? | 258,258 | 129,000 | 387,258 | 0.88% | 2 |
| 7 | `0x004395edb43efca9885cedad51ec9faf93bd34ac` | ? | 267,777 | 44,977 | 312,754 | 0.71% | 18,138 |
| 8 | `0xc7bbec68d12a0d1830360f8ec58fa599ba1b0e9b` | ? | 224,021 | 67,119 | 291,140 | 0.66% | 833 |
| 9 | `0x2260fac5e5542a773aa44fbcfedf7c193bc2c599` | **WBTC** | 223,544 | 53,713 | 277,257 | 0.63% | 3,633 |
| 10 | `0x87870bca3f3fd6335c3f4ce8392d69350b4fa4e2` | **Aave V3 Pool** | 215,860 | 27,961 | 243,821 | 0.55% | 3,402 |
| 11 | `0x603bb2c05d474794ea97805e8de69bccfb3bca12` | ? | 223,797 | 7,847 | 231,644 | 0.52% | 3,631 |
| 12 | `0xe0554a476a092703abdb3ef35c80e0d76d32939f` | ? | 174,015 | 51,774 | 225,789 | 0.51% | 460 |
| 13 | `0xbbbbbbbbbb9cc5e90e3b3af64bdaf62c37eeffcb` | Morpho Blue | 202,265 | 13,604 | 215,869 | 0.49% | 3,130 |
| 14 | `0x6b175474e89094c44da98b954eedeac495271d0f` | **DAI** | 166,799 | 43,647 | 210,446 | 0.48% | 7,507 |
| 15 | `0x1445f32d1a74872ba41f3d8cf4022e9996120b31` | ? | 151,168 | 35,983 | 187,151 | 0.42% | 1,293 |
| 16 | `0x2f50d538606fa9edd2b11e2446beb18c9d5846bb` | ? | 157,179 | 3 | 157,182 | 0.36% | 638 |
| 17 | `0x0a252663dbcc0b073063d6420a40319e438cfa59` | ? | 147,068 | 2,034 | 149,102 | 0.34% | 2,324 |
| 18 | `0x52aa899454998be5b000ad077a46bbe360f4e497` | ? | 108,295 | 31,834 | 140,129 | 0.32% | 180 |
| 19 | `0x75d10548d717b7a82ec1696cacfc7c9a5cb980d5` | ? | 75,927 | 37,751 | 113,678 | 0.26% | 2 |
| 20 | `0x3b4d794a66304f130a4db8f2551b0070dfcf5ca7` | ? | 88,560 | 20,155 | 108,715 | 0.25% | 6,627 |
| 21 | `0x88e6a0c2ddd26feeb64f039a2c41296fcb3f5640` | Uniswap V3 USDC/ETH | 79,972 | 22,467 | 102,439 | 0.23% | 1,694 |
| 22 | `0xc36442b4a4522e871399cd717abdd847ab11fe88` | Uniswap V3 Positions NFT | 86,781 | 15,577 | 102,358 | 0.23% | 11,399 |
| 23 | `0xef4fb24ad0916217251f553c0596f8edc630eb66` | ? | 79,403 | 21,715 | 101,118 | 0.23% | 8,743 |
| 24 | `0xc3616f5255d78a8a103340bc67a95321f5768440` | ? | 91,279 | 9,039 | 100,318 | 0.23% | 1,315 |
| 25 | `0xacdb27b266142223e1e676841c1e809255fc6d07` | ? | 80,148 | 19,413 | 99,561 | 0.23% | 456 |
| 26 | `0x0000000071727de22e5e9d8baf0edac6f37da032` | ERC-4337 EntryPoint | 68,319 | 27,942 | 96,261 | 0.22% | 3,471 |
| 27 | `0xe0e0e08a6a4b9dc7bd67bcb7aade5cf48157d444` | ? | 75,472 | 20,573 | 96,045 | 0.22% | 727 |
| 28 | `0xd533a949740bb3306d119cc777fa900ba034cd52` | **CRV** (Curve) | 71,694 | 22,584 | 94,278 | 0.21% | 1,130 |
| 29 | `0xba1333333333a1ba1108e8412f11850a5c319ba9` | ? | 75,496 | 16,463 | 91,959 | 0.21% | 852 |
| 30 | `0x3432b6a60d23ca0dfca7761b7ab56459d9c964d0` | Frax Share (FXS) | 74,347 | 17,309 | 91,656 | 0.21% | 4,579 |

**Top 30 contracts account for 57.0% of all storage operations**

---

## Concentration Analysis

| Scope | % of Storage Ops |
|-------|------------------|
| Top 1 (USDT) | 18.71% |
| Top 3 (USDT + USDC + WETH) | **41.55%** |
| Top 4 (+ XEN Crypto) | **45.75%** |
| Top 10 | 50.89% |
| Top 100 | 65.92% |
| Top 1000 | 85.62% |

---

## Contract Categories

### Stablecoins (35.6% combined)
- USDT: 18.71%
- USDC: 16.87%

### Wrapped Assets (6.6% combined)
- WETH: 5.97%
- WBTC: 0.63%

### DeFi Protocols
- XEN Crypto: 4.20% (ranked minting/staking)
- Uniswap Permit2: 1.71%
- Aave V3 Pool: 0.55%
- Morpho Blue: 0.49%
- DAI: 0.48%
- Uniswap V3 pools/positions: ~0.5%
- Curve (CRV): 0.21%

### Account Abstraction
- ERC-4337 EntryPoint: 0.22%

---

## Per-Block Statistics

| Metric | Value |
|--------|-------|
| Avg operations/block | 9,163 |
| Min operations/block | 33 |
| Max operations/block | 33,463 |

---

## Address Distribution (Histogram)

How many addresses fall into each "hotness" bucket:

| Storage Ops per Address | # of Addresses | % of Addresses | Notes |
|-------------------------|----------------|----------------|-------|
| 1 | 1,679 | 3.9% | Touched once |
| 2-10 | 12,157 | 27.9% | Cold |
| 11-100 | 19,089 | 43.8% | Lukewarm (most contracts) |
| 101-1K | 8,041 | 18.5% | Warm |
| 1K-10K | 2,241 | 5.1% | Hot |
| 10K-100K | 338 | 0.8% | Very hot |
| 100K-1M | 20 | 0.0% | Extremely hot |
| >1M | 4 | 0.0% | USDT, USDC, WETH, XEN |

This is a classic power-law distribution: most contracts (43.8%) are "lukewarm" with 11-100 accesses over 4,819 blocks, while only 4 contracts exceed 1 million accesses.

---

## Key Insights

### 1. Stablecoin Dominance
USDT and USDC alone account for **35.6%** of all storage operations. These are simple ERC-20 contracts with balance mappings - their dominance reflects the volume of stablecoin transfers on Ethereum.

### 2. XEN Crypto Surprise
XEN Crypto at #4 (4.2%) was unexpected. It has the most unique slots (365K) of any contract, reflecting its user-rank tracking mechanism. This is a "long tail" contract - many users with individual state.

### 3. Predictable Hot Set
The top 10 contracts are consistently hot across blocks:
- 3 stablecoins (USDT, USDC, DAI)
- 2 wrapped assets (WETH, WBTC)
- Core DeFi infrastructure (Uniswap, Aave, Permit2)

### 4. Read-Heavy Workload
84.5% SLOADs vs 15.5% SSTOREs indicates:
- Many balance checks that don't result in transfers
- DeFi protocols reading prices/reserves
- Access list warming reads

---

## Optimization Implications

### Tier 1: Contract-Aware Caching
Pre-warm and pin trie nodes for top 4-10 contracts:
- USDT, USDC, WETH, XEN Crypto = 45.75% of ops
- Add WBTC, Aave, DAI = ~48% of ops

### Tier 2: Slot-Level Hot Cache
Within contracts, cache frequently-accessed slots:
- USDT/USDC: ~150K-435K unique slots, but likely Pareto distribution
- WETH: only 17K slots - could cache entire contract storage

### Tier 3: Predictive Pre-warming
Given transaction mempool, predict which contract storage will be accessed:
- Token transfers are obvious (sender/receiver balances)
- DEX swaps touch known reserve slots

---

## Raw Data

- Trace file: `storage_trace.bin` (2.5 GB)
- CSV export: `storage_trace.csv`
- Analysis script: `tooling/analyze_storage_trace.py`
