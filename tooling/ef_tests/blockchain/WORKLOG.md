# EF Tests Work Log

## Current Investigation
**Iteration:** 8
**Status:** Investigating CREATE early failure BAL issue
**Working hypothesis:** BAL records new_address BEFORE early failure checks in generic_create

## Test Summary
- Total tests: 6158
- Passed: 6122
- Failed: 36
- Previous failures: 47
- Lowest failure count: 36 (NEW!)

## Key Findings

### Fix #6 Applied - Top-level BAL checkpoint (SUCCESS)
- Root cause: Initial call frame had no BAL checkpoint after call_frame_backup.clear()
- When top-level execution failed (INVALID, REVERT), inner call state changes weren't reverted from BAL
- Solution: Take BAL checkpoint immediately after clearing backup, before execution
- Location: `crates/vm/levm/src/vm.rs:502-508`
- **Result:** 11 tests fixed (47 → 36), no regressions

### Tests fixed by Fix #6:
- test_bal_aborted_account_access (all variants)
- test_bal_aborted_storage_access (all variants)
- test_bal_7002_request_invalid
- test_bal_inner_call_succeeds_outer_reverts_no_log
- All EIP-7778 gas accounting tests
- All EIP-8024 DUPN/SWAPN/EXCHANGE tests

## Current Failing Test Categories (36 remaining)
1. **EIP-7708 (ETH Transfer Logs)** - 17 tests - likely need ETH transfer log handling
2. **EIP-7928 (BAL 7702 delegation)** - 19 tests - mostly 7702 delegation edge cases

## Code Locations Identified
- `crates/vm/levm/src/vm.rs:502-508` - Top-level BAL checkpoint (FIXED)
- `crates/vm/levm/src/hooks/default_hook.rs:238-260` - pay_coinbase function (FIXED in iteration 6)
- 7702 delegation handling in BAL needs investigation

## Attempted Approaches (Successful)
1. Fix #1: Revert handling - storage reads persist on revert ✓ (3 tests fixed)
2. Fix #2: Net-zero storage filtering ✓ (6 tests fixed)
3. Fix #3: Remove premature coinbase addition ✓ (no regression)
4. Fix #4: Balance checkpoint/restore integrity ✓ (4 tests fixed)
5. Fix #5: Coinbase touched for user transactions ✓ (7 tests fixed)
6. Fix #6: Top-level BAL checkpoint ✓ (11 tests fixed)

## Failed Attempts
1. Filter out empty accounts from BAL ✗ (caused regression)
2. Unconditionally record coinbase in pay_coinbase ✗ (affected system contracts)

## Next Steps
1. [x] Fix top-level BAL checkpoint for transaction failure
2. [ ] Analyze EIP-7708 ETH transfer log tests
3. [ ] Investigate EIP-7928 7702 delegation tests (19 tests)
4. [ ] Look at create/selfdestruct scenarios

## Notes
- Progress: 64 → 58 → 54 → 47 → 36 failures (28 tests fixed total)
- EIP-7778 and EIP-8024 tests now all pass
- Remaining issues are EIP-7708 and EIP-7928 7702 delegation related
- Key insight: BAL checkpoints needed at multiple levels (nested calls AND top-level)
