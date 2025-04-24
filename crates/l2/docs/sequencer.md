# ethrex L2 Sequencer

## ToC

- [ethrex L2 Sequencer](#ethrex-l2-sequencer)
  - [ToC](#toc)
  - [Components](#components)
    - [Block Producer](#block-producer)
    - [L1 Watcher](#l1-watcher)
    - [L1 Transaction Sender (a.k.a. L1 Committer)](#l1-transaction-sender-aka-l1-committer)
    - [Prover Server](#prover-server)
  - [Configuration](#configuration)

## Components

The L2 Proposer is composed of the following components:

### Block Producer

Creates Blocks with a connection to the `auth.rpc` port.

### L1 Watcher

This component handles the L1->L2 messages. Without rest for every interval defined in the config file, it is always watching the L1 for new deposit events defined as `DepositInitiated()` that contain the deposit transaction to be executed on the L2. Once a new deposit event is detected, it will insert the deposit transaction into the L2.

In the future, it will also be watching for other L1->L2 messages.

### L1 Transaction Sender (a.k.a. L1 Committer)

As the name suggests, this component sends transactions to the L1. But not any transaction, only commit and verify transactions.

Commit transactions are sent when the Proposer wants to commit to a new block. These transactions contain the block data to be committed in the L1.

Verify transactions are sent by the Proposer after the prover has successfully generated a proof of block execution to verify it. These transactions contain the proof to be verified in the L1.

### Prover Server

The Prover Server is a simple TCP server that manages communication with a component called the `Prover Client`. The Prover Client acts as a simple TCP client, handling incoming requests to prove a block. It then "calls" a zkVM, generates the Groth16 proof, and sends it back to the Server. In this setup, the state is managed solely by the Prover Server, which, in theory, makes it less error-prone than the zkVMs.

For more information about the Prover Server, the [Prover Docs](./prover.md) provides more insight.

## Configuration

Configuration is done through environment variables. The easiest way to configure the Sequencer is by creating a `sequencer_config.toml` file and setting the variables there. Then, at start, it will read the file and set the variables.

> [!NOTE]
> The deployer.rs is in charge of parsing the `.toml` and creating/updating the `.env`
> If you don't deploy files, the `.toml` will not be parsed.

The following environment variables are available to configure the Proposer consider looking at the provided [sequencer_config_example.toml](../configs/sequencer_config_example.toml):

<!-- NOTE: Mantain the sections in the same order as present in [sequencer_config_example.toml](../configs/sequencer_config_example.toml). -->

- Under the `[deployer]` section:

  - `l1_address`: L1 account which will deploy the common bridge contracts in L1 with funds available for deployments.
  - `l1_private key`: Private key corresponding to the above address.
  - `pico_contract_verifier`: Address which will verify the `pico` proofs.
  - `pico_deploy_verifier`: Whether to deploy the `pico` verifier contract or not.
  - `risc0_contract_verifier`: Address which will verify the `risc0` proofs.
  - `sp1_contract_verifier`: Address which will verify the `sp1` proofs.
  - `sp1_deploy_verifier`: Whether to deploy the `sp1` verifier contract or not.
  - `salt_is_zero`: Whether a 0 value salt will be used. Keep as true for deterministic `create2` operations.

- Under the `[watcher]` section:

  - `bridge_address`: Address of the bridge contract on L1. This address is used to retrieve logs emitted by deposit events.
  - `check_interval_ms`: Interval in milliseconds to check for new events. If no new events or messages are detected, it does nothing.
  - `max_block_step`: Defines the maximum range of blocks to scan for new events during each polling cycle. Specifically, events are queried from last_block_fetched up to last_block_fetched + max_block_step. If the chain hasn’t progressed that far yet, the scan will end at the current latest block instead. This ensures we only query blocks that actually exist.
  - `l2_proposer_private_key`: Private key of the L2 proposer. 

- Under the `[proposer]` section:

  - `interval_ms`: Interval in milliseconds at which the proposer wakes up to produce a new block.
  - `coinbase address`: Address which will receive the execution fees.

- Under the `[committer]` section:

  - `l1_address`: Address of a funded account that it will be used to send commit transactions to the L1.
  - `l1_private_key`: Its private key.
  - `commit_time_ms`: Sleep time after sending the commit transaction with the proofs to the L1. If no new block has been fetched, we wait another `commit_time_ms` and check again.
  - `on_chain_proposer_address`: Address of the on-chain committer.
  - `arbitrary_base_blob_gas_price`: Sets the minimum price floor for blob transactions when posting L2 data to the L1. This parameter allows you to control the lower bound of what the sequencer is willing to pay for blob storage. Higher values ensure faster inclusion in L1 blocks but increase operating costs, while lower values reduce costs but may cause delays.

- Under the `[prover_server]` section:

  - `l1_address`: Address of the account that sends verify transaction to L1.
    - Must be different than the `committer.l1_address`, there might be conflicts with the transactions' nonce.
  - `l1_private_key`: Its private key.
  - `listen_ip`: IP to listen for proof data requests.
  - `listen_port`: Port to listen for proof data requests.
  - `proof_send_interval_ms`: Interval in which the committer wakes up and send commitments to the L1
  - `dev_mode`: Whether `dev_mode` is activated or not. If it is activated, proofs won't be verified just executed. This can be seen on the OnChainProposer contract, specifically on the verify function, proofs will be ignored.

If you want to use a different configuration file, you can set the:

- `CONFIGS_PATH`: The path where the `SEQUENCER_CONFIG_FILE` is located at.
- `SEQUENCER_CONFIG_FILE`: The `.toml` that contains the config for the `sequencer`.
