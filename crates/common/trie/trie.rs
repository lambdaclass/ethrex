pub mod db;
pub mod error;
pub mod logger;
mod nibbles;
pub mod node;
mod node_hash;
pub mod rkyv_utils;
mod rlp;
#[cfg(test)]
mod test_utils;
pub mod threadpool;
mod trie_iter;
pub mod trie_sorted;
mod verify_range;
use ethereum_types::H256;
use ethrex_crypto::keccak::keccak_hash;
use ethrex_rlp::constants::RLP_NULL;
use ethrex_rlp::encode::RLPEncode;
use rustc_hash::FxHashSet;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

pub use self::db::{InMemoryTrieDB, TrieDB};
pub use self::logger::{TrieLogger, TrieWitness};
pub use self::nibbles::Nibbles;
pub use self::threadpool::ThreadPool;
pub use self::verify_range::verify_range;
pub use self::{
    node::{Node, NodeRef},
    node_hash::NodeHash,
};

pub use self::error::{ExtensionNodeErrorData, InconsistentTreeError, TrieError};
use self::{node::LeafNode, trie_iter::TrieIterator};

use ethrex_rlp::decode::RLPDecode;
use lazy_static::lazy_static;

lazy_static! {
    // Hash value for an empty trie, equal to keccak(RLP_NULL)
    pub static ref EMPTY_TRIE_HASH: H256 = H256(
        keccak_hash([RLP_NULL]),
    );
}

/// RLP-encoded trie path
pub type PathRLP = Vec<u8>;
/// RLP-encoded trie value
pub type ValueRLP = Vec<u8>;
/// RLP-encoded trie node
pub type NodeRLP = Vec<u8>;
/// Represents a node in the Merkle Patricia Trie.
pub type TrieNode = (Nibbles, NodeRLP);

/// Ethereum-compatible Merkle Patricia Trie
pub struct Trie {
    db: Box<dyn TrieDB>,
    pub root: NodeRef,
    pending_removal: FxHashSet<Nibbles>,
    dirty: FxHashSet<Nibbles>,
}

impl Default for Trie {
    fn default() -> Self {
        Self::new_temp()
    }
}

impl Trie {
    /// Creates a new Trie from a clean DB
    pub fn new(db: Box<dyn TrieDB>) -> Self {
        Self {
            db,
            root: NodeRef::default(),
            pending_removal: Default::default(),
            dirty: Default::default(),
        }
    }

    /// Creates a trie from an already-initialized DB and sets root as the root node of the trie
    pub fn open(db: Box<dyn TrieDB>, root: H256) -> Self {
        Self {
            db,
            root: if root != *EMPTY_TRIE_HASH {
                NodeHash::from(root).into()
            } else {
                Default::default()
            },
            pending_removal: Default::default(),
            dirty: Default::default(),
        }
    }

    /// Return a reference to the internal database.
    ///
    /// Warning: All changes made to the db will bypass the trie and may cause the trie to suddenly
    ///   become inconsistent.
    pub fn db(&self) -> &dyn TrieDB {
        self.db.as_ref()
    }

    /// Retrieve an RLP-encoded value from the trie given its RLP-encoded path.
    pub fn get(&self, pathrlp: &[u8]) -> Result<Option<ValueRLP>, TrieError> {
        let path = Nibbles::from_bytes(pathrlp);

        if !self.dirty.contains(&path) && self.db().flatkeyvalue_computed(path.clone()) {
            let Some(value_rlp) = self.db.get(path)? else {
                return Ok(None);
            };
            if value_rlp.is_empty() {
                return Ok(None);
            }
            return Ok(Some(value_rlp));
        }

        Ok(match self.root {
            NodeRef::Node(ref node, _) => node.get(self.db.as_ref(), path)?,
            NodeRef::Hash(hash) if hash.is_valid() => {
                Node::decode(&self.db.get(Nibbles::default())?.ok_or_else(|| {
                    TrieError::InconsistentTree(Box::new(InconsistentTreeError::RootNotFound(
                        hash.finalize(),
                    )))
                })?)
                .map_err(TrieError::RLPDecode)?
                .get(self.db.as_ref(), path)?
            }
            _ => None,
        })
    }

    /// Insert an RLP-encoded value into the trie.
    pub fn insert(&mut self, path: PathRLP, value: ValueRLP) -> Result<(), TrieError> {
        let path = Nibbles::from_bytes(&path);
        self.pending_removal.remove(&path);
        self.dirty.insert(path.clone());

        if self.root.is_valid() {
            // If the trie is not empty, call the root node's insertion logic.
            self.root
                .get_node_mut(self.db.as_ref(), Nibbles::default())?
                .ok_or_else(|| {
                    TrieError::InconsistentTree(Box::new(InconsistentTreeError::RootNotFoundNoHash))
                })?
                .insert(self.db.as_ref(), path, value)?
        } else {
            // If the trie is empty, just add a leaf.
            self.root = Node::from(LeafNode::new(path, value)).into()
        };
        self.root.clear_hash();

        Ok(())
    }

    /// Insert a batch of (path, value) pairs into the trie. The pairs are sorted by path
    /// internally for optimal performance. Empty values are treated as removals.
    /// When duplicate keys exist, the last entry wins (preserving input order semantics).
    /// This method groups DB reads efficiently by loading each intermediate node at most once.
    pub fn insert_batch_sorted(
        &mut self,
        updates: Vec<(PathRLP, ValueRLP)>,
    ) -> Result<(), TrieError> {
        if updates.is_empty() {
            return Ok(());
        }

        // Deduplicate: keep only the last operation for each key (last-write-wins)
        let mut deduped: BTreeMap<PathRLP, ValueRLP> = BTreeMap::new();
        for (path, value) in updates {
            deduped.insert(path, value);
        }

        // Separate removes (empty value) from inserts
        let mut removes: Vec<PathRLP> = Vec::new();
        let mut inserts: Vec<(PathRLP, ValueRLP)> = Vec::new();
        for (path, value) in deduped {
            if value.is_empty() {
                removes.push(path);
            } else {
                inserts.push((path, value));
            }
        }

        // Process removes first via existing remove logic
        for path in &removes {
            self.remove(path)?;
        }

        if inserts.is_empty() {
            return Ok(());
        }

        // Convert to Nibbles and sort
        let mut nibble_updates: Vec<(Nibbles, ValueRLP)> = inserts
            .into_iter()
            .map(|(path, value)| (Nibbles::from_bytes(&path), value))
            .collect();
        nibble_updates.sort_by(|(a, _), (b, _)| a.cmp(b));

        // Mark all paths as dirty and remove from pending_removal
        for (path, _) in &nibble_updates {
            self.pending_removal.remove(path);
            self.dirty.insert(path.clone());
        }

        if self.root.is_valid() {
            self.root
                .get_node_mut(self.db.as_ref(), Nibbles::default())?
                .ok_or_else(|| {
                    TrieError::InconsistentTree(Box::new(InconsistentTreeError::RootNotFoundNoHash))
                })?
                .insert_batch(self.db.as_ref(), &nibble_updates)?;
        } else {
            // Empty trie: create first leaf, then batch insert the rest
            let (first_path, first_value) = nibble_updates.remove(0);
            self.root = Node::from(LeafNode::new(first_path, first_value)).into();
            if !nibble_updates.is_empty() {
                self.root
                    .get_node_mut(self.db.as_ref(), Nibbles::default())?
                    .ok_or_else(|| {
                        TrieError::InconsistentTree(Box::new(
                            InconsistentTreeError::RootNotFoundNoHash,
                        ))
                    })?
                    .insert_batch(self.db.as_ref(), &nibble_updates)?;
            }
        }
        self.root.clear_hash();

        Ok(())
    }

    /// Remove a value from the trie given its RLP-encoded path.
    /// Returns the value if it was succesfully removed or None if it wasn't part of the trie
    pub fn remove(&mut self, path: &[u8]) -> Result<Option<ValueRLP>, TrieError> {
        self.dirty.insert(Nibbles::from_bytes(path));
        if !self.root.is_valid() {
            return Ok(None);
        }
        self.pending_removal.insert(Nibbles::from_bytes(path));

        // If the trie is not empty, call the root node's removal logic.
        let (is_trie_empty, value) = self
            .root
            .get_node_mut(self.db.as_ref(), Nibbles::default())?
            .ok_or_else(|| {
                TrieError::InconsistentTree(Box::new(InconsistentTreeError::RootNotFoundNoHash))
            })?
            .remove(self.db.as_ref(), Nibbles::from_bytes(path))?;
        if is_trie_empty {
            self.root = NodeRef::default();
        } else {
            self.root.clear_hash();
        }

        Ok(value)
    }

    /// Return the hash of the trie's root node.
    /// Returns keccak(RLP_NULL) if the trie is empty
    /// Also commits changes to the DB
    pub fn hash(&mut self) -> Result<H256, TrieError> {
        self.commit()?;
        Ok(self.hash_no_commit())
    }

    /// Return the hash of the trie's root node.
    /// Returns keccak(RLP_NULL) if the trie is empty
    pub fn hash_no_commit(&self) -> H256 {
        if self.root.is_valid() {
            // 512 is the maximum size of an encoded node
            let mut buf = Vec::with_capacity(512);
            self.root.compute_hash_no_alloc(&mut buf).finalize()
        } else {
            *EMPTY_TRIE_HASH
        }
    }

    pub fn get_root_node(&self, path: Nibbles) -> Result<Arc<Node>, TrieError> {
        self.root
            .get_node_checked(self.db.as_ref(), path)?
            .ok_or_else(|| {
                TrieError::InconsistentTree(Box::new(InconsistentTreeError::RootNotFound(
                    self.root.compute_hash().finalize(),
                )))
            })
    }

    /// Returns a list of changes in a TrieNode format since last root hash processed.
    ///
    /// # Returns
    ///
    /// A tuple containing the hash and the list of changes.
    pub fn collect_changes_since_last_hash(&mut self) -> (H256, Vec<TrieNode>) {
        let updates = self.commit_without_storing();
        let ret_hash = self.hash_no_commit();
        (ret_hash, updates)
    }

    /// Compute the hash of the root node and flush any changes into the database.
    ///
    /// This method will also compute the hash of all internal nodes indirectly. It will not clear
    /// the cached nodes.
    pub fn commit(&mut self) -> Result<(), TrieError> {
        let acc = self.commit_without_storing();
        self.db.put_batch(acc)?;

        // Commit the underlying transaction
        self.db.commit()?;

        Ok(())
    }

    /// Computes the nodes that would be added if updating the trie.
    /// Nodes are given with their hash pre-calculated.
    pub fn commit_without_storing(&mut self) -> Vec<TrieNode> {
        let mut acc = Vec::new();
        if self.root.is_valid() {
            self.root.commit(Nibbles::default(), &mut acc);
        }
        if self.root.compute_hash() == NodeHash::Hashed(*EMPTY_TRIE_HASH) {
            acc.push((Nibbles::default(), vec![RLP_NULL]))
        }
        acc.extend(self.pending_removal.drain().map(|nib| (nib, vec![])));

        acc
    }

    /// Obtain a merkle proof for the given path.
    /// The proof will contain all the encoded nodes traversed until reaching the node where the path is stored (including this last node).
    /// The proof will still be constructed even if the path is not stored in the trie, proving its absence.
    ///
    /// Note: This method has a different behavior in regard to non-existent trie root nodes. Normal
    ///   behavior is to return `Err(InconsistentTrie)`, but this method will return
    ///   `Ok(Vec::new())` instead.
    pub fn get_proof(&self, path: &[u8]) -> Result<Vec<NodeRLP>, TrieError> {
        if self.root.is_valid() {
            let hash = self.root.compute_hash();

            let mut node_path = Vec::new();
            if let NodeHash::Inline((data, len)) = hash {
                node_path.push(data[..len as usize].to_vec());
            }

            let root = match self
                .root
                .get_node_checked(self.db.as_ref(), Nibbles::default())?
            {
                Some(x) => x,
                None => return Ok(Vec::new()),
            };
            root.get_path(self.db.as_ref(), Nibbles::from_bytes(path), &mut node_path)?;

            Ok(node_path)
        } else {
            Ok(Vec::new())
        }
    }

    /// Obtains all encoded nodes traversed until reaching the node where every path is stored.
    /// The list doesn't include the root node, this is returned separately.
    /// Will still be constructed even if some path is not stored in the trie.
    pub fn get_proofs(
        &self,
        paths: &[PathRLP],
    ) -> Result<(Option<NodeRLP>, Vec<NodeRLP>), TrieError> {
        if self.root.is_valid() {
            let encoded_root = self.get_root_node(Nibbles::default())?.encode_to_vec();

            let mut node_path: FxHashSet<_> = Default::default();
            for path in paths {
                let mut nodes = self.get_proof(path)?;
                nodes.swap_remove(0);
                node_path.extend(nodes);
            }

            Ok((Some(encoded_root), node_path.into_iter().collect()))
        } else {
            Ok((None, Vec::new()))
        }
    }

    pub fn empty_in_memory() -> Self {
        Self::new(Box::new(InMemoryTrieDB::new(Arc::new(Mutex::new(
            BTreeMap::new(),
        )))))
    }

    /// Gets node with embedded references to child nodes, all in just one `Node`.
    pub fn get_embedded_root(
        all_nodes: &BTreeMap<H256, Node>,
        root_hash: H256,
    ) -> Result<NodeRef, TrieError> {
        // If the root hash is of the empty trie then we can get away by setting the NodeRef to default
        if root_hash == *EMPTY_TRIE_HASH {
            return Ok(NodeRef::default());
        }

        let root_rlp = all_nodes.get(&root_hash).ok_or_else(|| {
            TrieError::InconsistentTree(Box::new(InconsistentTreeError::RootNotFound(root_hash)))
        })?;

        fn get_embedded_node(
            all_nodes: &BTreeMap<H256, Node>,
            cur_node: &Node,
        ) -> Result<Node, TrieError> {
            Ok(match cur_node.clone() {
                Node::Branch(mut node) => {
                    for choice in &mut node.choices {
                        let NodeRef::Hash(hash) = *choice else {
                            continue;
                        };

                        if hash.is_valid() {
                            *choice = match all_nodes.get(&hash.finalize()) {
                                Some(node) => get_embedded_node(all_nodes, node)?.into(),
                                None => hash.into(),
                            };
                        }
                    }

                    (*node).into()
                }
                Node::Extension(mut node) => {
                    let NodeRef::Hash(hash) = node.child else {
                        return Ok(node.into());
                    };

                    node.child = match all_nodes.get(&hash.finalize()) {
                        Some(node) => get_embedded_node(all_nodes, node)?.into(),
                        None => hash.into(),
                    };

                    node.into()
                }
                Node::Leaf(node) => node.into(),
            })
        }

        let root = get_embedded_node(all_nodes, root_rlp)?;
        Ok(root.into())
    }

    /// Builds a trie from a set of nodes with an empty InMemoryTrieDB as a backend because the nodes are embedded in the root.
    ///
    /// Note: This method will not ensure that all node references are valid. Invalid references
    ///   will cause other methods (including, but not limited to `Trie::get`, `Trie::insert` and
    ///   `Trie::remove`) to return `Err(InconsistentTrie)`.
    /// Note: This method will ignore any dangling nodes. All nodes that are not accessible from the
    ///   root node are considered dangling.
    pub fn from_nodes(
        root_hash: H256,
        state_nodes: &BTreeMap<H256, Node>,
    ) -> Result<Self, TrieError> {
        let mut trie = Trie::new(Box::new(InMemoryTrieDB::default()));
        let root = Self::get_embedded_root(state_nodes, root_hash)?;
        trie.root = root;

        Ok(trie)
    }

    /// Builds an in-memory trie from the given elements and returns its hash
    pub fn compute_hash_from_unsorted_iter(
        iter: impl Iterator<Item = (PathRLP, ValueRLP)>,
    ) -> H256 {
        let mut trie = Trie::stateless();
        for (path, value) in iter {
            // Unwraping here won't panic as our in_memory trie DB won't fail
            trie.insert(path, value).unwrap();
        }

        trie.hash_no_commit()
    }

    /// Creates a new stateless trie. This trie won't be able to store any nodes so all data will be lost after calculating the hash
    /// Only use it for proof verification or computing a hash from an iterator
    pub(crate) fn stateless() -> Trie {
        // We will only be using the trie's cache so we don't need a working DB
        struct NullTrieDB;

        impl TrieDB for NullTrieDB {
            fn get(&self, _key: Nibbles) -> Result<Option<Vec<u8>>, TrieError> {
                Ok(None)
            }

            fn put_batch(&self, _key_values: Vec<TrieNode>) -> Result<(), TrieError> {
                Ok(())
            }
        }

        Trie::new(Box::new(NullTrieDB))
    }

    /// Obtain the encoded node given its path.
    /// Allows usage of full paths (byte slice of 32 bytes) or compact-encoded nibble slices (with length lower than 32)
    pub fn get_node(&self, partial_path: &PathRLP) -> Result<Vec<u8>, TrieError> {
        // Convert compact-encoded nibbles into a byte slice if necessary
        let partial_path = match partial_path.len() {
            // Compact-encoded nibbles
            n if n < 32 => Nibbles::decode_compact(partial_path),
            // Full path (No conversion needed)
            32 => Nibbles::from_bytes(partial_path),
            // We won't handle paths with length over 32
            _ => return Ok(vec![]),
        };

        fn get_node_inner(
            db: &dyn TrieDB,
            current_path: Nibbles,
            node: &Node,
            mut partial_path: Nibbles,
        ) -> Result<Vec<u8>, TrieError> {
            // If we reached the end of the partial path, return the current node
            if partial_path.is_empty() {
                return Ok(node.encode_to_vec());
            }
            match node {
                Node::Branch(branch_node) => match partial_path.next_choice() {
                    Some(idx) => {
                        let child_ref = &branch_node.choices[idx];
                        if child_ref.is_valid() {
                            let child_path = current_path.append_new(idx as u8);
                            let child_node = child_ref
                                .get_node_checked(db, child_path.clone())?
                                .ok_or_else(|| {
                                    TrieError::InconsistentTree(Box::new(
                                        InconsistentTreeError::NodeNotFoundOnBranchNode(
                                            child_ref.compute_hash().finalize(),
                                            branch_node.compute_hash().finalize(),
                                            child_path.clone(),
                                        ),
                                    ))
                                })?;
                            get_node_inner(db, child_path, &child_node, partial_path)
                        } else {
                            Ok(vec![])
                        }
                    }
                    _ => Ok(vec![]),
                },
                Node::Extension(extension_node) => {
                    if partial_path.skip_prefix(&extension_node.prefix)
                        && extension_node.child.is_valid()
                    {
                        let child_path = partial_path.concat(&extension_node.prefix);
                        let child_node = extension_node
                            .child
                            .get_node_checked(db, child_path.clone())?
                            .ok_or_else(|| {
                                TrieError::InconsistentTree(Box::new(
                                    InconsistentTreeError::ExtensionNodeChildNotFound(
                                        ExtensionNodeErrorData {
                                            node_hash: extension_node
                                                .child
                                                .compute_hash()
                                                .finalize(),
                                            extension_node_hash: extension_node
                                                .compute_hash()
                                                .finalize(),
                                            extension_node_prefix: extension_node.prefix.clone(),
                                            node_path: child_path.clone(),
                                        },
                                    ),
                                ))
                            })?;
                        get_node_inner(db, child_path, &child_node, partial_path)
                    } else {
                        Ok(vec![])
                    }
                }
                Node::Leaf(_) => Ok(vec![]),
            }
        }

        // Fetch node
        if self.root.is_valid() {
            let root_node = self.get_root_node(Default::default())?;
            get_node_inner(
                self.db.as_ref(),
                Default::default(),
                &root_node,
                partial_path,
            )
        } else {
            Ok(Vec::new())
        }
    }

    pub fn root_node(&self) -> Result<Option<Arc<Node>>, TrieError> {
        if self.root.is_valid() {
            self.root.get_node(self.db.as_ref(), Nibbles::default())
        } else {
            Ok(None)
        }
    }

    /// Creates a new Trie based on a temporary InMemory DB
    pub fn new_temp() -> Self {
        let db = InMemoryTrieDB::new(Default::default());
        Trie::new(Box::new(db))
    }

    /// Creates a new Trie based on a temporary InMemory DB, with a specified root
    ///
    /// This is usually used to create a Trie from a root that was embedded with the rest of the nodes.
    pub fn new_temp_with_root(root: NodeRef) -> Self {
        let db = InMemoryTrieDB::new(Default::default());
        let mut trie = Trie::new(Box::new(db));
        trie.root = root;
        trie
    }

    /// Validates that the Trie isn't missing any nodes expected in the branches
    ///
    /// This is used internally with debug assertions to check the status of the trie
    /// after syncing operations.
    /// Note: this operation validates the hashes because the iterator uses
    /// get_node_checked. We shouldn't downgrade that to the unchecked version
    pub fn validate(self) -> Result<(), TrieError> {
        let mut expected_count = if self.root.is_valid() { 1 } else { 0 };
        for (_, node) in self.into_iter() {
            expected_count -= 1;
            match node {
                Node::Branch(branch_node) => {
                    expected_count += branch_node
                        .choices
                        .iter()
                        .filter(|child| child.is_valid())
                        .count();
                }
                Node::Extension(_) => {
                    expected_count += 1;
                }
                Node::Leaf(_) => {}
            }
        }
        if expected_count != 0 {
            return Err(TrieError::Verify(format!(
                "Node count mismatch, expected {expected_count} more"
            )));
        }
        Ok(())
    }
}

impl IntoIterator for Trie {
    type Item = (Nibbles, Node);

    type IntoIter = TrieIterator;

    fn into_iter(self) -> Self::IntoIter {
        TrieIterator::new(self)
    }
}

pub struct ProofTrie(Trie);

impl ProofTrie {
    pub fn insert(
        &mut self,
        partial_path: Nibbles,
        external_ref: NodeHash,
    ) -> Result<(), TrieError> {
        if self.0.root.is_valid() {
            // If the trie is not empty, call the root node's insertion logic.
            self.0
                .root
                .get_node_mut(self.0.db.as_ref(), Nibbles::default())?
                .ok_or_else(|| {
                    TrieError::InconsistentTree(Box::new(InconsistentTreeError::RootNotFoundNoHash))
                })?
                .insert(self.0.db.as_ref(), partial_path, external_ref)?;
            self.0.root.clear_hash();
        } else {
            self.0.root = external_ref.into();
        };

        Ok(())
    }

    pub fn hash(&self) -> H256 {
        self.0.hash_no_commit()
    }
}

impl From<Trie> for ProofTrie {
    fn from(value: Trie) -> Self {
        Self(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: insert key-value pairs sequentially and return the trie hash
    fn sequential_insert_hash(pairs: &[(Vec<u8>, Vec<u8>)]) -> H256 {
        let mut trie = Trie::new_temp();
        for (path, value) in pairs {
            if value.is_empty() {
                trie.remove(path).unwrap();
            } else {
                trie.insert(path.clone(), value.clone()).unwrap();
            }
        }
        trie.hash_no_commit()
    }

    /// Helper: insert key-value pairs via batch and return the trie hash
    fn batch_insert_hash(pairs: &[(Vec<u8>, Vec<u8>)]) -> H256 {
        let mut trie = Trie::new_temp();
        trie.insert_batch_sorted(pairs.to_vec()).unwrap();
        trie.hash_no_commit()
    }

    /// Generate deterministic H256-like keys for testing
    fn make_key(seed: u8) -> Vec<u8> {
        // Use keccak to get well-distributed keys
        ethrex_crypto::keccak::keccak_hash([seed]).to_vec()
    }

    #[test]
    fn batch_insert_matches_sequential_hash() {
        // Insert 100 random H256 keys both ways, compare trie root hash
        let pairs: Vec<(Vec<u8>, Vec<u8>)> = (0..100u8)
            .map(|i| (make_key(i), vec![i, i + 1, i + 2]))
            .collect();

        let seq_hash = sequential_insert_hash(&pairs);
        let batch_hash = batch_insert_hash(&pairs);
        assert_eq!(
            seq_hash, batch_hash,
            "Batch insert must produce identical root hash as sequential insert"
        );
    }

    #[test]
    fn batch_insert_into_existing_trie() {
        // Pre-populate trie with 50 keys, batch-insert 50 more, compare to sequential
        let first_half: Vec<(Vec<u8>, Vec<u8>)> = (0..50u8)
            .map(|i| (make_key(i), vec![i, i + 1]))
            .collect();
        let second_half: Vec<(Vec<u8>, Vec<u8>)> = (50..100u8)
            .map(|i| (make_key(i), vec![i, i + 1]))
            .collect();

        // Sequential: insert all 100
        let all_pairs: Vec<(Vec<u8>, Vec<u8>)> = (0..100u8)
            .map(|i| (make_key(i), vec![i, i + 1]))
            .collect();
        let expected_hash = sequential_insert_hash(&all_pairs);

        // Batch: insert first 50 sequentially, then batch-insert next 50
        let mut trie = Trie::new_temp();
        for (path, value) in &first_half {
            trie.insert(path.clone(), value.clone()).unwrap();
        }
        trie.insert_batch_sorted(second_half).unwrap();
        let actual_hash = trie.hash_no_commit();

        assert_eq!(
            expected_hash, actual_hash,
            "Batch insert into existing trie must match sequential"
        );
    }

    #[test]
    fn batch_insert_with_removes() {
        // Mix of inserts and removes (empty value = remove)
        let mut pairs: Vec<(Vec<u8>, Vec<u8>)> = (0..50u8)
            .map(|i| (make_key(i), vec![i, i + 1]))
            .collect();
        // Add some removes for keys that were inserted
        for i in [5u8, 15, 25, 35, 45] {
            pairs.push((make_key(i), vec![]));
        }

        let seq_hash = sequential_insert_hash(&pairs);
        let batch_hash = batch_insert_hash(&pairs);
        assert_eq!(
            seq_hash, batch_hash,
            "Batch insert with removes must match sequential"
        );
    }

    #[test]
    fn batch_insert_empty_trie() {
        let mut trie = Trie::new_temp();
        let pairs: Vec<(Vec<u8>, Vec<u8>)> = (0..10u8)
            .map(|i| (make_key(i), vec![i]))
            .collect();

        let expected = sequential_insert_hash(&pairs);
        trie.insert_batch_sorted(pairs).unwrap();
        assert_eq!(trie.hash_no_commit(), expected);
    }

    #[test]
    fn batch_insert_single_element() {
        let pairs = vec![(make_key(42), vec![42, 43, 44])];

        let seq_hash = sequential_insert_hash(&pairs);
        let batch_hash = batch_insert_hash(&pairs);
        assert_eq!(seq_hash, batch_hash);
    }

    #[test]
    fn batch_insert_empty_updates() {
        let mut trie = Trie::new_temp();
        // Insert something first
        trie.insert(make_key(1), vec![1, 2, 3]).unwrap();
        let hash_before = trie.hash_no_commit();

        // Batch insert with empty updates should not change anything
        trie.insert_batch_sorted(vec![]).unwrap();
        assert_eq!(trie.hash_no_commit(), hash_before);
    }

    #[test]
    fn batch_insert_duplicate_keys_last_wins() {
        // When batch contains duplicate keys, the last value should win
        // (since we sort by key and process in order)
        let key = make_key(7);
        let pairs_batch = vec![
            (key.clone(), vec![1]),
            (key.clone(), vec![2]),
            (key.clone(), vec![3]),
        ];

        // Sequential: last write wins
        let seq_hash = sequential_insert_hash(&pairs_batch);
        let batch_hash = batch_insert_hash(&pairs_batch);
        assert_eq!(seq_hash, batch_hash);
    }

    #[test]
    fn batch_insert_values_retrievable() {
        // Verify that values inserted via batch can be retrieved
        let pairs: Vec<(Vec<u8>, Vec<u8>)> = (0..20u8)
            .map(|i| (make_key(i), vec![i, i * 2]))
            .collect();

        let mut trie = Trie::new_temp();
        trie.insert_batch_sorted(pairs.clone()).unwrap();

        for (path, expected_value) in &pairs {
            let got = trie.get(path).unwrap();
            assert_eq!(
                got.as_deref(),
                Some(expected_value.as_slice()),
                "Value for key should be retrievable after batch insert"
            );
        }
    }
}
