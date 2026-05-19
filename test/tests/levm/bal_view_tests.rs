//! BAL lazy-cursor regression tests.
//!
//! All three tests exercise the helper functions directly (unit level) because
//! `seed_one_storage_slot_from_bal` and `seed_one_address_info_from_bal` are
//! `#[cfg(all(feature = "rayon", not(feature = "eip-8025")))]`-gated; reaching
//! `execute_block_parallel` from the test crate would require enabling that
//! feature pair and wiring up a full Amsterdam chain config, block, and signed
//! transactions. The helper-level tests cover the same off-by-one boundary and
//! storage-injection invariants that the lazy cursor relies on.

#[cfg(all(feature = "rayon", not(feature = "eip-8025")))]
mod inner {
    use ethereum_types::H160;
    use ethrex_common::{
        Address, U256,
        types::block_access_list::{
            AccountChanges, BalAddressIndex, BalanceChange, BlockAccessList, SlotChange,
            StorageChange,
        },
        utils::u256_to_h256,
    };
    use ethrex_levm::db::gen_db::{
        GeneralizedDatabase, LazyBalCursor, seed_one_address_info_from_bal,
        seed_one_storage_slot_from_bal,
    };
    use std::sync::Arc;

    use crate::levm::test_db::TestDatabase;

    const CONTRACT: Address = H160([
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xC0, 0xDE,
    ]);
    const SLOT: U256 = U256([0x10, 0, 0, 0]);
    const V0: U256 = U256([0xAA, 0, 0, 0]);
    const V1: U256 = U256([0xBB, 0, 0, 0]);

    /// Build a minimal BAL with one account (`CONTRACT`) that has a single
    /// storage slot written at `block_access_index = 1` (= tx 0's post-value).
    fn bal_single_slot_write_at_1() -> BlockAccessList {
        let slot_change = SlotChange::with_changes(SLOT, vec![StorageChange::new(1, V0)]);
        let acct = AccountChanges::new(CONTRACT).with_storage_changes(vec![slot_change]);
        BlockAccessList::from_accounts(vec![acct])
    }

    /// T1: `tx1_sees_tx0_write`
    ///
    /// Unit-level test of the off-by-one boundary: `seed_one_storage_slot_from_bal`
    /// with `max_idx = 1` must return `V0` (the write made by tx 0, whose
    /// BAL index is 1). This is the value tx 1 observes as the pre-state of the
    /// slot, mirroring what `LazyBalCursor` surfaces on cache-miss.
    #[test]
    fn tx1_sees_tx0_write() {
        let bal = bal_single_slot_write_at_1();
        let key = u256_to_h256(SLOT);

        // tx 1 has bal_index = 2, so max_idx = 1 (same as seed_db_from_bal semantics).
        let result = seed_one_storage_slot_from_bal(&bal, 0, key, 1);

        assert_eq!(
            result,
            Some(V0),
            "tx 1 should see tx 0's write (V0) as the slot pre-state"
        );
    }

    /// T2: `load_account_does_not_inject_storage`
    ///
    /// `seed_one_address_info_from_bal` must not populate `account.storage`.
    /// It handles balance/nonce/code only; storage is seeded separately (or
    /// lazily via the cursor). An extraneous storage injection would corrupt
    /// the initial-storage baseline used for net-zero filtering.
    #[test]
    fn load_account_does_not_inject_storage() {
        // Build a BAL entry with a balance change AND a storage write. The
        // account info seed must ignore the storage write.
        let slot_change = SlotChange::with_changes(SLOT, vec![StorageChange::new(1, V0)]);
        let acct = AccountChanges::new(CONTRACT)
            .with_balance_changes(vec![BalanceChange::new(1, U256::from(1_000u64))])
            .with_storage_changes(vec![slot_change]);
        let bal = BlockAccessList::from_accounts(vec![acct]);

        let db_backend = Arc::new(TestDatabase::new());
        let mut db = GeneralizedDatabase::new(db_backend);

        let applied = seed_one_address_info_from_bal(&mut db, &bal, 0, 1)
            .expect("seed_one_address_info_from_bal should not fail");

        assert!(applied, "balance change should have been applied");

        // Storage must not be injected by the info seed.
        let acct_state = db
            .current_accounts_state
            .get(&CONTRACT)
            .expect("account should be in cache after info seed");
        assert!(
            acct_state.storage.is_empty(),
            "seed_one_address_info_from_bal must not populate account.storage"
        );
    }

    /// T3: `sstore_sees_prior_write`
    ///
    /// Verifies that when the same slot has two `slot_changes` entries (written
    /// at indices 1 and 2), the cursor boundary semantics are correct:
    /// - `max_idx = 1` returns `V0` (only tx 0's write is visible)
    /// - `max_idx = 2` returns `V1` (tx 1's write is also visible)
    /// This mirrors the pre-write value tx 1 and tx 2 would observe respectively.
    #[test]
    fn sstore_sees_prior_write() {
        let slot_change = SlotChange::with_changes(
            SLOT,
            vec![
                StorageChange::new(1, V0), // tx 0 writes V0
                StorageChange::new(2, V1), // tx 1 writes V1
            ],
        );
        let acct = AccountChanges::new(CONTRACT).with_storage_changes(vec![slot_change]);
        let bal = BlockAccessList::from_accounts(vec![acct]);
        let key = u256_to_h256(SLOT);

        // tx 1 cursor (bal_index=2, max_idx=1): should see V0 from tx 0.
        let at_1 = seed_one_storage_slot_from_bal(&bal, 0, key, 1);
        assert_eq!(at_1, Some(V0), "at max_idx=1 should see V0 (tx 0's write)");

        // tx 2 cursor (bal_index=3, max_idx=2): should see V1 from tx 1.
        let at_2 = seed_one_storage_slot_from_bal(&bal, 0, key, 2);
        assert_eq!(at_2, Some(V1), "at max_idx=2 should see V1 (tx 1's write)");
    }

    /// T4b: `lazy_bal_takes_precedence_over_shared_base`
    ///
    /// Regression test for the consensus issue flagged in PR #6669 review: when an
    /// address is present in BOTH `shared_base` (pre-block snapshot of system-touched
    /// addresses) AND the BAL prefix (e.g. a system-contract predeploy mutated by a
    /// prior tx in the same block), `load_account` must surface the BAL-overlaid value,
    /// not the stale `shared_base` value.
    ///
    /// Setup mirrors `execute_block_parallel`:
    /// - `shared_base` holds `CONTRACT` with `balance = 0` (pre-block state).
    /// - BAL has a balance change for `CONTRACT` at `block_access_index = 1`
    ///   (= post-tx-0 state).
    /// - Per-tx DB for tx 1 is constructed with both `shared_base` and a
    ///   `LazyBalCursor` at `bal_index = 2` (so `max_idx = 1`).
    ///
    /// Expected: `load_account(CONTRACT)` returns the BAL post-balance (42_000),
    /// not the `shared_base` pre-balance (0). Before the fix, `shared_base` short-
    /// circuited the lazy hook and tx 1 saw the stale value.
    #[test]
    fn lazy_bal_takes_precedence_over_shared_base() {
        use ethrex_common::types::AccountInfo;
        use ethrex_levm::account::LevmAccount;
        use rustc_hash::FxHashMap;

        let post_balance = U256::from(42_000u64);

        let mut shared = FxHashMap::default();
        shared.insert(
            CONTRACT,
            LevmAccount {
                info: AccountInfo::default(),
                ..Default::default()
            },
        );
        let shared_base = Arc::new(shared);

        let acct = AccountChanges::new(CONTRACT)
            .with_balance_changes(vec![BalanceChange::new(1, post_balance)]);
        let bal = BlockAccessList::from_accounts(vec![acct]);
        let arc_bal = Arc::new(bal);
        let arc_idx = Arc::new(arc_bal.build_validation_index());

        let mut db =
            GeneralizedDatabase::new_with_shared_base(Arc::new(TestDatabase::new()), shared_base);
        db.lazy_bal = Some(LazyBalCursor {
            bal: arc_bal,
            bal_index: 2,
            index: arc_idx,
        });

        let acc = db.get_account(CONTRACT).expect("load_account must succeed");
        assert_eq!(
            acc.info.balance, post_balance,
            "lazy_bal overlay must take precedence over shared_base; saw stale shared_base value"
        );
    }

    /// T4: `lazy_load_account_partial_coverage_does_not_recurse`
    ///
    /// A BAL with a partial-coverage account (balance change only, no nonce,
    /// no code, no storage) triggers the `else` branch in
    /// `seed_one_address_info_from_bal`, which calls `db.get_account(addr)` to
    /// load the base state from the store before overlaying. Without the `.take()`
    /// fix in `load_account`, that inner `get_account` call would re-enter the
    /// lazy-BAL hook and recurse infinitely (stack overflow). This test verifies
    /// the fix: `load_account` on a per-tx DB with `lazy_bal = Some(...)` must
    /// complete successfully and apply the balance overlay.
    #[test]
    fn lazy_load_account_partial_coverage_does_not_recurse() {
        // Build a BAL with balance-only change at index 1 for CONTRACT.
        // No nonce, no code, no storage — this is the partial-coverage case.
        let balance_val = U256::from(42_000u64);
        let acct = AccountChanges::new(CONTRACT)
            .with_balance_changes(vec![BalanceChange::new(1, balance_val)]);
        let bal = BlockAccessList::from_accounts(vec![acct]);
        let arc_bal = Arc::new(bal);
        let index: BalAddressIndex = arc_bal.build_validation_index();
        let arc_idx = Arc::new(index);

        let mut db = GeneralizedDatabase::new(Arc::new(TestDatabase::new()));
        db.lazy_bal = Some(LazyBalCursor {
            bal: arc_bal,
            bal_index: 2, // tx 1's cursor: effective max_idx = 1
            index: arc_idx,
        });

        // This must NOT stack-overflow. The .take() fix in load_account ensures
        // the inner db.get_account call inside seed_one_address_info_from_bal
        // sees lazy_bal = None and falls straight to the store.
        let acc = db
            .get_account(CONTRACT)
            .expect("partial-coverage load_account must not recurse");
        assert_eq!(
            acc.info.balance, balance_val,
            "balance overlay from BAL should have been applied"
        );
    }
}
