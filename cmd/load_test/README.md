# Ethrex Load Tests

## About

This is a command line tool to execute ERC20 load tests on any execution node.

It requries two arguments:

- pkeys: a path to a file with private keys. These will be the EOAs executing the transactions throughout the tests.
- node: http address for the node under test's rpc.

To run a load test, first run the node using a command like the following in the root folder:

```bash
cargo run --bin ethrex --release --features dev -- --network test_data/genesis-l2-ci.json --dev
```

Genesis-l2-ci has many rich accounts and does not include the prague fork, which is important for dev mode until it's fixed.

After the node is runing, `cd` into this directory (`cmd/load_tests`) and execute the script with `cargo`. For example:

```bash
cargo run --bin=load_test -- --node=http://127.0.0.1:8545 --pkeys=../../test_data/private_keys.txt
```

You should see the ethrex client producing blocks and logs with the gas throughput.

## Getting performance metrics

Load tests are usually used to get performance metrics. We usually want to generate flamegraphs or samply reports.

To produce a flamegraph, run the node in the following way.

```bash
cargo flamegraph --root --bin ethrex --release --features dev -- --network test_data/genesis-l2-ci.json --dev
```

The "root" command is only needed for mac. It can be removed if running on linux.

For a samply report, run the following:

```bash
samply record cargo run --bin ethrex --release --features dev -- --network test_data/genesis-l2-ci.json --dev
```

## Interacting with reth

The same load test can be run, the only difference is how you run the node:

```bash
cargo run --release -- node --chain <path_to_ethrex>/test_data/genesis-l2-ci.json --dev --dev.block-time 5000ms --http.port 8545 --txpool.max-pending-txns 100000000 --txpool.max-new-txns 1000000000 --txpool.pending-max-count 100000000 --txpool.pending-max-size 10000000000 --txpool.basefee-max-count 100000000000 --txpool.basefee-max-size 1000000000000 --txpool.queued-max-count 1000000000
```

All of the txpool parameters are to make sure that it doesn't discard transactions sent by the load test. Trhoughput measurements in the logs are typically near 1Gigagas/second. To remove the database before getting measurements again:

```bash
cargo run --release -- db --chain <path_to_ethrex>/test_data/genesis-l2-ci.json drop -f
```

To get a flamegraph of its execution, run with the same parameters, just replace `cargo run --release` with `cargo flamegraph --bin reth --profiling`:

```bash
cargo flamegraph --bin reth --root --profiling -- node --chain ~/workspace/ethrex/test_data/genesis-l2-ci.json --dev --dev.block-time 5000ms --http.port 8545 --txpool.max-pending-txns 100000000 --txpool.max-new-txns 1000000000 --txpool.pending-max-count 100000000 --txpool.pending-max-size 10000000000 --txpool.basefee-max-count 100000000000 --txpool.basefee-max-size 1000000000000 --txpool.queued-max-count 1000000000
```

For samply we want to directly execute the binary, so that it records the binary and not cargo itself:

```bash
samply record ./target/profiling/reth node --chain ~/workspace/ethrex/test_data/genesis-l2-ci.json --dev --dev.block-time 5000ms --http.port 8545 --txpool.max-pending-txns 100000000 --txpool.max-new-txns 1000000000 --txpool.pending-max-count 100000000 --txpool.pending-max-size 10000000000 --txpool.basefee-max-count 100000000000 --txpool.basefee-max-size 1000000000000 --txpool.queued-max-count 1000000000
```