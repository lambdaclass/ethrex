# ethrex-replay

A tool for executing and proving Ethereum blocks, transactions, and L2 batches — inspired by [starknet-replay](https://github.com/lambdaclass/starknet-replay).

## Features

### L1

| Feature                           | Description                                                                                                                            |
| --------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------- |
| `ethrex-replay block`             | Replay a single block.                                                                                                                 |
| `ethrex-replay blocks`            | Replay a list of specific block numbers, a range of blocks, or from a specific block to the latest (see `ethrex-replay blocks --help`) |
| `ethrex-replay block-composition` |                                                                                                                                        |
| `ethrex-replay custom`            | Build your block before to replay it.                                                                                                  |
| `ethrex-replay transaction`       | Replay a single transaction of a block.                                                                                                |
| `ethrex-replay cache`             | Generate witness data prior to block replay (see `ethrex-replay cache --help`)                                                         |

### L2

| Feature                        | Description |
| ------------------------------ | ----------- |
| `ethrex-replay l2 batch`       |             |
| `ethrex-replay l2 block`       |             |
| `ethrex-replay l2 custom`      |             |
| `ethrex-replay l2 transaction` |             |

## Supported Clients

| Client     | `ethrex-replay block`         | notes                                            |
| ---------- | ----------------------------- | ------------------------------------------------ |
| ethrex     | ✅                            | `debug_executionWitness`                         |
| reth       | ✅                            | `debug_executionWitness`                         |
| geth       | ✅                            | `eth_getProof`                                   |
| nethermind | ✅                            | `eth_getProof`                                   |
| erigon     | ❌                            | V3 supports `eth_getProof` only for latest block |
| besu       | ❌                            | Doesn't return proof for non-existing accounts   |

We support any other client that is compliant with `eth_getProof` or `debug_executionWitness` endpoints.

Execution of some particular blocks with the `eth_getProof` method won't work with zkVMs. But without using these it should work for any block. Read more about this in [FAQ](./faq.md). Also, when running against a **full node** using `eth_getProof` if for some reason information retrieval were to take longer than 25 minutes it would probably fail because the node may have pruned its state (128 blocks * 12 seconds = 25,6 min), normally it doesn't take that much but be wary of that.

## Supported zkVM Replays (execution & proving)

> ✅: supported.
> ⚠️: supported, but flaky.
> 🔜: to be supported.

| zkVM   | Hoodi      | Sepolia   | Mainnet    | Public ethrex L2s |
| ------ | ---------- | --------- | ---------- | ----------------- |
| RISC0  | ✅         | ✅         | ✅         | ✅                |
| SP1    | ✅         | ✅         | ✅         | ✅                |
| OpenVM | ⚠️         | 🔜         | 🔜         | 🔜                |
| ZisK   | 🔜         | 🔜         | ⚠️         | 🔜                |
| Jolt   | 🔜         | 🔜         | 🔜         | 🔜                |
| Nexus  | 🔜         | 🔜         | 🔜         | 🔜                |
| Pico   | 🔜         | 🔜         | 🔜         | 🔜                |
| Ziren  | 🔜         | 🔜         | 🔜         | 🔜                |

## Getting Started

### Dependencies

These dependencies are optional, install them only if you want to run with the features `risc0` or `sp1` respectively. 
Make sure to use the correct versions of these.

#### [RISC0](https://dev.risczero.com/api/zkvm/install)

```sh
curl -L https://risczero.com/install | bash
rzup install cargo-risczero 3.0.3
rzup install risc0-groth16
rzup install rust
```

#### [SP1](https://docs.succinct.xyz/docs/sp1/getting-started/install)

```sh
curl -L https://sp1up.succinct.xyz | bash
sp1up --version 5.0.8
```

### Installation

#### From Cargo

```
# L1 Replay

## Install without features for vanilla execution (no prover backend)
cargo install --locked --git https://github.com/lambdaclass/ethrex.git ethrex-replay

## Install for CPU execution/proving with SP1
cargo install --locked --git https://github.com/lambdaclass/ethrex.git ethrex-replay --features sp1

## Install for CPU execution/proving with RISC0
cargo install --locked --git https://github.com/lambdaclass/ethrex.git ethrex-replay --features risc0

## Install for GPU execution/proving with SP1
cargo install --locked --git https://github.com/lambdaclass/ethrex.git ethrex-replay --features sp1,gpu

## Install for GPU execution/proving with RISC0
cargo install --locked --git https://github.com/lambdaclass/ethrex.git ethrex-replay --features risc0,gpu

# L2 Replay

## Install without features for vanilla execution (no prover backend)
cargo install --locked --git https://github.com/lambdaclass/ethrex.git ethrex-replay --features l2

## Install for CPU execution/proving with SP1
cargo install --locked --git https://github.com/lambdaclass/ethrex.git ethrex-replay --features l2,sp1

## Install for CPU execution/proving with RISC0
cargo install --locked --git https://github.com/lambdaclass/ethrex.git ethrex-replay --features l2,risc0

## Install for GPU execution/proving with SP1
cargo install --locked --git https://github.com/lambdaclass/ethrex.git ethrex-replay --features l2,sp1,gpu

## Install for GPU execution/proving with RISC0
cargo install --locked --git https://github.com/lambdaclass/ethrex.git ethrex-replay --features l2,risc0,gpu
```

### Run from Source

```
git clone git@github.com:lambdaclass/ethrex.git

cd ethrex

# L1 replay

## Vanilla execution (no prover backend)
cargo r -r -p ethrex-replay -- <COMMAND> [ARGS]

## SP1 backend
cargo r -r -p ethrex-replay --features sp1 -- <COMMAND> [ARGS]

## SP1 backend + GPU
cargo r -r -p ethrex-replay --features sp1,gpu -- <COMMAND> [ARGS]

## RISC0 backend
cargo r -r -p ethrex-replay --features risc0 -- <COMMAND> [ARGS]

## RISC0 backend + GPU
cargo r -r -p ethrex-replay --features risc0,gpu -- <COMMAND> [ARGS]

# L2 replay

## Vanilla execution (no prover backend)
cargo r -r -p ethrex-replay --features l2 -- <COMMAND> [ARGS]

## SP1 backend
cargo r -r -p ethrex-replay --features l2,sp1 -- <COMMAND> [ARGS]

## SP1 backend + GPU
SP1_PROVER=cuda cargo r -r -p ethrex-replay --features l2,sp1,gpu -- <COMMAND> [ARGS]

## RISC0 backend
cargo r -r -p ethrex-replay --features l2,risc0 -- <COMMAND> [ARGS]

## RISC0 backend + GPU
cargo r -r -p ethrex-replay --features l2,risc0,gpu -- <COMMAND> [ARGS]
```

#### Features

The following table lists the available features for `ethrex-replay`. To enable a feature, use the `--features` flag with `cargo install`, specifying a comma-separated list of features.

| Feature     | Description                                                                                                                                          |
| ----------- | ---------------------------------------------------------------------------------------------------------------------------------------------------- |
| `gpu`       | Enables GPU support with SP1 or RISC0 backends (must be combined with one of each features, e.g. `sp1,gpu` or `risc0,gpu`)                           |
| `risc0`     | Execution and proving is done with RISC0 backend                                                                                                     |
| `sp1`       | Execution and proving is done with SP1 backend                                                                                                       |
| `l2`        | Enables L2 batch execution and proving (can be combined with SP1 or RISC0 and GPU features, e.g. `sp1,l2,gpu`, `risc0,l2,gpu`, `sp1,l2`, `risc0,l2`) |
| `jemalloc`  | Use jemalloc as the global allocator. This is useful to combine with tools like Bytehound and Heaptrack for memory profiling                         |
| `profiling` | Useful to run with tools like Samply.                                                                                                                |

---

## Running Examples

### Examples ToC

- [Execute a single block from a public network](#execute-a-single-block-from-a-public-network)
- [Prove a single block](#prove-a-single-block)
- [Execute an L2 batch](#execute-an-l2-batch)
- [Prove an L2 batch](#prove-an-l2-batch)
- [Execute a transaction](#execute-a-transaction)
- [Plot block composition](#plot-block-composition)

> [!IMPORTANT]
> The following instructions assume that you've installed `ethrex-replay` as described in the [Getting Started](#getting-started) section.

### Execute a single block from a public network

> [!NOTE]
>
> 1. If `BLOCK_NUMBER` is not provided, the latest block will be executed.
> 2. If `ZKVM` is not provided, no zkVM will be used for execution.
> 3. If `RESOURCE` is not provided, CPU will be used for execution.
> 4. If `ACTION` is not provided, only execution will be performed.

```
ethrex-replay block <BLOCK_NUMBER> --zkvm <ZKVM> --resource <RESOURCE> --action <ACTION> --rpc-url <RPC_URL>
```

### Prove a single block

> [!NOTE]
>
> 1. If `BLOCK_NUMBER` is not provided, the latest block will be executed and proved.
> 2. Proving requires a prover backend to be enabled during installation (e.g., `sp1` or `risc0`).
> 3. Proving with GPU requires the `gpu` feature to be enabled during installation.
> 4. If proving with SP1, add `SP1_PROVER=cuda` to the command to enable GPU support.

```
ethrex-replay block <BLOCK_NUMBER> --zkvm <ZKVM> --resource gpu --action prove --rpc-url <RPC_URL>
```

### Execute an L2 batch

```
ethrex-replay l2 batch --batch <BATCH_NUMBER> --execute --rpc-url <RPC_URL>
```

### Prove an L2 batch

> [!NOTE]
>
> 1. Proving requires a prover backend to be enabled during installation (e.g., `sp1` or `risc0`). Proving with GPU requires the `gpu` feature to be enabled during installation.
> 2. If proving with SP1, add `SP1_PROVER=cuda` to the command to enable GPU support.
> 3. Batch replay requires the binary to be run/compiled with the `l2` feature.

```
ethrex-replay l2 batch --batch <BATCH_NUMBER> --prove --rpc-url <RPC_URL>
```

### Execute a transaction

> [!NOTE]
> L2 transaction replay requires the binary to be run/compiled with the `l2` feature.

```
ethrex-replay transaction <TX_HASH> --execute --rpc-url <RPC_URL>

ethrex-replay l2 transaction <TX_HASH> --execute --rpc-url <RPC_URL>
```

### Plot block composition

```
ethrex-replay block-composition --start-block <START_BLOCK> --end-block <END_BLOCK> --rpc-url <RPC_URL> --network <NETWORK>
```

---

## Benchmarking & Profiling

### Run Samply

We recommend building in `release-with-debug` mode so that the flamegraph is the most accurate.
```bash
cargo build -p ethrex-replay --profile release-with-debug --features <FEATURES>
```

#### On zkVMs

> [!IMPORTANT]
>
> 1. For profiling zkVMs like SP1 the `ethrex-replay` binary must be built with the `profiling` feature enabled.
> 2. The `TRACE_SAMPLE_RATE` environment variable controls the sampling rate (in milliseconds). Adjust it according to your needs.

```
TRACE_FILE=output.json TRACE_SAMPLE_RATE=1000 target/release-with-debug/ethrex-replay <COMMAND> [ARGS]
```

#### Execution without zkVMs

```bash
samply record target/release-with-debug/ethrex-replay <COMMAND> --no-zkvm [OTHER_ARGS]
```

### Run Bytehound

> [!IMPORTANT]
>
> 1. The following requires [Jemalloc](https://github.com/jemalloc/jemalloc) and [Bytehound](https://github.com/koute/bytehound) to be installed.
> 2. The `ethrex-replay` binary must be built with the `jemalloc` feature enabled.

```
export MEMORY_PROFILER_LOG=warn
LD_PRELOAD=/path/to/bytehound/preload/target/release/libbytehound.so:/path/to/libjemalloc.so  ethrex-replay <COMMAND> [ARGS]
```

### Run Heaptrack

> [!IMPORTANT]
>
> 1. The following requires [Jemalloc](https://github.com/jemalloc/jemalloc) and [Heaptrack](https://github.com/KDE/heaptrack) to be installed.
> 2. The `ethrex-replay` binary must be built with the `jemalloc` feature enabled.
> 3. Note that Heaptrack is a **Linux** profiler, so it won't work natively on macOS.

```
LD_PRELOAD=/path/to/libjemalloc.so heaptrack ethrex-replay <COMMAND> [ARGS]
heaptrack_print heaptrack.<program>.<pid>.gz > heaptrack.stacks
```

---

## Check All Available Commands

Run:

```sh
cargo r -r -p ethrex-replay -- --help
```
