//! The predicates that gate snap/2 BAL application on the range download's
//! progress (devp2p `caps/snap.md`, "Synchronization algorithm").
//!
//! Everything a block access list writes is gated on these. A false positive
//! patches a key the download has not reached, which the later range response
//! then overwrites with a value from an older root; a false negative drops a
//! patch the reconstruction needs. Neither shows up until the final root check,
//! with no way to attribute it, so the predicates are pinned here directly.

use ethrex_common::{BigEndianHash, H256, U256};
use ethrex_p2p::sync::snap2::{DownloadCursor, HashRange};

fn hash(byte: u8) -> H256 {
    H256::repeat_byte(byte)
}

fn at(value: u64) -> H256 {
    H256::from_uint(&U256::from(value))
}

/// A fresh cursor has served nothing, so no key may be patched.
#[test]
fn nothing_is_fetched_before_the_download_starts() {
    let cursor = DownloadCursor::new(4);
    assert!(!cursor.is_account_fetched(H256::zero()));
    assert!(!cursor.is_account_fetched(hash(0x80)));
    assert!(!cursor.is_account_fetched(H256::repeat_byte(0xff)));
    assert!(!cursor.is_storage_fetched(hash(0x80), H256::zero()));
    assert!(!cursor.is_complete());
}

/// Serving through a hash marks everything up to and including it as fetched,
/// and nothing beyond.
#[test]
fn advancing_accounts_moves_the_frontier_inclusively() {
    let mut cursor = DownloadCursor::new(1);
    cursor.advance_accounts(at(100));

    assert!(cursor.is_account_fetched(at(0)));
    assert!(cursor.is_account_fetched(at(99)));
    assert!(cursor.is_account_fetched(at(100)));
    assert!(!cursor.is_account_fetched(at(101)));
}

/// A range only answers for the keys it owns; work in a later chunk does not
/// make an earlier chunk's keys fetched.
#[test]
fn ranges_answer_independently() {
    let mut cursor = DownloadCursor::new(2);
    let ranges = cursor.pending_account_ranges().to_vec();
    assert_eq!(ranges.len(), 2);
    let second = ranges[1];

    // Serve the whole second half, leaving the first untouched.
    cursor.advance_accounts(second.last);

    assert!(!cursor.is_account_fetched(at(1)));
    assert!(cursor.is_account_fetched(second.next));
    assert!(cursor.is_account_fetched(second.last));
    assert_eq!(cursor.pending_account_ranges().len(), 1);
}

/// Serving the top of the hash space drains the final range rather than
/// overflowing the frontier back to zero.
#[test]
fn serving_the_last_hash_completes_the_space() {
    let mut cursor = DownloadCursor::new(1);
    cursor.advance_accounts(H256::repeat_byte(0xff));

    assert!(cursor.is_complete());
    assert!(cursor.is_account_fetched(H256::zero()));
    assert!(cursor.is_account_fetched(H256::repeat_byte(0xff)));
}

/// An account whose leaf has landed but whose storage has not been scheduled
/// must not have its slots patched: the storage download will serve them at a
/// newer root, already carrying the change.
#[test]
fn storage_is_unfetched_until_scheduled() {
    let cursor = DownloadCursor::new(1);
    let account = at(50);

    assert!(!cursor.is_account_fetched(account));
    assert!(!cursor.is_storage_fetched(account, hash(0x01)));
}

/// An account range advances only once its accounts' storage is in, so an
/// account behind the frontier has complete storage by construction.
#[test]
fn storage_is_fetched_once_the_account_range_passes_it() {
    let mut cursor = DownloadCursor::new(1);
    let account = at(50);
    cursor.advance_accounts(at(100));

    assert!(cursor.is_storage_fetched(account, H256::zero()));
    assert!(cursor.is_storage_fetched(account, H256::repeat_byte(0xff)));
}

/// A contract served in slot chunks answers per slot, not per account.
#[test]
fn large_contract_storage_answers_per_slot() {
    let mut cursor = DownloadCursor::new(1);
    let account = at(50);
    cursor.open_storage_ranges(
        account,
        vec![HashRange::new(H256::zero(), H256::repeat_byte(0xff))],
    );

    assert!(!cursor.is_storage_fetched(account, at(10)));

    cursor.advance_storage(account, at(10));
    assert!(cursor.is_storage_fetched(account, at(0)));
    assert!(cursor.is_storage_fetched(account, at(10)));
    assert!(!cursor.is_storage_fetched(account, at(11)));
}

/// Draining a contract's last slot range completes its storage.
#[test]
fn draining_slot_ranges_completes_storage() {
    let mut cursor = DownloadCursor::new(1);
    let account = at(50);
    cursor.open_storage_ranges(
        account,
        vec![
            HashRange::new(at(0), at(99)),
            HashRange::new(at(100), at(199)),
        ],
    );

    cursor.advance_storage(account, at(99));
    assert!(cursor.is_storage_fetched(account, at(50)));
    assert!(!cursor.is_storage_fetched(account, at(150)));

    cursor.advance_storage(account, at(199));
    assert!(cursor.is_storage_fetched(account, at(150)));
    // Past the last range the contract is done, so any slot answers fetched.
    assert!(cursor.is_storage_fetched(account, H256::repeat_byte(0xff)));
}

/// Completing storage outright, without slot chunks, is what the common case
/// of a contract served in one response reports.
#[test]
fn completing_storage_marks_every_slot_fetched() {
    let mut cursor = DownloadCursor::new(1);
    let account = at(50);
    cursor.complete_storage(account);

    assert!(cursor.is_storage_fetched(account, H256::zero()));
    assert!(cursor.is_storage_fetched(account, H256::repeat_byte(0xff)));
    // A different account is unaffected.
    assert!(!cursor.is_storage_fetched(at(51), H256::zero()));
}

/// Opening no slot ranges means there was nothing left to serve.
#[test]
fn opening_empty_slot_ranges_completes_storage() {
    let mut cursor = DownloadCursor::new(1);
    let account = at(50);
    cursor.open_storage_ranges(account, vec![]);

    assert!(cursor.is_storage_fetched(account, hash(0x07)));
}

/// Completion flags for accounts the frontier has passed are dropped: they are
/// redundant against the ranges, and mainnet has too many contracts to keep
/// every flag for the whole sync.
#[test]
fn completion_flags_are_pruned_behind_the_frontier() {
    let mut cursor = DownloadCursor::new(1);
    let passed = at(10);
    let ahead = at(500);
    cursor.complete_storage(passed);
    cursor.complete_storage(ahead);

    cursor.advance_accounts(at(100));

    // Both still answer correctly: the passed one from the ranges, the one
    // ahead of the frontier from its retained flag.
    assert!(cursor.is_storage_fetched(passed, hash(0x01)));
    assert!(cursor.is_storage_fetched(ahead, hash(0x01)));
}

/// Reopening slot ranges for an account that was marked complete puts it back
/// in progress, so a stale flag cannot report unserved slots as fetched.
#[test]
fn reopening_slot_ranges_clears_the_completion_flag() {
    let mut cursor = DownloadCursor::new(1);
    let account = at(50);
    cursor.complete_storage(account);
    cursor.open_storage_ranges(account, vec![HashRange::new(at(0), at(99))]);

    assert!(!cursor.is_storage_fetched(account, at(50)));
}

/// Advancing storage for an account with no open ranges is a no-op, not a
/// silent completion.
#[test]
fn advancing_unscheduled_storage_does_nothing() {
    let mut cursor = DownloadCursor::new(1);
    let account = at(50);
    cursor.advance_storage(account, at(10));

    assert!(!cursor.is_storage_fetched(account, at(10)));
}

/// The chunk split is a scheduling detail: however the space is divided, the
/// frontier answers the same.
#[test]
fn chunk_count_does_not_change_the_frontier() {
    for chunks in [1usize, 2, 3, 7, 64] {
        let mut cursor = DownloadCursor::new(chunks);
        let ranges = cursor.pending_account_ranges().to_vec();

        // The ranges must tile the whole space with no gap and no overlap.
        assert_eq!(ranges[0].next, H256::zero(), "chunks={chunks}");
        assert_eq!(
            ranges[ranges.len() - 1].last,
            H256::repeat_byte(0xff),
            "chunks={chunks}"
        );
        for pair in ranges.windows(2) {
            assert_eq!(
                pair[1].next.into_uint(),
                pair[0].last.into_uint() + U256::one(),
                "chunks={chunks}"
            );
        }

        // Serving every range in order completes the space exactly once.
        for range in &ranges {
            cursor.advance_accounts(range.last);
        }
        assert!(cursor.is_complete(), "chunks={chunks}");
    }
}
