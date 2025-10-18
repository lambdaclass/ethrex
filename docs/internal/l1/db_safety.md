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
These tables are only written to in the `apply_updates` function, which means there are no concurrent writes to them.

### `canonical_block_hashes`

Written to only in `forkchoice_update` and `remove_blocks`, but the last one is used to revert batches from a CLI
option, not in runtime.

## `chain_data`

Written to during ethrex initialization and then read on forkchoice_update.

## `receipts`

Written to only in `apply_updates`.

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
