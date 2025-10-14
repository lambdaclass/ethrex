# Transactions per Second benchmarks

Goal is to estimate L2 TPS, using a single prover running in a RTX 4090 GPU.

Benchmark server hardware:

- l2-gpu-3:
    - AMD EPYC 7713 64-Core Processor
    - 128 GB RAM
    - RTX 4090 24 GB
- ethrex-gpu-4090-1
    - AMD EPYC 7542 32-Core Processor
    - 48 GB RAM
    - RTX 4090 24 GB

## ETH Transfers only

Setup:

- Validium L2 (no blobs)
- Block time 12 seconds
- Batch time 12*64 = 768 seconds = 12m 48s (64 block batch)
- Prover: SP1

| TPS | Avg. batch size (blocks) | Avg. block gas | Proving time (avg. 2 batches) | Prover keeps up with chain? (proving time ≤ batch time) | Server (both have RTX 4090) |
| --- | --- | --- | --- | --- | --- |
| 1 | 63 | missing | 4m 15s | ✅ | l2-gpu-3 |
| 3 | 63 | 755k | 8m 49s | ✅ | ethrex-gpu-4090-1 |
| 5 | 63 | 1.25M | 13m 12s | ❌ | ethrex-gpu-4090-1 |

**Note:**

1. Validium doesn’t include blob KZG verification, which adds some overhead.
2. Validium doesn’t publishes blobs, which can limit batch size.
3. The L2 state is small. A bigger state means a bigger trie, which implies more trie/hashing operations that increase proving time.

### How to reproduce

1. Install [rex (ver readme)](https://github.com/lambdaclass/rex)
    1. clone the repo and execute `make cli`
2. Install [SP1 toolchain](https://docs.succinct.xyz/docs/sp1/getting-started/install#option-1-prebuilt-binaries-recommended)
3. Checkout the `l2/tps` ethrex branch
    1. in there I added a script to spam txs to ethrex
    2. and I modified the prover log level from **debug** to **info**
4. cd `crates/l2`, build and init the prover: 
    1. `make build-prover PROVER=sp1 G=1` 
    2. `make init-prover PROVER=sp1 G=1`
5. In a different terminal (cd `crates/l2`) init L1 and L2:
    1. `make init ETHREX_DEPLOYER_SP1_DEPLOY_VERIFIER=true ETHREX_NO_MONITOR=true ETHREX_L2_VALIDIUM=true ETHREX_BLOCK_PRODUCER_BLOCK_TIME=12000 ETHREX_COMMITTER_COMMIT_TIME=768000`
6. In a different terminal (cd `tooling/rex_scripts/` ) execute the TX spammer
    1. `N=<tx per sec> L2=true ./tx.py`
