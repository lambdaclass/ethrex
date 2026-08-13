//! Applying a block access list to the snap/2 flat state
//! (devp2p `caps/snap.md`, "Synchronization algorithm").
//!
//! The gate on download progress is the whole reason this is sound. Patching a
//! key the download has not reached corrupts the flat state twice over: the
//! range response that later covers it overwrites the patch with a value from
//! an older root, and if the patch lands after the response instead, the change
//! is applied on top of a leaf that already carries it. Neither shows up before
//! the final root check, so the gate is pinned here per key kind.

use bytes::Bytes;
use ethrex_common::{
    Address, BigEndianHash, H256, U256,
    constants::EMPTY_KECCAK_HASH,
    types::{
        AccountState,
        block_access_list::{
            AccountChanges, BalanceChange, BlockAccessList, CodeChange, NonceChange, SlotChange,
            StorageChange,
        },
    },
    utils::keccak,
};
use ethrex_p2p::sync::snap2::{DownloadCursor, FlatState, HashRange, apply_bal_flat};
use ethrex_storage::{EngineType, Store, hash_address, hash_key};

fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("ethrex-snap2-apply-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp datadir");
    dir
}

fn store() -> Store {
    Store::new("memory", EngineType::InMemory).expect("in-memory store")
}

fn address(byte: u8) -> Address {
    Address::repeat_byte(byte)
}

fn account_hash(byte: u8) -> H256 {
    H256::from_slice(&hash_address(&address(byte)))
}

fn slot_hash(slot: u64) -> H256 {
    H256::from_slice(&hash_key(&H256::from_uint(&U256::from(slot))))
}

/// A cursor that has served the whole state, so nothing is gated out.
fn fully_fetched() -> DownloadCursor {
    let mut cursor = DownloadCursor::new(1);
    cursor.advance_accounts(H256::repeat_byte(0xff));
    cursor
}

fn changes(addr: Address) -> AccountChanges {
    AccountChanges {
        address: addr,
        ..Default::default()
    }
}

fn with_balance(mut changes: AccountChanges, balance: u64) -> AccountChanges {
    changes.balance_changes.push(BalanceChange {
        post_balance: U256::from(balance),
        ..Default::default()
    });
    changes
}

fn with_nonce(mut changes: AccountChanges, nonce: u64) -> AccountChanges {
    changes.nonce_changes.push(NonceChange {
        post_nonce: nonce,
        ..Default::default()
    });
    changes
}

fn with_code(mut changes: AccountChanges, code: &[u8]) -> AccountChanges {
    changes.code_changes.push(CodeChange {
        new_code: Bytes::copy_from_slice(code),
        ..Default::default()
    });
    changes
}

fn with_slot(mut changes: AccountChanges, slot: u64, values: &[u64]) -> AccountChanges {
    changes.storage_changes.push(SlotChange {
        slot: U256::from(slot),
        slot_changes: values
            .iter()
            .map(|value| StorageChange {
                post_value: U256::from(*value),
                ..Default::default()
            })
            .collect(),
    });
    changes
}

fn bal(accounts: Vec<AccountChanges>) -> BlockAccessList {
    BlockAccessList::from_accounts(accounts)
}

#[tokio::test]
async fn writes_the_post_block_value_of_each_field() {
    let flat = FlatState::open(&temp_dir("post-values")).expect("open");
    let store = store();
    let cursor = fully_fetched();

    let code = b"\x60\x00".as_slice();
    let changes = with_code(with_nonce(with_balance(changes(address(1)), 500), 3), code);

    apply_bal_flat(&flat, &cursor, &store, &bal(vec![changes])).expect("apply");

    let account = flat
        .get_account(account_hash(1))
        .expect("read")
        .expect("account written");
    assert_eq!(account.balance, U256::from(500));
    assert_eq!(account.nonce, 3);
    assert_eq!(account.code_hash, keccak(code));

    flat.destroy().await.expect("destroy");
}

/// Only the last change in a block counts; the intermediate ones are the
/// per-transaction history and are not the post-block state.
#[tokio::test]
async fn the_last_change_in_the_block_wins() {
    let flat = FlatState::open(&temp_dir("last-wins")).expect("open");
    let store = store();
    let cursor = fully_fetched();

    let mut changes = changes(address(1));
    for balance in [10u64, 20, 30] {
        changes.balance_changes.push(BalanceChange {
            post_balance: U256::from(balance),
            ..Default::default()
        });
    }
    for nonce in [1u64, 2, 3] {
        changes.nonce_changes.push(NonceChange {
            post_nonce: nonce,
            ..Default::default()
        });
    }
    let changes = with_slot(changes, 7, &[100, 200, 300]);

    apply_bal_flat(&flat, &cursor, &store, &bal(vec![changes])).expect("apply");

    let account = flat
        .get_account(account_hash(1))
        .expect("read")
        .expect("present");
    assert_eq!(account.balance, U256::from(30));
    assert_eq!(account.nonce, 3);
    assert_eq!(
        flat.get_slot(account_hash(1), slot_hash(7)).expect("read"),
        Some(U256::from(300))
    );

    flat.destroy().await.expect("destroy");
}

/// A slot set to zero is removed, not stored as zero: the trie build must not
/// see a leaf for it.
#[tokio::test]
async fn a_zeroed_slot_is_removed() {
    let flat = FlatState::open(&temp_dir("zeroed-slot")).expect("open");
    let store = store();
    let cursor = fully_fetched();

    flat.put_slot(account_hash(1), slot_hash(7), U256::from(99))
        .expect("seed slot");

    let changes = with_slot(with_balance(changes(address(1)), 1), 7, &[0]);
    apply_bal_flat(&flat, &cursor, &store, &bal(vec![changes])).expect("apply");

    assert_eq!(
        flat.get_slot(account_hash(1), slot_hash(7)).expect("read"),
        None
    );

    flat.destroy().await.expect("destroy");
}

/// Slots the block only read carry no post-value and must leave the flat state
/// alone.
#[tokio::test]
async fn storage_reads_change_nothing() {
    let flat = FlatState::open(&temp_dir("storage-reads")).expect("open");
    let store = store();
    let cursor = fully_fetched();

    flat.put_slot(account_hash(1), slot_hash(7), U256::from(99))
        .expect("seed slot");

    let mut changes = with_balance(changes(address(1)), 1);
    changes.storage_reads.push(U256::from(7));
    apply_bal_flat(&flat, &cursor, &store, &bal(vec![changes])).expect("apply");

    assert_eq!(
        flat.get_slot(account_hash(1), slot_hash(7)).expect("read"),
        Some(U256::from(99))
    );

    flat.destroy().await.expect("destroy");
}

/// EIP-161: an account drained of balance, nonce and code no longer exists, so
/// the trie build must not pick up an empty leaf for it.
#[tokio::test]
async fn a_drained_account_is_deleted() {
    let flat = FlatState::open(&temp_dir("drained")).expect("open");
    let store = store();
    let cursor = fully_fetched();

    flat.put_account(
        account_hash(1),
        &AccountState {
            nonce: 4,
            balance: U256::from(1000),
            ..Default::default()
        },
    )
    .expect("seed account");

    let changes = with_nonce(with_balance(changes(address(1)), 0), 0);
    let stats = apply_bal_flat(&flat, &cursor, &store, &bal(vec![changes])).expect("apply");

    assert!(flat.get_account(account_hash(1)).expect("read").is_none());
    assert_eq!(stats.accounts_deleted, 1);
    assert_eq!(stats.accounts_written, 0);

    flat.destroy().await.expect("destroy");
}

/// An account created and drained within the same block was never in the flat
/// state and must not be created just to be deleted.
#[tokio::test]
async fn an_account_empty_on_both_sides_is_never_written() {
    let flat = FlatState::open(&temp_dir("empty-both-sides")).expect("open");
    let store = store();
    let cursor = fully_fetched();

    let changes = with_nonce(with_balance(changes(address(1)), 0), 0);
    let stats = apply_bal_flat(&flat, &cursor, &store, &bal(vec![changes])).expect("apply");

    assert!(flat.get_account(account_hash(1)).expect("read").is_none());
    assert_eq!(stats.accounts_written, 0);
    assert_eq!(stats.accounts_deleted, 0);

    flat.destroy().await.expect("destroy");
}

/// Emptiness is balance, nonce and code only. An account holding storage but
/// none of those is still empty under EIP-161, and the storage root is not part
/// of the test because it is stale here by construction.
#[tokio::test]
async fn emptiness_ignores_the_storage_root() {
    let flat = FlatState::open(&temp_dir("empty-with-storage")).expect("open");
    let store = store();
    let cursor = fully_fetched();

    flat.put_account(
        account_hash(1),
        &AccountState {
            nonce: 1,
            balance: U256::from(5),
            storage_root: H256::repeat_byte(0xab),
            code_hash: *EMPTY_KECCAK_HASH,
        },
    )
    .expect("seed account");

    let changes = with_nonce(with_balance(changes(address(1)), 0), 0);
    apply_bal_flat(&flat, &cursor, &store, &bal(vec![changes])).expect("apply");

    assert!(flat.get_account(account_hash(1)).expect("read").is_none());

    flat.destroy().await.expect("destroy");
}

/// Clearing code (an EIP-7702 delegation being removed) sets the empty code
/// hash rather than hashing an empty slice into the code store.
#[tokio::test]
async fn clearing_code_sets_the_empty_code_hash() {
    let flat = FlatState::open(&temp_dir("clear-code")).expect("open");
    let store = store();
    let cursor = fully_fetched();

    let changes = with_code(with_balance(changes(address(1)), 1), b"");
    apply_bal_flat(&flat, &cursor, &store, &bal(vec![changes])).expect("apply");

    let account = flat
        .get_account(account_hash(1))
        .expect("read")
        .expect("present");
    assert_eq!(account.code_hash, *EMPTY_KECCAK_HASH);

    flat.destroy().await.expect("destroy");
}

/// The storage root is left as served. It is wrong here by construction and is
/// recomputed from the reconstructed storage tries, so writing it would be
/// discarded.
#[tokio::test]
async fn the_storage_root_is_left_alone() {
    let flat = FlatState::open(&temp_dir("stale-root")).expect("open");
    let store = store();
    let cursor = fully_fetched();

    let stale = H256::repeat_byte(0xab);
    flat.put_account(
        account_hash(1),
        &AccountState {
            nonce: 1,
            balance: U256::from(5),
            storage_root: stale,
            code_hash: *EMPTY_KECCAK_HASH,
        },
    )
    .expect("seed account");

    let changes = with_slot(with_balance(changes(address(1)), 7), 3, &[42]);
    apply_bal_flat(&flat, &cursor, &store, &bal(vec![changes])).expect("apply");

    let account = flat
        .get_account(account_hash(1))
        .expect("read")
        .expect("present");
    assert_eq!(account.balance, U256::from(7));
    assert_eq!(account.storage_root, stale);

    flat.destroy().await.expect("destroy");
}

/// An account the download has not reached is skipped whole: the range
/// response that covers it later is served at a newer root and already carries
/// this change.
#[tokio::test]
async fn an_unfetched_account_is_not_patched() {
    let flat = FlatState::open(&temp_dir("unfetched-account")).expect("open");
    let store = store();
    // A cursor that has served nothing.
    let cursor = DownloadCursor::new(1);

    let changes = with_balance(changes(address(1)), 500);
    let stats = apply_bal_flat(&flat, &cursor, &store, &bal(vec![changes])).expect("apply");

    assert!(flat.get_account(account_hash(1)).expect("read").is_none());
    assert_eq!(stats.accounts_skipped, 1);
    assert_eq!(stats.accounts_written, 0);

    flat.destroy().await.expect("destroy");
}

/// Slots are gated separately from the account, and the asymmetry only runs
/// one way.
///
/// An account range advances only once the accounts it covers and their
/// storage are both in, so "account served, storage not" never occurs. The
/// reverse does: a contract's storage completes while its range is still
/// pending, because other accounts in the same range are outstanding. Its
/// slots are then patched while its leaf is not — the leaf is re-served at the
/// current pivot when the range is retried, already carrying the change.
#[tokio::test]
async fn a_completed_contract_takes_slot_patches_before_its_range_advances() {
    let flat = FlatState::open(&temp_dir("slot-gate")).expect("open");
    let store = store();

    let mut cursor = DownloadCursor::new(1);
    cursor.complete_storage(account_hash(1));
    assert!(!cursor.is_account_fetched(account_hash(1)));
    assert!(cursor.is_storage_fetched(account_hash(1), slot_hash(3)));

    let changes = with_slot(with_balance(changes(address(1)), 500), 3, &[42]);
    let stats = apply_bal_flat(&flat, &cursor, &store, &bal(vec![changes])).expect("apply");

    // The slot is patched.
    assert_eq!(
        flat.get_slot(account_hash(1), slot_hash(3)).expect("read"),
        Some(U256::from(42))
    );
    // The leaf is not.
    assert!(flat.get_account(account_hash(1)).expect("read").is_none());
    assert_eq!(stats.slots_written, 1);
    assert_eq!(stats.accounts_skipped, 1);

    flat.destroy().await.expect("destroy");
}

/// A contract still being served in slot chunks takes patches only for the
/// slots already delivered.
#[tokio::test]
async fn a_partly_served_contract_is_patched_per_slot() {
    let flat = FlatState::open(&temp_dir("partial-slot-gate")).expect("open");
    let store = store();

    let served = slot_hash(3);
    let pending = slot_hash(4);
    let (low, high, low_slot, high_slot) = if served < pending {
        (served, pending, 3u64, 4u64)
    } else {
        (pending, served, 4u64, 3u64)
    };

    let mut cursor = DownloadCursor::new(1);
    cursor.open_storage_ranges(
        account_hash(1),
        vec![HashRange::new(H256::zero(), H256::repeat_byte(0xff))],
    );
    cursor.advance_storage(account_hash(1), low);
    assert!(cursor.is_storage_fetched(account_hash(1), low));
    assert!(!cursor.is_storage_fetched(account_hash(1), high));

    let changes = with_slot(
        with_slot(changes(address(1)), low_slot, &[42]),
        high_slot,
        &[43],
    );
    let stats = apply_bal_flat(&flat, &cursor, &store, &bal(vec![changes])).expect("apply");

    assert_eq!(
        flat.get_slot(account_hash(1), low).expect("read"),
        Some(U256::from(42))
    );
    assert_eq!(flat.get_slot(account_hash(1), high).expect("read"), None);
    assert_eq!(stats.slots_written, 1);
    assert_eq!(stats.slots_skipped, 1);

    flat.destroy().await.expect("destroy");
}

/// Each account in the list is gated on its own position, not on the list's.
#[tokio::test]
async fn gating_is_per_account() {
    let flat = FlatState::open(&temp_dir("per-account-gate")).expect("open");
    let store = store();

    let fetched = account_hash(1);
    let unfetched = account_hash(2);
    // Serve exactly up to whichever account hashes lower, leaving the other
    // ahead of the frontier.
    let (low, high, low_addr, high_addr) = if fetched < unfetched {
        (fetched, unfetched, address(1), address(2))
    } else {
        (unfetched, fetched, address(2), address(1))
    };

    let mut cursor = DownloadCursor::new(1);
    cursor.advance_accounts(low);
    assert!(cursor.is_account_fetched(low));
    assert!(!cursor.is_account_fetched(high));

    let stats = apply_bal_flat(
        &flat,
        &cursor,
        &store,
        &bal(vec![
            with_balance(changes(low_addr), 100),
            with_balance(changes(high_addr), 200),
        ]),
    )
    .expect("apply");

    assert_eq!(stats.accounts_written, 1);
    assert_eq!(stats.accounts_skipped, 1);
    assert_eq!(
        flat.get_account(low)
            .expect("read")
            .expect("present")
            .balance,
        U256::from(100)
    );
    assert!(flat.get_account(high).expect("read").is_none());

    flat.destroy().await.expect("destroy");
}
