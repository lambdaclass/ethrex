# EF Tests Work Log

## Current Investigation
**Iteration:** 6
**Status:** Fix #5 applied successfully - 7 tests fixed
**Working hypothesis:** Continue analyzing remaining 47 failures

## Test Summary
- Total tests: 6158
- Passed: 6111
- Failed: 47
- Previous failures: 54
- Lowest failure count: 47 (NEW!)

## Key Findings

### Fix #5 Applied - Coinbase touched for user transactions (SUCCESS)
- Root cause: Coinbase wasn't being recorded in BAL for user transactions with zero priority fee
- System contracts have `gas_price = 0`, user transactions have `gas_price >= base_fee > 0`
- Solution: In `pay_coinbase`, check `!vm.env.gas_price.is_zero()` to identify user transactions
- Only record coinbase as touched for user transactions, not system contracts
- **Result:** 7 tests fixed (54 → 47), no regressions

### Tests fixed by Fix #5:
1. test_bal_coinbase_zero_tip
2. test_bal_7702_invalid_chain_id_authorization
3. test_bal_7702_invalid_nonce_authorization
4. test_bal_4788_empty_block
5. test_bal_4788_query
6. test_bal_7002_partial_sweep
7. test_bal_withdrawal_to_7702_delegation

## Current Failing Test Categories (47 remaining)
1. **EIP-7708 (ETH Transfer Logs)** - 18 tests - likely need log generation improvements
2. **EIP-7778 (Block Gas Accounting)** - 4 tests - gas accounting issues
3. **EIP-7928 (BAL)** - 22 tests - various BAL edge cases (7702, withdrawals, OOG)
4. **EIP-8024 (DUPN/SWAPN/EXCHANGE)** - 3 tests - stack underflow/immediate handling

## Code Locations Identified
- `crates/vm/levm/src/hooks/default_hook.rs:238-260` - pay_coinbase function (FIXED)
- System contracts have gas_price = 0, user txs have gas_price >= base_fee > 0

## Attempted Approaches (Successful)
1. Fix #1: Revert handling - storage reads persist on revert ✓ (3 tests fixed)
2. Fix #2: Net-zero storage filtering ✓ (6 tests fixed)
3. Fix #3: Remove premature coinbase addition ✓ (no regression)
4. Fix #4: Balance checkpoint/restore integrity ✓ (4 tests fixed)
5. Fix #5: Coinbase touched for user transactions ✓ (7 tests fixed)

## Failed Attempts
1. Filter out empty accounts from BAL ✗ (caused regression)
2. Unconditionally record coinbase in pay_coinbase ✗ (affected system contracts)

## Next Steps
1. [x] Fix coinbase recording for user transactions with zero tip
2. [ ] Analyze EIP-7708 ETH transfer log tests
3. [ ] Investigate EIP-7778 gas accounting
4. [ ] Look at remaining EIP-7928 tests (7702 delegation, OOG scenarios)

## Notes
- Progress: 64 → 58 → 54 → 47 failures (17 tests fixed total)
- Key insight: gas_price distinguishes user txs from system contracts
- Remaining issues likely need separate investigation per category
