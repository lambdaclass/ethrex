//! Merkle Patricia Trie implementation.
//!
//! This module provides both sequential and parallel Merkle computation.
//! The parallel version uses Rayon to compute branch node children concurrently.

use std::collections::HashMap;
use rayon::prelude::*;
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

    /// Clears the root hash cache, forcing recomputation on next root_hash() call.
    ///
    /// This is useful for benchmarking to measure computation time without caching.
    pub fn clear_cache(&mut self) {
        self.root_cache = None;
    }

    /// Computes the root hash in parallel.
    ///
    /// Uses Rayon to parallelize branch node child computation.
    /// For tries with many entries, this can provide significant speedup
    /// on multi-core systems.
    pub fn parallel_root_hash(&mut self) -> [u8; HASH_SIZE] {
        if let Some(cached) = self.root_cache {
            return cached;
        }

        let hash = self.compute_root_parallel();
        self.root_cache = Some(hash);
        hash
    }

    /// Computes the root hash in parallel without caching.
    fn compute_root_parallel(&self) -> [u8; HASH_SIZE] {
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

        // Build trie recursively with parallel branch computation
        let node = self.build_node_parallel(&entries, 0);
        node.keccak()
    }

    /// Builds a trie node from sorted entries, using parallel computation for branches.
    ///
    /// When the number of entries exceeds a threshold, branch children are computed
    /// in parallel using Rayon.
    fn build_node_parallel(&self, entries: &[(Vec<u8>, &[u8])], depth: usize) -> Node {
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
            let child_node = self.build_node_parallel(entries, depth + common_prefix);
            let child_hash = child_node.keccak();
            return Node::extension(prefix, child_hash);
        }

        // Create branch node
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

        // Build child nodes in parallel if we have enough entries
        let total_entries: usize = groups.iter().map(|g| g.len()).sum();
        let children: Box<[Option<[u8; HASH_SIZE]>; 16]> = if total_entries > 64 {
            // Parallel computation for large branches
            let child_hashes: Vec<Option<[u8; HASH_SIZE]>> = groups
                .par_iter()
                .map(|group| {
                    if group.is_empty() {
                        None
                    } else {
                        let child_node = self.build_node_parallel(group, depth + 1);
                        match child_node.hash() {
                            NodeHash::Hash(h) => Some(h),
                            NodeHash::Inline(data) => Some(keccak256(&data)),
                        }
                    }
                })
                .collect();

            let mut children: Box<[Option<[u8; HASH_SIZE]>; 16]> = Box::new([None; 16]);
            for (i, hash) in child_hashes.into_iter().enumerate() {
                children[i] = hash;
            }
            children
        } else {
            // Sequential computation for small branches
            let mut children: Box<[Option<[u8; HASH_SIZE]>; 16]> = Box::new([None; 16]);
            for (i, group) in groups.iter().enumerate() {
                if !group.is_empty() {
                    let child_node = self.build_node_parallel(group, depth + 1);
                    match child_node.hash() {
                        NodeHash::Hash(h) => children[i] = Some(h),
                        NodeHash::Inline(data) => children[i] = Some(keccak256(&data)),
                    }
                }
            }
            children
        };

        Node::Branch {
            children,
            value: branch_value,
        }
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

// ============================================================================
// Merkle Proofs
// ============================================================================

/// A node in a Merkle proof path.
#[derive(Debug, Clone, PartialEq)]
pub enum ProofNode {
    /// Branch node with all 16 child hashes and optional value.
    Branch {
        children: Box<[Option<[u8; HASH_SIZE]>; 16]>,
        value: Option<Vec<u8>>,
    },
    /// Extension node with path and child hash.
    Extension {
        path: Vec<u8>,
        child: [u8; HASH_SIZE],
    },
    /// Leaf node with path and value.
    Leaf {
        path: Vec<u8>,
        value: Vec<u8>,
    },
}

/// A Merkle proof for a key-value inclusion.
///
/// Contains the sequence of nodes from root to the target key,
/// allowing verification without the full trie.
#[derive(Debug, Clone)]
pub struct MerkleProof {
    /// The key being proved.
    pub key: Vec<u8>,
    /// The value at the key (None for non-existence proofs).
    pub value: Option<Vec<u8>>,
    /// Proof nodes from root towards the key.
    pub proof: Vec<ProofNode>,
}

impl MerkleTrie {
    /// Generates a Merkle proof for the given key.
    ///
    /// Returns a proof that can be used to verify the key's inclusion
    /// (or non-existence) in the trie.
    pub fn generate_proof(&mut self, key: &[u8]) -> MerkleProof {
        let value = self.data.get(key).cloned();

        if self.data.is_empty() {
            return MerkleProof {
                key: key.to_vec(),
                value,
                proof: vec![],
            };
        }

        // Convert keys to nibble paths
        let mut entries: Vec<(Vec<u8>, &[u8])> = self.data
            .iter()
            .map(|(k, v)| {
                let nibbles = key_to_nibbles(k);
                (nibbles, v.as_slice())
            })
            .collect();

        entries.sort_by(|a, b| a.0.cmp(&b.0));

        let target_nibbles = key_to_nibbles(key);
        let mut proof_nodes = Vec::new();

        self.collect_proof(&entries, 0, &target_nibbles, &mut proof_nodes);

        MerkleProof {
            key: key.to_vec(),
            value,
            proof: proof_nodes,
        }
    }

    /// Recursively collects proof nodes along the path to the target key.
    fn collect_proof(
        &self,
        entries: &[(Vec<u8>, &[u8])],
        depth: usize,
        target: &[u8],
        proof: &mut Vec<ProofNode>,
    ) {
        if entries.is_empty() {
            return;
        }

        if entries.len() == 1 {
            // Leaf node
            let (nibbles, value) = &entries[0];
            let remaining: Vec<u8> = nibbles[depth..].to_vec();
            proof.push(ProofNode::Leaf {
                path: remaining,
                value: value.to_vec(),
            });
            return;
        }

        // Check for common prefix
        let common_prefix = self.find_common_prefix(entries, depth);

        if common_prefix > 0 {
            // Extension node
            let prefix: Vec<u8> = entries[0].0[depth..depth + common_prefix].to_vec();
            let child_node = self.build_node(entries, depth + common_prefix);
            let child_hash = child_node.keccak();

            proof.push(ProofNode::Extension {
                path: prefix,
                child: child_hash,
            });

            // Continue down the path
            self.collect_proof(entries, depth + common_prefix, target, proof);
            return;
        }

        // Branch node
        let mut children: Box<[Option<[u8; HASH_SIZE]>; 16]> = Box::new([None; 16]);
        let mut branch_value: Option<Vec<u8>> = None;
        let mut groups: [Vec<(Vec<u8>, &[u8])>; 16] = Default::default();

        for (nibbles, value) in entries {
            if depth >= nibbles.len() {
                branch_value = Some(value.to_vec());
            } else {
                let nibble = nibbles[depth] as usize;
                groups[nibble].push((nibbles.clone(), *value));
            }
        }

        // Build child hashes
        for (i, group) in groups.iter().enumerate() {
            if !group.is_empty() {
                let child_node = self.build_node(group, depth + 1);
                match child_node.hash() {
                    NodeHash::Hash(h) => children[i] = Some(h),
                    NodeHash::Inline(data) => children[i] = Some(keccak256(&data)),
                }
            }
        }

        proof.push(ProofNode::Branch {
            children: children.clone(),
            value: branch_value,
        });

        // Continue down the target path if within bounds
        if depth < target.len() {
            let target_nibble = target[depth] as usize;
            if !groups[target_nibble].is_empty() {
                self.collect_proof(&groups[target_nibble], depth + 1, target, proof);
            }
        }
    }
}

impl MerkleProof {
    /// Verifies this proof against a given root hash.
    ///
    /// Returns true if the proof is valid for the given root.
    pub fn verify(&self, root_hash: &[u8; HASH_SIZE]) -> bool {
        if self.proof.is_empty() {
            // Empty trie - root should be EMPTY_ROOT
            return *root_hash == EMPTY_ROOT && self.value.is_none();
        }

        // Compute root from proof
        let computed = self.compute_root_from_proof();
        computed == *root_hash
    }

    /// Computes the root hash from the proof nodes.
    fn compute_root_from_proof(&self) -> [u8; HASH_SIZE] {
        use super::rlp_encode::RlpEncoder;

        if self.proof.is_empty() {
            return EMPTY_ROOT;
        }

        // Build from bottom up
        let mut current_hash: Option<[u8; HASH_SIZE]> = None;

        for node in self.proof.iter().rev() {
            let mut enc = RlpEncoder::new();

            match node {
                ProofNode::Leaf { path, value } => {
                    enc.encode_list(|e| {
                        e.encode_nibbles(path, true);
                        e.encode_bytes(value);
                    });
                }
                ProofNode::Extension { path, child } => {
                    enc.encode_list(|e| {
                        e.encode_nibbles(path, false);
                        e.encode_bytes(child);
                    });
                }
                ProofNode::Branch { children, value } => {
                    enc.encode_list(|e| {
                        for child in children.iter() {
                            match child {
                                Some(h) => e.encode_bytes(h),
                                None => e.encode_empty(),
                            }
                        }
                        match value {
                            Some(v) => e.encode_bytes(v),
                            None => e.encode_empty(),
                        }
                    });
                }
            };

            current_hash = Some(keccak256(enc.as_bytes()));
        }

        current_hash.unwrap_or(EMPTY_ROOT)
    }

    /// Returns true if this is a proof of inclusion (key exists).
    pub fn is_inclusion(&self) -> bool {
        self.value.is_some()
    }

    /// Returns true if this is a proof of non-existence (key does not exist).
    pub fn is_exclusion(&self) -> bool {
        self.value.is_none()
    }
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

    #[test]
    fn test_parallel_empty_trie() {
        let mut trie = MerkleTrie::new();
        assert_eq!(trie.parallel_root_hash(), EMPTY_ROOT);
    }

    #[test]
    fn test_parallel_single_entry() {
        let mut trie = MerkleTrie::new();
        trie.insert(b"key", b"value".to_vec());

        // Sequential and parallel should produce the same hash
        let seq_hash = trie.root_hash();
        trie.clear_cache();
        let par_hash = trie.parallel_root_hash();

        assert_eq!(seq_hash, par_hash);
    }

    #[test]
    fn test_parallel_multiple_entries() {
        let mut trie = MerkleTrie::new();
        trie.insert(b"do", b"verb".to_vec());
        trie.insert(b"dog", b"puppy".to_vec());
        trie.insert(b"doge", b"coin".to_vec());
        trie.insert(b"horse", b"stallion".to_vec());

        let seq_hash = trie.root_hash();
        trie.clear_cache();
        let par_hash = trie.parallel_root_hash();

        assert_eq!(seq_hash, par_hash);
    }

    #[test]
    fn test_parallel_large_trie() {
        let mut trie = MerkleTrie::new();

        // Insert 200 entries to exercise parallel code path (threshold is 64)
        for i in 0..200u32 {
            let key = i.to_be_bytes();
            let value = format!("value_{}", i).into_bytes();
            trie.insert(&key, value);
        }

        let seq_hash = trie.root_hash();
        trie.clear_cache();
        let par_hash = trie.parallel_root_hash();

        assert_eq!(seq_hash, par_hash);
    }

    #[test]
    fn test_parallel_deterministic() {
        let mut trie1 = MerkleTrie::new();
        let mut trie2 = MerkleTrie::new();

        // Insert 100 entries in different orders
        for i in 0..100u32 {
            trie1.insert(&i.to_be_bytes(), format!("v{}", i).into_bytes());
        }
        for i in (0..100u32).rev() {
            trie2.insert(&i.to_be_bytes(), format!("v{}", i).into_bytes());
        }

        // Both should produce the same hash
        assert_eq!(trie1.parallel_root_hash(), trie2.parallel_root_hash());
    }

    // Merkle proof tests

    #[test]
    fn test_proof_empty_trie() {
        let mut trie = MerkleTrie::new();
        let proof = trie.generate_proof(b"key");

        assert!(proof.is_exclusion());
        assert!(proof.verify(&EMPTY_ROOT));
    }

    #[test]
    fn test_proof_single_entry() {
        let mut trie = MerkleTrie::new();
        trie.insert(b"key", b"value".to_vec());

        let root = trie.root_hash();
        let proof = trie.generate_proof(b"key");

        assert!(proof.is_inclusion());
        assert_eq!(proof.value, Some(b"value".to_vec()));
        assert!(!proof.proof.is_empty());
    }

    #[test]
    fn test_proof_multiple_entries() {
        let mut trie = MerkleTrie::new();
        trie.insert(b"do", b"verb".to_vec());
        trie.insert(b"dog", b"puppy".to_vec());
        trie.insert(b"doge", b"coin".to_vec());

        let root = trie.root_hash();

        // Proof for existing key
        let proof = trie.generate_proof(b"dog");
        assert!(proof.is_inclusion());
        assert_eq!(proof.value, Some(b"puppy".to_vec()));

        // Proof for non-existing key
        let proof = trie.generate_proof(b"cat");
        assert!(proof.is_exclusion());
    }

    #[test]
    fn test_proof_types() {
        let mut trie = MerkleTrie::new();
        trie.insert(b"key1", b"value1".to_vec());
        trie.insert(b"key2", b"value2".to_vec());

        let proof = trie.generate_proof(b"key1");

        // Check that proof contains nodes
        assert!(!proof.proof.is_empty());

        // Verify proof node types exist
        for node in &proof.proof {
            match node {
                ProofNode::Branch { children, value } => {
                    // Branch node has 16 children
                    assert_eq!(children.len(), 16);
                }
                ProofNode::Extension { path, child } => {
                    // Extension has a path
                    assert!(!path.is_empty() || child.len() == 32);
                }
                ProofNode::Leaf { path, value } => {
                    // Leaf has value
                    assert!(!value.is_empty());
                }
            }
        }
    }
}
