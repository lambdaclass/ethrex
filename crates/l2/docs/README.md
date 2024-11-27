# ethrex L2 Docs

For a high level overview of the L2:

- [General Overview](./overview.md)

For more detailed documentation on each part of the system:

- [Contracts](./contracts.md)
- [Execution program](./program.md)
- [Proposer](./proposer.md)
- [Prover](./prover.md)

## Configuration

Configuration is done through env vars. A detailed list is available in each part documentation.

## Testing

Load tests are available via L2 CLI. The test take a list of private keys and send a bunch of transactions from each of them to some address. To run them, use the following command on the root of this repo:

```bash
ethrex_l2 test load --path ./test_data/private_keys.txt -i 1000 -v  --value 1
```

The command will, for each private key in the `private_keys.txt` file, send 1000 transactions with a value of `1` to a random account. If you want to send all transfers to the same account, pass

```
--to <account_address>
```

The `private_keys.txt` file contains the private key of every account we use for load tests.

Use `--help` to see more available options.

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
sudo -E CARGO_PROFILE_RELEASE_DEBUG=true cargo flamegraph --bin ethrex --features dev  --  --network test_data/genesis-l2.json --http.port 1729
```

For `reth`:

```
sudo cargo flamegraph --profile profiling -- node --chain <path_to_genesis-load-test.json> --dev --dev.block-time 5000ms --http.port 1729
```

### Samply

To run with samply, run

```
samply record ./target/profiling/reth node --chain ../ethrex/test_data/genesis-load-test.json --dev --dev.block-time 5000ms --http.port 1729
```
