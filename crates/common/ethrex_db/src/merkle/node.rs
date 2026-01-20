//! Merkle trie node types.

use tiny_keccak::{Hasher, Keccak};
use super::rlp_encode::RlpEncoder;

/// Hash size (Keccak-256).
pub const HASH_SIZE: usize = 32;

/// Node type in the Merkle trie.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NodeType {
    /// Empty node.
    Empty,
    /// Leaf node with path and value.
    Leaf,
    /// Extension node with path and child hash.
    Extension,
    /// Branch node with up to 16 children and optional value.
    Branch,
}

/// Reference to a child node in the trie.
///
/// Per Ethereum's MPT spec:
/// - If the RLP encoding of a child is >= 32 bytes, store the keccak256 hash
/// - If the RLP encoding is < 32 bytes, embed the RLP directly (inline)
#[derive(Clone, Debug)]
pub enum ChildRef {
    /// Empty child (null).
    Empty,
    /// Child whose RLP encoding is >= 32 bytes - stored as keccak256 hash.
    Hash([u8; HASH_SIZE]),
    /// Child whose RLP encoding is < 32 bytes - stored inline.
    /// The Vec contains the actual RLP-encoded node.
    Inline(Vec<u8>),
}

impl ChildRef {
    /// Creates a ChildRef from an encoded node.
    ///
    /// If the encoded node is >= 32 bytes, it's hashed.
    /// If < 32 bytes, it's stored inline.
    pub fn from_encoded(encoded: Vec<u8>) -> Self {
        if encoded.len() >= HASH_SIZE {
            let hash = keccak256(&encoded);
            ChildRef::Hash(hash)
        } else {
            ChildRef::Inline(encoded)
        }
    }

    /// Returns the hash for this child reference.
    ///
    /// For Hash: returns the hash directly.
    /// For Inline: computes keccak256 of the inline data.
    /// For Empty: this shouldn't be called (will return zeros).
    pub fn to_hash(&self) -> [u8; HASH_SIZE] {
        match self {
            ChildRef::Hash(h) => *h,
            ChildRef::Inline(data) => keccak256(data),
            ChildRef::Empty => [0u8; HASH_SIZE],
        }
    }

    /// Returns true if this is empty.
    pub fn is_empty(&self) -> bool {
        matches!(self, ChildRef::Empty)
    }
}

/// A node in the Merkle Patricia Trie.
#[derive(Clone, Debug)]
pub enum Node {
    /// Empty node (null).
    Empty,

    /// Leaf node: contains the remainder of the key and the value.
    Leaf {
        /// Remaining nibbles of the key.
        path: Vec<u8>,
        /// The value stored at this leaf.
        value: Vec<u8>,
    },

    /// Extension node: contains a shared path prefix and a child.
    Extension {
        /// Shared path prefix (nibbles).
        path: Vec<u8>,
        /// Reference to child node (hash or inline).
        child: ChildRef,
    },

    /// Branch node: has up to 16 children (one for each nibble) and an optional value.
    Branch {
        /// Children (16 slots, one per nibble). Uses ChildRef for proper inline handling.
        children: Box<[ChildRef; 16]>,
        /// Optional value stored at this branch.
        value: Option<Vec<u8>>,
    },
}

impl Node {
    /// Creates an empty node.
    pub fn empty() -> Self {
        Node::Empty
    }

    /// Creates a leaf node.
    pub fn leaf(path: Vec<u8>, value: Vec<u8>) -> Self {
        Node::Leaf { path, value }
    }

    /// Creates an extension node with a hash child.
    pub fn extension(path: Vec<u8>, child: [u8; HASH_SIZE]) -> Self {
        Node::Extension { path, child: ChildRef::Hash(child) }
    }

    /// Creates an extension node with a ChildRef.
    pub fn extension_with_child_ref(path: Vec<u8>, child: ChildRef) -> Self {
        Node::Extension { path, child }
    }

    /// Creates an empty branch node.
    pub fn branch() -> Self {
        Node::Branch {
            children: Box::new([
                ChildRef::Empty, ChildRef::Empty, ChildRef::Empty, ChildRef::Empty,
                ChildRef::Empty, ChildRef::Empty, ChildRef::Empty, ChildRef::Empty,
                ChildRef::Empty, ChildRef::Empty, ChildRef::Empty, ChildRef::Empty,
                ChildRef::Empty, ChildRef::Empty, ChildRef::Empty, ChildRef::Empty,
            ]),
            value: None,
        }
    }

    /// Creates a branch node with the given children.
    pub fn branch_with_children(children: Box<[ChildRef; 16]>, value: Option<Vec<u8>>) -> Self {
        Node::Branch { children, value }
    }

    /// Returns the node type.
    pub fn node_type(&self) -> NodeType {
        match self {
            Node::Empty => NodeType::Empty,
            Node::Leaf { .. } => NodeType::Leaf,
            Node::Extension { .. } => NodeType::Extension,
            Node::Branch { .. } => NodeType::Branch,
        }
    }

    /// RLP encodes the node.
    pub fn encode(&self) -> Vec<u8> {
        let mut encoder = RlpEncoder::new();
        self.encode_to(&mut encoder);
        encoder.into_bytes()
    }

    /// RLP encodes the node to the given encoder.
    pub fn encode_to(&self, encoder: &mut RlpEncoder) {
        match self {
            Node::Empty => {
                encoder.encode_empty();
            }
            Node::Leaf { path, value } => {
                encoder.encode_list(|e| {
                    e.encode_nibbles(path, true);
                    e.encode_bytes(value);
                });
            }
            Node::Extension { path, child } => {
                encoder.encode_list(|e| {
                    e.encode_nibbles(path, false);
                    match child {
                        ChildRef::Hash(hash) => e.encode_bytes(hash),
                        ChildRef::Inline(data) => e.encode_raw(data),
                        ChildRef::Empty => e.encode_empty(),
                    }
                });
            }
            Node::Branch { children, value } => {
                encoder.encode_list(|e| {
                    for child in children.iter() {
                        match child {
                            ChildRef::Hash(hash) => e.encode_bytes(hash),
                            ChildRef::Inline(data) => e.encode_raw(data),
                            ChildRef::Empty => e.encode_empty(),
                        }
                    }
                    match value {
                        Some(v) => e.encode_bytes(v),
                        None => e.encode_empty(),
                    }
                });
            }
        }
    }

    /// Computes the hash of the node.
    ///
    /// If the encoded node is >= 32 bytes, returns the Keccak-256 hash.
    /// If the encoded node is < 32 bytes, the node is embedded inline.
    pub fn hash(&self) -> NodeHash {
        let encoded = self.encode();

        if encoded.len() >= HASH_SIZE {
            let mut hasher = Keccak::v256();
            hasher.update(&encoded);
            let mut hash = [0u8; HASH_SIZE];
            hasher.finalize(&mut hash);
            NodeHash::Hash(hash)
        } else {
            NodeHash::Inline(encoded)
        }
    }

    /// Computes the Keccak-256 hash of the node (always returns a hash).
    pub fn keccak(&self) -> [u8; HASH_SIZE] {
        let encoded = self.encode();
        let mut hasher = Keccak::v256();
        hasher.update(&encoded);
        let mut hash = [0u8; HASH_SIZE];
        hasher.finalize(&mut hash);
        hash
    }
}

/// Result of hashing a node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NodeHash {
    /// The hash of the encoded node.
    Hash([u8; HASH_SIZE]),
    /// The node is small enough to be embedded inline.
    Inline(Vec<u8>),
}

impl NodeHash {
    /// Returns the hash bytes (for inline nodes, returns the keccak of the inline data).
    pub fn as_hash(&self) -> [u8; HASH_SIZE] {
        match self {
            NodeHash::Hash(h) => *h,
            NodeHash::Inline(data) => {
                let mut hasher = Keccak::v256();
                hasher.update(data);
                let mut hash = [0u8; HASH_SIZE];
                hasher.finalize(&mut hash);
                hash
            }
        }
    }

    /// Returns true if this is a full hash (not inline).
    pub fn is_hash(&self) -> bool {
        matches!(self, NodeHash::Hash(_))
    }
}

/// Computes Keccak-256 hash of data.
pub fn keccak256(data: &[u8]) -> [u8; HASH_SIZE] {
    let mut hasher = Keccak::v256();
    hasher.update(data);
    let mut hash = [0u8; HASH_SIZE];
    hasher.finalize(&mut hash);
    hash
}

/// The empty trie root hash (keccak of RLP empty string).
pub const EMPTY_ROOT: [u8; HASH_SIZE] = [
    0x56, 0xe8, 0x1f, 0x17, 0x1b, 0xcc, 0x55, 0xa6,
    0xff, 0x83, 0x45, 0xe6, 0x92, 0xc0, 0xf8, 0x6e,
    0x5b, 0x48, 0xe0, 0x1b, 0x99, 0x6c, 0xad, 0xc0,
    0x01, 0x62, 0x2f, 0xb5, 0xe3, 0x63, 0xb4, 0x21,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_node() {
        let node = Node::empty();
        assert_eq!(node.node_type(), NodeType::Empty);
    }

    #[test]
    fn test_leaf_node() {
        let node = Node::leaf(vec![1, 2, 3], vec![0xAB, 0xCD]);
        assert_eq!(node.node_type(), NodeType::Leaf);

        let encoded = node.encode();
        assert!(!encoded.is_empty());
    }

    #[test]
    fn test_branch_node() {
        let mut node = Node::branch();
        if let Node::Branch { children, value } = &mut node {
            children[0] = ChildRef::Hash([0u8; 32]);
            *value = Some(vec![0x42]);
        }

        assert_eq!(node.node_type(), NodeType::Branch);
    }

    #[test]
    fn test_empty_root_hash() {
        // Empty trie root is keccak256(RLP(""))
        let hash = keccak256(&[0x80]);
        assert_eq!(hash, EMPTY_ROOT);
    }

    #[test]
    fn test_node_hash() {
        let node = Node::leaf(vec![1, 2, 3, 4, 5, 6, 7, 8], vec![0u8; 100]);
        let hash = node.hash();
        assert!(hash.is_hash());
    }
}
