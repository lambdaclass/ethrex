# Ethrex L2

In this mode, the ethrex code is repurposed to run a rollup that settles on Ethereum as the L1.

The main differences between this mode and regular ethrex are:

- In regular rollup mode, there is no consensus; the node is turned into a sequencer that proposes blocks for the network. In based rollup mode, consensus is achieved by a mechanism that rotates sequencers, enforced by the L1.
- Block execution is proven using a RISC-V zkVM (or attested to using TDX, a Trusted Execution Environment) and its proofs (or signatures/attestations) are sent to L1 for verification.
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

For how to install our dependencies, go to their official documentation:

- [Rust](https://www.rust-lang.org/tools/install)
- [Solc 0.29](https://docs.soliditylang.org/en/latest/installing-solidity.html)
- [Docker](https://docs.docker.com/engine/install/)

## How to run

### Initialize the network

> [!IMPORTANT]
> Before this step:
>
> 1. Make sure you are inside the `crates/l2` directory.
> 2. Make sure the Docker daemon is running.

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



# Ethrex L2 documentation

For general documentation, see:

- [Getting started](./getting_started.md) contains guides on setting up and interacting with an ethrex L2 stack.
- [General overview](./overview.md) for a high-level view of the ethrex L2 stack.
- [Smart contracts](./contracts.md) has information on L1 and L2 smart contracts.
- [Components](./components.md) for more detailed documentation on each off-chain component.
- [Based roadmap (draft)](./roadmap.md) contains ethrex's roadmap for becoming based.


## Developer documentation

Documentation useful for ethrex development can be found in the ["Developers"](../developers) section.
