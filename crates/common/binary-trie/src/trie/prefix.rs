//! Key prefixes: the bit string naming a subtree of the trie.

use super::bits::bytes_to_bits;

/// A prefix of trie keys, as 0/1 values MSB-first — the same
/// representation branch prefixes and [`BitPath`]s use.
///
/// **Why a type rather than a `&[u8]`.** Keys are bytes and prefixes are
/// bits, and both are `&[u8]` in this crate: an API taking bare bits
/// would accept a key by mistake and silently answer about the wrong
/// subtree, since a 34-byte key read as 34 bits is a perfectly valid
/// prefix. Construction therefore goes through [`KeyPrefix::from_bytes`],
/// which does the expansion, and the bits are only reachable as
/// [`KeyPrefix::as_bits`].
///
/// **Why bits and not whole bytes.** The zone and stem boundaries the
/// embedding cares about are byte-aligned, but the interesting *ranges*
/// inside a stem are not: an account's header storage is sub-indices
/// `64..=127`, which is exactly the sub-index bytes beginning `01`. That
/// range is one bit-prefix and therefore one traversal; as a byte prefix
/// it would be 64 separate lookups. Hence [`KeyPrefix::and_bits`].
///
/// [`BitPath`]: super::path::BitPath
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyPrefix(Vec<u8>);

impl KeyPrefix {
    /// The prefix covering every key that starts with `bytes`.
    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self(bytes_to_bits(bytes))
    }

    /// Narrow this prefix by the further `bits`.
    pub fn and_bits(mut self, bits: &[u8]) -> Self {
        debug_assert!(bits.iter().all(|b| *b <= 1), "prefix bits must be 0 or 1");
        self.0.extend_from_slice(bits);
        self
    }

    /// Length in bits.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether this prefix constrains nothing, and so names the whole
    /// trie. The trie's operations refuse it — see
    /// [`BinaryTrie::contains_prefix`].
    ///
    /// [`BinaryTrie::contains_prefix`]: super::BinaryTrie::contains_prefix
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn as_bits(&self) -> &[u8] {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_bytes_expands_msb_first() {
        assert_eq!(
            KeyPrefix::from_bytes(&[0b1010_0001]).as_bits(),
            &[1, 0, 1, 0, 0, 0, 0, 1]
        );
        assert_eq!(KeyPrefix::from_bytes(&[0xff; 33]).len(), 264);
    }

    #[test]
    fn and_bits_narrows_past_the_byte_boundary() {
        let prefix = KeyPrefix::from_bytes(&[0x00]).and_bits(&[0, 1]);
        assert_eq!(prefix.len(), 10);
        assert_eq!(prefix.as_bits(), &[0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);
    }

    #[test]
    fn the_empty_prefix_is_recognisable() {
        assert!(KeyPrefix::from_bytes(&[]).is_empty());
        assert!(!KeyPrefix::from_bytes(&[]).and_bits(&[0]).is_empty());
        assert!(!KeyPrefix::from_bytes(&[0x00]).is_empty());
    }
}
