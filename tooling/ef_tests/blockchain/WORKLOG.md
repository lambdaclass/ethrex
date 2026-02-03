# EF Tests Work Log

## Current Investigation
**Iteration:** 9
**Status:** Analyzing remaining failures
**Working hypothesis:** EIP-7708 tests need ETH transfer log handling; EIP-7928 tests have 7702 delegation edge cases

## Test Summary
- Total tests: 6158
- Passed: 6133
- Failed: 25
- Previous failures: 31
- Lowest failure count: 25 (NEW!)

## Key Findings

### Fix #9 Applied - SSTORE BAL recording timing (SUCCESS)
- Root cause: BAL recording happened AFTER main gas check in op_sstore
- When OOG at main gas check (but after passing SSTORE_STIPEND), implicit SLOAD wasn't recorded
- Solution: Move `record_storage_slot_to_bal` to AFTER stipend check but BEFORE main gas check
- Location: `crates/vm/levm/src/opcode_handlers/stack_memory_storage_flow.rs:176-237`
- **Result:** 1 test fixed (test_bal_sstore_and_oog), no regressions

### Fix #10 Applied - Empty code handling (SUCCESS)
- Root cause: Empty code was always skipped, but should be recorded for delegation clear
- CREATE empty: no initial code → empty = no change (skip)
- Delegation clear: had code → empty = change (record)
- Solution: Track `addresses_with_initial_code` and only skip if no initial code
- Locations:
  - `crates/common/types/block_access_list.rs:791-812`
  - `crates/vm/levm/src/db/gen_db.rs:544-559`
- **Result:** 5 tests fixed (test_bal_create_transaction_empty_code + others), no regressions

## Current Failing Test Categories (25 remaining)

### EIP-7708 ETH Transfer Logs (13 tests)
These tests likely need actual ETH transfer event log handling, which may be a separate feature from BAL:
- test_call_to_delegated_account_with_value
- test_contract_creation_tx
- test_finalization_selfdestruct_logs
- test_selfdestruct_during_initcode
- test_selfdestruct_finalization_after_priority_fee
- test_selfdestruct_log_at_fork_transition
- test_selfdestruct_same_tx_via_call
- test_selfdestruct_to_different_address_same_tx
- test_selfdestruct_to_self_cross_tx_no_log
- test_selfdestruct_to_self_same_tx
- test_selfdestruct_to_system_address
- test_transfer_to_delegated_account_emits_log
- test_transfer_to_special_address

### EIP-7928 BAL 7702 Delegation (12 tests)
Mostly 7702 delegation edge cases:
- test_bal_7702_delegated_storage_access
- test_bal_7702_delegated_via_call_opcode
- test_bal_7702_double_auth_reset
- test_bal_7702_double_auth_swap
- test_bal_all_transaction_types
- test_bal_call_7702_delegation_and_oog
- test_bal_callcode_7702_delegation_and_oog
- test_bal_call_no_delegation_oog_after_target_access
- test_bal_create_selfdestruct_to_self_with_call
- test_bal_delegatecall_7702_delegation_and_oog
- test_bal_staticcall_7702_delegation_and_oog
- test_bal_withdrawal_and_new_contract

## Code Locations Identified
- `crates/vm/levm/src/vm.rs:502-508` - Top-level BAL checkpoint (FIXED)
- `crates/vm/levm/src/hooks/default_hook.rs:238-260` - pay_coinbase function (FIXED)
- `crates/vm/levm/src/opcode_handlers/stack_memory_storage_flow.rs` - SSTORE BAL recording (FIXED)
- `crates/vm/levm/src/db/gen_db.rs:544-559` - update_account_bytecode (FIXED)
- `crates/vm/levm/src/utils.rs:370-460` - EIP-7702 authorization processing
- `crates/common/types/block_access_list.rs` - BAL recorder

## Attempted Approaches (Successful)
1. Fix #1: Revert handling - storage reads persist on revert ✓
2. Fix #2: Net-zero storage filtering ✓
3. Fix #3: Remove premature coinbase addition ✓
4. Fix #4: Balance checkpoint/restore integrity ✓
5. Fix #5: Coinbase touched for user transactions ✓
6. Fix #6: Top-level BAL checkpoint ✓
7. Fix #7: CREATE BAL recording after early failure ✓
8. Fix #8: Only final nonce per transaction ✓
9. Fix #9: SSTORE BAL recording before main gas check ✓
10. Fix #10: Empty code handling for CREATE vs delegation clear ✓

## Failed Attempts
1. Filter out empty accounts from BAL ✗ (caused regression)
2. Unconditionally record coinbase in pay_coinbase ✗ (affected system contracts)

## Next Steps
1. [x] Fix SSTORE BAL recording timing
2. [x] Fix empty code handling for CREATE vs delegation clear
3. [ ] Analyze EIP-7708 ETH transfer log tests (may need separate implementation)
4. [ ] Investigate remaining EIP-7928 7702 delegation tests
5. [ ] Look at test_bal_withdrawal_and_new_contract

## Notes
- Progress: 64 → 31 → 25 failures (39 tests fixed total)
- EIP-7778 and EIP-8024 tests all pass
- Key insight: Empty code changes need context-aware handling (CREATE vs delegation clear)
- EIP-7708 tests may need separate ETH transfer log feature implementation
- Debug output: `DEBUG_BAL=1 cargo test ...`
