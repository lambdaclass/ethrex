use ethereum_types::H256;
use ethrex_rlp::error::RLPDecodeError;
use thiserror::Error;

use crate::Nibbles;

#[derive(Debug, Error)]
pub enum TrieError {
    #[error(transparent)]
    RLPDecode(#[from] RLPDecodeError),
    #[error("Verification Error: {0}")]
    Verify(String),
    #[error("Inconsistent internal tree structure: {0}")]
    InconsistentTree(Box<InconsistentTreeError>),
    // Box was added to make the error smaller since the InconsistentTreeError variants size vary up to more than 168 bytes.
    #[error("Lock Error: Panicked when trying to acquire a lock")]
    LockError,
    #[error("Database error: {0}")]
    DbError(anyhow::Error),
    #[error("Invalid trie input")]
    InvalidInput,
}

#[derive(Debug, Error)]
pub enum InconsistentTreeError {
    #[error("Child node of {0}, differs from expected")]
    ExtensionNodeChildDiffers(ExtensionNodeErrorData),
    #[error("No Child Node found of {0}")]
    ExtensionNodeChildNotFound(ExtensionNodeErrorData),
    #[error("Node with hash {0:#x} not found in Branch Node with hash {1:#x} using path {2:?}")]
    NodeNotFoundOnBranchNode(H256, H256, Nibbles),
    #[error("Root node with hash {0:#x} not found")]
    RootNotFound(H256),
    #[error("Root node not found")]
    RootNotFoundNoHash,
    #[error("Node count mismatch during validation: {0}")]
    NodeCountMismatch(NodeCountMismatchData),
}

#[derive(Debug)]
pub struct ExtensionNodeErrorData {
    pub node_hash: H256,
    pub extension_node_hash: H256,
    pub extension_node_prefix: Nibbles,
    pub node_path: Nibbles,
}

impl std::fmt::Display for ExtensionNodeErrorData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Node with hash {:#x}, child of the Extension Node (hash {:#x}, prefix {:?}) on path {:?}",
            self.node_hash, self.extension_node_hash, self.extension_node_hash, self.node_path
        )
    }
}

#[derive(Debug, Clone, Copy)]
pub enum NodeType {
    Branch,
    Extension,
    Leaf,
}

impl std::fmt::Display for NodeType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NodeType::Branch => write!(f, "Branch"),
            NodeType::Extension => write!(f, "Extension"),
            NodeType::Leaf => write!(f, "Leaf"),
        }
    }
}

#[derive(Debug)]
pub struct NodeCountMismatchData {
    /// The difference between expected and actual node counts.
    /// Positive = missing nodes, negative = extra nodes
    pub count_difference: isize,
    /// Path to the last successfully validated node
    pub last_valid_path: Option<Nibbles>,
    pub last_node_type: Option<NodeType>,
    pub nodes_traversed: usize,
    /// Hash of the last validated node (for cross-referencing)
    pub last_node_hash: Option<H256>,
}

impl std::fmt::Display for NodeCountMismatchData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mismatch = if self.count_difference > 0 {
            format!("Expected {} more node(s)", self.count_difference)
        } else {
            format!("Found {} unexpected node(s)", self.count_difference.abs())
        };
        let node_type = self
            .last_node_type
            .map(|t| t.to_string())
            .unwrap_or_else(|| "unknown".to_string());
        let hash = self
            .last_node_hash
            .as_ref()
            .map(|h| format!("{:#x}", h))
            .unwrap_or_else(|| "unknown".to_string());

        write!(
            f,
            "{}. Traversed {} nodes. Last valid node: {} at path {:?} (hash: {})",
            mismatch, self.nodes_traversed, node_type, self.last_valid_path, hash
        )
    }
}
