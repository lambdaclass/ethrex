# Ethrex Load Tests

## About

This is a command line tool to execute ERC20 load tests on any execution node.

```
Usage: load_test [OPTIONS] --pkeys <PKEYS>

Options:
  -n, --node <NODE>            URL of the node being tested. [default: http://localhost:8545]
  -k, --pkeys <PKEYS>          Path to the file containing private keys.
  -t, --test-type <TEST_TYPE>  Type of test to run. Can be eth_transfers or erc20. [default: erc20] [possible values: eth-transfers, erc20, fibonacci, io-heavy]
  -N, --tx-amount <TX_AMOUNT>  Number of transactions to send for each account. [default: 1000]
  -w, --wait <WAIT>            Timeout to wait for all transactions to be included. If 0 is specified, wait indefinitely. [default: 0]
  -h, --help                   Print help
```

The only mandatory argument is the path to the rich account private keys file.

## Simple run

Before starting, consider increasing the maximum amount of open files for the current shell with the following command:

```bash
ulimit -n 65536
```

On some machines, this fixes the `ERROR axum::serve::listener: accept error: Too many open files (os error 24)` and sometimes nonce related errors.

To run a load test, first run the node using a command like the following in the root folder:

```bash
cargo run --bin ethrex --release -- --network fixtures/genesis/load-test.json --dev
```

Genesis-l2-ci has many rich accounts and does not include the prague fork, which is important for dev mode until it's fixed.

After the node is runing, still in the repo root folder, execute the script with `make`. For example:

```bash
# Eth transfer load test
make load-test

# ERC 20 transfer load test
make load-test-erc20

# Tests a contract that executes fibonacci (high cpu)
make load-test-fibonacci

# Tests a contract that makes heavy access to storage slots
make load-test-io
```

You should see the ethrex client producing blocks and logs with the gas throughput.

To execute it with non-default parameters, you can do so with `cargo`:

```bash
cargo run --manifest-path ./tooling/load_test/Cargo.toml -- -k ./fixtures/keys/private_keys.txt -t erc20 -N 1000 -n http://localhost:8545
```

You may want to delete the dev database in between runs with

```bash
make rm-test-db
```

## Getting performance metrics

Load tests are usually used to get performance metrics. We usually want to generate flamegraphs or samply reports.

To produce a flamegraph, run the node in the following way.

```bash
cargo flamegraph --root --bin ethrex --release -- --network fixtures/genesis/load-test.json --dev
```

The "root" command is only needed for mac. It can be removed if running on linux.

For a samply report, run the following:

```bash
cargo b --profile release-with-debug && \
 ./target/release-with-debug/ethrex removedb --force && \
 samply record -r 10000 ./target/release-with-debug/ethrex --network fixtures/genesis/load-test.json --dev
```

## Interacting with reth

The same load test can be run, the only difference is how you run the node:

```bash
cargo run --release -- node --chain <path_to_ethrex>/fixtures/genesis/load-test.json --dev --dev.block-time 5000ms --http.port 8545 --txpool.max-pending-txns 100000000 --txpool.max-new-txns 1000000000 --txpool.pending-max-count 100000000 --txpool.pending-max-size 10000000000 --txpool.basefee-max-count 100000000000 --txpool.basefee-max-size 1000000000000 --txpool.queued-max-count 1000000000
```

All of the txpool parameters are to make sure that it doesn't discard transactions sent by the load test. Trhoughput measurements in the logs are typically near 1Gigagas/second. To remove the database before getting measurements again:

```bash
cargo run --release -- db --chain <path_to_ethrex>/fixtures/genesis/load-test.json drop -f
```

To get a flamegraph of its execution, run with the same parameters, just replace `cargo run --release` with `cargo flamegraph --bin reth --profiling`:

```bash
cargo flamegraph --bin reth --root --profiling -- node --chain ~/workspace/ethrex/fixtures/genesis/load-test.json --dev --dev.block-time 5000ms --http.port 8545 --txpool.max-pending-txns 100000000 --txpool.max-new-txns 1000000000 --txpool.pending-max-count 100000000 --txpool.pending-max-size 10000000000 --txpool.basefee-max-count 100000000000 --txpool.basefee-max-size 1000000000000 --txpool.queued-max-count 1000000000
```

For samply we want to directly execute the binary, so that it records the binary and not cargo itself:

```bash
samply record ./target/profiling/reth node --chain ~/workspace/ethrex/fixtures/genesis/load-test.json --dev --dev.block-time 5000ms --http.port 8545 --txpool.max-pending-txns 100000000 --txpool.max-new-txns 1000000000 --txpool.pending-max-count 100000000 --txpool.pending-max-size 10000000000 --txpool.basefee-max-count 100000000000 --txpool.basefee-max-size 1000000000000 --txpool.queued-max-count 1000000000
```
