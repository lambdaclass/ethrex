# Ethrex Load Tests

This is a command line tool to execute different types of load tests on any execution node.

It requries three arguments:

- pekys: a path to a file with private keys. These will be the EOAs executing the transactions throughout the tests.
- node: ip:port for the node under test's rpc.
- test: may be raw, erc20 or io. The first one executes raw transactions, the second one executes token transfers, and the last one storage access operations.

To execute it, cd into this directory (cmd/load_tests) and execute it with cargo. For example:

```bash
cargo run -- --test=raw --node=127.0.0.1:1729 --pkeys=../../test_data/private_keys.txt                
```
