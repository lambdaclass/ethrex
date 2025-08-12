# ethrex L2 Contracts

> [!IMPORTANT]
> This documentation is about the current state of the `based` feature development and not about the final implementation. It is subject to change as the feature evolves and their still could be unmitigated issues.

> [!NOTE]
> This is an extension of the [ethrex-L2-Contracts documentation](../fundamentals/contracts.md) and is intended to be merged with it in the future.

## L1 Side

In addition to the components described in the [ethrex-L2-Contracts documentation](../fundamentals/contracts.md), the based feature introduces new contracts and modifies existing ones to enhance decentralization, security, and transparency. Below are the key updates and additions:

### OnChainProposer (Modified)

The `OnChainProposer` contract, which handles batch proposals and management on L1, has been updated with the following modifications:

- **New Constant:**
  A public constant `SEQUENCER_REGISTRY` has been added. This constant holds the address of the `SequencerRegistry` contract, linking the two contracts for sequencer management.
- **Modifier Update:**
  The `onlySequencer` modifier has been renamed to `onlyLeadSequencer`. It now checks whether the caller is the current lead Sequencer, as determined by the `SequencerRegistry` contract. This ensures that only the designated leader can commit batches.
- **Initialization:**
  The `initialize` method now accepts the address of the `SequencerRegistry` contract as a parameter. During initialization, this address is set to the `SEQUENCER_REGISTRY` constant, establishing the connection between the contracts.
- **Batch Commitment:**
  The `commitBatch` method has been revised to improve data availability and streamline sequencer validation:
  - It now requires an RLP-encoded list of blocks included in the batch. This list is published on L1 to ensure transparency and enable verification.
  - The list of sequencers has been removed from the method parameters. Instead, the `SequencerRegistry` contract is now responsible for tracking and validating sequencers.
- **Event Modification:**
  The `BatchCommitted` event has been updated to include the batch number of the committed batch. This addition enhances traceability and allows external systems to monitor batch progression more effectively.
- **Batch Verification:**
  The `verifyBatch` method has been made more flexible and decentralized:
  - The `onlySequencer` modifier has been removed, allowing anyone—not just the lead Sequencer—to verify batches.
  - The restriction preventing multiple verifications of the same batch has been lifted. While multiple verifications are now permitted, only one valid verification is required to advance the L2 state. This change improves resilience and reduces dependency on a single actor.

### SequencerRegistry (New Contract)

The `SequencerRegistry` is a new contract designed to manage the pool of Sequencers and oversee the leader election process in a decentralized manner.

- **Registration:**
  - Anyone can register as a Sequencer by calling the `register` method and depositing a minimum collateral of 1 ETH. This collateral serves as a Sybil resistance mechanism, ensuring that only committed participants join the network.
  - Sequencers can exit the registry by calling the `unregister` method, which refunds their 1 ETH collateral upon successful deregistration.
- **Leader Election:**
  The leader election process operates on a round-robin basis to fairly distribute the lead Sequencer role:
  - **Single Sequencer Case:** If only one Sequencer is registered, it remains the lead Sequencer indefinitely.
  - **Multiple Sequencers:** When two or more Sequencers are registered, the lead Sequencer rotates every 32 batches. This ensures that no single Sequencer dominates the network for an extended period.
- **Future Leader Prediction:**
  The `futureLeaderSequencer` method allows querying the lead Sequencer for a batch n batches in the future. The calculation is based on the following logic:

  **Inputs:**

  - `sequencers`: An array of registered Sequencer addresses.
  - `currentBatch`: The next batch to be committed, calculated as `lastCommittedBatch() + 1` from the `OnChainProposer` contract.
  - `nBatchesInTheFuture`: A parameter specifying how many batches ahead to look.
  - `targetBatch`: Calculated as `currentBatch` + `nBatchesInTheFuture`.
  - `BATCHES_PER_SEQUENCER`: A constant set to 32, representing the number of batches each lead Sequencer gets to commit.

  **Logic:**

  ```solidity
  uint256 _currentBatch = IOnChainProposer(ON_CHAIN_PROPOSER).lastCommittedBatch() + 1;
  uint256 _targetBatch = _currentBatch + nBatchesInTheFuture;
  uint256 _id = _targetBatch / BATCHES_PER_SEQUENCER;
  address _leader = sequencers[_id % sequencers.length];
  ```

  **Example:** Assume 3 Sequencers are registered: `[S0, S1, S2]`, and the current committed batch is 0:

  - For batches 0–31: `_id = 0 / 32 = 0, 0 % 3 = 0`, lead Sequencer = `S0`.
  - For batches 32–63: `_id = 32 / 32 = 1, 1 % 3 = 1`, lead Sequencer = `S1`.
  - For batches 64–95: `_id = 64 / 32 = 2, 2 % 3 = 2`, lead Sequencer = `S2`.
  - For batches 96–127: `_id = 96 / 32 = 3, 3 % 3 = 0`, lead Sequencer = `S0`.

  This round-robin rotation repeats every 96 committed batches (32 committed batches per Sequencer × 3 Sequencers), ensuring equitable distribution of responsibilities.
