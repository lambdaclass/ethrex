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
