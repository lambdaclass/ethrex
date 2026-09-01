use ethrex_trie::{Nibbles, PathCursor};
use std::cmp::Ordering;

#[test]
fn consumed_and_remaining_split_the_path() {
    let mut cursor = PathCursor::new(&[1, 2, 3, 4, 5]);
    assert_eq!(cursor.consumed(), &[] as &[u8]);
    assert_eq!(cursor.remaining(), &[1, 2, 3, 4, 5]);

    assert_eq!(cursor.next(), Some(1));
    assert_eq!(cursor.consumed(), &[1]);
    assert_eq!(cursor.remaining(), &[2, 3, 4, 5]);

    let advanced = cursor.advanced(4);
    assert_eq!(advanced.consumed(), &[1, 2, 3, 4, 5]);
    assert!(advanced.is_empty());
    // `advanced` returns a copy: the original is untouched.
    assert_eq!(cursor.remaining(), &[2, 3, 4, 5]);
}

#[test]
fn next_consumes_the_leaf_flag_but_is_not_a_choice() {
    let mut cursor = PathCursor::new(&[16]);
    assert_eq!(cursor.next_choice(), None);
    assert!(cursor.is_empty());
    assert_eq!(cursor.consumed(), &[16]);
}

#[test]
fn next_on_an_exhausted_cursor_yields_none() {
    let mut cursor = PathCursor::new(&[]);
    assert_eq!(cursor.next(), None);
    assert_eq!(cursor.next_choice(), None);
    assert!(cursor.is_empty());
}

#[test]
fn skip_prefix_only_advances_on_a_match() {
    let mut cursor = PathCursor::new(&[1, 2, 3, 4, 5]);
    assert!(!cursor.skip_prefix(&[1, 2, 4]));
    assert_eq!(cursor.remaining(), &[1, 2, 3, 4, 5]);
    assert!(cursor.skip_prefix(&[1, 2, 3]));
    assert_eq!(cursor.consumed(), &[1, 2, 3]);
    assert_eq!(cursor.remaining(), &[4, 5]);
    // A prefix longer than what is left never matches.
    assert!(!cursor.skip_prefix(&[4, 5, 6]));
    assert_eq!(cursor.remaining(), &[4, 5]);
}

#[test]
fn count_and_compare_prefix_look_at_the_remaining_path() {
    let cursor = PathCursor::new(&[1, 2, 3, 4, 5]).advanced(2);
    assert_eq!(cursor.count_prefix(&[3, 4, 9]), 2);
    assert_eq!(cursor.count_prefix(&[9]), 0);
    assert_eq!(cursor.compare_prefix(&[3, 4]), Ordering::Equal);
    assert_eq!(cursor.compare_prefix(&[3, 4, 5, 6]), Ordering::Equal);
    assert_eq!(cursor.compare_prefix(&[3, 5]), Ordering::Less);
    assert_eq!(cursor.compare_prefix(&[2]), Ordering::Greater);
}

#[test]
fn to_nibbles_owns_the_remaining_path() {
    let cursor = PathCursor::new(&[1, 2, 3]).advanced(1);
    assert_eq!(cursor.to_nibbles(), Nibbles::from_hex(vec![2, 3]));
}

#[test]
#[should_panic(expected = "path cursor advanced past the end of the path")]
fn advancing_past_the_end_panics() {
    let _ = PathCursor::new(&[1, 2]).advanced(3);
}

#[test]
#[should_panic(expected = "path cursor advanced past the end of the path")]
fn advancing_by_an_amount_that_would_wrap_panics() {
    // Release builds have no overflow checks, so the bound has to be checked
    // on the addition itself rather than on its (possibly wrapped) result.
    let _ = PathCursor::new(&[1, 2]).advanced(1).advanced(usize::MAX);
}

#[test]
fn a_cursor_can_be_taken_from_an_owned_path() {
    let path = Nibbles::from_hex(vec![1, 2, 3]);
    assert_eq!(path.cursor().remaining(), &[1, 2, 3]);
    assert_eq!(
        PathCursor::from(&path).remaining(),
        path.cursor().remaining()
    );
}
