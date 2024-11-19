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

TODO

## Configuration

Configuration is done through environment variables. The easiest way to configure the Proposer is by creating a `.env` file and setting the variables there. Then, at start, it will read the file and set the variables.

The following environment variables are available to configure the Proposer:

- `ETH_RPC_URL`: URL of the L1 RPC.
- `L1_WATCHER_BRIDGE_ADDRESS`: Address of the bridge contract on L1.
- `L1_WATCHER_TOPICS`: Topics to filter the L1 events.
- `L1_WATCHER_CHECK_INTERVAL_MS`: Interval in milliseconds to check for new events.
- `L1_WATCHER_MAX_BLOCK_STEP`: Maximum number of blocks to look for when checking for new events.
- `L1_WATCHER_L2_PROPOSER_PRIVATE_KEY`: Private key of the L2 proposer.
- `ENGINE_API_RPC_URL`: URL of the EngineAPI.
- `ENGINE_API_JWT_PATH`: Path to the JWT authentication file, required to connect to the EngineAPI.
- `PROVER_SERVER_LISTEN_IP`: IP to listen for proof data requests.
- `PROVER_SERVER_LISTEN_PORT`: Port to listen for proof data requests.
- `PROVER_PROVER_SERVER_ENDPOINT`: Endpoint for the prover server.
- `PROVER_ELF_PATH`: Path to the ELF file for the prover.
- `PROPOSER_ON_CHAIN_PROPOSER_ADDRESS`: Address of the on-chain proposer.
- `PROPOSER_L1_ADDRESS`: Address of the L1 proposer.
- `PROPOSER_L1_PRIVATE_KEY`: Private key of the L1 proposer.
- `PROPOSER_INTERVAL_MS`: Interval in milliseconds to produce new blocks for the proposer.

If you want to use a different configuration file, you can set the `ENV_FILE` environment variable to the path of the file.
