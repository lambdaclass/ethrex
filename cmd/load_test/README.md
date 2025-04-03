# Load Test Tool

This module presents the following possible loadtest scenarios/types:

- `PlainTransactions`: Sends multiple EIP1559 transfers.
- `Fibonacci`: Deploys a contract that calculates the fibonacci of a big number. Then it sends multiple transactions to calculate the number in all of the txs.
- `IoHeavy`: Deploys a contract that interacts with 100 storage slots. Then it sends multiple transactions to interact with it.
- `Erc20`: Deploys an ERC20 contract. Then it sends multiple transactions interacting with it (mint, claim and transfer operations).

## Usage

```
Usage: load_test [OPTIONS] --path <PATH>

Options:
  -p, --path <PATH>              Path to the file containing private keys.
  -t, --to <TO>                  Address to send the transactions. Defaults to random.
  -a, --value <VALUE>            Value to send in each transaction. [default: 1000]
  -i, --iterations <ITERATIONS>  Number of transactions per private key. [default: 1000]
  -v, --verbose                  Prints each transaction.
  -y, --test_type <TEST_TYPE>    Specify the type of test. [default: plain-transactions] [possible values: plain-transactions, fibonacci, io-heavy, erc20]
      --pk <PRIVATE_KEY>         Rich account's private_key. [default: 0x385c546456b6a603a1cfcaa9ec9494ba4832da08dd6bcf4de9a71e4a01b74924]
  -u, --url <ETHREX_URL>         ethrex's RPC URL. [default: http://localhost:8545]
  -h, --help                     Print help
```

### Frequent Usecases

Go to the repository's root dir and run the following command:

```sh
cargo run --manifest-path cmd/load_test/Cargo.toml -- --path ./test_data/private_keys.txt
```

The `iterations` means that each pk in the `./test_data/private_keys.txt` file will send `i` transactions.
At the moment, the file contains 171 accounts. So by default it will be $171 \times 1000 = 171000$ txs.

Also, the root's Makefile contains some targets to facilitate the process `load test` process.

Flamegraph:

- You will need two terminals:
  1. `sudo make start-node-with-flamegraph`
  2. `sudo make load-test`
     - OR:
       - `load-test-erc20`
       - `load-test-fibonacci`
       - `load-test-io`

Samply:

- You will need two terminals:
  1. `sudo make start-node-with-samply`
  2. `sudo make load-test`
     - OR:
       - `load-test-erc20`
       - `load-test-fibonacci`
       - `load-test-io`

When the node is stopeed, it will generate a file named `profile.json.gz`, you can open it at https://profiler.firefox.com/.
