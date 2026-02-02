# EF Tests Work Log

## Current Investigation
**Iteration:** 1
**Focusing on:** BlockAccessListHashMismatch (323 failures)
**Working hypothesis:** The `restore` function in BAL checkpoint incorrectly restores storage_reads, discarding reads made during reverted calls.

## Test Summary
- Total tests: 6158
- Passed: 6094
- Failed: 64 (official count, 330 logged failures due to duplicate reporting)
- Main error: BlockAccessListHashMismatch (323)
- Secondary: ReceiptsRootMismatch (3)

## Key Findings
- Tests with only system contract calls (no user txs) PASS
- Tests with transactions that REVERT fail
- The execution-specs `merge_on_failure` function shows:
  1. Storage reads should PERSIST on revert (union with parent)
  2. Storage writes should be CONVERTED TO READS on revert
- Our `restore` function DISCARDS storage_reads by restoring to checkpoint

## Code Locations Identified
- `crates/common/types/block_access_list.rs:871` - `restore()` function
- `/data2/edgar/work/execution-specs/src/ethereum/forks/amsterdam/state_tracker.py:410` - `merge_on_failure`

## Attempted Approaches
(None yet)

## Bug Analysis
The issue is in `BlockAccessListCheckpoint::restore()`:
```rust
self.storage_reads = checkpoint.storage_reads_snapshot;
```
This REPLACES storage_reads with the snapshot, DISCARDING any reads made during the reverted call.

Should instead:
1. Keep all storage_reads (reads persist on revert)
2. Convert reverted writes to reads
3. Only truncate state changes (balance, nonce, code, write values)

## Next Steps
1. [x] Identified root cause in `restore()` function
2. [x] Fixed the restore function - reduced failures from 323 to 320
3. [ ] Investigate remaining 320 BlockAccessListHashMismatch failures
4. [ ] May involve EIP-7708 ETH transfer logs or value transfer handling

## Investigation Notes for Next Session
- Fix #1 addressed revert handling for storage reads
- Remaining failures involve tests with:
  - `with_value` scenarios (ETH transfers)
  - `delegated_account` scenarios (EIP-7702)
  - Various EIP-7708 ETH transfer log tests
- Need to compare expected vs actual BAL for remaining failures
- Possible issues: balance change recording during value transfers, delegation handling

## Notes
- EIP-7928: "State changes from reverted calls are discarded, but all accessed addresses must be included."
- Storage reads are accesses, not state changes, so they should persist
- Storage writes that revert should become reads (slot was accessed but value didn't change)
