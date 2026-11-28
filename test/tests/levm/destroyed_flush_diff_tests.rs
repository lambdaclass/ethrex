//! The streaming executor's per-flush diff (`get_state_transitions_tx`) for accounts
//! destroyed within the block.
//!
//! Every few transactions the executor drains `current_accounts_state` and folds the
//! drained accounts into `initial_accounts_state`, the baseline the next flush diffs
//! against. A destroyed-and-recreated account is folded wholesale, keeping its destroyed
//! status so reads of slots not written since keep returning zero rather than the stale
//! storage still in the store. The diff must then tell two things apart:
//!
//! - destroyed within this batch (the baseline is the pre-destruction state): every
//!   slot's old value is zero and the trie storage is removed;
//! - the baseline is already post-destruction: the slots it holds are the committed
//!   values, and the diff is against them, so a write of zero to one of them is a
//!   deletion that must reach the merkleizer.
//!
//! Also, a `Destroyed` (never re-modified) account may hold slots: a reverted
//! re-creation puts back the zeros its writes replaced. Those have no old value in the
//! baseline, and the flush must not fail on them.

use std::sync::Arc;

use ethrex_common::{
    Address, H256, U256,
    constants::EMPTY_KECCAK_HASH,
    seeded_hash::SeededHashMap,
    types::{AccountInfo, AccountUpdate},
};
use ethrex_levm::{
    account::{AccountStatus, LevmAccount},
    db::gen_db::GeneralizedDatabase,
};

use super::test_db::TestDatabase;

const TARGET: u64 = 0xDEAD;

fn target() -> Address {
    Address::from_low_u64_be(TARGET)
}

fn slot(n: u64) -> H256 {
    H256::from_low_u64_be(n)
}

fn account(
    status: AccountStatus,
    nonce: u64,
    balance: u64,
    has_storage: bool,
    exists: bool,
    slots: &[(u64, u64)],
) -> LevmAccount {
    let mut storage = SeededHashMap::default();
    for (key, value) in slots {
        storage.insert(slot(*key), U256::from(*value));
    }
    LevmAccount {
        info: AccountInfo {
            code_hash: *EMPTY_KECCAK_HASH,
            balance: U256::from(balance),
            nonce,
        },
        storage,
        has_storage,
        status,
        exists,
    }
}

/// A flush whose diff baseline is `initial` and whose drained state is `current`.
fn flush(initial: LevmAccount, current: LevmAccount) -> Result<Vec<AccountUpdate>, String> {
    let mut db = GeneralizedDatabase::new(Arc::new(TestDatabase::new()));
    db.initial_accounts_state.insert(target(), initial);
    db.current_accounts_state.insert(target(), current);
    db.get_state_transitions_tx().map_err(|e| e.to_string())
}

fn added(update: &AccountUpdate) -> Vec<(u64, u64)> {
    let mut added: Vec<(u64, u64)> = update
        .added_storage
        .iter()
        .map(|(key, value)| (key.to_low_u64_be(), value.low_u64()))
        .collect();
    added.sort_unstable();
    added
}

/// A slot committed by the batch that re-created the account is written back to zero in
/// a later batch: the deletion is emitted, diffing against the committed value and not
/// against zero as if the account had just been destroyed.
#[test]
fn clearing_a_slot_committed_after_recreation_is_emitted() {
    let baseline = account(
        AccountStatus::DestroyedModified,
        1,
        0,
        false,
        true,
        &[(4, 7)],
    );
    let drained = account(
        AccountStatus::DestroyedModified,
        1,
        0,
        false,
        true,
        &[(4, 0)],
    );

    let updates = flush(baseline, drained).expect("flush");

    assert_eq!(updates.len(), 1, "got {updates:?}");
    assert_eq!(added(&updates[0]), vec![(4, 0)]);
    assert!(!updates[0].removed_storage);
    assert!(!updates[0].removed);
}

/// A re-creation of the account destroyed earlier in the same batch was reverted, which
/// left the zeros its writes replaced on the still `Destroyed` account. Those slots have
/// no old value in the pre-destruction baseline, and the flush must not fail on them.
#[test]
fn reverted_recreation_in_the_destroying_batch_does_not_fail_the_flush() {
    let baseline = account(AccountStatus::Unmodified, 0, 1, false, true, &[]);
    let drained = account(AccountStatus::Destroyed, 0, 0, false, true, &[(5, 0)]);

    let updates = flush(baseline, drained).expect("flush");

    assert_eq!(updates.len(), 1, "got {updates:?}");
    assert!(updates[0].removed, "the destroyed account is removed");
    assert!(updates[0].added_storage.is_empty());
}

/// The same, one batch later: the baseline is the folded destroyed account.
#[test]
fn reverted_recreation_after_a_flush_does_not_fail_the_flush() {
    let baseline = account(AccountStatus::Destroyed, 0, 0, false, true, &[]);
    let drained = account(AccountStatus::Destroyed, 0, 0, false, true, &[(4, 0)]);

    let updates = flush(baseline, drained).expect("flush");

    assert!(updates.is_empty(), "nothing changed, got {updates:?}");
}

/// Destroyed again in a later batch: the slots the earlier batches committed and the
/// re-creation did not write back are deleted, and the rewritten one is updated.
#[test]
fn second_destruction_deletes_the_slots_committed_since_the_first() {
    let baseline = account(
        AccountStatus::DestroyedModified,
        1,
        0,
        false,
        true,
        &[(4, 7), (5, 9), (6, 0)],
    );
    let drained = account(
        AccountStatus::DestroyedModified,
        1,
        0,
        false,
        true,
        &[(5, 1)],
    );

    let updates = flush(baseline, drained).expect("flush");

    assert_eq!(updates.len(), 1, "got {updates:?}");
    assert_eq!(added(&updates[0]), vec![(4, 0), (5, 1)]);
}

/// The first destruction of an account with trie storage still removes that storage and
/// diffs every slot against zero.
#[test]
fn first_destruction_removes_trie_storage() {
    let baseline = account(AccountStatus::Unmodified, 1, 0, true, true, &[(4, 5)]);
    let drained = account(
        AccountStatus::DestroyedModified,
        1,
        0,
        false,
        true,
        &[(4, 5), (6, 3)],
    );

    let updates = flush(baseline, drained).expect("flush");

    assert_eq!(updates.len(), 1, "got {updates:?}");
    assert!(updates[0].removed_storage);
    assert_eq!(added(&updates[0]), vec![(4, 5), (6, 3)]);
}
