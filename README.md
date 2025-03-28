# ethrex

Ethereum Rust Execution L1 and L2 client.

[![Telegram Chat][tg-badge]][tg-url]
[![license](https://img.shields.io/github/license/lambdaclass/ethrex)](/LICENSE)

[tg-badge]: https://img.shields.io/endpoint?url=https%3A%2F%2Ftg.sumanjay.workers.dev%2Fethrex_client%2F&logo=telegram&label=chat&color=neon
[tg-url]: https://t.me/ethrex_client

# L1 and L2 support

This client supports running in two different modes:

- As a regular Ethereum execution client
- As a ZK-Rollup, where block execution is proven and the proof sent to an L1 network for verification, thus inheriting the L1's security.

We call the first one ethrex L1 and the second one ethrex L2.

## Philosophy

Many long-established clients accumulate bloat over time. This often occurs due to the need to support legacy features for existing users or through attempts to implement overly ambitious software. The result is often complex, difficult-to-maintain, and error-prone systems.

In contrast, our philosophy is rooted in simplicity. We strive to write minimal code, prioritize clarity, and embrace simplicity in design. We believe this approach is the best way to build a client that is both fast and resilient. By adhering to these principles, we will be able to iterate fast and explore next-generation features early, either from the Ethereum roadmap or from innovations from the L2s.

Read more about our engineering philosophy [here](https://blog.lambdaclass.com/lambdas-engineering-philosophy/)

## Design Principles

- Ensure effortless setup and execution across all target environments.
- Be vertically integrated. Have the minimal amount of dependencies.
- Be structured in a way that makes it easy to build on top of it, i.e rollups, vms, etc.
- Have a simple type system. Avoid having generics leaking all over the codebase.
- Have few abstractions. Do not generalize until you absolutely need it. Repeating code two or three times can be fine.
- Prioritize code readability and maintainability over premature optimizations.
- Avoid concurrency split all over the codebase. Concurrency adds complexity. Only use where strictly necessary.

# ethrex L1

## Quick Start (L1 localnet)

### Prerequisites
- [Kurtosis](https://docs.kurtosis.com/install/#ii-install-the-cli)
- [Rust](#rust)
- [Docker](https://docs.docker.com/engine/install/)
```shell
make localnet
```

This make target will:
1. Build our node inside a docker image.
2. Fetch our fork [ethereum package](https://github.com/ethpandaops/ethereum-package), a private testnet on which multiple ethereum clients can interact.
3. Start the localnet with kurtosis.

If everything went well, you should be faced with our client's logs (ctrl-c to leave)

To stop everything, simply run:
```shell
make stop-localnet
```

## Dev Setup
### Build

#### Rust
To build the node, you will need the rust toolchain. To do so, use `rustup` following [this link](https://www.rust-lang.org/tools/install)

### Database
Currently, the database is `libmdbx`, it will be set up
when you start the client. The location of the db's files will depend on your OS:
- Mac: `~/Library/Application Support/ethrex`
- Linux: `~/.config/ethrex`

You can delete the db with:
```bash
cargo run --bin ethrex -- removedb
```
### Dev Mode
In order to run `ethrex` without a Consensus Client and with the `InMemory` engine, to start from scratch each time we fire it up, the following make target can be used:

```bash
make dev
```

- RPC endpoint: localhost:8545
- Genesis file: ./test_data/genesis-l1.json

### Test

For testing, we're using three kinds of tests.

##### Ethereum Foundation Tests

These are the official execution spec tests, you can execute them with:

```bash
make test
```

This will download the test cases from the [official execution spec tests repo](https://github.com/ethereum/execution-spec-tests/) and run them with our glue code
under `cmd/ef_tests/tests`.

##### Crate Specific Tests

The second kind are each crate's tests, you can run them like this:

```bash
make test CRATE=<crate>
```
For example:
```bash
make test CRATE="ethrex-blockchain"
```

##### Hive Tests

Finally, we have End-to-End tests with hive.
Hive is a system which simply sends RPC commands to our node,
and expects a certain response. You can read more about it [here](https://github.com/ethereum/hive/blob/master/docs/overview.md).

###### Prereqs
We need to have go installed for the first time we run hive, an easy way to do this is adding the asdf go plugin:

```shell
asdf plugin add golang https://github.com/asdf-community/asdf-golang.git

# If you need to se GOROOT please follow: https://github.com/asdf-community/asdf-golang?tab=readme-ov-file#goroot
```

And uncommenting the golang line in the asdf `.tool-versions` file:
```
rust 1.82.0
golang 1.23.2
```

###### Running Simulations
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

###### Assertoor

We run some assertoot checks on our CI, to execute them locally you can run the following:
```bash
make localnet-assertoor-tx
# or
make localnet-assertoor-blob
```

Those are two different set of assertoor checks the details are as follows:

*assertoor-tx*
- [eoa-transaction-test](https://raw.githubusercontent.com/ethpandaops/assertoor/refs/heads/master/playbooks/stable/eoa-transactions-test.yaml)

*assertoor-blob*
- [blob-transaction-test](https://raw.githubusercontent.com/ethpandaops/assertoor/refs/heads/master/playbooks/stable/blob-transactions-test.yaml)
- _Custom_ [el-stability-check](https://raw.githubusercontent.com/lambdaclass/ethrex/refs/heads/main/.github/config/assertoor/el-stability-check.yaml)

For reference on each individual check see the [assertoor-wiki](https://github.com/ethpandaops/assertoor/wiki#supported-tasks-in-assertoor)

### Run

Example run:
```bash
cargo run --bin ethrex -- --network test_data/genesis-kurtosis.json
```

The `network` argument is mandatory, as it defines the parameters of the chain.
For more information about the different cli arguments check out the next section.

### CLI Commands

```
> cargo run --release --bin ethrex -- --help

Usage: ethrex [OPTIONS] [COMMAND]

Commands:
  removedb  Remove the database
  import    Import blocks to the database
  help      Print this message or the help of the given subcommand(s)

Options:
  -h, --help
          Print help (see a summary with '-h')

  -V, --version
          Print version

RPC options:
      --http.addr <ADDRESS>
          Listening address for the http rpc server.

          [default: localhost]

      --http.port <PORT>
          Listening port for the http rpc server.

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

Node options:
      --log.level <LOG_LEVEL>
          Possible values: info, debug, trace, warn, error

          [default: INFO]

      --network <GENESIS_FILE_PATH>
          Alternatively, the name of a known network can be provided instead to use its preset genesis file and include its preset bootnodes. The networks currently supported include holesky, sepolia and ephemery.

      --datadir <DATABASE_DIRECTORY>
          If the datadir is the word `memory`, ethrex will use the `InMemory Engine`.

          [default: ethrex]

      --metrics.port <PROMETHEUS_METRICS_PORT>


      --dev
          If set it will be considered as `true`. The Binary has to be built with the `dev` feature enabled.

      --evm <EVM_BACKEND>
          Has to be `levm` or `revm`

          [default: revm]

P2P options:
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

      --bootnodes <BOOTNODE_LIST>...
          Comma separated enode URLs for P2P discovery bootstrap.

      --syncmode <SYNC_MODE>
          Can be either "full" or "snap" with "full" as default value.

          [default: full]
```

# ethrex L2

In this mode, the ethrex code is repurposed to run a rollup that settles on Ethereum as the L1.

The main differences between this mode and regular ethrex are:

- There is no consensus, the node is turned into a sequencer that proposes blocks for the network.
- Block execution is proven using a RISC-V zkVM and its proofs are sent to L1 for verification.
- A set of Solidity contracts to be deployed to the L1 are included as part of network initialization.
- Two new types of transactions are included: deposits (native token mints) and withdrawals.

At a high level, the following new parts are added to the node:

- A `proposer` component, in charge of continually creating new blocks from the mempool transactions. This replaces the regular flow that an Ethereum L1 node has, where new blocks come from the consensus layer through the `forkChoiceUpdate` -> `getPayload` -> `NewPayload` Engine API flow in communication with the consensus layer.
- A `prover` subsystem, which itself consists of two parts:
  - A `proverClient` that takes new blocks from the node, proves them, then sends the proof back to the node to send to the L1. This is a separate binary running outside the node, as proving has very different (and higher) hardware requirements than the sequencer.
  - A `proverServer` component inside the node that communicates with the prover, sending witness data for proving and receiving proofs for settlement on L1.
- L1 contracts with functions to commit to new state and then verify the state transition function, only advancing the state of the L2 if the proof verifies. It also has functionality to process deposits and withdrawals to/from the L2.
- The EVM is lightly modified with new features to process deposits and withdrawals accordingly.

## Prerequisites

- [Rust (explained in L1 requirements section above)](#build)
- [Docker](https://docs.docker.com/engine/install/) (with [Docker Compose](https://docs.docker.com/compose/install/))
- [The Solidity Compiler](https://docs.soliditylang.org/en/latest/installing-solidity.html) (solc)

## How to run

### Initialize the network

> [!IMPORTANT]
> Before this step:
>
> 1. Make sure you are inside the `crates/l2` directory.
> 2. Make sure the Docker daemon is running.
> 3. Make sure you have created a `sequencer_config.toml` file following the `sequencer_config_example.toml` file.

```
make init
```

This will setup a local Ethereum network as the L1, deploy all the needed contracts on it, then start an ethrex L2 node pointing to it.

### Restarting the network

> [!WARNING]
> This command will cleanup your running L1 and L2 nodes.

```
make restart
```

## Local L1 Rich Wallets

Most of them are [here](https://github.com/ethpandaops/ethereum-package/blob/main/src/prelaunch_data_generator/genesis_constants/genesis_constants.star), but there's an extra one:

```
{
    "address": "0x3d1e15a1a55578f7c920884a9943b3b35d0d885b",
    "private_key": "0x385c546456b6a603a1cfcaa9ec9494ba4832da08dd6bcf4de9a71e4a01b74924"
}
```

## ethrex L2 Docs

- [ethrex L2 Docs](./crates/l2/docs/README.md)
- [ethrex L2 CLI Docs](./cmd/ethrex_l2/README.md)


## 📚 References and acknowledgements

The following links, repos, companies and projects have been important in the development of this repo, we have learned a lot from them and want to thank and acknowledge them.

- [Ethereum](https://ethereum.org/en/)
- [ZKsync](https://zksync.io/)
- [Starkware](https://starkware.co/)
- [Polygon](https://polygon.technology/)
- [Optimism](https://www.optimism.io/)
- [Arbitrum](https://arbitrum.io/)
- [Geth](https://github.com/ethereum/go-ethereum)
- [Taiko](https://taiko.xyz/)
- [RISC Zero](https://risczero.com/)
- [SP1](https://github.com/succinctlabs/sp1)
- [Aleo](https://aleo.org/)
- [Neptune](https://neptune.cash/)
- [Mina](https://minaprotocol.com/)
- [Nethermind](https://www.nethermind.io/)
- [Commonware](https://commonware.xyz/)

If we forgot to include anyone, please file an issue so we can add you. We always strive to reference the inspirations and code we use, but as an organization with multiple people, mistakes can happen, and someone might forget to include a reference.

# Security

We take security seriously. If you discover a vulnerability in this project, please report it responsibly.

- You can report vulnerabilities directly via the **[GitHub "Report a Vulnerability" feature](../../security/advisories/new)**.
- Alternatively, send an email to **[security@lambdaclass.com](mailto:security@lambdaclass.com)**.

For more details, please refer to our [Security Policy](./.github/SECURITY.md).
