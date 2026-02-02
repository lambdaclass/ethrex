# EF Tests Blockchain - Fix Log

**Started:** 2026-02-02
**Completed:** In Progress
**Total Iterations:** 2
**Final Status:** ❌ In Progress

---

## Summary
- Initial failures: 64
- Current failures: 58
- Total fixes applied: 2
- Tests fixed: 9 (64 → 58, plus 3 from Fix #1)
- Failed attempts: 0

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

## Failed Attempts

(None yet)

---

## Remaining Issues
- [ ] BlockAccessListHashMismatch (58 instances remaining)
  - 30 EIP-7928 BAL tests
  - 21 EIP-7708 ETH transfer log tests
  - 4 EIP-7778 gas accounting tests
  - 3 EIP-8024 tests (BAL-related)
- [ ] ReceiptsRootMismatch (included in some of above)

## Notes
- Fix #2 reduced failures from 64 to 58 (6 tests fixed)
- All remaining failures are BAL-related (BlockAccessListHashMismatch)
- EIP-7708 tests may need ETH transfer log handling
- EIP-7778 tests may involve gas accounting separate from BAL
- Remaining EIP-7928 tests likely involve edge cases: 7702 delegation, withdrawals, precompiles, OOG scenarios
