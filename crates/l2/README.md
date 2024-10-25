# Ethereum Rust L2

## Table of Contents

- [Ethereum Rust L2](#ethereum-rust-l2)
  - [Table of Contents](#table-of-contents)
  - [Roadmap](#roadmap)
    - [Milestone 0](#milestone-0)
      - [Status](#status)
    - [Milestone 1: MVP](#milestone-1-mvp)
      - [Status](#status-1)
    - [Milestone 2: Block Execution Proofs](#milestone-2-block-execution-proofs)
      - [Status](#status-2)
    - [Milestone 3: State diffs + Data compression + EIP 4844 (Blobs)](#milestone-3-state-diffs--data-compression--eip-4844-blobs)
      - [Status](#status-3)
    - [Milestone 4: Custom Native token](#milestone-4-custom-native-token)
      - [Status](#status-4)
    - [Milestone 5: Security (TEEs and Multi Prover support)](#milestone-5-security-tees-and-multi-prover-support)
      - [Status](#status-5)
    - [Milestone 6: Account Abstraction](#milestone-6-account-abstraction)
      - [Status](#status-6)
    - [Milestone 7: Based Contestable Rollup](#milestone-7-based-contestable-rollup)
      - [Status](#status-7)
    - [Milestone 8: Validium](#milestone-8-validium)
      - [Status](#status-8)
  - [Prerequisites](#prerequisites)
  - [How to run](#how-to-run)
    - [Initialize the network](#initialize-the-network)
    - [Restarting the network](#restarting-the-network)
  - [Local L1 Rich Wallets](#local-l1-rich-wallets)
  - [Docs](#docs)
  - [📚 References and acknowledgements](#-references-and-acknowledgements)

## Roadmap

| Milestone | Description                                                                                                                                                                                                                                                                                                       | Status |
| --------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------ |
| 0         | Users can deposit Eth in the L1 (Ethereum) and receive the corresponding funds on the L2.                                                                                                                                                                                                                         | ✅      |
| 1         | The network supports basic L2 functionality, allowing users to deposit and withdraw funds to join and exit the network, while also interacting with the network as they do normally on the Ethereum network (deploying contracts, sending transactions, etc).                                                     | 🏗️      |
| 2         | The block execution is proven with a RISC-V zkVM and the proof is verified by the Verifier L1 contract.                                                     | 🏗️      |
| 3         | The network now commits to state diffs instead of the full state, lowering the commit transactions costs. These diffs are also submitted in compressed form, further reducing costs. It also supports EIP 4844 for L1 commit transactions, which means state diffs are sent as blob sidecars instead of calldata. | ❌      |
| 4         | The L2 can also be deployed using a custom native token, meaning that a certain ERC20 can be the common currency that's used for paying network fees.                                                                                                                                                             | ❌      |
| 5         | The L2 has added security mechanisms in place, running on Trusted Execution Environments and Multi Prover setup where multiple guarantees (Execution on TEEs, zkVMs/proving systems) are required for settlement on the L1. This better protects against possible security bugs on implementations.               | ❌      |
| 6         | The L2 supports native account abstraction following EIP 7702, allowing for custom transaction validation logic and paymaster flows.                             | ❌      |
| 7         | The network can be run as a Based Contestable Rollup, meaning sequencing is done by the Ethereum Validator set; transactions are sent to a private mempool and L1 Validators that opt into the L2 sequencing propose blocks for the L2 on every L1 block.                                                       | ❌      |
| 8         | The L2 can be initialized in Validium Mode, meaning the Data Availability layer is no longer the L1, but rather a DA layer of the user's choice.                             | ❌      |

### Milestone 0

Users can deposit Eth in the L1 (Ethereum) and receive the corresponding funds on the L2.

#### Status

|           | Name                          | Description                                                                 | Status |
| --------- | ----------------------------- | --------------------------------------------------------------------------- | ------ |
| Contracts | `CommonBridge`                | Deposit method implementation                                               | ✅     |
|           | `OnChainOperator`             | Commit and verify methods (placeholders for this stage)                     | ✅     |
| VM        |                               | Adapt EVM to handle deposits                                                | ✅     |
| Proposer  | `Proposer`                    | Proposes new blocks to be executed                                          | ✅     |
|           | `L1Watcher`                   | Listens for and handles L1 deposits                                         | ✅     |
|           | `L1TxSender`                  | commits new block proposals and sends block execution proofs to be verified | ✅     |
|           | Deposit transactions handling | new transaction type for minting funds corresponding to deposits            | ✅     |
| CLI       | `stack`                       | Support commands for initializing the network                               | ✅     |
| CLI       | `config`                      | Support commands for network config management                              | ✅     |
| CLI       | `wallet deposit`              | Support command por depositing funds on L2                                  | ✅     |
| CLI       | `wallet transfer`             | Support command for transferring funds on L2                                | ✅     |

### Milestone 1: MVP

The network supports basic L2 functionality, allowing users to deposit and withdraw funds to join and exit the network, while also interacting with the network as they do normally on the Ethereum network (deploying contracts, sending transactions, etc).

#### Status

|           | Name                           | Description                                                                                                     | Status |
| --------- | ------------------------------ | --------------------------------------------------------------------------------------------------------------- | ------ |
| Contracts | `CommonBridge`                 | Withdraw method implementation                                                                                  | ❌     |
|           | `OnChainOperator`              | Commit and verify implementation                                                                                | 🏗️     |
|           | `Verifier`                     | verifier                                                                                                        | 🏗️     |
|           | Withdraw transactions handling | New transaction type for burning funds on L2 and unlock funds on L1                                             | 🏗️     |
| Prover    | `Prover Client`                | Asks for block execution data to prove, generates proofs of execution and submits proofs to the `Prover Server` | 🏗️     |

### Milestone 2: Block Execution Proofs

The L2's block execution is proven with a RISC-V zkVM and the proof is verified by the Verifier L1 contract. This work is being done in parallel with other milestones as it doesn't block anything else.

#### Status

|           | Name              | Description                                                                                                        | Status |
| --------- | ----------------- | ------------------------------------------------------------------------------------------------------------------ | ------ |
| VM        |                   | `Return` the storage touched on block execution to pass the prover as a witness                                    | 🏗️     |
| Contracts | `OnChainOperator` | Call the actual SNARK proof verification on the `verify` function implementation                                   | 🏗️     |
| Proposer  | `Prover Server`   | Feeds the `Prover Client` with block data to be proven and delivers proofs to the `L1TxSender` for L1 verification | 🏗️     |
| Prover    | `Prover Client`   | Asks for block execution data to prove, generates proofs of execution and submits proofs to the `Prover Server`    | 🏗️     |

### Milestone 3: State diffs + Data compression + EIP 4844 (Blobs)

The network now commits to state diffs instead of the full state, lowering the commit transactions costs. These diffs are also submitted in compressed form, further reducing costs.

It also supports EIP 4844 for L1 commit transactions, which means state diffs are sent as blob sidecars instead of calldata.

#### Status

|           | Name                | Description                                                                 | Status |
| --------- | ------------------- | --------------------------------------------------------------------------- | ------ |
| Contracts | OnChainOperator     | Differentiate whether to execute in calldata or blobs mode                  | ❌     |
| Prover    | RISC-V zkVM         | Prove state diffs compression                                               | ❌     |
|           | RISC-V zkVM         | Adapt state proofs                                                          | ❌     |
| VM        |                     | The VM should return which storage slots were modified                      | ❌     |
| Proposer  | Prover Server       | Sends state diffs to the prover                                             | ❌     |
|           | L1TxSender          | Differentiate whether to send the commit transaction with calldata or blobs | ❌     |
|           |                     | Add program for proving blobs                                               | ❌     |
| CLI       | `reconstruct-state` | Add a command for reconstructing the state                                  | ❌     |
|           | `init`              | Adapt network initialization to either send blobs or calldata               | ❌     |

### Milestone 4: Custom Native token

The L2 can also be deployed using a custom native token, meaning that a certain ERC20 can be the common currency that's used for paying network fees.

#### Status

|     | Name           | Description                                                                               | Status |
| --- | -------------- | ----------------------------------------------------------------------------------------- | ------ |
|     | `CommonBridge` | For native token withdrawals, infer the native token and reimburse the user in that token | ❌     |
|     | `CommonBridge` | For native token deposits, msg.value = 0 and valueToMintOnL2 > 0                          | ❌     |
|     | `CommonBridge` | Keep track of chain's native token                                                        | ❌     |
|     | `deposit`      | Handle native token deposits                                                              | ❌     |
|     | `withdraw`     | Handle native token withdrawals                                                           | ❌     |

### Milestone 5: Security (TEEs and Multi Prover support)

The L2 has added security mechanisms in place, running on Trusted Execution Environments and Multi Prover setup where multiple guarantees (Execution on TEEs, zkVMs/proving systems) are required for settlement on the L1. This better protects against possible security bugs on implementations.

#### Status

|           | Name | Description                                          | Status |
| --------- | ---- | ---------------------------------------------------- | ------ |
| VM/Prover |      | Support proving with multiple different zkVMs        | ❌     |
| Contracts |      | Support verifying multiple different zkVM executions | ❌     |
| VM        |      | Support running the operator on a TEE environment    | ❌     |

### Milestone 6: Account Abstraction

The L2 supports native account abstraction following EIP 7702, allowing for custom transaction validation logic and paymaster flows.

#### Status

|     | Name | Description | Status |
| --- | ---- | ----------- | ------ |

TODO: Expand on account abstraction tasks.

### Milestone 7: Based Contestable Rollup

The network can be run as a Based Rollup, meaning sequencing is done by the Ethereum Validator set; transactions are sent to a private mempool and L1 Validators that opt into the L2 sequencing propose blocks for the L2 on every L1 block.

#### Status

|     | Name              | Description                                                                    | Status |
| --- | ----------------- | ------------------------------------------------------------------------------ | ------ |
|     | `OnChainOperator` | Add methods for proposing new blocks so the sequencing can be done from the L1 | ❌      |

TODO: Expand on this.

### Milestone 8: Validium

The L2 can be initialized in Validium Mode, meaning the Data Availability layer is no longer the L1, but rather a DA layer of the user's choice.

#### Status

|           | Name          | Description                                          | Status |
| --------- | ------------- | ---------------------------------------------------- | ------ |
| Contracts | BlockExecutor | Do not check data availability in Validium mode      | ❌     |
| Proposer  | L1TxSender    | Do not send data in commit transactions              | ❌     |
| CLI       | `init`        | Adapt network initialization to support Validium L2s | ❌     |
| Misc      |               | Add a DA integration example for Validium mode       | ❌     |

## Prerequisites

- [Rust (explained in the repo's main README)](../../README.md)
- [Docker](https://docs.docker.com/engine/install/) (with [Docker Compose](https://docs.docker.com/compose/install/))

## How to run

### Initialize the network

> [!IMPORTANT]
> Before this step:
>
> 1. make sure the Docker daemon is running.
> 2. make sure you have created a `.env` file following the `.env.example` file.

```
make
```

This will setup a local Ethereum network as the L1, deploy all the needed contracts on it, then start an Ethereum Rust L2 node pointing to it.

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

## Docs

- [Ethereum Rust L2 Docs](./docs/README.md)
- [Ethereum Rust L2 CLI Docs](../../cmd/ethereum_rust_l2/README.md)

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
