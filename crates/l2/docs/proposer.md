# ethrex L2 Proposer

## ToC

- [ethrex L2 Proposer](#ethrex-l2-proposer)
  - [ToC](#toc)
  - [Components](#components)
    - [L1 Watcher](#l1-watcher)
    - [L1 Transaction Sender](#l1-transaction-sender)
    - [Prover Server](#prover-server)
  - [Configuration](#configuration)

## Components

The L2 Proposer is composed of the following components:

### L1 Watcher

This component handles the L1->L2 messages. Without rest, it is always watching the L1 for new deposit events defined as `DepositInitiated()` that contain the deposit transaction to be executed on the L2. Once a new deposit event is detected, it will insert the deposit transaction into the L2.

In the future, it will also be watching for other L1->L2 messages.

### L1 Transaction Sender

As the name suggests, this component sends transactions to the L1. But not any transaction, only commit and verify transactions.

Commit transactions are sent when the Proposer wants to commit to a new block. These transactions contain the block data to be committed in the L1.

Verify transactions are sent by the Proposer after the prover has successfully generated a proof of block execution to verify it. These transactions contain the proof to be verified in the L1.

### Prover Server

The Prover Server is a simple TCP server that manages communication with a component called the `Prover Client`. The Prover Client acts as a simple TCP client, handling incoming requests to prove a block. It then "calls" a zkVM, generates the Groth16 proof, and sends it back to the Server. In this setup, the state is managed solely by the Prover Server, which, in theory, makes it less error-prone than the zkVMs.

For more information about the Prover Server, the [Prover Docs](./prover.md) provides more insight.

## Configuration

Configuration is done through environment variables. The easiest way to configure the Proposer is by creating a `config.toml` file and setting the variables there. Then, at start, it will read the file and set the variables.

The following environment variables are available to configure the Proposer (consider looking at the provided [config_example.toml](../config_example.toml):

<!-- NOTE: Mantain the sections in the same order as present in [config_example.toml](../config_example.toml). -->

- Under the [deployer] section:
    - `address`: L1 account which will deploy the common bridge contracts in L1.
    - `private key`: Its private key.
    - `risc0_contract_verifier`: Address which will verify the `risc0` proofs.
    - `sp1_contract_verifier`: Address which will verify the `sp1` proofs.
    - `sp1_deploy_verifier`: Whether to deploy the sp1 verifier
    - `salt_is_zero`: Whether a 0 value salt will be used. Keep as true for deterministic `create2` operations.

- Under the [eth] section:
    - `rpc_url`: URL of the L1 RPC.

- Under the [engine] section:
    - `rpc_url`: URL of the EngineAPI.
    - `jwt_path`: Path to the JWT authentication file, required to connect to the EngineAPI.

- Under the [watcher] section:
    - `bridge_address`: Address of the bridge contract on L1.
    - `check_interval_ms`: Interval in milliseconds to check for new events.
    - `max_block_step`: Maximum number of blocks to look for when checking for new events.
    - `l2_proposer_private_key`: Private key of the L2 proposer.

- Under the [proposer] section:
    - `interval_ms`: Interval in milliseconds to produce new blocks for the proposer.
    - `coinbase address`: Address which will receive the execution fees.

Under the [committer] section:
    - `l1_address`: Address of the L1 committer.
    - `l1_private_key`: Private key of the L1 committer.
    - `on_chain_proposer_address`: Address of the on-chain committer.

- Under the [prover] section:
    - `sp1_prover`: Configure how the `sp1_prover` computes its proofs, `"local"` for real proofs and `"mock"` for fake proofs.
    - `risc0_dev_mode`: Whether `risc0`'s dev mode is on.

- Under the [prover.client] section:
    - `prover_server_endpoint`: Endpoint for the prover server.
    - `interval_ms`: Interval in milliseconds to prove new blocks (Currently unused).

- Under the [prover.server] section:
    - `listen_ip`: IP to listen for proof data requests.
    - `listen_port`: Port to listen for proof data requests.
    - `verifier_address`: Address of the account that sends verify transaction to L1.
    - `verifier_private_key`: Private key for the account that sends verify transaction to L1.
    - `dev_mode`: whether `dev_mode` is activated.


If you want to use a different configuration file, you can set the `CONFIG_FILE` environment variable to the path of the file.
