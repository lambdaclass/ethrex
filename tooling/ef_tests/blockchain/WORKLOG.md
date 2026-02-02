# EF Tests Work Log

## Current Investigation
**Iteration:** 4
**Status:** Fix #4 applied - balance checkpoint/restore integrity
**Working hypothesis:** Need to investigate remaining failures for patterns

## Test Summary
- Total tests: 6158
- Passed: 6104
- Failed: 54
- Previous failures: 58
- Lowest failure count: 54

## Key Findings

### Fix #4 Applied - Balance checkpoint/restore integrity
- The `record_balance_change` function was updating balance entries in-place
- This broke checkpoint/restore because checkpoints capture LENGTH, not VALUES
- Fixed by always pushing new entries, then taking only the final balance in build()
- This ensures truncation to checkpoint length preserves the correct values
- Tests fixed: 4 (58 → 54)

### Debugging Approach
- Added debug output to validation.rs (enabled with DEBUG_BAL=1)
- Added debug output to build() and restore() (enabled with DEBUG_BAL_BUILD=1)
- Traced through balance_changes to find the root cause
- Key insight: checkpoint was taken with len=1, but the value at index 0 was being updated in-place

## Code Locations Identified
- `crates/common/types/block_access_list.rs:738` - record_balance_change (now always pushes)
- `crates/common/types/block_access_list.rs:874-895` - build() balance filtering (now takes only final)
- `crates/common/types/block_access_list.rs:957` - restore() function
- `crates/common/validation.rs:173` - BAL hash validation (has debug output)

## Attempted Approaches
1. Fix #1: Revert handling - storage reads persist on revert ✓ (3 tests fixed)
2. Fix #2: Net-zero storage filtering ✓ (6 tests fixed)
3. Fix #3: Remove premature coinbase addition ✓ (no regression)
4. Fix #4: Balance checkpoint/restore integrity ✓ (4 tests fixed)
5. Failed: Filter out empty accounts from BAL ✗ (caused regression)

## Next Steps
1. [ ] Analyze remaining 54 failures for common patterns
2. [ ] Check EIP-7708 ETH transfer log tests specifically
3. [ ] Check EIP-7778 gas accounting tests
4. [ ] Investigate EIP-7702 delegation edge cases

## Notes
- All remaining failures are BAL-related (BlockAccessListHashMismatch)
- Empty accounts (no changes) ARE expected in some tests (failed CREATE targets)
- The same pattern (checkpoint/restore) may apply to nonce_changes and code_changes
- Progress: 64 → 58 → 54 failures (10 tests fixed total)
