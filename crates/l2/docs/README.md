# ethrex L2 Docs

For a high level overview of the L2:

- [General Overview](./overview.md)

For more detailed documentation on each part of the system:

- [Contracts](./contracts.md)
- [Execution program](./program.md)
- [Proposer](./proposer.md)
- [Prover](./prover.md)

## Quick HandsOn

1. `cd crates/l2`
2. `make rm-db-l2 && make down`
   - It will remove any old database, if present, stored in your computer. The absolute path of libmdbx is defined by [data_dir](https://docs.rs/dirs/latest/dirs/fn.data_dir.html).
3. `cp configs/config_example.toml configs/config.toml` &rarr; check if you want to change any config.
   - Ideally the `salt_is_zero` can be set to `false` to allow multiple contracts deployments.
4. `make init`
   - Init the L1 in a docker container on port `8545`.
   - Deploy the needed contracts for the L2 on the L1.
   - Start the L2 locally on port `1729`.


For more information on how to run the L2 node with the prover attached to it, the [Prover Docs](./prover.md) provides more insight.

## Configuration

Configuration consists of two steps, the parsing of a `.toml` config file and the creation and modification of a `.env` file, then each component reads the `.env` to load the environment variables. A detailed list is available in each part documentation.

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

### CLI

To have more control over the load tests and its parameters, you can use the CLI (the Makefile targets use the CLI underneath).

The tests take a list of private keys and send a bunch of transactions from each of them to some address (either the address of some account to send eth to or the address of the contract that we're interacting with). 

The CLI can be installed with the `cli` target:

```sh
make cli
```

To run the load-test, use the following command on the root of this repo:

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
