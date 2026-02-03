# Fundamentals

In L2 mode, the ethrex code is repurposed to run a rollup that settles on Ethereum as the L1.

The main differences between this mode and regular ethrex are:

- In regular rollup mode, there is no consensus; the node is turned into a sequencer that proposes blocks for the chain. In based rollup mode, consensus is achieved by a mechanism that rotates sequencers, enforced by the L1.
- Block execution is proven using a RISC-V zkVM (or attested to using TDX, a Trusted Execution Environment) and its proofs (or signatures/attestations) are sent to L1 for verification.
- A set of Solidity contracts to be deployed to the L1 are included as part of chain initialization.
- Two new types of transactions are included: deposits (native token mints) and withdrawals.

At a high level, the following new parts are added to the node:

- A `proposer` component, in charge of continually creating new blocks from the mempool transactions. This replaces the regular flow that an Ethereum L1 node has, where new blocks come from the consensus layer through the `forkChoiceUpdate` -> `getPayload` -> `NewPayload` Engine API flow in communication with the consensus layer.
- A `prover` subsystem, which itself consists of two parts:
  - A `proverClient` that takes new blocks from the node, proves them, then sends the proof back to the node to send to the L1. This is a separate binary running outside the node, as proving has very different (and higher) hardware requirements than the sequencer.
  - A `proverServer` component inside the node that communicates with the prover, sending witness data for proving and receiving proofs for settlement on L1.
- L1 contracts with functions to commit to new state and then verify the state transition function, only advancing the state of the L2 if the proof verifies. It also has functionality to process deposits and withdrawals to/from the L2.
- The EVM is lightly modified with new features to process deposits and withdrawals accordingly.

## Ethrex L2 documentation

For general documentation, see:

- [Architecture overview](../architecture/overview.md) for a high-level view of the ethrex L2 stack.
- [Smart contracts](./contracts.md) has information on L1 and L2 smart contracts including steps to upgrade and transfer ownserhip.
- [Based sequencing](./based.md) contains ethrex's roadmap for becoming based.
- [State diffs](./state_diffs.md) explains the mechanism needed to provide data availability.
- How asset [deposits](./deposits.md) and [withdrawals](./withdrawals.md) work.
- [Fee token](./fee_token.md)
- [Exit window](./exit_window.md) and [Timelock](./timelock.md) for upgrade safety mechanisms.
- [Aligned Layer Integration](./ethrex_l2_aligned_integration.md) details how ethrex L2 integrates with Aligned Layer for proof aggregation and verification.
