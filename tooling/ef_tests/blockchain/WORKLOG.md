# EF Tests Work Log

## Current Investigation
**Iteration:** 2
**Focusing on:** Remaining BlockAccessListHashMismatch failures (58 total)
**Working hypothesis:** Additional BAL tracking issues remain for specific scenarios

## Test Summary
- Total tests: 6158
- Passed: 6100
- Failed: 58
- Previous failures: 64

## Breakdown of Remaining Failures
- 30 EIP-7928 BAL tests (down from 31)
- 21 EIP-7708 ETH transfer log tests (unchanged)
- 4 EIP-7778 gas accounting tests (unchanged)
- 3 EIP-8024 tests (down from 8) - still BAL-related errors

## Key Findings

### Fix #2 Applied - Net-zero storage write filtering
- Added `tx_initial_storage` to track pre-transaction storage values
- Added `capture_pre_storage()` method for first-write-wins capture
- Added `filter_net_zero_storage()` to convert net-zero writes to reads
- Modified `record_storage_write()` to deduplicate same-index writes
- Filtering happens at transaction boundaries and before build()

### Remaining Issues to Investigate
1. EIP-7708 ETH transfer logs - may need ETH transfer tracking in BAL
2. EIP-7778 gas accounting - may be separate from BAL issues
3. EIP-7928 specific tests - likely edge cases in:
   - 7702 delegation scenarios
   - Withdrawal handling
   - Precompile interactions
   - OOG (out-of-gas) scenarios

## Code Locations Identified
- `crates/common/types/block_access_list.rs:531` - BlockAccessListRecorder struct
- `crates/common/types/block_access_list.rs:574` - set_block_access_index (triggers filtering)
- `crates/common/types/block_access_list.rs:630` - filter_net_zero_storage
- `crates/common/types/block_access_list.rs:676` - capture_pre_storage
- `crates/vm/levm/src/db/gen_db.rs:676` - calls capture_pre_storage before writes
- `/data2/edgar/work/execution-specs/src/ethereum/forks/amsterdam/state_tracker.py` - reference impl

## Attempted Approaches
1. Fix #1: Revert handling - storage reads persist on revert ✓ (3 tests fixed)
2. Fix #2: Net-zero storage filtering ✓ (6 tests fixed)

## Next Steps
1. [ ] Investigate EIP-7708 ETH transfer log failures
2. [ ] Check if ETH transfers need separate BAL tracking (balance changes vs transfer logs)
3. [ ] Investigate EIP-7778 gas accounting - may be unrelated to BAL
4. [ ] Look at remaining EIP-7928 edge cases (7702, withdrawals, precompiles)

## Notes
- All remaining failures are BlockAccessListHashMismatch (plus a few ReceiptsRootMismatch)
- EIP-7708 defines ETH transfer logs - may need special handling
- The 3 remaining EIP-8024 tests (stack_underflow, exchange tests) are also BAL issues, not opcode bugs
