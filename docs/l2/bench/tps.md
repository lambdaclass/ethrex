# Transactions per second benchmarks
Benchmark server hardware:

- l2-gpu-3:
    - AMD EPYC 7713 64-Core Processor
    - 128 GB RAM
    - RTX 4090 24 GB
- ethrex-gpu-4090-1
    - AMD EPYC 7542 32-Core Processor
    - 48 GB RAM
    - RTX 4090 24 GB

# Transactions per Second

Common setup:
- Block time 12 seconds
- Batch time 12*64 = 768 seconds = 12m 48s (64 block batch)
- Prover: SP1, running in a RTX 4090

## ETH Transfers only

Validium L2 (no blobs):

| TPS | Avg. batch size (blocks) | Avg. block gas | Proving time (avg. 2 batches) | Prover keeps up with chain? (proving time ≤ batch time) | Server (both have RTX 4090) |
| --- | --- | --- | --- | --- | --- |
| 1 | 63 | missing | 4m 15s | ✅ | l2-gpu-3 |
| 3 | 63 | 755k | 8m 49s | ✅ | ethrex-gpu-4090-1 |
| 5 | 63 | 1.25M | 13m 12s | ❌ | ethrex-gpu-4090-1 |

## ERC20 Transfers only

Validium L2 (no blobs):

| TPS | Avg. batch size (blocks) | Avg. block gas | Proving time (avg. 2 batches) | Prover keeps up with chain? (proving time ≤ batch time) | Server (both have RTX 4090) |
| --- | --- | --- | --- | --- | --- |
| 1 | 63 | 625k | 7m 27s | ✅ | l2-gpu-3 |
| 2 | 63 | 1.25M | 10m 6s | ✅ | ethrex-gpu-4090-1 |
| 3 | 63 | 1.87M | 21m 30s | ❌ | l2-gpu-3 |
| 4 | 63 | 2.52M | 20m 8s | ❌ | ethrex-gpu-4090-1 |

Rollup L2 (publishes blobs)

| TPS | Avg. batch size (blocks) | Avg. block gas | Proving time (avg. 2 batches) | Prover keeps up with chain? (proving time ≤ batch time) | Server (both have RTX 4090) |
| --- | --- | --- | --- | --- | --- |
| 2 | 63 | 1.38M | 10m 52s | ✅ | ethrex-gpu-4090-1 |
| 3 | 63 | 2.10M | 17m 12s | ❌ | l2-gpu-3 |

Rollup L2 (publishes blobs), with 1.000.000 genesis accounts (big state)

| TPS | Avg. batch size (blocks) | Avg. block gas | Proving time (avg. 2 batches) | Prover keeps up with chain? (proving time ≤ batch time) | Server (both have RTX 4090) |
| --- | --- | --- | --- | --- | --- |
| 2 | 63 | 1.07M | 10m 6s | ✅ | l2-gpu-3 |
| 3 | 63 | 1.87M | 16m 36s | ❌ | l2-gpu-3 |

**Note:**

1. Validium doesn’t include blob KZG verification, which adds some overhead.
2. Validium doesn’t publish blobs, which can limit batch size.
3. The L2 state is small. A bigger state means a bigger trie, which implies more trie/hashing operations that increase proving time.
4. For the big state case, the accounts don’t have storage nor code.

### How to reproduce

1. Install [SP1 toolchain](https://docs.succinct.xyz/docs/sp1/getting-started/install#option-1-prebuilt-binaries-recommended)
2. Checkout the `l2/tps` ethrex branch
    1. in there I modified load tests to add a tps test
    2. and I modified the prover log level from **debug** to **info**
3. cd `crates/l2`, build and init the prover: 
    1. `make build-prover PROVER=sp1 G=1` 
    2. `make init-prover PROVER=sp1 G=1`
4. In a different terminal (cd `crates/l2`) init L1 and L2:
    1. for this execute `make init` setting env. vars according to the L2 setup:
        1. `ETHREX_DEPLOYER_SP1_DEPLOYER_VERIFIER=true`
        2. `ETHREX_NO_MONITOR=true`
        3. `ETHREX_L2_VALIDIUM=<true,false>`
        4. `ETHREX_BLOCK_PRODUCER_BLOCK_TIME=12000`
        5. `ETHREX_COMMITTER_COMMIT_TIME=768000`
5. In a different terminal (cd `tooling/load_test/` ) execute the TPS test
    ```rust
    cargo r -r -- load \
	--node http://127.0.0.1:1729 \
	--pkeys ../../fixtures/keys/private_keys.txt \
	--test-type <eth-transfers,erc20> \
	tps \
	--rate <tx per second>
    ```
