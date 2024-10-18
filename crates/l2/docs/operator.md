# Ethereum Rust L2 Operator

## ToC

- [Components](#components)
    - [L1 Watcher](#l1-watcher)
    - [L1 Transaction Sender](#l1-transaction-sender)
    - [Block Producer](#block-producer)
    - [Proof Data Provider](#proof-data-provider)
- [Configuration](#configuration)

## Components

The L2 operator is composed of the following components:

### L1 Watcher

This component handles the L1->L2 messages. Without rest, it is always watching the L1 for new deposit events defined as `DepositInitiated()` that contain the deposit transaction to be executed on the L2. Once a new deposit event is detected, it will insert the deposit transaction into the L2.

In the future, it will also be watching for other L1->L2 messages.

### L1 Transaction Sender

As the name suggests, this component sends transactions to the L1. But not any transaction, only commit and verify transactions.

Commit transactions are sent when the operator wants to commit to a new block. These transactions contain the block data to be committed in the L1.

Verify transactions are sent by the operator after the prover has successfully generated a proof of block execution to verify it. These transactions contain the proof to be verified in the L1.

### Block Producer

This component is responsible for producing new blocks ready to be committed. For the moment, as we do not have consensus in the L2, this component is a mock of the Engine API.

### Proof Data Provider

TODO

## Configuration

Configuration is done through environment variables. The easiest way to configure the operator is by creating a `.env` file and setting the variables there. Then, at start, it will read the file and set the variables.

The following environment variables are available to configure the operator:

- `ETH_RPC_URL`: URL of the L1 RPC.
- `L1_WATCHER_BRIDGE_ADDRESS`: Address of the bridge contract on L1.
- `L1_WATCHER_TOPICS`: Topics to filter the L1 events.
- `L1_WATCHER_CHECK_INTERVAL_MS`: Interval in milliseconds to check for new events.
- `L1_WATCHER_MAX_BLOCK_STEP`: Maximum number of blocks to look for when checking for new events.
- `L1_WATCHER_L2_OPERATOR_PRIVATE_KEY`: Private key of the L2 operator.
- `ENGINE_API_RPC_URL`: URL of the EngineAPI.
- `ENGINE_API_JWT_PATH`: Path to the JWT authentication file, required to connect to the EngineAPI.
- `PROVER_SERVER_LISTEN_IP`: IP to listen for proof data requests.
- `PROVER_SERVER_LISTEN_PORT`: Port to listen for proof data requests.
- `OPERATOR_ON_CHAIN_OPERATOR_ADDRESS`: Address of the block executor contract on L1.
- `OPERATOR_L1_ADDRESS`: Address of the L1 operator.
- `OPERATOR_L1_PRIVATE_KEY`: Private key of the L1 operator.
- `OPERATOR_INTERVAL_MS`: Interval in milliseconds to produce new blocks.

If you want to use a different configuration file, you can set the `ENV_FILE` environment variable to the path of the file.
