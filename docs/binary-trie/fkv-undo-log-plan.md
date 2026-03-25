# FKV Undo Log for Reorg Support

**Status**: Planned
**Context**: The binary trie branch has single-version FKV (latest state only). Any reorg corrupts state because the second fork executes against the first fork's state. This plan adds a minimal undo log to support reorgs up to 16 blocks deep.

## How it works

1. New `FKV_UNDO_LOG` column family in RocksDB
2. Before each FKV write, save old value (or "absent" marker) to undo log
3. On reorg: replay undos in reverse to restore FKV to the fork point
4. Reload binary trie from disk checkpoint, re-execute new fork's blocks
5. Prune undo entries older than 16 blocks

## Undo log key format

```
[block_number_be(8) | seq_be(4) | tag(1)]
```

- `block_number`: 8 bytes big-endian, enables efficient range scans
- `seq`: 4 bytes big-endian, monotonic within a block (handles multiple mutations)
- `tag`: `0x00` = account, `0x01` = storage

## Undo log value format

```
[fkv_key_len_le(2) | fkv_key_bytes | old_value_bytes]
```

Empty `old_value_bytes` means the key was absent before the write (delete on rollback).

## Implementation phases

### Phase 1: New CF and helpers

- Add `FKV_UNDO_LOG` constant to `crates/storage/api/tables.rs`
- Add helper functions in `store.rs`:
  - `undo_log_key(block_number, seq, tag) -> [u8; 13]`
  - `undo_log_value(fkv_key, old_value) -> Vec<u8>`
  - `parse_undo_entry(value) -> (fkv_key, Option<old_value>)`

### Phase 2: Record undo entries during FKV writes

- In `apply_updates()`, before each FKV put/delete, read old value via `fkv_read` and write undo entry
- Dedup per key per block using a local `HashSet` (only record first mutation per key)
- Use the last block's number in the batch as the undo log block number

### Phase 3: Rollback method

- `rollback_fkv_to_block(target_block)`:
  1. Iterate undo log from latest block down to `target_block + 1`
  2. For each entry: restore old value to FKV (put or delete)
  3. Delete consumed undo entries
  4. Commit atomically

### Phase 4: Integrate into fork choice

- In `apply_fork_choice` (fork_choice.rs), detect reorgs and return reorg info (fork point block number + new canonical blocks)
- RPC fork choice handler orchestrates: rollback FKV, reload binary trie from checkpoint, re-execute new fork's blocks
- Reorgs deeper than 16 blocks return sync status

### Phase 5: Pruning

- Call `prune_undo_log(current_block)` at the end of `apply_updates()`
- Deletes entries for blocks older than `current_block - 16`
- Separate write batch, non-fatal on failure

### Phase 6: Tests

- Undo key/value roundtrip
- Write block N, write block N+1, rollback to N, verify FKV matches block N
- Absent case: account created in block N+1, rollback removes it
- Pruning: 20 blocks written, entries for blocks 1-4 pruned

## Design decisions

- **Reorg depth**: 16 blocks (covers >99% of real reorgs, keeps undo log small)
- **Re-execution integration**: `apply_fork_choice` returns reorg info, RPC handler orchestrates re-execution (keeps fork choice module free of execution dependencies)
- **Binary trie rebuild**: Reload from disk checkpoint + re-execute (acceptable for small reorgs, max ~32 blocks)
- **Flush threshold**: Keep at 128 (don't increase write amplification for the rare reorg case)
- **Batch mode**: Undo keyed by last block number in batch, entire batch undone atomically

## Limitations

- Reorgs deeper than 16 blocks fall back to sync
- Re-execution cost: up to `flush_threshold + reorg_depth` blocks (~144 worst case)
- This is a stopgap. The long-term solution is a proper layer cache mirroring main's `TrieLayerCache` (see the layer cache plan discussion)

## Future: Layer cache

The proper long-term solution is per-block node layers for the binary trie (mirroring main's `TrieLayerCache`):
- Each block's trie diffs stored in a layer, chain of layers keyed by state root
- Reads walk layers newest-to-oldest, fall through to disk
- Reorgs discard orphaned layers (no re-execution needed)
- FKV writes deferred to commit time (~128 blocks)
- Removes the storage_root sentinel hack (reads go through binary trie directly)

This is ~1-2k lines and a significant architectural change, planned as a follow-up.
