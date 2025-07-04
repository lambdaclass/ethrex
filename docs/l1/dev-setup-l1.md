# L1 dev setup

## Build

### Rust

To build the node, you will need the rust toolchain. To do so, use `rustup` following [this link](https://www.rust-lang.org/tools/install)

## Database

Currently, the database is `libmdbx`, it will be set up
when you start the client. The location of the db's files will depend on your OS:

- Mac: `~/Library/Application Support/ethrex`
- Linux: `~/.config/ethrex`

You can delete the db with:

```bash
cargo run --bin ethrex -- removedb
```

## Dev Mode

In order to run `ethrex` without a Consensus Client and with the `InMemory` engine, to start from scratch each time we fire it up, the following make target can be used:

```bash
make dev
```

- RPC endpoint: localhost:8545
- Genesis file: ./test_data/genesis-l1.json

## Test

For testing, we're using three kinds of tests.

### Ethereum Foundation Tests

These are the official execution spec tests, you can execute them with:

```bash
make test
```

This will download the test cases from the [official execution spec tests repo](https://github.com/ethereum/execution-spec-tests/) and run them with our glue code
under `cmd/ef_tests/tests`.

### Crate Specific Tests

The second kind are each crate's tests, you can run them like this:

```bash
make test CRATE=<crate>
```

For example:

```bash
make test CRATE="ethrex-blockchain"
```

### Load tests

More information in the [load test documentation](../developers/l2/load_tests.md).

### Hive Tests

Finally, we have End-to-End tests with hive.
Hive is a system which simply sends RPC commands to our node,
and expects a certain response. You can read more about it [here](https://github.com/ethereum/hive/blob/master/docs/overview.md).

#### Prereqs

We need to have go installed for the first time we run hive, an easy way to do this is adding the asdf go plugin:

```shell
asdf plugin add golang https://github.com/asdf-community/asdf-golang.git

# If you need to set GOROOT please follow: https://github.com/asdf-community/asdf-golang?tab=readme-ov-file#goroot
```

And uncommenting the golang line in the asdf `.tool-versions` file:

```text
rust 1.88.0
golang 1.23.2
```

#### Running Simulations

Hive tests are categorized by "simulations', and test instances can be filtered with a regex:

```bash
make run-hive-debug SIMULATION=<simulation> TEST_PATTERN=<test-regex>
```

This is an example of a Hive simulation called `ethereum/rpc-compat`, which will specificaly
run chain id and transaction by hash rpc tests:

```bash
make run-hive SIMULATION=ethereum/rpc-compat TEST_PATTERN="/eth_chainId|eth_getTransactionByHash"
```

If you want debug output from hive, use the run-hive-debug instead:

```bash
make run-hive-debug SIMULATION=ethereum/rpc-compat TEST_PATTERN="*"
```

This example runs **every** test under rpc, with debug output

#### Assertoor

We run some assertoot checks on our CI, to execute them locally you can run the following:

```bash
make localnet-assertoor-tx
# or
make localnet-assertoor-blob
```

Those are two different set of assertoor checks the details are as follows:

_assertoor-tx_

- [eoa-transaction-test](https://raw.githubusercontent.com/ethpandaops/assertoor/refs/heads/master/playbooks/stable/eoa-transactions-test.yaml)

_assertoor-blob_

- [blob-transaction-test](https://raw.githubusercontent.com/ethpandaops/assertoor/refs/heads/master/playbooks/stable/blob-transactions-test.yaml)
- _Custom_ [el-stability-check](https://raw.githubusercontent.com/lambdaclass/ethrex/refs/heads/main/.github/config/assertoor/el-stability-check.yaml)

For reference on each individual check see the [assertoor-wiki](https://github.com/ethpandaops/assertoor/wiki#supported-tasks-in-assertoor)

## Run

Example run:

```bash
cargo run --bin ethrex -- --network test_data/genesis-kurtosis.json
```

The `network` argument is mandatory, as it defines the parameters of the chain.
For more information about the different cli arguments check out the next section.

## CLI Commands

<!-- BEGIN_CLI_HELP -->

```
ethrex Execution client

Usage: ethrex [OPTIONS] [COMMAND]

Commands:
  removedb            Remove the database
  import              Import blocks to the database
  export              Export blocks in the current chain into a file in rlp encoding
  compute-state-root  Compute the state root from a genesis file
  l2
  help                Print this message or the help of the given subcommand(s)

Options:
  -h, --help
          Print help (see a summary with '-h')

  -V, --version
          Print version

Node options:
      --network <GENESIS_FILE_PATH>
          Alternatively, the name of a known network can be provided instead to use its preset genesis file and include its preset bootnodes. The networks currently supported include holesky, sepolia, hoodi and mainnet. If not specified, defaults to mainnet.

          [env: ETHREX_NETWORK=]

      --datadir <DATABASE_DIRECTORY>
          If the datadir is the word `memory`, ethrex will use the `InMemory Engine`.

          [env: ETHREX_DATADIR=]
          [default: ethrex]

      --force
          Delete the database without confirmation.

      --metrics.addr <ADDRESS>
          [default: 0.0.0.0]

      --metrics.port <PROMETHEUS_METRICS_PORT>
          [env: ETHREX_METRICS_PORT=]
          [default: 9090]

      --metrics
          Enable metrics collection and exposition

      --dev
          If set it will be considered as `true`. If `--network` is not specified, it will default to a custom local devnet. The Binary has to be built with the `dev` feature enabled.

      --evm <EVM_BACKEND>
          Has to be `levm` or `revm`

          [env: ETHREX_EVM=]
          [default: levm]

      --log.level <LOG_LEVEL>
          Possible values: info, debug, trace, warn, error

          [default: INFO]

P2P options:
      --bootnodes <BOOTNODE_LIST>...
          Comma separated enode URLs for P2P discovery bootstrap.

      --syncmode <SYNC_MODE>
          Can be either "full" or "snap" with "full" as default value.

          [default: full]

      --p2p.enabled


      --p2p.addr <ADDRESS>
          [default: 0.0.0.0]

      --p2p.port <PORT>
          [default: 30303]

      --discovery.addr <ADDRESS>
          UDP address for P2P discovery.

          [default: 0.0.0.0]

      --discovery.port <PORT>
          UDP port for P2P discovery.

          [default: 30303]

RPC options:
      --http.addr <ADDRESS>
          Listening address for the http rpc server.

          [env: ETHREX_HTTP_ADDR=]
          [default: localhost]

      --http.port <PORT>
          Listening port for the http rpc server.

          [env: ETHREX_HTTP_PORT=]
          [default: 8545]

      --authrpc.addr <ADDRESS>
          Listening address for the authenticated rpc server.

          [default: localhost]

      --authrpc.port <PORT>
          Listening port for the authenticated rpc server.

          [default: 8551]

      --authrpc.jwtsecret <JWTSECRET_PATH>
          Receives the jwt secret used for authenticated rpc requests.

          [default: jwt.hex]
```

<!-- END_CLI_HELP -->

## Syncing with Holesky

### Step 1: Set up a jwt secret for both clients

As an example, we put the secret in a `secrets` directory in the home folder.

```bash
mkdir -p ~/secrets
openssl rand -hex 32 | tr -d "\n" | tee ~/secrets/jwt.hex
```

We will pass this new file’s path as an argument for both clients.

### Step 2: Launch Ethrex

Pass holesky as a network and the jwt secret we set in the previous step.
This will launch the node in full sync mode, in order to test out snap sync you can add the flag `--syncmode snap`.

```bash
cargo run --release --bin ethrex -- --http.addr 0.0.0.0 --network holesky --authrpc.jwtsecret ~/secrets/jwt.hex
```

### Step 3: Set up a Consensus Node

For this quick tutorial we will be using lighthouse, but you can learn how to install and run any consensus node by reading their documentation.

You can choose your preferred installation method from [lighthouse's installation guide](https://lighthouse-book.sigmaprime.io/installation.html) and then run the following command to launch the node and sync it from a public endpoint

```bash
lighthouse bn --network holesky --execution-endpoint http://localhost:8551 --execution-jwt ~/secrets/jwt.hex --http --checkpoint-sync-url https://checkpoint-sync.holesky.ethpandaops.io
```

When using lighthouse directly from its repository, replace `lighthouse bn` with `cargo run --bin lighthouse -- bn`

Aside from holesky, these steps can also be used to connect to other supported networks by replacing the `--network` argument by another supported network and looking up a checkpoint sync endpoint for that network [in this community-maintained list](https://eth-clients.github.io/checkpoint-sync-endpoints/)

If you have a running execution node that you want to connect to your ethrex node you can do so by passing its enode as a bootnode using the `--bootnodes` flag

Once the node is up and running you will be able to see logs indicating the start of each sync cycle along with from which block hash to which block hash we are syncing. You will also get regular logs with the completion rate and estimated finish time for state sync and state rebuild processes during snap sync. This will look something like this:

```bash
INFO ethrex_p2p::sync: Syncing from current head 0xb5f7…bde4 to sync_head 0xce96…fa5e
INFO ethrex_p2p::sync::state_sync: Downloading state trie, completion rate: 68%, estimated time to finish: 1h20m14s
INFO ethrex_p2p::sync::trie_rebuild: State Trie Rebuild Progress: 68%, estimated time to finish: 1h5m45s
```

If you want to restart the sync from the very start you can do so by wiping the database using the following command:

```bash
cargo run --bin ethrex -- removedb
```
