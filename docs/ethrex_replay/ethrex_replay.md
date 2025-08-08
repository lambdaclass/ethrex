# ethrex-replay

A tool for executing and proving Ethereum blocks, transactions, and L2 batches — inspired by [starknet-replay](https://github.com/lambdaclass/starknet-replay).
Currently ethrex replay only works against ethrex nodes with the `debug_executionWitness` RPC endpoint.

## Getting Started

> **Note:** All commands must be run from the `ethrex/cmd/ethrex_replay` directory.

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

### Environment Variables

Before running any command, set the following environment variables depending on the operation:

```sh
export RPC_URL=<RPC_URL>
export BLOCK_NUMBER=<BLOCK_NUMBER>
export BATCH_NUMBER=<BATCH_NUMBER>
export TX_HASH=<TRANSACTION_HASH>
export START_BLOCK=<START_BLOCK>
export END_BLOCK=<END_BLOCK>
export NETWORK=<mainnet|cancun|holesky|hoodi|sepolia|chainId>
export L2=true
```

#### Variable Descriptions

- `RPC_URL`: Ethereum JSON-RPC endpoint used to fetch on-chain data.
- `BLOCK_NUMBER`: Block number to replay. If unset, the latest block will be used.
- `BATCH_NUMBER`: L2 batch number to execute or prove.
- `TX_HASH`: Hash of the transaction to replay.
- `START_BLOCK` / `END_BLOCK`: Defines the block range to analyze and plot.
- `NETWORK`: Logical network name or chain ID. Defaults to `mainnet`.
- `L2`: Set to `true` to run transactions in L2 mode.

> You only need to set the variables required by the command you're running.

---

## Running Examples

### Execute a single block (no proving)

Required: `RPC_URL`.
Optionally: `BLOCK_NUMBER`, `NETWORK`

```sh
make sp1           # SP1 (CPU)
make sp1-gpu       # SP1 (GPU)
make risc0         # RISC0 (CPU)
make risc0-gpu     # RISC0 (GPU)
```

### Prove a single block

Required: `RPC_URL`.
Optionally: `BLOCK_NUMBER`, `NETWORK`.

```sh
make prove-sp1
make prove-sp1-gpu
make prove-risc0
make prove-risc0-gpu
```

### Execute an L2 batch (no proving)

Required: `RPC_URL`, `BATCH_NUMBER`, `NETWORK`.

```sh
make batch-sp1
make batch-sp1-gpu
make batch-risc0
make batch-risc0-gpu
```

### Prove an L2 batch

Required: `RPC_URL`, `BATCH_NUMBER`, `NETWORK`.

```sh
make prove-batch-sp1
make prove-batch-sp1-gpu
make prove-batch-risc0
make prove-batch-risc0-gpu
```

### Execute a transaction

Required: `RPC_URL`, `TX_HASH`, `NETWORK`. 
Optionally: `L2=true` (if the transaction is L2-specific)

```sh
make transaction
```

### Plot block composition

Required: `RPC_URL`, `START_BLOCK`, `END_BLOCK`.
Optionally: `NETWORK`

```sh
make plot
```

---

## Check All Available Commands

Run:

```sh
make help
```
