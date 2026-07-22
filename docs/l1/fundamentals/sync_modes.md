# Sync Modes

## Full sync

Full syncing works by downloading and executing every block from genesis. This means that full syncing will only work for networks that started after [The Merge](https://ethereum.org/en/roadmap/merge/), as ethrex only supports post merge execution.

## Snap sync

For snap sync, you can view the [main document here](./snap_sync.md).

## Historical chain backfill

A snap-synced node backfills only block **headers** below the sync pivot; it does
not download the block bodies or receipts for pre-pivot blocks. As a result,
historical RPC queries for those blocks return empty:
`eth_getBlockByNumber`/`ByHash`, `eth_getBlockReceipts`,
`eth_getTransactionByHash`, and `eth_getTransactionReceipt` return `null`, and
`eth_getLogs` over a pre-pivot range fails.

Historical chain backfill is an **opt-in** background process that downloads and
validates the missing bodies and receipts after snap sync completes, so the node
can serve historical block, transaction, receipt, and log queries.

### Enabling it

| Flag | Env | Values | Default |
| --- | --- | --- | --- |
| `--history.chain` | `ETHREX_HISTORY_CHAIN` | `off`, `postmerge`, `all` | `off` |
| `--history.transactions` | `ETHREX_HISTORY_TRANSACTIONS` | number of blocks (`0` = whole backfilled range) | `0` |

- **`off`** (default): headers-only below the pivot — current behavior.
- **`postmerge`**: backfill down to the network's merge (Paris) activation block.
  This is the recommended value: post-merge history is what the peer set reliably
  serves, and it is what most applications need.
- **`all`**: backfill down to genesis. **Best-effort** — after the 2025 history
  expiry rollout many peers no longer serve pre-merge bodies/receipts, so this can
  stall at a block it cannot fetch (it reports the stall rather than failing the
  node).

`--history.transactions` controls how far back the transaction-lookup index
(`eth_getTransactionByHash`) is kept, independently of the block/receipt data,
mirroring geth's flag of the same name.

### How it works

Backfill fills in reverse — from the pivot downward toward the floor — one
bounded batch at a time. It runs at lower priority than following the chain head:
it waits until initial sync finishes, sleeps between batches, and never lets the
tip fall behind. Progress is tracked in `earliest_block_number` (the lowest block
with full data), which also serves as the durable resume cursor, so an
interrupted backfill resumes across restarts without gaps. Bodies and receipts
are validated against the already-synced header chain (transactions/receipts
roots) before being stored; a receipt's logs bloom is recomputed from its logs,
so it works with eth/68 and eth/69 peers alike.

Expect a substantial disk-usage increase when enabled (on the order of hundreds
of GB for mainnet), since it adds the bodies and receipts a headers-only node
omits.

### Limitations

- **Not an archive node.** Backfill restores historical *chain* data (blocks,
  transactions, receipts, logs); it does not restore historical *state*.
  `eth_call`, `eth_getBalance`, and tracing at old block heights remain bounded to
  the recent in-memory state window regardless of this setting.
- **`eth_getLogs` becomes correct, not fast.** Backfill lets historical log
  queries return results instead of failing, but there is no log/bloom index, so
  wide historical ranges are still served by a linear per-block bloom scan.
- **Receipts are fetched from eth/68, eth/69, and eth/71 peers.** eth/71
  (EIP-8159) reuses eth/69's receipt format, so only the rare, skipped eth/70
  (whose `GetReceipts` is paginated) is unsupported and simply not used.
