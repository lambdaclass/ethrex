# ethrex L2 Sequencer

## ToC

- [ethrex L2 Sequencer](#ethrex-l2-sequencer)
  - [ToC](#toc)
  - [Components](#components)
    - [Block Producer](#block-producer)
    - [L1 Watcher](#l1-watcher)
    - [L1 Transaction Sender (a.k.a. L1 Committer)](#l1-transaction-sender-aka-l1-committer)
    - [Proof Coordinator](#proof-coordinator)
  - [Configuration](#configuration)

## Components

The L2 Proposer is composed of the following components:

### Block Producer

Creates Blocks with a connection to the `auth.rpc` port.

### L1 Watcher

This component monitors the L1 for new deposits made by users. For that, it queries the CommonBridge contract on L1 at regular intervals (defined by the config file) for new DepositInitiated() events. Once a new deposit event is detected, it creates the corresponding deposit transaction on the L2.

### L1 Transaction Sender (a.k.a. L1 Committer)

As the name suggests, this component sends transactions to the L1. But not any transaction, only commit and verify transactions.

Commit transactions are sent when the Proposer wants to commit to a new batch of blocks. These transactions contain the batch data to be committed in the L1.

Verify transactions are sent by the Proposer after the prover has successfully generated a proof of block execution to verify it. These transactions contains the new state root of the L2, the hash of the state diffs produced in the block, the root of the withdrawals logs merkle tree and the hash of the processed deposits.

### Proof Coordinator

The Proof Coordinator is a simple TCP server that manages communication with a component called the Prover. The Prover acts as a simple TCP client that makes requests to prove a block to the Coordinator. It responds with the proof input data required to generate the proof. Then, the Prover executes a zkVM, generates the Groth16 proof, and sends it back to the Coordinator.

The Proof Coordinator centralizes the responsibility of determining which block needs to be proven next and how to retrieve the necessary data for proving. This design simplifies the system by reducing the complexity of the Prover, it only makes requests and proves blocks.

For more information about the Proof Coordinator, the Prover, and the proving process itself, see the [Prover Docs](./prover.md).

### L1 Proof Sender

The L1 Proof Sender is responsible for interacting with Ethereum L1 to manage proof verification. Its key functionalities include:

- Connecting to Ethereum L1 to send proofs for verification.
- Dynamically determine required proof types based on active verifier contracts (`PICOVERIFIER`, `R0VERIFIER`, `SP1VERIFIER`).
- Ensure blocks are verified in the correct order by invoking the `verify(..)` function in the `OnChainProposer` contract. Upon successful verification, an event is emitted to confirm the block's verification status.
- Operating on a configured interval defined by `proof_send_interval_ms`.

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

  - `interval_ms`: Interval in milliseconds at which the proposer builds a new block out of the current transactions on the mempool.
  - `coinbase address`: Address which will receive the execution fees.

- Under the `[committer]` section:

  - `l1_address`: Address of a funded account that will be used to send commit transactions to the L1.
  - `l1_private_key`: Its private key.
  - `commit_time_ms`: Sleep time after sending the commit transaction to the L1. If no new block has been built, the committer will simply wait another `commit_time_ms` and check again.
  - `on_chain_proposer_address`: Address of the on-chain committer.
  - `elasticity_multiplier`: Value used to set the gas limit of a block. This value is multiplied by the gas target value of the block.
  - `arbitrary_base_blob_gas_price`: Sets the minimum price floor for blob transactions when posting L2 data to the L1. This parameter allows you to control the lower bound of what the sequencer is willing to pay for blob storage. Higher values ensure faster inclusion in L1 blocks but increase operating costs, while lower values reduce costs but may cause delays.

- Under the `[prover_server]` section:

  - `l1_address`: Address of the account that sends verify transaction to L1.
    - Must be different than the `committer.l1_address`, there might be conflicts with the transactions' nonce.
  - `l1_private_key`: Its private key.
  - `listen_ip`: IP to listen for proof data requests.
  - `listen_port`: Port to listen for proof data requests.
  - `proof_send_interval_ms`: Periodicity with which the proof sender looks for new proofs to send to the L1 for verification.
  - `dev_mode`: Whether `dev_mode` is activated or not. If it is activated, the prover won't actually generate proofs, but just execute the blocks it is given, and the proof sender will send empty proofs to the L1. The L1, in turn, will ignore proofs.

- Under the `[eth]` section:

  - `rpc_url`: The URL for interacting with the RPC endpoint from the client.
  - `max_number_of_retries`: Value to set how many times are you willing to bump the gas in order to send the transaction.
  - `backoff_factor`: Base of the exponential used to calculate the backoff for the wait time when bumping the gas.
  - `min_retry_delay`: Minimum wait time for expecting to receive the receipt of the transaction. If not received, bump the transaction
  - `max_retry_delay`: Maximum wait time to before expecting the transaction's receipt. This prevents from long wait times.
  - `maximum_allowed_max_fee_per_gas`: Sets a hard cap on the maximum fee per gas you are willing to pay for a single transaction.
  - `maximum_allowed_max_fee_per_blob_gas`: Sets a hard cap on the maximum fee per blob gas you are willing to pay for a single transaction.

If you want to use a different configuration file, you can set the:

- `CONFIGS_PATH`: The path where the `SEQUENCER_CONFIG_FILE` is located at.
- `SEQUENCER_CONFIG_FILE`: The `.toml` that contains the config for the `sequencer`.
