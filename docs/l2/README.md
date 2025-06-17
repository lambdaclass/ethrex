# Ethrex L2 documentation

See [General Overview](./overview.md) for a high-level view of the ethrex L2 stack.

[Getting started](./getting_started.md) contains a brief guide on setting up an ethrex L2 stack.

For more detailed documentation on each part of the system:

- [Sequencer](./sequencer.md): Describes the components and configuration of the L2 sequencer node.
- [Contracts](./contracts.md): Explains the L1 and L2 smart contracts used by the system.
- [Prover](./prover.md): Details how block execution proofs are generated and verified using zkVMs.
- [State Diffs](./state_diffs.md): Specifies the format for state changes published for data availability.
- [Withdrawals](./withdrawals.md): Explains the mechanism for withdrawing funds from L2 back to L1.

For how to install our dependencies, go to their official documentation:

- [Rust](https://www.rust-lang.org/tools/install)
- [Solc 0.29](https://docs.soliditylang.org/en/latest/installing-solidity.html)
- [Docker](https://docs.docker.com/engine/install/)

## Configuration

Configuration consists of the creation and modification of a `.env` file done automatically by the contract deployer, then each component reads the `.env` to load the environment variables. A detailed list is available in each part documentation.

## Testing

Load tests are available via L2 CLI and Makefile targets.

### Makefile

There are currently three different load tests you can run:

```
make load-test
make load-test-fibonacci
make load-test-io
```

The first one sends regular transfers between accounts, the second runs an EVM-heavy contract that computes fibonacci numbers, the third a heavy IO contract that writes to 100 storage slots per transaction.

## Load test comparison against Reth

To run a load test on Reth, clone the repo, then run

```
cargo run --release -- node --chain <path_to_genesis-load-test.json> --dev --dev.block-time 5000ms --http.port 1729
```

to spin up a reth node in `dev` mode that will produce a block every 5 seconds.

Reth has a default mempool size of 10k transactions. If the load test goes too fast it will reach the limit; if you want to increase mempool limits pass the following flags:

```
--txpool.max-pending-txns 100000000 --txpool.max-new-txns 1000000000 --txpool.pending-max-count 100000000 --txpool.pending-max-size 10000000000 --txpool.basefee-max-count 100000000000 --txpool.basefee-max-size 1000000000000 --txpool.queued-max-count 1000000000
```

### Changing block gas limit

By default the block gas limit is the one Ethereum mainnet uses, i.e. 30 million gas. If you wish to change it, just edit the `gasLimit` field in the genesis file (in the case of `ethrex` it's `genesis-l2.json`, in the case of `reth` it's `genesis-load-test.json`). Note that the number has to be passed as a hextstring.

## Flamegraphs

To analyze performance during load tests (both `ethrex` and `reth`) you can use `cargo flamegraph` to generate a flamegraph of the node.

For `ethrex`, you can run the server with:

```
sudo -E CARGO_PROFILE_RELEASE_DEBUG=true cargo flamegraph --bin ethrex --features dev  --  --network test_data/genesis-l2.json --http.port 1729 --dev
```

For `reth`:

```
sudo cargo flamegraph --profile profiling -- node --chain <path_to_genesis-load-test.json> --dev --dev.block-time 5000ms --http.port 1729
```

### With Make Targets

There are some make targets inside the root's Makefile.

You will need two terminals:
1. `make start-node-with-flamegraph` &rarr; This starts the ethrex client.
2. `make flamegraph` &rarr; This starts a script that sends a bunch of transactions, the script will stop ethrex when the account reaches a certain balance.

### Samply

To run with samply, run

```
samply record ./target/profiling/reth node --chain ../ethrex/test_data/genesis-load-test.json --dev --dev.block-time 5000ms --http.port 1729
```
