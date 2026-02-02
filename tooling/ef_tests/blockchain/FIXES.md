# EF Tests Blockchain - Fix Log

**Started:** 2026-02-02
**Completed:** In Progress
**Total Iterations:** 1
**Final Status:** ❌ In Progress

---

## Summary
- Initial failures: 64 (323 BlockAccessListHashMismatch, 3 ReceiptsRootMismatch)
- Current failures: 64 (320 BlockAccessListHashMismatch, 3 ReceiptsRootMismatch)
- Total fixes applied: 1
- Tests fixed: 3 (BlockAccessListHashMismatch reduced from 323 to 320)
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
-        // This correctly handles reverted writes: if a slot was originally a read,
-        // then written (removing it from reads), and then reverted, it will be
-        // restored as a read.
-        self.storage_reads = checkpoint.storage_reads_snapshot;
-
-        // Restore storage_writes: truncate change vectors
+    pub fn restore(&mut self, checkpoint: BlockAccessListCheckpoint) {
+        // Step 1: Collect slots that were written after checkpoint (to convert to reads)
+        let mut reverted_write_slots: BTreeMap<Address, BTreeSet<U256>> = BTreeMap::new();
+        for (addr, slots) in &self.storage_writes {
+            let checkpoint_lens = checkpoint.storage_writes_len.get(addr);
+            for (slot, changes) in slots {
+                let checkpoint_len = checkpoint_lens.and_then(|m| m.get(slot)).copied().unwrap_or(0);
+                if changes.len() > checkpoint_len {
+                    reverted_write_slots.entry(*addr).or_default().insert(*slot);
+                }
+            }
+        }
+
+        // Step 2: Keep current reads (new reads during reverted call persist)
+        // Step 3: Restore reads that became writes (union with snapshot)
+        for (addr, snapshot_reads) in checkpoint.storage_reads_snapshot {
+            let current_reads = self.storage_reads.entry(addr).or_default();
+            for slot in snapshot_reads {
+                current_reads.insert(slot);
+            }
+        }
+
+        // Step 4: Convert reverted writes to reads
+        for (addr, slots) in reverted_write_slots {
+            let current_reads = self.storage_reads.entry(addr).or_default();
+            for slot in slots {
+                current_reads.insert(slot);
+            }
+        }
+
+        // Step 5: Truncate storage_writes (keep only writes from before checkpoint)
```

---

## Failed Attempts

(None yet)

---

## Remaining Issues
- [ ] BlockAccessListHashMismatch (320 instances) - still investigating
- [ ] ReceiptsRootMismatch (3 instances)

## Notes
- The fix reduced BlockAccessListHashMismatch from 323 to 320 (3 tests fixed)
- Remaining failures may involve other BAL recording issues (e.g., value transfers, delegated accounts)
- Need to investigate EIP-7708 ETH transfer logs interaction with BAL
