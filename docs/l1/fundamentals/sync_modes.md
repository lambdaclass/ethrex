# Sync Modes

## Full sync

Full syncing works by downloading and executing every block from genesis. This means that full syncing will only work for networks that started after [The Merge](https://ethereum.org/en/roadmap/merge/), as ethrex only supports post merge execution.

## Snap sync

For snap sync, you can view the [main document here](./snap_sync.md).

## Historical chain backfill

A snap-synced node keeps only block **headers** below the sync pivot, so
historical RPC queries for pre-pivot blocks (`eth_getBlockByNumber`,
`eth_getBlockReceipts`, `eth_getTransactionByHash`, `eth_getLogs`, …) return
empty. Historical chain backfill is an **opt-in** background process
(`--history.chain postmerge`) that downloads and validates the missing bodies and
receipts after snap sync completes, so the node can serve those queries.

See [Historical chain backfill](./history_backfill.md) for the flags, how it
works, durability across restarts, and its limitations.
