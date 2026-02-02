# EF Tests Blockchain - Fix Log

**Started:** 2026-02-02
**Completed:** In Progress
**Total Iterations:** 4
**Final Status:** ❌ In Progress

---

## Summary
- Initial failures: 64
- Current failures: 54
- Total fixes applied: 4
- Tests fixed: 13+ (64 → 54)
- Failed attempts: 1

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
- **Diff:**
```diff
-    pub fn restore(&mut self, checkpoint: BlockAccessListCheckpoint) {
-        // Restore storage_reads from snapshot.
-        self.storage_reads = checkpoint.storage_reads_snapshot;
+    pub fn restore(&mut self, checkpoint: BlockAccessListCheckpoint) {
+        // Step 1: Collect slots that were written after checkpoint (to convert to reads)
+        let mut reverted_write_slots: BTreeMap<Address, BTreeSet<U256>> = BTreeMap::new();
+        // ... (preserve reads, union with snapshot, convert reverted writes to reads)
```

---

### Fix #2 — Net-zero storage write filtering
- **Iteration:** 2
- **Test(s):** `test_dupn_pc_advances_by_2` and 5 others (EIP-8024, EIP-7928)
- **Error:** BlockAccessListHashMismatch
- **Files:**
  - `crates/common/types/block_access_list.rs` (multiple locations)
  - `crates/vm/levm/src/db/gen_db.rs:676`
- **Root Cause:** Per EIP-7928, if a storage slot's value is changed but its post-transaction value equals its pre-transaction value, the slot MUST NOT be recorded as modified - it should be a read instead. Our code was:
  1. Recording multiple writes per slot per transaction instead of just the final value
  2. Not filtering net-zero writes (writes that return to original value)
- **Solution:**
  1. Added `tx_initial_storage: BTreeMap<(Address, U256), U256>` to track pre-transaction values
  2. Added `capture_pre_storage()` method with first-write-wins semantics
  3. Modified `record_storage_write()` to update existing entry if same block_access_index
  4. Added `filter_net_zero_storage()` to convert net-zero writes to reads at transaction boundaries
  5. Call filtering in `set_block_access_index()` before switching transactions and in `build()` for the final transaction
- **Diff (key parts):**
```diff
+    /// Per-transaction initial storage values for net-zero filtering.
+    tx_initial_storage: BTreeMap<(Address, U256), U256>,

+    pub fn capture_pre_storage(&mut self, address: Address, slot: U256, value: U256) {
+        self.tx_initial_storage.entry((address, slot)).or_insert(value);
+    }

+    fn filter_net_zero_storage(&mut self) {
+        // Compare final values against pre-tx values
+        // Convert net-zero writes to reads
+    }

 // In gen_db.rs:
+            // Capture pre-storage value for net-zero filtering
+            recorder.capture_pre_storage(address, slot, current_value);
```

---

### Fix #3 — Remove premature coinbase addition to BAL
- **Iteration:** 3
- **Test(s):** `test_bal_withdrawal_empty_block` and related withdrawal-only tests
- **Error:** BlockAccessListHashMismatch (coinbase included with no changes)
- **File:** `crates/vm/backends/levm/mod.rs:73-86`
- **Root Cause:** The code was unconditionally adding the coinbase to `touched_addresses` at block start when there were transactions OR withdrawals. However, per EIP-7928, the coinbase should only appear in the BAL if it has actual state changes (receives priority fees). In blocks with only withdrawals and no transactions, the coinbase receives no fees and should not be in the BAL.
- **Solution:** Removed the premature coinbase addition. The coinbase is now only added to the BAL when it actually receives fees via the `pay_coinbase` function during transaction finalization.
- **Diff:**
```diff
-        // Record coinbase if block has txs or withdrawals (per EIP-7928)
-        if record_bal {
-            let has_txs_or_withdrawals = !block.body.transactions.is_empty()
-                || block
-                    .body
-                    .withdrawals
-                    .as_ref()
-                    .is_some_and(|w| !w.is_empty());
-            if has_txs_or_withdrawals && let Some(recorder) = db.bal_recorder_mut() {
-                recorder.record_touched_address(block.header.coinbase);
-            }
-        }
+        // Note: Coinbase is NOT pre-recorded here. Per EIP-7928, the coinbase should only
+        // appear in the BAL if it has actual state changes (receives priority fees).
+        // The increase_account_balance call in pay_coinbase will record the balance change.
```

---

### Fix #4 — Balance changes checkpoint/restore integrity
- **Iteration:** 4
- **Test(s):** `test_failed_create_with_value_no_log` and 3 others
- **Error:** BlockAccessListHashMismatch (missing balance_changes)
- **Files:**
  - `crates/common/types/block_access_list.rs` (record_balance_change, build)
- **Root Cause:** The `record_balance_change` function was updating balance entries in-place when the `block_access_index` was the same. This caused problems with checkpoint/restore:
  1. Checkpoint captures LENGTH, not VALUES
  2. If balance was updated in-place after checkpoint, the updated value would be preserved on restore
  3. Example: balance_changes = [(1, 2)] -> checkpoint (len=1) -> update to [(1, 1)] -> restore truncates to len=1 -> result is [(1, 1)] instead of [(1, 2)]
- **Solution:**
  1. Modified `record_balance_change` to ALWAYS push new entries instead of updating in-place
  2. Modified `build()` to take only the FINAL balance change per transaction (using `tx_changes.last()`)
  3. This allows checkpoint/restore to correctly truncate to the checkpoint state
- **Diff (key parts):**
```diff
 pub fn record_balance_change(&mut self, address: Address, post_balance: U256) {
     // ...
     let changes = self.balance_changes.entry(address).or_default();
-    // Update the last entry if it's for the same block_access_index
-    if let Some(last) = changes.last_mut() {
-        if last.0 == self.current_index {
-            last.1 = post_balance;
-        } else {
-            changes.push((self.current_index, post_balance));
-        }
-    } else {
-        changes.push((self.current_index, post_balance));
-    }
+    // Always push new entries to support checkpoint/restore.
+    // The last entry for each transaction will be used in build().
+    changes.push((self.current_index, post_balance));
 }

 // In build():
-    // Only include changes if NOT a round-trip within this transaction
-    if !is_round_trip {
-        for post_balance in tx_changes {
-            account_changes.add_balance_change(BalanceChange::new(*index, *post_balance));
-        }
-    }
+    // Only include the FINAL balance change if NOT a round-trip
+    if !is_round_trip {
+        if let Some(final_balance) = final_for_tx {
+            account_changes.add_balance_change(BalanceChange::new(*index, final_balance));
+        }
+    }
```

---

## Failed Attempts

### Attempt #1 — Filter out empty accounts from BAL
- **Iteration:** 3
- **Test(s):** Multiple tests
- **Approach:** Modified `build()` to skip adding `AccountChanges` where `is_empty()` returns true
- **Why it failed:** Caused regression (58 → 112 failures). Some tests EXPECT empty accounts in the BAL (e.g., failed CREATE target addresses that were touched but have no state changes after revert)
- **Reverted:** Yes

---

## Remaining Issues
- [ ] BlockAccessListHashMismatch (54 instances remaining)
  - EIP-7928 BAL tests (various edge cases)
  - EIP-7708 ETH transfer log tests
  - EIP-7778 gas accounting tests
  - EIP-8024 tests (BAL-related)
- [ ] ReceiptsRootMismatch (included in some of above)

## Notes
- Fix #2 reduced failures from 64 to 58 (6 tests fixed)
- Fix #3 prevents empty coinbase in withdrawal-only blocks (no regression)
- Fix #4 fixes balance checkpoint/restore integrity (58 → 54, 4 tests fixed)
- All remaining failures are BAL-related (BlockAccessListHashMismatch)
- Empty accounts (no changes) ARE expected in some tests (failed CREATE targets)
- EIP-7708 tests may need ETH transfer log handling
- EIP-7778 tests may involve gas accounting separate from BAL
- Remaining EIP-7928 tests likely involve edge cases: 7702 delegation, withdrawals, precompiles, OOG scenarios
- Debug output added to validation.rs (enable with DEBUG_BAL=1)
