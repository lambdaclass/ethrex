# Sync Modes

## Full sync

Full syncing works by downloading and executing every block from genesis. This means that full syncing will only work for networks that started after [The Merge](https://ethereum.org/en/roadmap/merge/), as ethrex only supports post merge execution.

## Snap sync

Snap syncing is a much faster alternative to full sync that works by downloading and executing only the latest blocks from the network. For a much more in depth description on how snap sync works under the hood please read the [snap networking documentation](./networking/Sync.md)
