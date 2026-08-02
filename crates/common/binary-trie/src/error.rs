use thiserror::Error;

use crate::trie::MAX_KEY_LENGTH;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum BinaryTrieError {
    /// The empty key is a prefix of every other key.
    #[error("empty key")]
    EmptyKey,
    /// Key exceeds [`MAX_KEY_LENGTH`], past which a branch prefix bit
    /// count could overflow its two-byte encoding.
    #[error("key longer than {MAX_KEY_LENGTH} bytes")]
    KeyTooLong,
    /// Inserting this key would make some key a prefix of another,
    /// which the tree cannot represent (a leaf terminates its path).
    #[error("key is a prefix of another key in the trie")]
    PrefixViolation,
    /// Balance does not fit the 16-byte field of the basic data leaf.
    #[error("balance does not fit the 16-byte basic-data field")]
    BalanceTooLarge,
}
