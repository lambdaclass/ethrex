# EF Tests Work Log

## Current Investigation
**Iteration:** 10
**Status:** Investigating why split gas charging didn't fix OOG tests
**Working hypothesis:** OOG happens in a different place than expected, or BAL checkpoint/restore is involved

## Test Summary
- Total tests: 6158
- Passed: 6138
- Failed: 20
- Previous failures: 25
- Lowest failure count: 20 (NEW!)

## Key Findings

### Fix #11 Applied - Record EIP-7702 delegation target in BAL (SUCCESS)
- **Tests fixed:** 5 (25 → 20)
  - test_bal_7702_delegated_storage_access
  - test_bal_7702_delegated_via_call_opcode
  - test_bal_all_transaction_types
  - test_call_to_delegated_account_with_value
  - test_transfer_to_delegated_account_emits_log
- **Root cause:** Delegation target was not being recorded in BAL when calling delegated accounts
- **Solution:** Record `code_address` (delegation target) when `is_delegation_7702` is true in all CALL opcodes

### Fix #12 In Progress - Split gas charging for OOG handling
- **Tests NOT fixed:** OOG tests still failing
- **Implementation:** Split gas charging into two stages:
  1. Charge static cost (target access) → record target
  2. Charge delegation cost → record delegation only if successful
- **Problem:** Tests still failing, need to investigate why

## Current Failing Test Categories (20 remaining)

### EIP-7708 ETH Transfer Logs (11 tests)
- test_contract_creation_tx
- test_finalization_selfdestruct_logs
- test_selfdestruct_* (9 tests)
- test_transfer_to_special_address

### EIP-7928 BAL 7702 Delegation OOG (9 tests)
- test_bal_7702_double_auth_reset
- test_bal_7702_double_auth_swap
- test_bal_call_7702_delegation_and_oog
- test_bal_callcode_7702_delegation_and_oog
- test_bal_delegatecall_7702_delegation_and_oog
- test_bal_staticcall_7702_delegation_and_oog
- test_bal_call_no_delegation_oog_after_target_access
- test_bal_create_selfdestruct_to_self_with_call
- test_bal_withdrawal_and_new_contract

## Code Locations Identified
- `crates/vm/levm/src/opcode_handlers/system.rs` - CALL opcodes with split gas charging
- `crates/vm/levm/src/hooks/default_hook.rs:575-592` - Initial tx delegation handling
- `crates/vm/levm/src/utils.rs:336-365` - eip7702_get_code function

## Attempted Approaches (Successful)
1-10. Previous fixes ✓
11. Record delegation target in BAL ✓

## Attempted Approaches (In Progress)
12. Split gas charging for OOG - implemented but not fixing tests

## Next Steps
1. [ ] Debug OOG test to see what's happening
2. [ ] Check if BAL checkpoint/restore is affecting results
3. [ ] Check if OOG happens at a different point than expected

## Notes
- Progress: 64 → 20 failures (44 tests fixed total)
- EIP-7778 and EIP-8024 tests all pass
- Debug output: `DEBUG_BAL=1 cargo test ...`
