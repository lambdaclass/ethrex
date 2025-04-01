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