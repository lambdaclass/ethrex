# EF Tests Blockchain - Fix Log

**Started:** 2026-02-02
**Completed:** In Progress
**Total Iterations:** 11
**Final Status:** ❌ In Progress

---

## Summary
- Initial failures: 64
- Current failures: 17
- Total fixes applied: 13
- Tests fixed: 47 (64 → 17)
- Failed attempts: 2

---

## Fixes Applied

### Fix #1 — BAL checkpoint restore: preserve storage reads on revert
- **Iteration:** 1
- **Test(s):** `test_bal_4788_query[no_value-invalid_timestamp]` and 2 others
- **Error:** BlockAccessListHashMismatch
- **File:** `crates/common/types/block_access_list.rs:871`
- **Root Cause:** The `restore()` function was discarding storage reads made during reverted calls by replacing `self.storage_reads` with the checkpoint snapshot. Per EIP-7928, storage reads should persist even when calls revert because they are accesses, not state changes.
- **Solution:** Modified `restore()` to:
  1. Keep all current storage_reads (new reads during reverted call persist)
  2. Union with snapshot reads (restore reads that became writes)
  3. Convert reverted writes to reads (writes that revert become reads)

---

### Fix #2 — Net-zero storage write filtering
- **Iteration:** 2
- **Test(s):** `test_dupn_pc_advances_by_2` and 5 others (EIP-8024, EIP-7928)
- **Error:** BlockAccessListHashMismatch
- **Files:**
  - `crates/common/types/block_access_list.rs` (multiple locations)
  - `crates/vm/levm/src/db/gen_db.rs:676`
- **Root Cause:** Per EIP-7928, if a storage slot's value is changed but its post-transaction value equals its pre-transaction value, the slot MUST NOT be recorded as modified - it should be a read instead.
- **Solution:** Added `tx_initial_storage` tracking and `filter_net_zero_storage()` method.

---

### Fix #3 — Remove premature coinbase addition to BAL
- **Iteration:** 3
- **Test(s):** `test_bal_withdrawal_empty_block` and related withdrawal-only tests
- **Error:** BlockAccessListHashMismatch (coinbase included with no changes)
- **File:** `crates/vm/backends/levm/mod.rs:73-86`
- **Root Cause:** Code was unconditionally adding coinbase at block start. Per EIP-7928, coinbase should only appear if it has actual state changes.
- **Solution:** Removed premature coinbase addition; coinbase now only added when it receives fees.

---

### Fix #4 — Balance changes checkpoint/restore integrity
- **Iteration:** 4
- **Test(s):** `test_failed_create_with_value_no_log` and 3 others
- **Error:** BlockAccessListHashMismatch (missing balance_changes)
- **Files:** `crates/common/types/block_access_list.rs`
- **Root Cause:** `record_balance_change` was updating entries in-place, breaking checkpoint/restore.
- **Solution:** Always push new entries; `build()` takes only FINAL balance change per transaction.

---

### Fix #6 — Top-level call frame BAL checkpoint for transaction failure
- **Iteration:** 7
- **Test(s):** `test_bal_aborted_account_access`, `test_bal_aborted_storage_access`, and 9 others
- **Error:** BlockAccessListHashMismatch (inner call state changes not reverted)
- **File:** `crates/vm/levm/src/vm.rs:502-508`
- **Root Cause:** Initial call frame had no BAL checkpoint after `call_frame_backup.clear()`.
- **Solution:** Take BAL checkpoint immediately after clearing backup, before execution starts.

---

### Fix #7 — Move CREATE BAL recording after early failure checks
- **Iteration:** 8
- **Test(s):** `test_bal_create_early_failure`, `test_create_insufficient_balance_no_log`
- **Error:** BlockAccessListHashMismatch (extra contract address in BAL)
- **File:** `crates/vm/levm/src/opcode_handlers/system.rs:704-740`
- **Root Cause:** BAL recorded `new_address` BEFORE early failure checks.
- **Solution:** Moved BAL recording to AFTER early failure checks pass.

---

### Fix #8 — Only record final nonce per transaction in BAL
- **Iteration:** 8
- **Test(s):** `test_bal_7702_delegation_clear`, `test_bal_7702_delegation_create`, `test_bal_7702_delegation_update`
- **Error:** BlockAccessListHashMismatch (multiple nonce changes per tx)
- **File:** `crates/common/types/block_access_list.rs:882-887`
- **Root Cause:** `build()` was adding ALL nonce changes; EIP-7928 requires only FINAL nonce per tx.
- **Solution:** Group nonce changes by tx index and only keep final value.

---

### Fix #9 — SSTORE BAL recording before main gas check
- **Iteration:** 9
- **Test(s):** `test_bal_sstore_and_oog` (2 variants)
- **Error:** BlockAccessListHashMismatch (missing storage read)
- **File:** `crates/vm/levm/src/opcode_handlers/stack_memory_storage_flow.rs:176-237`
- **Root Cause:** SSTORE recorded to BAL AFTER the main gas check. If OOG occurred during gas charge (but after passing SSTORE_STIPEND check), the implicit SLOAD was not recorded.
- **Solution:** Moved `record_storage_slot_to_bal` call to AFTER SSTORE_STIPEND check but BEFORE main gas check. Per EIP-7928 test comment: "passes stipend, does SLOAD, fails charge_gas" should still record the storage read.
- **Diff:**
```diff
         let (current_value, storage_slot_was_cold) = self.access_storage_slot(to, key)?;
         let original_value = self.get_original_storage(to, key)?;

+        // Record storage read to BAL AFTER SSTORE_STIPEND check passes, BEFORE main gas check.
+        // Per EIP-7928: if SSTORE passes stipend but fails main gas charge, the implicit SLOAD
+        // has already happened and should be recorded.
+        self.record_storage_slot_to_bal(to, key);
+
         // Gas Refunds
         ...
         self.current_call_frame.increase_consumed_gas(gas_cost::sstore(...))?;
-
-        // Record storage read to BAL AFTER all gas checks pass per EIP-7928
-        self.record_storage_slot_to_bal(to, key);
```

---

### Fix #10 — Empty code handling: CREATE vs delegation clear
- **Iteration:** 9
- **Test(s):** `test_bal_create_transaction_empty_code`, `test_bal_7702_delegation_clear`
- **Error:** BlockAccessListHashMismatch (spurious code change OR missing code change)
- **Files:**
  - `crates/common/types/block_access_list.rs:791-812`
  - `crates/vm/levm/src/db/gen_db.rs:544-559`
- **Root Cause:** Empty code was either always recorded or never recorded. The correct behavior depends on context:
  - CREATE with empty initcode: no initial code → empty = no change (DON'T record)
  - EIP-7702 delegation clear: had delegation code → empty = actual change (DO record)
- **Solution:**
  1. Added `addresses_with_initial_code: BTreeSet<Address>` to track which addresses had non-empty code initially
  2. Added `capture_initial_code_presence()` method
  3. Modified `record_code_change()` to only skip empty code if address had NO initial code
  4. Call `capture_initial_code_presence()` in `update_account_bytecode()` before recording change
- **Diff (key parts):**
```diff
+    /// Addresses that had non-empty code at the start (before any code changes).
+    addresses_with_initial_code: BTreeSet<Address>,

+    pub fn capture_initial_code_presence(&mut self, address: Address, has_code: bool) {
+        if has_code {
+            self.addresses_with_initial_code.insert(address);
+        }
+    }

     pub fn record_code_change(&mut self, address: Address, new_code: Bytes) {
+        // If new code is empty, only record if the address had initial code
+        if new_code.is_empty() {
+            if !self.addresses_with_initial_code.contains(&address) {
+                // No initial code and setting to empty = no change, skip
+                self.touched_addresses.insert(address);
+                return;
+            }
+            // Had initial code and setting to empty = delegation clear, record it
+        }
         // ... rest of function
     }
```

---

### Fix #11 — Record EIP-7702 delegation target in BAL
- **Iteration:** 10
- **Test(s):** `test_bal_7702_delegated_storage_access`, `test_bal_7702_delegated_via_call_opcode`, `test_bal_all_transaction_types`, `test_call_to_delegated_account_with_value`, `test_transfer_to_delegated_account_emits_log`
- **Error:** BlockAccessListHashMismatch (delegation target missing from BAL)
- **Files:**
  - `crates/vm/levm/src/opcode_handlers/system.rs` (CALL, CALLCODE, DELEGATECALL, STATICCALL)
  - `crates/vm/levm/src/hooks/default_hook.rs` (set_bytecode_and_code_address)
- **Root Cause:** When calling an EIP-7702 delegated account, the delegation target (code origin) was not recorded in BAL. Per EIP-7928, the delegation target is an account access and must appear in BAL.
- **Solution:** After calling `eip7702_get_code`, if delegation exists, also record `code_address` (delegation target) to BAL. This applies to:
  1. All CALL opcodes (CALL, CALLCODE, DELEGATECALL, STATICCALL)
  2. Initial transaction call setup in `set_bytecode_and_code_address`
- **Diff (key pattern applied to all call opcodes):**
```diff
         // Record address touch for BAL (after gas checks pass per EIP-7928)
         if let Some(recorder) = self.db.bal_recorder.as_mut() {
             recorder.record_touched_address(callee);
+            // If EIP-7702 delegation, also record the delegation target (code source)
+            if is_delegation_7702 {
+                recorder.record_touched_address(code_address);
+            }
         }
```

---

### Fix #12 — Split gas charging for EIP-7702 delegation OOG handling
- **Iteration:** 10
- **Test(s):** `test_bal_call_7702_delegation_and_oog`, etc. (NOT yet fixed - in progress)
- **Error:** BlockAccessListHashMismatch (delegation target in BAL when OOG before delegation access)
- **Files:** `crates/vm/levm/src/opcode_handlers/system.rs`
- **Root Cause:** Gas was charged as `cost + eip7702_gas_consumed` atomically. When OOG occurred after target access but before delegation access, neither target nor delegation appeared in BAL (since OOG reverts to checkpoint). Per EIP-7928, target should appear but NOT delegation.
- **Solution (in progress):** Split gas charging into two stages:
  1. First charge static cost (target access, transfer, memory) → record target to BAL
  2. Then charge delegation cost separately → only record delegation if this succeeds
- **Status:** Implemented but not yet fixing tests - requires further investigation

---

### Fix #13 — EIP-7708 transfer logs for contract creation transactions
- **Iteration:** 11
- **Test(s):** `test_contract_creation_tx`, `test_selfdestruct_to_self_cross_tx_no_log`, and 1 other
- **Error:** ReceiptsRootMismatch (missing EIP-7708 transfer log)
- **File:** `crates/vm/levm/src/execution_handlers.rs:121-139`
- **Root Cause:** Contract creation transactions with value were not emitting EIP-7708 ETH transfer logs. The `transfer_value` function in `default_hook.rs` only emits logs for message call transactions (has `if !vm.is_create()?` guard). For creation transactions, value transfer happens in `handle_create_transaction` but no log was emitted there.
- **Solution:** Added EIP-7708 transfer log emission in `handle_create_transaction()`:
  1. Added imports for `create_eth_transfer_log` from utils and `Fork` from ethrex_common::types
  2. After `increase_account_balance(new_contract_address, value)`, emit transfer log when fork >= Amsterdam and value > 0
  3. Log emits from origin (sender) to new_contract_address with value
- **Diff:**
```diff
+use crate::utils::create_eth_transfer_log;
+use ethrex_common::types::{Code, Fork};

 pub fn handle_create_transaction(&mut self) -> Result<Option<ContextResult>, VMError> {
     ...
-    self.increase_account_balance(new_contract_address, self.current_call_frame.msg_value)?;
+    let value = self.current_call_frame.msg_value;
+    self.increase_account_balance(new_contract_address, value)?;
+
+    // EIP-7708: Emit transfer log for nonzero-value contract creation transactions.
+    if self.env.config.fork >= Fork::Amsterdam && !value.is_zero() {
+        let log = create_eth_transfer_log(self.env.origin, new_contract_address, value);
+        self.substate.add_log(log);
+    }

     self.increment_account_nonce(new_contract_address)?;
     Ok(None)
 }
```

---

## Failed Attempts

### Attempt #1 — Filter out empty accounts from BAL
- **Iteration:** 3
- **Test(s):** Multiple tests
- **Approach:** Modified `build()` to skip adding `AccountChanges` where `is_empty()` returns true
- **Why it failed:** Caused regression (58 → 112 failures). Some tests EXPECT empty accounts in the BAL.
- **Reverted:** Yes

### Attempt #2 — Record coinbase as touched in pay_coinbase
- **Iteration:** 5
- **Test(s):** `test_bal_coinbase_zero_tip` (was actually PASSING)
- **Approach:** Added `record_touched_address(coinbase)` regardless of fee amount
- **Why it failed:** Caused regression (54 → 58 failures). Incorrectly added coinbase to withdrawal-only blocks.
- **Reverted:** Yes

---

## Remaining Issues (17 failures)
- [ ] EIP-7708 ETH transfer logs (selfdestruct): 9 tests
  - test_finalization_selfdestruct_logs
  - test_selfdestruct_during_initcode
  - test_selfdestruct_finalization_after_priority_fee
  - test_selfdestruct_log_at_fork_transition
  - test_selfdestruct_same_tx_via_call
  - test_selfdestruct_to_different_address_same_tx
  - test_selfdestruct_to_self_same_tx
  - test_selfdestruct_to_system_address
  - test_transfer_to_special_address
- [ ] EIP-7928 BAL 7702 delegation: 8 tests
  - test_bal_7702_double_auth_reset
  - test_bal_7702_double_auth_swap
  - test_bal_call_7702_delegation_and_oog
  - test_bal_callcode_7702_delegation_and_oog
  - test_bal_delegatecall_7702_delegation_and_oog
  - test_bal_staticcall_7702_delegation_and_oog
  - test_bal_call_no_delegation_oog_after_target_access
  - test_bal_create_selfdestruct_to_self_with_call

## Progress History
- Initial: 64 failures
- After Fix #2: 58 failures (6 tests fixed)
- After Fix #4: 54 failures (4 tests fixed)
- After Fix #5: 47 failures (7 tests fixed)
- After Fix #6: 36 failures (11 tests fixed)
- After Fix #7-8: 31 failures (5 tests fixed)
- After Fix #9-10: 25 failures (6 tests fixed)
- After Fix #11: 20 failures (5 tests fixed)
- After Fix #13: 17 failures (3 tests fixed)

## Notes
- EIP-7778 and EIP-8024 tests all pass now
- Debug output available via DEBUG_BAL=1 environment variable
- Remaining EIP-7708 tests involve SELFDESTRUCT log handling
- Remaining EIP-7928 tests involve 7702 delegation OOG edge cases and double auth
