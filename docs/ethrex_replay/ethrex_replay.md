# ethrex-replay

A tool for executing and proving Ethereum blocks, transactions, and L2 batches — inspired by [starknet-replay](https://github.com/lambdaclass/starknet-replay).
Currently ethrex replay only works against ethrex nodes with the `debug_executionWitness` RPC endpoint.

## Status

| Node       | Network | `ethrex-replay execute block` | Additional Comments                                                                                           | `ethrex-replay prove block` | Additional Comments                                                                  |
| ---------- | ------- | ----------------------------- | ------------------------------------------------------------------------------------------------------------- | --------------------------- | ------------------------------------------------------------------------------------ |
| reth       | hoodi   | ✅                            | Most of the recent blocks can be executed with or without SP1 and proved with SP1                             | ✅                          | Every block that is successfully executed with SP1, can also be successfully proved. |
| reth       | sepolia | ✅                            | Most of the recent blocks can be executed with or without SP1 and proved with SP1                             | ✅                          | SP1 panicked in all attempts to prove blocks.                                        |
| reth       | mainnet | ✅                            | Works reliably. Execution with SP1 works in most of the blocks, but with SP1 it works with ~8/10 success rate | 🏗️                          | -                                                                                    |
| geth       | hoodi   | ✅                            | -                                                                                                             | 🔜                          | -                                                                                    |
| nethermind | hoodi   | 🏗️                            | Fails sometimes.                                                                                              | 🔜                          | -                                                                                    |
| ethrex     | hoodi   | ✅                            | -                                                                                                             | 🔜                          | -                                                                                    |
| ethrex     | sepolia | ✅                            | -                                                                                                             | 🔜                          | -                                                                                    |
| ethrex     | mainnet | 🔜                            | -                                                                                                             | 🔜                          | -                                                                                    |
| erigon     | hoodi   | 🔜                            | -                                                                                                             | 🔜                          | -                                                                                    |
| geth       | sepolia | 🔜                            | -                                                                                                             | 🔜                          | -                                                                                    |
| nethermind | sepolia | 🔜                            | -                                                                                                             | 🔜                          | -                                                                                    |
| erigon     | sepolia | 🔜                            | -                                                                                                             | 🔜                          | -                                                                                    |
| geth       | mainnet | 🔜                            | -                                                                                                             | 🔜                          | -                                                                                    |
| nethermind | mainnet | 🔜                            | -                                                                                                             | 🔜                          | -                                                                                    |
| erigon     | mainnet | 🔜                            | -                                                                                                             | 🔜                          | -                                                                                    |

## Client Compatibility

| Client     | `ethrex-replay execute block` | `ethrex-replay prove block` |
| ---------- | ----------------------------- | --------------------------- |
| reth       | ✅                            | ✅                          |
| geth       | ✅                            | ✅                          |
| nethermind | 🏗️                            | 🏗️                          |
| ethrex     | ✅                            | ✅                          |
| erigon     | 🔜                            | 🔜                          |

## Getting Started

### Dependencies

#### [RISC0](https://dev.risczero.com/api/zkvm/install)

```sh
curl -L https://risczero.com/install | bash
rzup install cargo-risczero 2.3.1
rzup install rust
```

#### [SP1](https://docs.succinct.xyz/docs/sp1/introduction)

```sh
curl -L https://sp1up.succinct.xyz | bash
sp1up --version 5.0.8
```

### Installation

#### From Source

> [!IMPORTANT]
> The following instructions show how to install the tool without features. In the section below, we list the available features and how to enable them during installation.

**Build for L1 CPU execution/proving**

```
git clone git@github.com:lambdaclass/ethrex.git

cd ethrex

cargo install --locked --path ./cmd/ethrex_replay
```

**Build for L1 GPU execution/proving with SP1**

> [!WARNING]
> Building with GPU support requires a CUDA-capable GPU and the CUDA toolkit installed.

```
git clone git@github.com:lambdaclass/ethrex.git

cd ethrex

cargo install --locked --path ./cmd/ethrex_replay --features gpu
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
> If `BLOCK_NUMBER` is not provided, the latest block will be executed.

```
ethrex-replay execute block <BLOCK_NUMBER> --rpc-url <RPC_URL> --network <NETWORK>
```

### Prove a single block

> [!NOTE]
>
> 1. If `BLOCK_NUMBER` is not provided, the latest block will be executed and proved.
> 2. Proving requires a prover backend to be enabled during installation (e.g., `sp1` or `risc0`).
> 3. Proving with GPU requires the `gpu` feature to be enabled during installation.
> 4. If proving with SP1, add `SP1_PROVER=cuda` to the command to enable GPU support.

```
ethrex-replay prove block <BLOCK_NUMBER> --rpc-url <RPC_URL> --network <NETWORK>
```

### Execute an L2 batch

```
ethrex-replay execute batch <BATCH_NUMBER> --rpc-url <RPC_URL> --network <NETWORK>
```

### Prove an L2 batch

> [!NOTE]
>
> 1. Proving requires a prover backend to be enabled during installation (e.g., `sp1` or `risc0`). Proving with GPU requires the `gpu` feature to be enabled during installation.
> 2. If proving with SP1, add `SP1_PROVER=cuda` to the command to enable GPU support.

```
ethrex-replay prove batch <BATCH_NUMBER> --rpc-url <RPC_URL> --network <NETWORK>
```

### Execute a transaction

```
ethrex-replay execute transaction <TX_HASH> --rpc-url <RPC_URL> --network <NETWORK>
```

### Plot block composition

```
ethrex-replay block-composition <START_BLOCK> <END_BLOCK> --rpc-url <RPC_URL> --network <NETWORK>
```

---

## Benchmarking & Profiling

### Run Samply

> [!IMPORTANT]
>
> 1. The `ethrex-replay` binary must be built with the `profiling` feature enabled.
> 2. The `TRACE_SAMPLE_RATE` environment variable controls the sampling rate (in milliseconds). Adjust it according to your needs.

```
TRACE_FILE=output.json TRACE_SAMPLE_RATE=1000 ethrex-replay <COMMAND> [ARGS]
```

### Run Bytehound

> [!IMPORTANT]
>
> 1. The following requires [Jemalloc](https://github.com/jemalloc/jemalloc) and to be installed.
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
