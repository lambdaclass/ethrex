# Database safety without Rocksdb transactions

## Content addressed tables

- (block)`headers`
- (block)`bodies`
- `account_codes`
- `pending_blocks`

These tables are content addressed, which makes them safe because writes to them are atomic,
and them being content addressable means anyone reading from them either sees their
only possible value or they don't, but nothing else.

## Other Tables

- `block_numbers`
- `transaction_locations`

Written by `apply_updates` during block import, and **also deleted from by the
history pruner** (`Store::prune_block_heights`) when `--history.retention` is
enabled. The pruner is a background task, so on such a node these tables do have
concurrent writers.

For `transaction_locations` this is a known, accepted race, documented on
`prune_block_heights`: the pruner trims a transaction's location list with a
read-modify-write that is not serialized against block imports, which append via the
merge operator. A merge committed between the pruner's read and its commit is
overwritten. Reaching that window requires a transaction whose only prior inclusions
were orphans old enough to be pruned being re-included mid-pass; the consequence is a
lost location entry for such a transaction, not corruption of unrelated data.

For `block_numbers` the pruner only deletes rows belonging to non-canonical
(orphaned) hashes at heights below the finalized point, which no other writer touches.

### `canonical_block_hashes`

Written to only in `forkchoice_update` and `remove_blocks`, but the last one is used to revert batches from a CLI
option, not in runtime. The history pruner reads this table but never writes it.

### `block_hashes_by_number`

Indexes every known block hash per height (`BE block_number || block_hash`), so the
pruner can enumerate canonical and orphaned blocks at a height. Written on every
header-insertion path alongside `headers`, and range-deleted by the pruner. Values are
empty, so a concurrent re-write of the same key is a no-op and the only observable
race is a stray entry surviving a pass (see `prune_block_heights`), which readers
tolerate because they gate on `EarliestBlockNumber`.

## `chain_data`

Written to during ethrex initialization and then read on forkchoice_update.

`EarliestBlockNumber` is the exception: it is a monotonic high-water mark advanced at
runtime by snap-sync completion and by each history-pruner pass. Writers must never
lower it (`Store::advance_earliest_block_number` enforces this, and the pruner's own
write is gated inside its atomic batch), because readers use it to decide whether
missing history is "pruned" or "not yet known".

## `receipts`

Written to in `apply_updates`, and deleted from by the history pruner alongside the
corresponding block bodies.

## `snap_state`

Written to only during snap sync and mostly a legacy table used to signal the rest of the code when snap sync has finished.

## `trie_nodes`

All writes to the state and storage tries are done through the `apply_updates` function,
called only after block execution.
There is only one other place where we write to the tries, and that's during snap
sync, through the `write_storage_trie_nodes_batch` function (and similarly for state trie nodes);
this does not pose a problem because there is no block execution until snap sync is done.

There is also a `put_batch` function for the trie itself, but it is only used inside snap sync and 
genesis setup, but nowhere else.

## `invalid_ancestors`

Written to in `set_latest_valid_ancestor`, called from every engine api endpoint and during full sync.

TODO: check validity of this.

## `full_sync_headers`

Written to and read only sequentially on the same function during full sync.
