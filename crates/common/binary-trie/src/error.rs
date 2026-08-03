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
    /// A stored node could not be decoded. Since a node's stored bytes
    /// are its hashing preimage, this means the store returned
    /// something the tree never wrote.
    #[error("malformed stored node: {0}")]
    MalformedNode(&'static str),
    /// The storage backend failed, or is missing a node the tree
    /// expects: everything a correct tree over a correct store never
    /// sees.
    #[error("trie backend: {0}")]
    Backend(String),
    /// Balance does not fit the 16-byte field of the basic data leaf.
    #[error("balance does not fit the 16-byte basic-data field")]
    BalanceTooLarge,
}
