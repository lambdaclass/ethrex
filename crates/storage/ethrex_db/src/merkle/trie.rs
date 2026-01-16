//! Merkle Patricia Trie implementation.

use std::collections::HashMap;
use thiserror::Error;

use super::node::{Node, NodeHash, HASH_SIZE, EMPTY_ROOT, keccak256};

/// Trie errors.
#[derive(Error, Debug)]
pub enum TrieError {
    #[error("Key not found")]
    NotFound,
    #[error("Invalid node")]
    InvalidNode,
    #[error("Corrupted trie")]
    Corrupted,
}

/// A simple in-memory Merkle Patricia Trie.
///
/// This implementation stores all nodes in memory and recomputes
/// the root hash when requested.
pub struct MerkleTrie {
    /// Key-value store (key bytes -> value bytes).
    data: HashMap<Vec<u8>, Vec<u8>>,
    /// Cached root hash (invalidated on changes).
    root_cache: Option<[u8; HASH_SIZE]>,
}

impl MerkleTrie {
    /// Creates a new empty trie.
    pub fn new() -> Self {
        Self {
            data: HashMap::new(),
            root_cache: Some(EMPTY_ROOT),
        }
    }

    /// Returns true if the trie is empty.
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Returns the number of entries.
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Inserts a key-value pair.
    pub fn insert(&mut self, key: &[u8], value: Vec<u8>) {
        if value.is_empty() {
            self.data.remove(key);
        } else {
            self.data.insert(key.to_vec(), value);
        }
        self.root_cache = None;
    }

    /// Gets a value by key.
    pub fn get(&self, key: &[u8]) -> Option<&[u8]> {
        self.data.get(key).map(|v| v.as_slice())
    }

    /// Removes a key.
    pub fn remove(&mut self, key: &[u8]) -> Option<Vec<u8>> {
        let result = self.data.remove(key);
        if result.is_some() {
            self.root_cache = None;
        }
        result
    }

    /// Computes and returns the root hash.
    pub fn root_hash(&mut self) -> [u8; HASH_SIZE] {
        if let Some(cached) = self.root_cache {
            return cached;
        }

        let hash = self.compute_root();
        self.root_cache = Some(hash);
        hash
    }

    /// Computes the root hash without caching.
    fn compute_root(&self) -> [u8; HASH_SIZE] {
        if self.data.is_empty() {
            return EMPTY_ROOT;
        }

        // Convert keys to nibble paths
        let mut entries: Vec<(Vec<u8>, &[u8])> = self.data
            .iter()
            .map(|(k, v)| {
                let nibbles = key_to_nibbles(k);
                (nibbles, v.as_slice())
            })
            .collect();

        // Sort by nibble path for deterministic ordering
        entries.sort_by(|a, b| a.0.cmp(&b.0));

        // Build trie recursively
        let node = self.build_node(&entries, 0);
        node.keccak()
    }

    /// Builds a trie node from sorted entries at the given nibble depth.
    fn build_node(&self, entries: &[(Vec<u8>, &[u8])], depth: usize) -> Node {
        if entries.is_empty() {
            return Node::Empty;
        }

        if entries.len() == 1 {
            // Single entry - create a leaf
            let (nibbles, value) = &entries[0];
            let remaining: Vec<u8> = nibbles[depth..].to_vec();
            return Node::leaf(remaining, value.to_vec());
        }

        // Check for common prefix
        let common_prefix = self.find_common_prefix(entries, depth);

        if common_prefix > 0 {
            // Create extension node
            let prefix: Vec<u8> = entries[0].0[depth..depth + common_prefix].to_vec();
            let child_node = self.build_node(entries, depth + common_prefix);
            let child_hash = child_node.keccak();
            return Node::extension(prefix, child_hash);
        }

        // Create branch node
        let mut children: Box<[Option<[u8; HASH_SIZE]>; 16]> = Box::new([None; 16]);
        let mut branch_value: Option<Vec<u8>> = None;

        // Group entries by first nibble at current depth
        let mut groups: [Vec<(Vec<u8>, &[u8])>; 16] = Default::default();

        for (nibbles, value) in entries {
            if depth >= nibbles.len() {
                // This entry's key ends here - it's the branch value
                branch_value = Some(value.to_vec());
            } else {
                let nibble = nibbles[depth] as usize;
                groups[nibble].push((nibbles.clone(), *value));
            }
        }

        // Build child nodes
        for (i, group) in groups.iter().enumerate() {
            if !group.is_empty() {
                let child_node = self.build_node(group, depth + 1);
                match child_node.hash() {
                    NodeHash::Hash(h) => children[i] = Some(h),
                    NodeHash::Inline(data) => {
                        // For inline nodes, hash the data
                        children[i] = Some(keccak256(&data));
                    }
                }
            }
        }

        Node::Branch {
            children,
            value: branch_value,
        }
    }

    /// Finds the length of the common prefix among entries starting at the given depth.
    fn find_common_prefix(&self, entries: &[(Vec<u8>, &[u8])], depth: usize) -> usize {
        if entries.is_empty() {
            return 0;
        }

        let first = &entries[0].0;
        if depth >= first.len() {
            return 0;
        }

        let mut common_len = first.len() - depth;

        for (nibbles, _) in &entries[1..] {
            let max_check = (nibbles.len() - depth).min(common_len);
            let mut prefix_len = 0;

            for i in 0..max_check {
                if nibbles[depth + i] == first[depth + i] {
                    prefix_len += 1;
                } else {
                    break;
                }
            }

            common_len = prefix_len;
            if common_len == 0 {
                break;
            }
        }

        common_len
    }

    /// Iterates over all key-value pairs.
    pub fn iter(&self) -> impl Iterator<Item = (&[u8], &[u8])> {
        self.data.iter().map(|(k, v)| (k.as_slice(), v.as_slice()))
    }
}

impl Default for MerkleTrie {
    fn default() -> Self {
        Self::new()
    }
}

/// Converts a key (bytes) to nibbles.
fn key_to_nibbles(key: &[u8]) -> Vec<u8> {
    let mut nibbles = Vec::with_capacity(key.len() * 2);
    for byte in key {
        nibbles.push(byte >> 4);
        nibbles.push(byte & 0x0F);
    }
    nibbles
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_trie() {
        let mut trie = MerkleTrie::new();
        assert!(trie.is_empty());
        assert_eq!(trie.root_hash(), EMPTY_ROOT);
    }

    #[test]
    fn test_single_entry() {
        let mut trie = MerkleTrie::new();
        trie.insert(b"key", b"value".to_vec());

        assert!(!trie.is_empty());
        assert_eq!(trie.get(b"key"), Some(b"value".as_slice()));

        let hash = trie.root_hash();
        assert_ne!(hash, EMPTY_ROOT);
    }

    #[test]
    fn test_multiple_entries() {
        let mut trie = MerkleTrie::new();
        trie.insert(b"do", b"verb".to_vec());
        trie.insert(b"dog", b"puppy".to_vec());
        trie.insert(b"doge", b"coin".to_vec());
        trie.insert(b"horse", b"stallion".to_vec());

        assert_eq!(trie.len(), 4);
        assert_eq!(trie.get(b"dog"), Some(b"puppy".as_slice()));
        assert_eq!(trie.get(b"horse"), Some(b"stallion".as_slice()));

        let hash = trie.root_hash();
        assert_ne!(hash, EMPTY_ROOT);
    }

    #[test]
    fn test_remove() {
        let mut trie = MerkleTrie::new();
        trie.insert(b"key1", b"value1".to_vec());
        trie.insert(b"key2", b"value2".to_vec());

        let hash1 = trie.root_hash();

        trie.remove(b"key2");
        assert_eq!(trie.get(b"key2"), None);

        let hash2 = trie.root_hash();
        assert_ne!(hash1, hash2);
    }

    #[test]
    fn test_deterministic_hash() {
        let mut trie1 = MerkleTrie::new();
        trie1.insert(b"a", b"1".to_vec());
        trie1.insert(b"b", b"2".to_vec());

        let mut trie2 = MerkleTrie::new();
        // Insert in different order
        trie2.insert(b"b", b"2".to_vec());
        trie2.insert(b"a", b"1".to_vec());

        assert_eq!(trie1.root_hash(), trie2.root_hash());
    }

    #[test]
    fn test_key_to_nibbles() {
        let nibbles = key_to_nibbles(&[0xAB, 0xCD]);
        assert_eq!(nibbles, vec![0xA, 0xB, 0xC, 0xD]);
    }

    #[test]
    fn test_update_value() {
        let mut trie = MerkleTrie::new();
        trie.insert(b"key", b"value1".to_vec());
        let hash1 = trie.root_hash();

        trie.insert(b"key", b"value2".to_vec());
        let hash2 = trie.root_hash();

        assert_ne!(hash1, hash2);
        assert_eq!(trie.get(b"key"), Some(b"value2".as_slice()));
    }
}
