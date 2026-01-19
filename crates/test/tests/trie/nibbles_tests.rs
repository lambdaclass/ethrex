use ethrex_trie::Nibbles;
use std::cmp::Ordering;

#[test]
fn skip_prefix_true() {
    let mut a = Nibbles::from_hex(vec![1, 2, 3, 4, 5]);
    let b = Nibbles::from_hex(vec![1, 2, 3]);
    assert!(a.skip_prefix(&b));
    assert_eq!(a.as_ref(), &[4, 5])
}

#[test]
fn skip_prefix_true_same_length() {
    let mut a = Nibbles::from_hex(vec![1, 2, 3, 4, 5]);
    let b = Nibbles::from_hex(vec![1, 2, 3, 4, 5]);
    assert!(a.skip_prefix(&b));
    assert!(a.is_empty());
}

#[test]
fn skip_prefix_longer_prefix() {
    let mut a = Nibbles::from_hex(vec![1, 2, 3]);
    let b = Nibbles::from_hex(vec![1, 2, 3, 4, 5]);
    assert!(!a.skip_prefix(&b));
    assert_eq!(a.as_ref(), &[1, 2, 3])
}

#[test]
fn skip_prefix_false() {
    let mut a = Nibbles::from_hex(vec![1, 2, 3, 4, 5]);
    let b = Nibbles::from_hex(vec![1, 2, 4]);
    assert!(!a.skip_prefix(&b));
    assert_eq!(a.as_ref(), &[1, 2, 3, 4, 5])
}

#[test]
fn count_prefix_all() {
    let a = Nibbles::from_hex(vec![1, 2, 3, 4, 5]);
    let b = Nibbles::from_hex(vec![1, 2, 3, 4, 5]);
    assert_eq!(a.count_prefix(&b), a.len());
}

#[test]
fn count_prefix_partial() {
    let a = Nibbles::from_hex(vec![1, 2, 3, 4, 5]);
    let b = Nibbles::from_hex(vec![1, 2, 3]);
    assert_eq!(a.count_prefix(&b), b.len());
}

#[test]
fn count_prefix_none() {
    let a = Nibbles::from_hex(vec![1, 2, 3, 4, 5]);
    let b = Nibbles::from_hex(vec![2, 3, 4, 5, 6]);
    assert_eq!(a.count_prefix(&b), 0);
}

#[test]
fn compare_prefix_equal() {
    let a = Nibbles::from_hex(vec![1, 2, 3, 4, 5]);
    let b = Nibbles::from_hex(vec![1, 2, 3, 4, 5]);
    assert_eq!(a.compare_prefix(&b), Ordering::Equal);
}

#[test]
fn compare_prefix_less() {
    let a = Nibbles::from_hex(vec![1, 2, 3, 4, 5]);
    let b = Nibbles::from_hex(vec![1, 2, 4, 4, 5]);
    assert_eq!(a.compare_prefix(&b), Ordering::Less);
}

#[test]
fn compare_prefix_greater() {
    let a = Nibbles::from_hex(vec![1, 2, 4, 4, 5]);
    let b = Nibbles::from_hex(vec![1, 2, 3, 4, 5]);
    assert_eq!(a.compare_prefix(&b), Ordering::Greater);
}

#[test]
fn compare_prefix_equal_b_longer() {
    let a = Nibbles::from_hex(vec![1, 2, 3]);
    let b = Nibbles::from_hex(vec![1, 2, 3, 4, 5]);
    assert_eq!(a.compare_prefix(&b), Ordering::Equal);
}

#[test]
fn compare_prefix_equal_a_longer() {
    let a = Nibbles::from_hex(vec![1, 2, 3, 4, 5]);
    let b = Nibbles::from_hex(vec![1, 2, 3]);
    assert_eq!(a.compare_prefix(&b), Ordering::Equal);
}
