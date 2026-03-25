//! Stub types replacing ethrex_trie for the snap/healing subsystem.
//!
//! These subsystems are disabled on the binary trie branch. The types here
//! are minimal placeholders that allow the code to compile without the
//! ethrex-trie crate dependency.

use ethrex_common::H256;
use std::hash::{Hash, Hasher};

/// Stub replacement for ethrex_trie::Nibbles.
/// Represents a path in the MPT as a byte vector.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Nibbles(Vec<u8>);

impl Hash for Nibbles {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

impl Nibbles {
    pub fn from_hex(bytes: Vec<u8>) -> Self {
        Nibbles(bytes)
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        self.0.clone()
    }

    pub fn encode_compact(&self) -> Vec<u8> {
        self.0.clone()
    }

    pub fn append_new(&self, _nibble: u8) -> Self {
        Nibbles(self.0.clone())
    }

    pub fn concat(&self, _other: &Nibbles) -> Self {
        Nibbles(self.0.clone())
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Stub replacement for ethrex_trie::Node.
#[derive(Debug, Clone)]
pub enum Node {
    Branch(BranchNode),
    Extension(ExtensionNode),
    Leaf(LeafNode),
}

impl Node {
    pub fn decode(_bytes: &[u8]) -> Result<Self, ethrex_rlp::error::RLPDecodeError> {
        Err(ethrex_rlp::error::RLPDecodeError::MalformedData)
    }

    pub fn compute_hash(&self, _crypto: &ethrex_crypto::NativeCrypto) -> NodeHash {
        NodeHash
    }
}

/// Stub replacement for ethrex_trie::node::NodeHash.
#[derive(Debug, Clone)]
pub struct NodeHash;

impl NodeHash {
    pub fn finalize(&self, _crypto: &ethrex_crypto::NativeCrypto) -> H256 {
        H256::zero()
    }

    pub fn is_valid(&self) -> bool {
        false
    }
}

/// Stub replacement for ethrex_trie::node::NodeRef.
#[derive(Debug, Clone)]
pub enum NodeRef {
    Node(Box<Node>, ()),
    Hash(NodeHash),
}

impl NodeRef {
    pub fn is_valid(&self) -> bool {
        false
    }

    pub fn compute_hash(&self, _crypto: &ethrex_crypto::NativeCrypto) -> NodeHash {
        NodeHash
    }

    pub fn get_node_checked(
        &self,
        _trie_state: &dyn TrieDB,
        _path: Nibbles,
    ) -> Result<Option<()>, TrieError> {
        Ok(None)
    }
}

#[derive(Debug, Clone)]
pub struct BranchNode {
    pub choices: Vec<NodeRef>,
}

#[derive(Debug, Clone)]
pub struct ExtensionNode {
    pub child: NodeRef,
    pub prefix: Nibbles,
}

#[derive(Debug, Clone)]
pub struct LeafNode {
    pub partial: Nibbles,
    pub value: Vec<u8>,
}

/// Stub replacement for ethrex_trie::TrieDB trait.
pub trait TrieDB: Send + Sync {
    fn get(&self, _key: Nibbles) -> Result<Option<Vec<u8>>, TrieError>;
    fn put_batch(&self, _batch: Vec<(Nibbles, Vec<u8>)>) -> Result<(), TrieError>;
}

/// Stub replacement for ethrex_trie::TrieError.
#[derive(Debug, thiserror::Error)]
#[error("MPT trie error (binary trie branch): {0}")]
pub struct TrieError(pub String);

/// Stub replacement for ethrex_trie::Trie (rocksdb feature only, snap_sync.rs).
pub struct Trie;

/// Stub replacement for ethrex_trie::verify_range.
/// Always returns Ok(false) (no more data) since MPT snap sync is disabled.
pub fn verify_range(
    _root: H256,
    _left_bound: &H256,
    _keys: &[H256],
    _values: &[Vec<u8>],
    _proof: &[Vec<u8>],
) -> Result<bool, TrieError> {
    // MPT range verification not supported on binary trie branch
    Ok(false)
}

/// Stub replacement for ethrex_trie::trie_sorted::TrieGenerationError.
#[derive(Debug, thiserror::Error)]
#[error("MPT trie generation error (binary trie branch): {0}")]
pub struct TrieGenerationError(pub String);
