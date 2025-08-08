# ethrex L2 Sequencer

> [!IMPORTANT]
> This documentation is about the current state of the `based` feature development and not about the final implementation. It is subject to change as the feature evolves and their still could be unmitigated issues.

> [!NOTE]
> This is an extension of the [ethrex-L2-Sequencer documentation](../fundamentals/components/sequencer.md) and is intended to be merged with it in the future.

## Components

In addition to the components outlined in the [ethrex-L2-Sequencer documentation](../fundamentals/components/sequencer.md), the `based` feature introduces new components to enable decentralized L2 sequencing. These additions enhance the system's ability to operate across multiple nodes, ensuring resilience, scalability, and state consistency.

### Sequencer State

> [!NOTE]
> While not a traditional component, the **Sequencer State** is a fundamental element of the `based` feature and deserves its own dedicated section.

The `based` feature decentralizes L2 sequencing, moving away from a single, centralized Sequencer to a model where multiple nodes can participate, with only one acting as the lead Sequencer at any time. This shift requires nodes to adapt their behavior depending on their role, leading to the introduction of the **Sequencer State**. The Sequencer State defines two possible modes:

- `Sequencing`: The node is the lead Sequencer, responsible for proposing and committing new blocks to the L2 chain.
- `Following`: The node is not the lead Sequencer and must synchronize with and follow the blocks proposed by the current lead Sequencer.

To keep the system simple and avoid intricate inter-process communication, the Sequencer State is implemented as a **global state**, accessible to all Sequencer components. This design allows each component to check the state and adjust its operations accordingly. The **State Updater** component manages this global state.

### State Updater

The **State Updater** is a new component tasked with maintaining and updating the Sequencer State. It interacts with the **Sequencer Registry** contract on L1 to determine the current lead Sequencer and adjusts the node’s state based on this information and local conditions. Its responsibilities include:

- **Periodic Monitoring**: The State Updater runs at regular intervals, querying the `SequencerRegistry` contract to identify the current lead Sequencer.
- **State Transitions**: It manages transitions between `Sequencing` and `Following` states based on these rules:
  - If the node is designated as the lead Sequencer, it enters the `Sequencing` state.
  - If the node is not the lead Sequencer, it enters the `Following` state.
  - When a node ceases to be the lead Sequencer, it transitions to `Following` and reverts any uncommitted state to ensure consistency with the network.
  - When a node becomes the lead Sequencer, it transitions to `Sequencing` only if it is fully synced (i.e., has processed all blocks up to the last committed batch). If not, it remains in `Following` until it catches up.

This component ensures that the node’s behavior aligns with its role, preventing conflicts and maintaining the integrity of the L2 state across the network.

### Block Fetcher

Decentralization poses a risk: a lead Sequencer could advance the L2 chain without sharing blocks, potentially isolating other nodes. To address this, the `OnChainProposer` contract (see [ethrex-L2-Contracts documentation](../fundamentals/contracts.md)) has been updated to include an RLP-encoded list of blocks committed in each batch. This makes block data publicly available on L1, enabling nodes to reconstruct the L2 state if needed.

The **Block Fetcher** is a new component designed to retrieve these blocks from L1 when the node is in the `Following` state. Its responsibilities include:

- **Querying L1**: It queries the `OnChainProposer` contract to identify the last committed batch.
- **Scouting Transactions**: Similar to how the L1 Watcher monitors deposit transactions, the Block Fetcher scans L1 for commit transactions containing the RLP-encoded block list.
- **State Reconstruction**: It uses the retrieved blocks to rebuild the L2 state, ensuring the node remains synchronized with the network.

> [!NOTE]
> Currently, the Block Fetcher is the primary mechanism for nodes to sync with the lead Sequencer. Future enhancements will introduce P2P gossiping to enable direct block sharing between nodes, improving efficiency.
