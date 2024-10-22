# Lambda Ethereum Rust Execution Client

[![Telegram Chat][tg-badge]][tg-url]
[![license](https://img.shields.io/github/license/lambdaclass/ethereum_rust)](/LICENSE)

[tg-badge]: https://img.shields.io/endpoint?url=https%3A%2F%2Ftg.sumanjay.workers.dev%2Frust_ethereum%2F&logo=telegram&label=chat&color=neon
[tg-url]: https://t.me/rust_ethereum

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

If we forgot to include anyone, please file an issue so we can add you. We always strive to reference the inspirations and code we use, but as an organization with multiple people, mistakes can happen, and someone might forget to include a reference.

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

# L1 and L2 support

This client supports running in two different modes:

- As a regular Ethereum execution client, like `geth`.
- As a ZK-Rollup, where block execution is proven and the proof sent to an L1 network for verification, thus inheriting the L1's security.

We call the first one Lambda Ethereum Rust L1 and the second one Lambda Ethereum Rust L2.

# Lambda Ethereum Rust L2

[Full Roadmap](./crates/l2/README.md)

The main differences between this mode and regular Ethereum Rust are:

- There is no consensus, only one sequencer proposes blocks for the network.
- Block execution is proven using a RISC-V zkVM and its proofs are sent to L1 for verification.
- A set of Solidity contracts to be deployed to the L1 are included as part of network initialization.
- Two new types of transactions are included: deposits (native token mints) and withdrawals.

[](https://github.com/user-attachments/assets/99c96831-a641-4978-b70c-4acb519d0098)



# Lambda Ethereum Rust L1

### Table of Contents

- [Lambda Ethereum Rust Execution Client](#lambda-ethereum-rust-execution-client)
  - [📚 References and acknowledgements](#-references-and-acknowledgements)
  - [Philosophy](#philosophy)
  - [Design Principles](#design-principles)
- [L1 and L2 support](#l1-and-l2-support)
- [Lambda Ethereum Rust L2](#lambda-ethereum-rust-l2)
- [Lambda Ethereum Rust L1](#lambda-ethereum-rust-l1)
    - [Table of Contents](#table-of-contents)
  - [Roadmap](#roadmap)
    - [Milestone 1: Read-only RPC Node Support](#milestone-1-read-only-rpc-node-support)
    - [Milestone 2: History \& Reorgs](#milestone-2-history--reorgs)
    - [Milestone 3: Block building](#milestone-3-block-building)
    - [Milestone 4: P2P Network](#milestone-4-p2p-network)
    - [Milestone 5: State Sync](#milestone-5-state-sync)
  - [Quick Start (L1 localnet)](#quick-start-l1-localnet)
    - [Prerequisites](#prerequisites)
  - [Dev Setup](#dev-setup)
    - [Build](#build)
      - [Rust](#rust)
    - [Database](#database)
    - [Test](#test)
        - [Ethereum Foundation Tests](#ethereum-foundation-tests)
        - [Crate Specific Tests](#crate-specific-tests)
        - [Hive Tests](#hive-tests)
    - [Run](#run)
    - [CLI Commands](#cli-commands)
- [Crates documentation](#crates-documentation)

## Roadmap

An Ethereum execution client consists roughly of the following parts:

- A storage component, in charge of persisting the chain's  data. This requires, at the very least, storing it in a Merkle Patricia Tree data structure to calculate state roots. It also requires some on-disk database; we currently use [libmdbx](https://github.com/erthink/libmdbx) but intend to change that in the future.
- A JSON RPC API. A set of HTTP endpoints meant to provide access to the data above and also interact with the network by sending transactions. Also included here is the `Engine API`, used for communication between the execution and consensus layers.
- A Networking layer implementing the peer to peer protocols used by the Ethereum Network. The most important ones are:
    - The `disc` protocol for peer discovery, using a Kademlia DHT for efficient searches.
    - The `RLPx` transport protocol used for communication between nodes; used by other protocols that build on top to exchange information, sync state, etc. These protocols built on top are usually called `capabilities`.
    - The Ethereum Wire Protocol (`ETH`), used for state synchronization and block/transaction propagation, among other things. This runs on top of `RLPx`.
    - The `SNAP` protocol, used for exchanging state snapshots. Mainly needed for **snap sync**, a more optimized way of doing state sync than the old fast sync (you can read more about it [here](https://blog.ethereum.org/2021/03/03/geth-v1-10-0)).
- Block building and Fork choice management (i.e. logic to both build blocks so a validator can propose them and set where the head of the chain is currently at, according to what the consensus layer determines). This is essentially what our `blockchain` crate contains.
- The block execution logic itself, i.e., an EVM implementation. We are finishing an implementation of our own called [levm](https://github.com/lambdaclass/ethereum_rust/tree/main/crates/vm/levm) (Lambda EVM).

Because most of the milestones below do not overlap much, we are currently working on them in parallel.

### Milestone 1: Read-only RPC Node Support

Implement the bare minimum required to:

- Execute incoming blocks and store the resulting state on an on-disk database (`libmdbx`). No support for reorgs/forks, every block has to be the child of the current head.
- Serve state through a JSON RPC API. No networking yet otherwise (i.e. no p2p).

In a bit more detail:

|  Task Description      | Status                                                                 | 
| --------- |  --------------------------------------------------------------------------- | 
|  Add `libmdbx` bindings and basic API, create tables for state (blocks, transactions, etc)                                               | ✅     
|   EVM wrapper for block execution                                                       | ✅     |
|    JSON RPC API server setup                                                      | ✅     |
|    RPC State-serving endpoints                                                     | 🏗️  (almost done, a few endpoint are left)   |
|    Basic Engine API implementation. Set new chain head (`forkchoiceUpdated`) and new block (`newPayload`).                                                   | ✅   

See detailed issues and progress for this milestone [here](https://github.com/lambdaclass/ethereum_rust/milestone/1).

### Milestone 2: History & Reorgs

Implement support for block reorganizations and historical state queries. This milestone involves persisting the state trie to enable efficient access to historical states and implementing a tree structure for the blockchain to manage multiple chain branches. It also involves a real implementation of the `engine_forkchoiceUpdated` Engine API when we do not have to build the block ourselves (i.e. when `payloadAttributes` is null).

|  Task Description      | Status                                                                 | 
| --------- |  --------------------------------------------------------------------------- | 
|   Persist data on an on-disk Merkle Patricia Tree using `libmdbx`                                       | ✅ 
|   Engine API `forkchoiceUpdated` implementation (without `payloadAttributes`)                                                     | 🏗️     
|    Support for RPC historical queries, i.e. queries (`eth_call`, `eth_getBalance`, etc) at any block                                       | ✅   

Detailed issues and progress [here](https://github.com/lambdaclass/ethereum_rust/milestone/4).

### Milestone 3: Block building

Add the ability to build new payloads (blocks), so the consensus client can propose new blocks based on transactions received from the RPC endpoints.

|  Task Description      | Status                                                                 | 
| --------- |  --------------------------------------------------------------------------- | 
|   `engine_forkchoiceUpdated` implementation with a non-null `payloadAttributes`                                      | 🏗️     
|   `engine_getPayload` endpoint implementation that builds blocks.                                                     | 🏗️     
|    Implement a mempool and the `eth_sendRawTransaction` endpoint where users can send transactions                                      | ✅   

Detailed issues and progress [here](https://github.com/lambdaclass/ethereum_rust/milestone/5).

### Milestone 4: P2P Network

Implement the peer to peer networking stack, i.e. the DevP2P protocol. This includes `discv4`, `RLPx` and the `eth` capability. This will let us get and retrieve blocks and transactions from other nodes. We'll add the transactions we receive to the mempool. We'll also download blocks from other nodes when we get payloads where the parent isn't in our local chain.

|  Task Description      | Status                                                                  | 
| --------- |  --------------------------------------------------------------------------- | 
|   Implement `discv4` for peer discovery                                    | ✅     
|   Implement the `RLPx` transport protocol                                                     | 🏗️     
|  Implement the `eth` capability                                     | 🏗️  

Detailed issues and progress [here](https://github.com/lambdaclass/ethereum_rust/milestone/2).

### Milestone 5: State Sync

Add support for the `SNAP` protocol, which lets us get a recent copy of the blockchain state instead of going through all blocks from genesis. This is used for used for snap sync. Since we don't support older versions of the spec by design, this is a prerequisite to being able to sync the node with public networks, including mainnet.

|  Task Description      | Status                                                                 | 
| --------- |  --------------------------------------------------------------------------- | 
|   Implement `SNAP` protocol for snap syncing                                    | ❌     

Detailed issues and progress [here](https://github.com/lambdaclass/ethereum_rust/milestone/3).

## Quick Start (L1 localnet)

![Demo](https://raw.githubusercontent.com/lambdaclass/ethereum_rust/8e3b69d727225686eec30b2c2b79cecdf7eac2d9/Demo.png)

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
To build the node, you will need the rust toolchain:
1. First, [install asdf](https://asdf-vm.com/guide/getting-started.html):
2. Add the rust plugin:
```shell
asdf plugin-add rust https://github.com/asdf-community/asdf-rust.git
```
3. cd into the project and run:
```shell
asdf install
```

You now should be able to build the client:
```bash
make build
```
### Database
Currently, the database is `libmdbx`, it will be set up
when you start the client. The location of the db's files will depend on your OS:
- Mac: `~/Library/Application Support/ethereum_rust`
- Linux: `~/.config/ethereum_rust`

You can delete the db with:
```bash
cargo run --bin ethereum_rust -- removedb
```

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
make test CRATE="ethereum_rust-blockchain"
```


##### Hive Tests

Finally, we have End-to-End tests with hive.
Hive is a system which simply sends RPC commands to our node,
and expects a certain response. You can read more about it [here](https://github.com/ethereum/hive/blob/master/docs/overview.md).
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

### Run

Example run:
```bash
cargo run --bin ethereum_rust -- --network test_data/genesis-kurtosis.json
```

The `network` argument is mandatory, as it defines the parameters of the chain.
For more information about the different cli arguments check out the next section.

### CLI Commands

Ethereum Rust supports the following command line arguments:
- `--network <FILE>`: Receives a `Genesis` struct in json format. This is the only argument which is required. You can look at some example genesis files at `test_data/genesis*`.
- `--datadir <DIRECTORY>`: Receives the name of the directory where the Database is located.
- `--import <FILE>`: Receives an rlp encoded `Chain` object (aka a list of `Block`s). You can look at the example chain file at `test_data/chain.rlp`.
- `--http.addr <ADDRESS>`: Listening address for the http rpc server. Default value: localhost.
- `--http.port <PORT>`: Listening port for the http rpc server. Default value: 8545.
- `--authrpc.addr <ADDRESS>`: Listening address for the authenticated rpc server. Default value: localhost.
- `--authrpc.port <PORT>`: Listening port for the authenticated rpc server. Default value: 8551.
- `--authrpc.jwtsecret <FILE>`: Receives the jwt secret used for authenticated rpc requests. Default value: jwt.hex.
- `--p2p.addr <ADDRESS>`: Default value: 0.0.0.0.
- `--p2p.port <PORT>`: Default value: 30303.
- `--discovery.addr <ADDRESS>`: UDP address for P2P discovery. Default value: 0.0.0.0.
- `--discovery.port <PORT>`: UDP port for P2P discovery. Default value: 30303.
- `--bootnodes <BOOTNODE_LIST>`: Comma separated enode URLs for P2P discovery bootstrap.
- `--log.level <LOG_LEVEL>`: The verbosity level used for logs. Default value: info. possible values: info, debug, trace, warn, error

# Crates documentation

Documentation for each crate can be found on the following links (still a work in progress, we will be adding more documentation as we go).

-   [Networking](./crates/net/README.md)
-   [L2](./crates/l2/README.md)
