# Sync Modes

## Full sync

Full syncing works by downloading and executing every block from genesis. This means that full syncing will only work for networks that started after [The Merge](https://ethereum.org/en/roadmap/merge/), as ethrex only supports post merge execution.

## Snap sync

For snap sync, you can view the [main document here](./snap_sync.md).

Snap sync uses the snap/1 subprotocol for state download and, post-Amsterdam fork, the snap/2 subprotocol (EIP-8189) for state healing. snap/2 replaces trie-node healing with Block Access List (BAL) replay: the node fetches the BAL diffs produced by EIP-7928 and applies them block-by-block to advance the state root to the chain head. snap/1 trie-node healing is retained as a fallback for snap/1-only peers.
