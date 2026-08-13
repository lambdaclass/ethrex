# Sync Modes

## Full sync

Full syncing works by downloading and executing every block from genesis. This means that full syncing will only work for networks that started after [The Merge](https://ethereum.org/en/roadmap/merge/), as ethrex only supports post merge execution.

## Snap sync

For snap sync, you can view the [main document here](./snap_sync.md).

### snap/2 (EIP-8189)

ethrex advertises `snap/2` alongside `snap/1` unless its own state sync still
depends on `GetTrieNodes`, which snap/2 removes; the version is negotiated
per-peer at handshake.

On a post-Amsterdam chain with a `snap/2` peer available, state sync downloads
flat key-value state, patches it with the block access lists of the blocks that
passed while it downloaded, then rebuilds every trie locally and verifies the
result against the pivot's state root. There is no trie healing at all.

It falls back to `snap/1` when no `snap/2` peer is available, when the pivot is
pre-Amsterdam, or when BAL validation fails.

Implementation notes and known gaps are in
[snap/2 internals](../../internal/l1/snap_v2.md).
