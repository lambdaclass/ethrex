//! Bit paths: where a node sits in the tree, and therefore how it is
//! keyed in the database.

/// The path from the trie root to a node: the bits consumed to reach
/// it, one 0/1 value per element, MSB-first — the same representation
/// branch prefixes use.
///
/// Paths key nodes in the database rather than hashes, following the
/// MPT's `TrieDB`: a path is known on the way *down*, so a traversal
/// can fetch the child it is about to visit, and a node that changes
/// overwrites itself in place instead of accumulating versions.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BitPath(Vec<u8>);

impl BitPath {
    /// The root's path: no bits consumed.
    pub const fn new() -> Self {
        Self(Vec::new())
    }

    pub fn from_bits(bits: &[u8]) -> Self {
        debug_assert!(bits.iter().all(|b| *b <= 1), "path bits must be 0 or 1");
        Self(bits.to_vec())
    }

    /// Path of the child reached from a branch at this path by walking
    /// its `prefix` and then taking the `bit` side of its split.
    ///
    /// Splitting a branch leaves the absolute paths of everything below
    /// it unchanged — the bits the parent stops consuming are exactly
    /// the bits the new child starts consuming — so a stored subtree
    /// never has to be rewritten because an ancestor split.
    pub fn child(&self, prefix: &[u8], bit: u8) -> Self {
        debug_assert!(bit <= 1, "split bit must be 0 or 1");
        let mut bits = Vec::with_capacity(self.0.len() + prefix.len() + 1);
        bits.extend_from_slice(&self.0);
        bits.extend_from_slice(prefix);
        bits.push(bit);
        Self(bits)
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn as_bits(&self) -> &[u8] {
        &self.0
    }

    /// Database key: a four-byte big-endian bit count followed by the
    /// bits packed MSB-first.
    ///
    /// The count is what makes the key injective: `[1]` and `[1, 0]`
    /// pack to the same bytes without it, and one node would silently
    /// overwrite the other.
    ///
    /// The same shape as [`encode_bit_prefix`] but with a wider count,
    /// deliberately. That function encodes a *branch prefix*, which is
    /// always shorter than the keys sharing it, so two bytes suffice
    /// and are fixed by consensus. A *path* has no such headroom: a
    /// leaf's path can be its key's entire bit length, which at
    /// [`MAX_KEY_LENGTH`] is 65536 bits — one past what two bytes
    /// hold. This key format is ours alone, so it widens rather than
    /// forcing the spec's key bound down to accommodate it.
    ///
    /// [`encode_bit_prefix`]: super::bits::encode_bit_prefix
    /// [`MAX_KEY_LENGTH`]: super::MAX_KEY_LENGTH
    pub fn to_db_key(&self) -> Vec<u8> {
        let bits = &self.0;
        let mut key = vec![0u8; 4 + bits.len().div_ceil(8)];
        key[..4].copy_from_slice(&(bits.len() as u32).to_be_bytes());
        for (i, bit) in bits.iter().enumerate() {
            key[4 + i / 8] |= bit << (7 - i % 8);
        }
        key
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_path_is_the_root() {
        assert!(BitPath::new().is_empty());
        assert_eq!(BitPath::new().len(), 0);
        assert_eq!(BitPath::new().to_db_key(), vec![0x00, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn db_key_is_a_counted_bit_packing() {
        assert_eq!(
            BitPath::from_bits(&[1, 0, 1]).to_db_key(),
            vec![0x00, 0x00, 0x00, 0x03, 0b1010_0000]
        );
    }

    #[test]
    fn the_longest_possible_path_fits_the_count() {
        // A leaf's path can be its key's full bit length — 65536 bits
        // at MAX_KEY_LENGTH, one past a two-byte count. The key's own
        // count is four bytes precisely so the spec's key bound does
        // not have to shrink for it.
        let path = BitPath::from_bits(&vec![1u8; crate::trie::MAX_KEY_LENGTH * 8]);
        assert_eq!(path.to_db_key().len(), 4 + crate::trie::MAX_KEY_LENGTH);
    }

    #[test]
    fn child_extends_by_the_prefix_and_the_split_bit() {
        let path = BitPath::from_bits(&[1]);
        assert_eq!(path.child(&[0, 0], 1), BitPath::from_bits(&[1, 0, 0, 1]));
        assert_eq!(path.child(&[], 0), BitPath::from_bits(&[1, 0]));
    }
}
