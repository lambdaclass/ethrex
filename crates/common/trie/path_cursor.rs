use core::cmp::Ordering;

use crate::nibbles::{Nibbles, count_common_prefix};

/// A read-only cursor over a nibble path being descended through the trie.
///
/// [`Nibbles`] is the *owned path*; this is the *traversal state* over one. It is
/// `Copy` and allocation-free: [`Self::consumed`] is the visited node's
/// root-relative path (which is also its database key) and [`Self::remaining`] is
/// what is left to match against the nodes below.
///
/// Because both accessors hand out slices of the original path with its lifetime
/// (not borrows of the cursor), a key can be read out and the cursor moved on in
/// the same expression, which is what the descent code does at every level.
#[derive(Clone, Copy, Debug)]
pub struct PathCursor<'a> {
    nibbles: &'a [u8],
    /// How many leading nibbles of `nibbles` have been consumed. Always `<= nibbles.len()`.
    idx: usize,
}

impl<'a> PathCursor<'a> {
    /// Creates a cursor positioned at the start of `nibbles`.
    #[inline]
    pub const fn new(nibbles: &'a [u8]) -> Self {
        Self { nibbles, idx: 0 }
    }

    /// The part of the path that is still to be matched.
    #[inline]
    pub fn remaining(&self) -> &'a [u8] {
        &self.nibbles[self.idx..]
    }

    /// The part of the path already descended, i.e. the root-relative path of the
    /// node the cursor currently points at, which is its database key.
    #[inline]
    pub fn consumed(&self) -> &'a [u8] {
        &self.nibbles[..self.idx]
    }

    /// Returns true if the whole path has been consumed.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.remaining().is_empty()
    }

    /// Consumes and returns the next nibble, or `None` if the path is exhausted.
    #[allow(clippy::should_implement_trait)]
    #[inline]
    pub fn next(&mut self) -> Option<u8> {
        let nibble = *self.remaining().first()?;
        self.idx += 1;
        Some(nibble)
    }

    /// Consumes the next nibble and returns it if it is a valid branch choice
    /// index (below 16).
    ///
    /// The nibble is consumed either way: a leaf flag (16) here means the path
    /// terminates at the branch node being visited, and the caller reads the
    /// node's own value instead of descending.
    #[inline]
    pub fn next_choice(&mut self) -> Option<usize> {
        self.next().filter(|choice| *choice < 16).map(usize::from)
    }

    /// If `prefix` is a prefix of the remaining path, consumes it and returns
    /// true. Otherwise leaves the cursor untouched and returns false.
    #[inline]
    pub fn skip_prefix(&mut self, prefix: &[u8]) -> bool {
        if self.remaining().starts_with(prefix) {
            self.idx += prefix.len();
            true
        } else {
            false
        }
    }

    /// Returns a copy of this cursor advanced by `n` nibbles.
    ///
    /// # Panics
    ///
    /// If `n` is larger than the remaining path. Advancing past the end would
    /// otherwise produce a cursor that silently reports an exhausted path.
    #[inline]
    pub fn advanced(self, n: usize) -> Self {
        // A checked add, not `self.idx + n`: release builds have no overflow
        // checks, and a wrapped index would sail past the bound below and then
        // hand out silently wrong `consumed()` / `remaining()` slices.
        let idx = self
            .idx
            .checked_add(n)
            .filter(|idx| *idx <= self.nibbles.len())
            .expect("path cursor advanced past the end of the path");
        Self {
            nibbles: self.nibbles,
            idx,
        }
    }

    /// Number of leading nibbles the remaining path shares with `other`.
    #[inline]
    pub fn count_prefix(&self, other: &[u8]) -> usize {
        count_common_prefix(self.remaining(), other)
    }

    /// Compares the remaining path with `other`, comparing only their shared
    /// length so that one being a prefix of the other counts as equal.
    #[inline]
    pub fn compare_prefix(&self, other: &[u8]) -> Ordering {
        let remaining = self.remaining();
        let len = remaining.len().min(other.len());
        remaining[..len].cmp(&other[..len])
    }

    /// Copies the remaining path into an owned [`Nibbles`], for storing it as a
    /// new node's partial path or prefix.
    #[inline]
    pub fn to_nibbles(&self) -> Nibbles {
        Nibbles::from_hex(self.remaining().to_vec())
    }
}

impl<'a> From<&'a Nibbles> for PathCursor<'a> {
    #[inline]
    fn from(nibbles: &'a Nibbles) -> Self {
        Self::new(nibbles.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
