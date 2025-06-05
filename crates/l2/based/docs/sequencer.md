# ethrex L2 Sequencer

> [!NOTE]
> This is an extension of [ethrex-L2-Sequencer](../../docs/sequencer.md) documentation, and it is meant to be merged with it in the future.

## Components

Besides the components described in the [ethrex-L2-Sequencer](../../docs/sequencer.md) documentation, the based feature adds the following components:

### Sequencer State

> [!NOTE]
> Although this is not a component in the strict sense, it is a crucial part of the based feature implementation and it is worth mentioning it in its own section.

As part of the based feature implementation it comes the L2 sequencing decentralization. Before it, the node was built to always Sequence (as it was centralized), but now multiple nodes can be part of the network and only one of them can sequence at a time.

This emerged the need for the node to behave differently depending on whether it is the Sequencer or not. For this we gave the Sequencer the notion of state. This is the `SequencerState`, and it can be one of the following:

- `Sequencing`.
- `Following`.

As we intend to keep the system as simple as possible and the process intercommunication null, the sequencer state is a sort of a "global" state that is shared by all the components of the Sequencer. This means that all components can access the current Sequencer state and act accordingly.

This global state is managed by the State Updater component.

### State Updater

The State Updater is a component responsible for managing the Sequencer state. It monitors the Sequencer Registry contract to determine if the current node is the lead Sequencer or not. Based on this information, and other local state, it updates the Sequencer state accordingly.

The State Updater runs periodically, checking the Sequencer Registry contract for the current lead Sequencer and updating the local state. It also handles transitions between `Sequencing` and `Following` states, ensuring that components behave correctly based on the current state.

The state transition has the following rules:

- If the current node is the lead Sequencer we say it is `Sequencing`.
- If the current node is not the lead Sequencer we say it is `Following`.
- If a node stops being the current lead Sequencer, it will transition to `Following` state. During this transition, all the uncommitted state is reverted.
- If it's a node's turn to lead as Sequencer, it will transition to `Sequencing` state if and only if it is up-to-date, meaning that it has processed all the blocks up to the last committed batch. Otherwise, it will remain in `Following` state until it catches up.

### Block Fetcher

Decentralization introduces risks, such as the possibility for a lead Sequencer to advance the network on its own without sharing the blocks with the rest of the network.

To mitigate this risk, we've modified the `OnChainProposer` (as said in [ethrex-L2-Contracts](../../docs/contracts.md)) to include the list of blocks committed in the batch. This enables the possibility to reconstruct the L2 state from the L1 (at least for the time the data is available).

The Block Fetcher is a component responsible for fetching the blocks from the L1 when the Sequencer is in `Following` state. It queries the `OnChainProposer` contract for the last committed batch, and scouts the L1 for the commit transactions that contain the RLP-encoded list of blocks (similar as the L1 Watcher scouts for deposit transactions). It then reconstructs the L2 state from these blocks.

> [!NOTE]
> At this stage of the implementation, the Block Fetcher is the only syncing mechanism we have for other nodes to catch up with the Sequencer. In the future, we plan to use P2P gossiping to share the blocks between nodes.
