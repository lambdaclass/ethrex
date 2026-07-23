# Sync Modes

## Full sync

Full syncing works by downloading and executing every block from genesis. This means that full syncing will only work for networks that started after [The Merge](https://ethereum.org/en/roadmap/merge/), as ethrex only supports post merge execution.

## Snap sync

For snap sync, you can view the [main document here](./snap_sync.md).

### snap/2 (EIP-8189)

ethrex advertises both `snap/1` and `snap/2`; the version is negotiated
per-peer at handshake. When a `snap/2` peer is connected and the pivot is
post-Amsterdam, the post-bulk-download healing pass downloads block access
lists for the catch-up range and applies them locally instead of running
`GetTrieNodes` round-trips. Falls back to `snap/1` healing when no `snap/2`
peer is available, the pivot is pre-Amsterdam, or BAL validation fails.
