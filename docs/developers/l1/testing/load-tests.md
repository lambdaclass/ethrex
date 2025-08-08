# Load tests

Before starting, consider increasing the maximum amount of open files for the current shell with the following command:

```bash
ulimit -n 65536
```

To run a load test, first run the node using a command like the following in the root folder:

```bash
cargo run --bin ethrex --release -- --network fixtures/genesis/load-test.json --dev
```

There are currently three different load tests you can run:

The first one sends regular transfers between accounts, the second runs an EVM-heavy contract that computes fibonacci numbers, the third a heavy IO contract that writes to 100 storage slots per transaction.

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


