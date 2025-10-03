use ethereum_types::H256;

use crate::{Nibbles, db::nibbles_to_fixed_size};

/// Struct representing the key of a node
#[derive(Debug, Clone, Default)]
pub struct NodeKey {
    pub nibble: Nibbles,
    pub hash: H256,
}

impl NodeKey {
    pub fn to_fixed_size(&self) -> [u8; 65] {
        let nibble = nibbles_to_fixed_size(self.nibble.clone());
        let hash = self.hash.0;
        let mut fixed_size = [0u8; 65];
        fixed_size[0..33].copy_from_slice(&nibble);
        fixed_size[33..65].copy_from_slice(&hash);
        fixed_size
    }
}

impl PartialEq for NodeKey {
    fn eq(&self, other: &Self) -> bool {
        self.nibble == other.nibble && self.hash == other.hash
    }
}

impl std::hash::Hash for NodeKey {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.nibble.hash(state);
        self.hash.hash(state);
    }
}

impl Eq for NodeKey {}

impl PartialOrd for NodeKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for NodeKey {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match self.nibble.cmp(&other.nibble) {
            std::cmp::Ordering::Equal => self.hash.cmp(&other.hash),
            ord => ord,
        }
    }
}
