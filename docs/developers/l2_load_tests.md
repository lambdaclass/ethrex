# L2 load-tests

## Testing

Load tests are available via L2 CLI and Makefile targets.

### Makefile

There are currently three different load tests you can run:

```sh
make load-test
make load-test-erc20
make load-test-fibonacci
make load-test-io
```

The first one sends regular transfers between accounts, the second runs an EVM-heavy contract that computes fibonacci numbers, the third a heavy IO contract that writes to 100 storage slots per transaction.

## Load test comparison against Reth

To run a load test on Reth, clone the repo, then run

```sh
cargo run --release -- node --chain <path_to_genesis-load-test.json> --dev --dev.block-time 5000ms --http.port 1729
```

to spin up a reth node in `dev` mode that will produce a block every 5 seconds.

Reth has a default mempool size of 10k transactions. If the load test goes too fast it will reach the limit; if you want to increase mempool limits pass the following flags:

```sh
--txpool.max-pending-txns 100000000 --txpool.max-new-txns 1000000000 --txpool.pending-max-count 100000000 --txpool.pending-max-size 10000000000 --txpool.basefee-max-count 100000000000 --txpool.basefee-max-size 1000000000000 --txpool.queued-max-count 1000000000
```

### Changing block gas limit

By default the block gas limit is the one Ethereum mainnet uses, i.e. 30 million gas. If you wish to change it, just edit the `gasLimit` field in the genesis file (in the case of `ethrex` it's `genesis-l2.json`, in the case of `reth` it's `genesis-load-test.json`). Note that the number has to be passed as a hextstring.

## Flamegraphs

To analyze performance during load tests (both `ethrex` and `reth`) you can use `cargo flamegraph` to generate a flamegraph of the node.

For `ethrex`, you can run the server with:

```sh
sudo -E CARGO_PROFILE_RELEASE_DEBUG=true cargo flamegraph --bin ethrex --features dev --root -- --network test_data/genesis-l2.json --http.port 1729 --dev
```

For `reth`:

```sh
sudo cargo flamegraph --profile profiling -- node --chain <path_to_genesis-load-test.json> --dev --dev.block-time 5000ms --http.port 1729
```

### With Make Targets

There are some make targets inside the root's Makefile.

You will need two terminals:

1. `make start-node-with-flamegraph` &rarr; This starts the ethrex client.
2. `make flamegraph` &rarr; This starts a script that sends a bunch of transactions, the script will stop ethrex when the account reaches a certain balance.

### Samply

To run with samply, run

```sh
samply record ./target/profiling/reth node --chain ../ethrex/test_data/genesis-load-test.json --dev --dev.block-time 5000ms --http.port 1729
```
