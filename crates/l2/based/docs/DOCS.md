# Based ethrex

## Table of Contents

- [Roadmap](#Roadmap)
  - [Milestone 1:](#milestone-1)
    - [Checkpoint 0: Transactions Broadcasting](#checkpoint-0-transactions-broadcasting)
    - [Checkpoint 1: Dummy Based Messaging](#checkpoint-1-dummy-based-messaging)
    - [Checkpoint 2: Non-signed Based Messaging](#checkpoint-2-non-signed-based-messaging)
    - [Checkpoint 3: Signed Messaging](#checkpoint-3-signed-messaging)
    - [Checkpoint 4: Syncing](#checkpoint-4-syncing)
  - [Milestone 2: P2P](#milestone-2-p2p)
  - [Milestone 3: Testnet](#milestone-3-testnet)
  - [Milestone 4](#milestone-4)
- [Run Locally](#run-locally)
  - [1. Deploying L1 Contracts](#1-deploying-l1-contracts)
  - [2. Running a node](#2-running-a-node)
  - [3. Becoming a Sequencer](#3-becoming-a-sequencer)
- [Diff](#diff)

## Roadmap

> [!NOTE]
> This roadmap is still a WIP and is subject to change.

### Milestone 1: MVP

- Sequencers **register** via an L1 smart contract.
- Any Node:
  - **follows** the Lead Sequencer **via L1 syncing**.
- Lead Sequencer:
  - is **elected through a Round-Robin** election in L1,
  - **produces** L2 blocks,
  - **posts** L2 batches to L1 during their allowed period.
- `OnChainProposer`’s `verifyBatch` method is **callable by anyone**. **Only one valid proof is needed** to advance the network.
- `OnChainProposer`’s `commitBatch` method is **callable by the lead Sequencer**.

### Milestone 2: P2P

- **Lead Sequencer**: Broadcasts `NewBlock` and `SealBatch` messages to the network.
- **Any Node**:
  - broadcasts transactions to the network;
  - receives, handles, and broadcasts `NewBlock` and `SealBatch` messages to the network;
  - on `NewBlock`s
    - validates the message signature,
    - stores the block,
    - or queue it if it is not the next one,
    - broadcasts it to the network;
  - on `SealBatch`s
    - validates the message signature;
    - seals the batch,
    - or queue it if it miss some blocks,
    - broadcasts it to the network;
- **Nodes State**: A new state emerges from the current Following state, this is the Syncing state.
  - **Next Batch**: The L2 batch being built by the lead Sequencer.
  - **Up-To-Date Nodes:** Nodes that have the last committed batch in their storage and only miss the next batch.
  - **Following:** We say that up-to-date nodes are **following** the lead Sequencer.
  - **Syncing:** Nodes are **syncing** if they are not up-to-date. They’ll stop syncing after they reach the **following** state.

#### Checkpoint 0: Transactions Broadcasting

- **All Nodes**: broadcasts transactions to the network.

#### Checkpoint 1: Dummy Based Messaging

- **Lead Sequencer**:
  - broadcasts **empty `NewBlock`** messages, as it produces blocks, to the network;
  - broadcasts **empty `SealBatch`** messages, as it seals batches, to the network.
- **Any Node**:
  - only logs `NewBlock` and `SealBatch` reception,
  - broadcasts them to the network.

#### Checkpoint 2: Non-signed Based Messaging

- **Lead Sequencer**:
  - broadcasts **non-empty `NewBlock`** messages, as it produces blocks, to the network;
    ```rust
    pub struct NewBlock {
        batch_number: u64,
        encoded_block: Vec<u8>,
    }
    ```
  - broadcasts **non-empty `SealBatch`** messages, as it seals batches, to the network.
    ```rust
    pub struct SealBatch {
        batch_number: u64,
        encoded_batch: Vec<u64>,
    }
    ```

#### Checkpoint 3: Signed Messaging

- on `NewBlock`s
  - stores the block,
  - or queue it if it is not the next one,
  - broadcasts it to the network;
- on `SealBatch`s
  - seals the batch,
  - or queue it if it miss some blocks,
  - broadcasts it to the network;

#### Checkpoint 4: Syncing

TODO

### Milestone 3: Testnet

- Web page simil [https://beaconcha.in](https://beaconcha.in/) to visualize
  - Sequencers.
  - Sequencing rounds and their progress.
  -

### Milestone 4:

TODO

## Run Locally

Running a based stack locally is essentially the same as running an ethrex stack but with a few differences in the deployment process:

### 1. Deploying L1 Contracts

> [!IMPORTANT]
> You need to have an L1 running to deploy the contracts. Run `make init-local-l1` to do so (ensure Docker running).

In a console with `crates/l2` as the current directory, run the following command to deploy the L1 contracts for a based L2:

```bash
cargo run --release --bin ethrex_l2_l1_deployer --manifest-path contracts/Cargo.toml -- \
	--genesis-l1-path ../../test_data/genesis-l1-dev.json \
	--genesis-l2-path ../../test_data/genesis-l2.json \
	--contracts-path contracts \
	--sp1.verifier-address 0x00000000000000000000000000000000000000aa \
	--pico.verifier-address 0x00000000000000000000000000000000000000aa \
	--risc0.verifier-address 0x00000000000000000000000000000000000000aa \
	--tdx.verifier-address 0x00000000000000000000000000000000000000aa \
	--bridge-owner <ADDRESS> \
	--on-chain-proposer-owner <ADDRESS> \
	--deposit-rich \
	--private-keys-file-path ../../test_data/private_keys_l1.txt \
	--deploy-based-contracts \
	--sequencer-registry-owner <ADDRESS>
```

This command will:

1. Deploy the L1 contracts, including the based contracts `SequencerRegistry`, and a modified `OnChainProposer`.
2. Deposit funds in the accounts from `../../test_data/private_keys_l1.txt`.
3. Skip deploying the verifier contracts by specifying `0x00000000000000000000000000000000000000aa` as their address. This means that the node will run in "dev mode" and that the proof verification will not be performed. This is useful for local development and testing, but should not be used in production environments.

Optionally, you can define an owner for the `CommonBridge`, `OnChainProposer`, and `SequencerRegistry` contracts by replacing `<ADDRESS>` with the desired address. If left empty, the contracts will be deployed with the default owner address, that is to say, the address of the deployer.

> [!NOTE]
> Save the addresses of the deployed proxy contracts, as you will need them to run the L2 node.

### 2. Running a node

> [!IMPORTANT]
> You need to have an L1 running with the contracts deployed to run the L2 node. See the previous steps.

In a console with `ethrex/` (repository root) as the current directory, run the following command to start a based L2 node:

```bash
cargo run --release --bin ethrex --features l2 -- l2 init \
  --watcher.block-delay 0 \
  --committer-l1-private-key 0x385c546456b6a603a1cfcaa9ec9494ba4832da08dd6bcf4de9a71e4a01b74924 \
  --proof-coordinator-l1-private-key 0x39725efee3fb28614de3bacaffe4cc4bd8c436257e2c8bb887c4b5c4be45e76d \
  --network test_data/genesis-l2.json \
  --datadir <DATA_DIRECTORY> \
  --proof-coordinator-listen-ip 127.0.0.1 \
  --proof-coordinator-listen-port 4566 \
  --http.port 1729 \
  --state-updater.sequencer-registry <SEQUENCER_REGISTRY_ADDRESS> \
  --on-chain-proposer-address <ON_CHAIN_PROPOSER_ADDRESS> \
  --bridge-address <COMMON_BRIDGE_ADDRESS> \
  --based
```

After running this command, the node will start syncing with the L1 and will be able to follow the lead Sequencer.

> [!IMPORTANT]
> If there is no state in L1 (this could happen if you are running a fresh L1), the node will display a log message indicating that it is up-to-date. This is expected behavior, as the node will not have any blocks to process until the lead Sequencer produces a new block.

> [!NOTE]
>
> 1. Replace `<DATA_DIRECTORY>`, `<SEQUENCER_REGISTRY_ADDRESS>`, `<ON_CHAIN_PROPOSER_ADDRESS>`, and `<COMMON_BRIDGE_ADDRESS>` with the appropriate values from the previous step.
> 2. If you want to run multiple nodes, ensure that the following values are different for each node:
>
> - `--proof-coordinator-listen-port`
> - `--http.port`
> - `--datadir`
> - `--committer-l1-private-key`
> - `--proof-coordinator-l1-private-key`

### 3. Becoming a Sequencer

For nodes to become lead Sequencers they need to register themselves in the `SequencerRegistry` contract. This can be done by calling the `register` method of the contract with the node's address.

To register a node as a Sequencer, you can use the following command using `rex`:

```bash
rex send <REGISTRY_CONTRACT> 1000000000000000000 <REGISTRANT_PRIVATE_KEY> -- "register(address)" <SEQUENCER_ADDRESS> // registers SEQUENCER_ADDRESS as a Sequencer supplying 1 ETH as collateral (the minimum).
```

> [!IMPORTANT]
> The `SEQUENCER_ADDRESS` must be the address of the node's committer since it is the one that will be posting the batches to the L1.

Once registered, the node will be able to participate in the Sequencer election process and become the lead Sequencer when its turn comes.

> [!NOTE]
>
> 1. Replace `<REGISTRY_CONTRACT>`, `<REGISTRANT_PRIVATE_KEY>`, and `<SEQUENCER_ADDRESS>` with the appropriate values.
> 2. The registrant is not necessarily related to the sequencer, one could pay the registration for some else.
> 3. If only one Sequencer is registered, it will always be elected as the lead Sequencer. If multiple Sequencers are registered, they will be elected in a Round-Robin fashion (32 batches each as defined in the `SequencerRegistry` contract).

## Diff

This section aims to highlight the differences between the based L2 and the ethrex L2.

- The `OnChainProposer` was slightly modified:
  - The `verifyBatch` method is now callable by anyone, allowing the network to advance with only one valid proof. It is also possible to verify a batch more than once.
  - The `commitBatch` method now accepts a list of blocks that were included in the batch, allowing nodes to sync via L1.
  - The `SEQUENCER_REGISTRY` constant was added to the contract, which is the address of the `SequencerRegistry` contract.
  - The `initialize` method was modified to accept the `SEQUENCER_REGISTRY` address as an argument, allowing the contract to be initialized with the correct `SequencerRegistry`.
  - The `onlySequencer` modifier now relies on the `SequencerRegistry` contract to check if the caller is a registered Sequencer.
- The `SequencerRegistry` contract was added, which allows Sequencers to register and deregister themselves. It is also in charge of the Sequencer election process which as of now is a Round-Robin election.
- A `SequencerState` enum was added to control the node's responsibilities:
  - `Sequencing`: The node is the lead Sequencer.
  - `Following`: The node is following the lead Sequencer.
- Some L2 components were modified to have behave differently depending on the node's state:
  - `L1Committer`:
    - `Sequencing`: Behaves as normal.
    - `Following`: Does nothing.
  - `L1Watcher`:
    - `Sequencing`: Behaves as normal.
    - `Following`: Does nothing.
  - `L1ProofSender`:
    - `Sequencing`: Behaves as normal.
    - `Following`: Does nothing.
  - `L1ProofCoordinator`:
    - `Sequencing`: Behaves as normal.
    - `Following`: Does nothing.
- New L2 components were added:
  - `StateUpdater`: updates the node's shared state based on who the lead Sequencer is.
  - `BlockFetcher`: syncs the node via L1.
    - `Sequencing`: Does nothing.
    - `Following`: Fetches the next batch from L1 and updates the node's state.
