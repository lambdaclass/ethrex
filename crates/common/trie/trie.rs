pub mod db;
pub mod error;
mod nibbles;
mod node;
mod node_hash;
mod rlp;
mod state;
#[cfg(test)]
mod test_utils;
mod trie_iter;
mod verify_range;
use ethereum_types::H256;
use ethrex_rlp::constants::RLP_NULL;
use ethrex_rlp::encode::RLPEncode;
use node_hash::NodeHash;
use sha3::{Digest, Keccak256};
use std::collections::HashSet;

pub use self::db::{InMemoryTrieDB, TrieDB};
pub use self::nibbles::Nibbles;
pub use self::verify_range::verify_range;
pub use self::{node::Node, node_hash::NodeHash, state::TrieState};

pub use self::error::TrieError;
use self::{node::LeafNode, trie_iter::TrieIterator};

use lazy_static::lazy_static;

lazy_static! {
    // Hash value for an empty trie, equal to keccak(RLP_NULL)
    pub static ref EMPTY_TRIE_HASH: H256 = H256::from_slice(
        Keccak256::new()
            .chain_update([RLP_NULL])
            .finalize()
            .as_slice(),
    );
}

/// RLP-encoded trie path
pub type PathRLP = Vec<u8>;
/// RLP-encoded trie value
pub type ValueRLP = Vec<u8>;
/// RLP-encoded trie node
pub type NodeRLP = Vec<u8>;

/// Libmdx-based Ethereum Compatible Merkle Patricia Trie
pub struct Trie {
    /// Hash of the current node
    root: Option<NodeHash>,
    /// Contains the trie's nodes
    pub(crate) state: TrieState,
}

impl Trie {
    /// Creates a new Trie from a clean DB
    pub fn new(db: Box<dyn TrieDB>) -> Self {
        Self {
            state: TrieState::new(db),
            root: None,
        }
    }

    /// Creates a trie from an already-initialized DB and sets root as the root node of the trie
    pub fn open(db: Box<dyn TrieDB>, root: H256) -> Self {
        let root = (root != *EMPTY_TRIE_HASH).then_some(root.into());
        Self {
            state: TrieState::new(db),
            root,
        }
    }

    /// Retrieve an RLP-encoded value from the trie given its RLP-encoded path.
    pub fn get(&self, path: &PathRLP) -> Result<Option<ValueRLP>, TrieError> {
        if let Some(root) = &self.root {
            let root_node = self
                .state
                .get_node(*root)?
                .ok_or(TrieError::InconsistentTree)?;
            root_node.get(&self.state, Nibbles::from_bytes(path))
        } else {
            Ok(None)
        }
    }

    /// Insert an RLP-encoded value into the trie.
    pub fn insert(&mut self, path: PathRLP, value: ValueRLP) -> Result<(), TrieError> {
        let root = self.root.take();
        if let Some(root_node) = root
            .map(|root| self.state.get_node(root))
            .transpose()?
            .flatten()
        {
            // If the trie is not empty, call the root node's insertion logic
            let root_node = root_node.insert(&mut self.state, Nibbles::from_bytes(&path), value)?;
            self.root = Some(root_node.insert_self(&mut self.state)?)
        } else {
            // If the trie is empty, just add a leaf.
            let new_leaf = Node::from(LeafNode::new(Nibbles::from_bytes(&path), value));
            self.root = Some(new_leaf.insert_self(&mut self.state)?)
        }
        Ok(())
    }

    /// Remove a value from the trie given its RLP-encoded path.
    /// Returns the value if it was succesfully removed or None if it wasn't part of the trie
    pub fn remove(&mut self, path: PathRLP) -> Result<Option<ValueRLP>, TrieError> {
        let root = self.root.take();
        if let Some(root) = root {
            let root_node = self
                .state
                .get_node(root)?
                .ok_or(TrieError::InconsistentTree)?;
            let (root_node, old_value) =
                root_node.remove(&mut self.state, Nibbles::from_bytes(&path))?;
            self.root = root_node
                .map(|root| root.insert_self(&mut self.state))
                .transpose()?;
            Ok(old_value)
        } else {
            Ok(None)
        }
    }

    /// Return the hash of the trie's root node.
    /// Returns keccak(RLP_NULL) if the trie is empty
    /// Also commits changes to the DB
    pub fn hash(&mut self) -> Result<H256, TrieError> {
        self.commit()?;
        Ok(self
            .root
            .as_ref()
            .map(|root| root.finalize())
            .unwrap_or(*EMPTY_TRIE_HASH))
    }

    /// Return the hash of the trie's root node.
    /// Returns keccak(RLP_NULL) if the trie is empty
    pub fn hash_no_commit(&self) -> H256 {
        self.root
            .as_ref()
            .map(|root| root.finalize())
            .unwrap_or(*EMPTY_TRIE_HASH)
    }

    pub fn commit(&mut self) -> Result<(), TrieError> {
        if let Some(ref root) = self.root {
            self.state.commit(root)?;
        }
        Ok(())
    }

    /// Obtain a merkle proof for the given path.
    /// The proof will contain all the encoded nodes traversed until reaching the node where the path is stored (including this last node).
    /// The proof will still be constructed even if the path is not stored in the trie, proving its absence.
    pub fn get_proof(&self, path: &PathRLP) -> Result<Vec<NodeRLP>, TrieError> {
        // Will store all the encoded nodes traversed until reaching the node containing the path
        let mut node_path = Vec::new();
        let Some(root) = &self.root else {
            return Ok(node_path);
        };
        // If the root is inlined, add it to the node_path
        if let NodeHash::Inline(_) = root {
            node_path.push(root.as_ref().to_vec());
        }
        if let Some(root_node) = self.state.get_node(*root)? {
            root_node.get_path(&self.state, Nibbles::from_bytes(path), &mut node_path)?;
        }
        Ok(node_path)
    }

    /// Obtains all encoded nodes traversed until reaching the node where every path is stored.
    /// The list doesn't include the root node, this is returned separately.
    /// Will still be constructed even if some path is not stored in the trie.
    pub fn get_proofs(
        &self,
        paths: &[PathRLP],
    ) -> Result<(Option<NodeRLP>, Vec<NodeRLP>), TrieError> {
        let Some(root_node) = self
            .root
            .as_ref()
            .map(|root| self.state.get_node(*root))
            .transpose()?
            .flatten()
        else {
            return Ok((None, Vec::new()));
        };

        let mut node_path = Vec::new();
        for path in paths {
            let mut nodes = self.get_proof(path)?;
            nodes.swap_remove(0);
            node_path.extend(nodes); // skip root node
        }

        // dedup
        // TODO: really inefficient, by making the traversing smarter we can avoid having
        // duplicates
        let node_path: HashSet<_> = node_path.into_iter().collect();
        let node_path = Vec::from_iter(node_path);
        Ok((Some(root_node.encode_raw()), node_path))
    }

    /// Creates a cached Trie (with [NullTrieDB]) from a list of encoded nodes.
    /// Generally used in conjuction with [Trie::get_proofs].
    pub fn from_nodes(
        root_node: Option<&NodeRLP>,
        other_nodes: &[NodeRLP],
    ) -> Result<Self, TrieError> {
        let mut trie = Trie::stateless();

        if let Some(root_node) = root_node {
            let root_node = Node::decode_raw(root_node)?;
            trie.root = Some(root_node.insert_self(&mut trie.state)?);
        }

        for node in other_nodes.iter().map(|node| Node::decode_raw(node)) {
            node?.insert_self(&mut trie.state)?;
        }

        Ok(trie)
    }

    // TODO:
    // - Move to ArrayVec
    // - Fix extension case, when does it matter?
    //   I think there can be at most one, connecting to the last branch
    //   There is one from 129 to 255: prefix path being 818x and extension
    //   the middle 18
    //   Then 256 through to 512 will have one path 82001x and extension
    //   prefix 001 (2 will be a branch, shared with 0 and 1)
    //   I think it can be handled in the finalize!
    //   There will be at most one segment made of branches with exactly
    //   one populated child (or zero before doing the bubble up)
    pub fn compute_hash_from_compact_iter<T>(
        items: &[T],
        encode: fn(&T, &mut dyn bytes::BufMut),
    ) -> H256 {
        use smallvec::SmallVec;

        if items.is_empty() {
            return *EMPTY_TRIE_HASH;
        }

        // At the moment this fails for over 0x80 that end in 0x0
        if items.len() > 0x80 && (items.len() - 1) & 0xf == 0x0 {
            let iter = items.into_iter().enumerate().map(|(idx, val)| {
                let mut buffer = vec![];
                encode(val, &mut buffer);
                (idx.encode_to_vec(), buffer)
            });
            return Self::compute_hash_from_unsorted_iter(iter);
        }

        fn h(buffer: &[u8]) -> SmallVec<[u8; 32]> {
            let mut result = SmallVec::new();
            if buffer.len() >= 32 {
                let hash = Keccak256::digest(buffer);
                result.extend_from_slice(&hash);
            } else {
                result.extend_from_slice(buffer);
            }
            result
        }

        #[derive(Default)]
        struct TrieStack {
            root: SmallVec<[u8; 32]>,
            nodes: [[SmallVec<[u8; 32]>; 16]; 10],
            buffer: Vec<u8>,
            last_key: SmallVec<[u8; 10]>,
            max_idx: u32,
        }

        impl TrieStack {
            pub fn new(max: usize) -> Self {
                TrieStack {
                    max_idx: max as u32,
                    buffer: Vec::with_capacity(1024),
                    ..Default::default()
                }
            }
            pub fn push_leaf(&mut self, idx: u32, path: &[u8], value: &[u8]) {
                let current_key = key(idx);
                let leaf = if self.max_idx == idx && idx > 128 && idx.trailing_zeros() >= 4 {
                    let diff_start = current_key
                        .iter()
                        .zip(self.last_key.iter())
                        .position(|(m, l)| m != l)
                        .unwrap();
                    let raw_path = &current_key[diff_start..];
                    let mut path = SmallVec::<[u8; 32]>::new();
                    match raw_path.len() % 2 {
                        0 => {
                            path.push(0x20);
                            for b in raw_path.chunks(2) {
                                let (n0, n1) = (b[0], b[1]);
                                path.push(n0 << 4 | n1);
                            }
                        }
                        1 => {
                            path.push(0x30 | raw_path[0]);
                            for b in raw_path[1..].chunks(2) {
                                let (n0, n1) = (b[0], b[1]);
                                path.push(n0 << 4 | n1);
                            }
                        }
                        _ => (),
                    }
                    // eprintln!(
                    //     "RAW: {} COMPACT: {}",
                    //     hex::encode(&raw_path),
                    //     hex::encode(&path)
                    // );
                    hash_leaf(&path, value, &mut self.buffer)
                } else {
                    hash_leaf(path, value, &mut self.buffer)
                };
                self.last_key = current_key;
                // if matches!(idx, 0..140) {
                // eprintln!(
                //     "PUSH LEAF IDX: {idx} PATH: {} VALUE: {} KEY: {} LEAF: {}",
                //     hex::encode(path),
                //     hex::encode(value),
                //     hex::encode(&self.last_key),
                //     hex::encode(&leaf)
                // );
                // }
                let mut lvl = self.last_key.len() - 1;
                let mut k = self.last_key[lvl] as usize;
                self.nodes[lvl][k] = leaf;
                // 7f => 80
                // 8180 => 9001
                // 8181 => 9002
                while idx != self.max_idx && k == 0xf {
                    let branch_hash = hash_branch(&self.nodes[lvl], &mut self.buffer);
                    self.nodes[lvl] = Default::default();
                    self.last_key.truncate(lvl);
                    //eprintln!("LAST_KEY: {}", hex::encode(&self.last_key));
                    lvl -= 1;
                    k = self.last_key[lvl] as usize;
                    //eprintln!("L: {} K: {}", lvl, k);
                    // eprintln!(
                    //     "COMPACT LEVEL: {} TO KEY: {} HASH: {}",
                    //     lvl + 1,
                    //     k,
                    //     hex::encode(&branch_hash)
                    // );
                    self.nodes[lvl][k] = branch_hash;
                    // FIXME: missing extension logic for this case:
                    // when there would be an extension and the last leaf
                    // falls here, the finalize might get confused and think
                    // it was only a leaf due to its count becoming 1
                    // maybe just checking if this is the last index and
                    // skipping helps
                }
            }
            pub fn finalize_current(&mut self) {
                if self.last_key.is_empty() {
                    return;
                }
                let max_key = key(self.max_idx);
                if self.max_idx > 128 && self.last_key == max_key {
                    let mut i = self.last_key.len() - 1;
                    let mut counts: [_; 10] = std::array::from_fn(|i| {
                        self.nodes[i].iter().filter(|b| !b.is_empty()).count()
                    });
                    while i >= 1 {
                        // eprintln!("I: {i} COUNTS: {:?}", &counts);
                        let k = self.last_key[i - 1] as usize;
                        // eprintln!("K: {k}");

                        let mut j = i - 1;
                        while j > 0 && counts[j] == 0 {
                            j -= 1;
                        }
                        if counts[i] == 1 {
                            // eprintln!("MOVING LEAF");
                            // Leaf
                            let leaf = self.nodes[i][k].clone();
                            let k = max_key[j] as usize;
                            self.nodes[j][k] = leaf;
                            counts[j] += 1;
                            i = j;
                            continue;
                        }
                        let mut child = hash_branch(&self.nodes[i], &mut self.buffer);
                        if j < i - 1 {
                            // Extension
                            let extension_start = j + 1;
                            let extension_len = i - extension_start;
                            // eprintln!("EXTENSION LEN: {extension_len}");
                            let mut prefix = SmallVec::<[u8; 12]>::new();
                            match extension_len % 2 {
                                0 => {
                                    prefix.push(0);
                                    for b in max_key[extension_start..i].chunks(2) {
                                        let (n0, n1) = (b[0], b[1]);
                                        prefix.push(n0 << 4 | n1);
                                    }
                                }
                                1 => {
                                    prefix.push(0x10 | max_key[extension_start]);
                                    for b in max_key[extension_start + 1..i].chunks(2) {
                                        let (n0, n1) = (b[0], b[1]);
                                        prefix.push(n0 << 4 | n1);
                                    }
                                }
                                _ => (),
                            };
                            child = hash_extension(&prefix, &child, &mut self.buffer);
                        }
                        // Regular branch
                        // eprintln!("COMPUTING BRANCH");
                        let k = max_key[j] as usize;
                        self.nodes[j][k] = child;
                        counts[j] += 1;
                        i = j;
                    }
                    return;
                }
                for i in (1..self.last_key.len()).rev() {
                    let k = self.last_key[i - 1] as usize;
                    // eprintln!("K: {k}");

                    self.nodes[i - 1][k] = hash_branch(&self.nodes[i], &mut self.buffer);
                    self.nodes[i] = Default::default();
                    // eprintln!(
                    //     "COMPACT LEVEL: {} TO KEY: {} HASH: {}",
                    //     i,
                    //     k,
                    //     hex::encode(&self.nodes[i - 1][k])
                    // );
                }
            }
            pub fn finalize(&mut self) -> H256 {
                if self.last_key.is_empty() {
                    return *EMPTY_TRIE_HASH;
                }
                // if key(self.max_idx) == self.last_key {
                //     eprintln!("FINALIZING FOR {}", self.max_idx);
                // }
                self.finalize_current();
                self.root = hash_branch(&self.nodes[0], &mut self.buffer);
                // eprintln!("FINISHING ROOT: {}", hex::encode(&self.root));
                if self.root.len() < 32 {
                    let mut root = SmallVec::new();
                    root.extend_from_slice(&Keccak256::digest(&self.root));
                    self.root = root;
                }
                // eprintln!("FINISHED ROOT: {}", hex::encode(&self.root));
                H256::from_slice(&self.root)
            }
        }

        fn key(idx: u32) -> SmallVec<[u8; 10]> {
            let mut res = SmallVec::new();
            match idx {
                0 => res.extend_from_slice(&[8, 0]),
                i @ 1..128 => {
                    let (h, l) = ((i >> 4) as u8, (i & 0xf) as u8);
                    res.extend_from_slice(&[h, l]);
                }
                i => {
                    res.extend_from_slice(&[8, 0]);
                    let bytes = i.to_be_bytes();
                    let iter = bytes
                        .into_iter()
                        .skip_while(|b| *b == 0)
                        .flat_map(|b| [b >> 4, b & 0xf]);
                    res.extend(iter);
                    res[1] = (res.len() / 2 - 1) as u8;
                }
            }
            res
        }
        fn hash_extension(prefix: &[u8], child: &[u8], buffer: &mut Vec<u8>) -> SmallVec<[u8; 32]> {
            // TODO: prefix needs that ugly compact encoding too
            // eprintln!(
            //     "EXTENSION: PREFIX: {} CHILD: {}",
            //     hex::encode(prefix),
            //     hex::encode(child),
            // );
            let inner_len =
                <[u8] as RLPEncode>::length(prefix) + <[u8] as RLPEncode>::length(child);
            ethrex_rlp::encode::encode_length(inner_len, buffer);
            <[u8] as RLPEncode>::encode(prefix, buffer);
            match child.len() {
                32 => <[u8] as RLPEncode>::encode(child, buffer),
                _ => buffer.extend_from_slice(child),
            }
            let hash = h(&*buffer);
            buffer.clear();
            hash
        }
        fn hash_branch(
            values: &[SmallVec<[u8; 32]>; 16],
            buffer: &mut Vec<u8>,
        ) -> SmallVec<[u8; 32]> {
            let mut count = 0;
            let mut first = 16;
            for j in 0..16 {
                if !values[j].is_empty() {
                    count += 1;
                    first = j.min(first);
                }
            }
            // eprintln!("COUNT: {count} FIRST: {first}");
            match count {
                0 => return SmallVec::new(),
                1 => return values[first].clone(),
                _ => (),
            }
            let inner_len = 1 + values
                .iter()
                .map(|v| match v.len() {
                    0 => 1,
                    32 => 33,
                    _ => v.len(),
                })
                .sum::<usize>();
            // for (i, v) in values.iter().enumerate() {
            //     if v.len() != 0 {
            //         eprintln!("B{}: {}", i, hex::encode(v));
            //     }
            // }
            ethrex_rlp::encode::encode_length(inner_len, buffer);
            for v in values.iter() {
                match v.len() {
                    0 => <[u8] as RLPEncode>::encode(&[], buffer),
                    32 => <[u8] as RLPEncode>::encode(v, buffer),
                    _ => buffer.extend_from_slice(v),
                }
            }
            [].encode(buffer);
            let hash = h(&buffer);
            //eprintln!("BRANCH: {}", hex::encode(&*buffer));
            //eprintln!("HASH: {}", hex::encode(&hash));
            buffer.clear();
            hash
        }
        fn hash_leaf(path: &[u8], value: &[u8], buffer: &mut Vec<u8>) -> SmallVec<[u8; 32]> {
            let path_len = <[u8] as RLPEncode>::length(path);
            let value_len = <[u8] as RLPEncode>::length(value);
            let inner_len = path_len + value_len;
            ethrex_rlp::encode::encode_length(inner_len, buffer);
            <[u8] as RLPEncode>::encode(path, buffer);
            <[u8] as RLPEncode>::encode(value, buffer);
            //eprintln!("LEAF ENCODED: {}", hex::encode(&*buffer));
            let hash = h(buffer);
            buffer.clear();
            hash
        }
        // fn path_from_keys(old: &[u8], new: &[u8]) -> SmallVec<[u8; 12]> {
        //     let prefix_len = old
        //         .iter()
        //         .zip(new.iter())
        //         .position(|(o, n)| o != n)
        //         .unwrap_or_default();
        //     let slice = match new.len() - prefix_len {
        //         0 => &[][..],
        //         1 => &[0x20],
        //         2 => &[0x30 + new[prefix_len]],
        //         3 => &[0x20, new[prefix_len], new[prefix_len + 1]],
        //         4 => &[],
        //         5 => &[],
        //         6 => &[],
        //         7 => &[],
        //         8 => &[],
        //         9 => &[],
        //         10 => &[],
        //         11 => &[],
        //         12 => &[],
        //         _ => &[],
        //     };
        //     SmallVec::from_slice(slice)
        // }
        fn path(idx: usize, len: usize) -> &'static [u8] {
            if idx >= len {
                return &[];
            }
            if idx == 0 {
                return match len {
                    1 => &[0x20, 0x80],
                    l if l <= 128 => &[0x30],
                    _ => &[0x20],
                };
            }
            if idx == 1 {
                return match len {
                    2 => &[0x31],
                    _ => &[0x20],
                };
            }
            if idx != len - 1 {
                return &[0x20];
            }
            if matches!(idx, 0x80..=0xff) {
                return match idx as u8 {
                    0x80 => &[0x20, 0x80],
                    0x90 => &[0x20, 0x90],
                    0xa0 => &[0x20, 0xa0],
                    0xb0 => &[0x20, 0xb0],
                    0xc0 => &[0x20, 0xc0],
                    0xd0 => &[0x20, 0xd0],
                    0xe0 => &[0x20, 0xe0],
                    0xf0 => &[0x20, 0xf0],
                    _ => &[0x20],
                };
            }
            // FIXME:
            // 1. Prefix will need to depend on the last non-zero nibble
            // 2. 0x8y branches start with the first index being 1, as 0 bytes are skipped in RLP
            //    In that regard they behave like the first branch
            let trailing_zero_nibbles = idx.trailing_zeros() as usize / 4;
            &[
                [0x20, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
                [0x30, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
            ][trailing_zero_nibbles % 2][..1 + trailing_zero_nibbles / 2]
        }

        let len = items.len();
        let mut buffer = Vec::with_capacity(4096);
        let mut stack = TrieStack::new(len - 1);
        for i in 1..len.min(128) {
            encode(&items[i], &mut buffer);
            stack.push_leaf(i as u32, path(i, len), &buffer);
            buffer.clear();
        }
        stack.finalize_current();
        encode(&items[0], &mut buffer);
        stack.push_leaf(0, path(0, len), &buffer);
        buffer.clear();
        for i in 128..len {
            encode(&items[i], &mut buffer);
            stack.push_leaf(i as u32, path(i, len), &buffer);
            buffer.clear();
        }
        stack.finalize()
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
        trie.root
            .as_ref()
            .map(|root| root.finalize())
            .unwrap_or(*EMPTY_TRIE_HASH)
    }

    /// Creates a new stateless trie. This trie won't be able to store any nodes so all data will be lost after calculating the hash
    /// Only use it for proof verification or computing a hash from an iterator
    pub(crate) fn stateless() -> Trie {
        // We will only be using the trie's cache so we don't need a working DB
        struct NullTrieDB;

        impl TrieDB for NullTrieDB {
            fn get(&self, _key: NodeHash) -> Result<Option<Vec<u8>>, TrieError> {
                Ok(None)
            }

            fn put(&self, _key: NodeHash, _value: Vec<u8>) -> Result<(), TrieError> {
                Ok(())
            }

            fn put_batch(&self, _key_values: Vec<(NodeHash, Vec<u8>)>) -> Result<(), TrieError> {
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

        // Fetch node
        let Some(root_node) = self
            .root
            .as_ref()
            .map(|root| self.state.get_node(*root))
            .transpose()?
            .flatten()
        else {
            return Ok(vec![]);
        };
        self.get_node_inner(root_node, partial_path)
    }

    fn get_node_inner(&self, node: Node, mut partial_path: Nibbles) -> Result<Vec<u8>, TrieError> {
        // If we reached the end of the partial path, return the current node
        if partial_path.is_empty() {
            return Ok(node.encode_raw());
        }
        match node {
            Node::Branch(branch_node) => match partial_path.next_choice() {
                Some(idx) => {
                    let child_hash = &branch_node.choices[idx];
                    if child_hash.is_valid() {
                        let child_node = self
                            .state
                            .get_node(*child_hash)?
                            .ok_or(TrieError::InconsistentTree)?;
                        self.get_node_inner(child_node, partial_path)
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
                    let child_node = self
                        .state
                        .get_node(extension_node.child)?
                        .ok_or(TrieError::InconsistentTree)?;
                    self.get_node_inner(child_node, partial_path)
                } else {
                    Ok(vec![])
                }
            }
            Node::Leaf(_) => Ok(vec![]),
        }
    }

    /// Returns a mutable reference to the trie's internal node state
    /// [WARNING] This will allow directly manipulating the trie's state and
    /// may lead to inconsistent trie structures if not used resposibly
    pub fn state_mut(&mut self) -> &mut TrieState {
        &mut self.state
    }

    /// Returns a reference to the trie's internal node state
    pub fn state(&mut self) -> &TrieState {
        &self.state
    }

    #[cfg(test)]
    /// Creates a new Trie based on a temporary InMemory DB
    fn new_temp() -> Self {
        use std::collections::HashMap;
        use std::sync::Arc;
        use std::sync::Mutex;

        let hmap: HashMap<NodeHash, Vec<u8>> = HashMap::new();
        let map = Arc::new(Mutex::new(hmap));
        let db = InMemoryTrieDB::new(map);
        Trie::new(Box::new(db))
    }
}

impl IntoIterator for Trie {
    type Item = (Nibbles, Node);

    type IntoIter = TrieIterator;

    fn into_iter(self) -> Self::IntoIter {
        TrieIterator::new(self)
    }
}

#[cfg(test)]
mod test {
    use cita_trie::{MemoryDB as CitaMemoryDB, PatriciaTrie as CitaTrie, Trie as CitaTrieTrait};
    use std::sync::Arc;

    use super::*;

    use hasher::HasherKeccak;
    use hex_literal::hex;
    use proptest::{
        collection::{btree_set, vec},
        prelude::*,
        proptest,
    };

    #[test]
    fn compute_hash() {
        let mut trie = Trie::new_temp();
        trie.insert(b"first".to_vec(), b"value".to_vec()).unwrap();
        trie.insert(b"second".to_vec(), b"value".to_vec()).unwrap();

        assert_eq!(
            trie.hash().unwrap().as_ref(),
            hex!("f7537e7f4b313c426440b7fface6bff76f51b3eb0d127356efbe6f2b3c891501")
        );
    }

    #[test]
    fn compute_hash_long() {
        let mut trie = Trie::new_temp();
        trie.insert(b"first".to_vec(), b"value".to_vec()).unwrap();
        trie.insert(b"second".to_vec(), b"value".to_vec()).unwrap();
        trie.insert(b"third".to_vec(), b"value".to_vec()).unwrap();
        trie.insert(b"fourth".to_vec(), b"value".to_vec()).unwrap();

        assert_eq!(
            trie.hash().unwrap().0.to_vec(),
            hex!("e2ff76eca34a96b68e6871c74f2a5d9db58e59f82073276866fdd25e560cedea")
        );
    }

    #[test]
    fn get_insert_words() {
        let mut trie = Trie::new_temp();
        let first_path = b"first".to_vec();
        let first_value = b"value_a".to_vec();
        let second_path = b"second".to_vec();
        let second_value = b"value_b".to_vec();
        // Check that the values dont exist before inserting
        assert!(trie.get(&first_path).unwrap().is_none());
        assert!(trie.get(&second_path).unwrap().is_none());
        // Insert values
        trie.insert(first_path.clone(), first_value.clone())
            .unwrap();
        trie.insert(second_path.clone(), second_value.clone())
            .unwrap();
        // Check values
        assert_eq!(trie.get(&first_path).unwrap(), Some(first_value));
        assert_eq!(trie.get(&second_path).unwrap(), Some(second_value));
    }

    #[test]
    fn get_insert_zero() {
        let mut trie = Trie::new_temp();
        trie.insert(vec![0x0], b"value".to_vec()).unwrap();
        let first = trie.get(&[0x0][..].to_vec()).unwrap();
        assert_eq!(first, Some(b"value".to_vec()));
    }

    #[test]
    fn get_insert_a() {
        let mut trie = Trie::new_temp();
        trie.insert(vec![16], vec![0]).unwrap();
        trie.insert(vec![16, 0], vec![0]).unwrap();

        let item = trie.get(&vec![16]).unwrap();
        assert_eq!(item, Some(vec![0]));

        let item = trie.get(&vec![16, 0]).unwrap();
        assert_eq!(item, Some(vec![0]));
    }

    #[test]
    fn get_insert_b() {
        let mut trie = Trie::new_temp();
        trie.insert(vec![0, 0], vec![0, 0]).unwrap();
        trie.insert(vec![1, 0], vec![1, 0]).unwrap();

        let item = trie.get(&vec![1, 0]).unwrap();
        assert_eq!(item, Some(vec![1, 0]));

        let item = trie.get(&vec![0, 0]).unwrap();
        assert_eq!(item, Some(vec![0, 0]));
    }

    #[test]
    fn get_insert_c() {
        let mut trie = Trie::new_temp();
        let vecs = vec![
            vec![26, 192, 44, 251],
            vec![195, 132, 220, 124, 112, 201, 70, 128, 235],
            vec![126, 138, 25, 245, 146],
            vec![129, 176, 66, 2, 150, 151, 180, 60, 124],
            vec![138, 101, 157],
        ];
        for x in &vecs {
            trie.insert(x.clone(), x.clone()).unwrap();
        }
        for x in &vecs {
            let item = trie.get(x).unwrap();
            assert_eq!(item, Some(x.clone()));
        }
    }

    #[test]
    fn get_insert_d() {
        let mut trie = Trie::new_temp();
        let vecs = vec![
            vec![52, 53, 143, 52, 206, 112],
            vec![14, 183, 34, 39, 113],
            vec![55, 5],
            vec![134, 123, 19],
            vec![0, 59, 240, 89, 83, 167],
            vec![22, 41],
            vec![13, 166, 159, 101, 90, 234, 91],
            vec![31, 180, 161, 122, 115, 51, 37, 61, 101],
            vec![208, 192, 4, 12, 163, 254, 129, 206, 109],
        ];
        for x in &vecs {
            trie.insert(x.clone(), x.clone()).unwrap();
        }
        for x in &vecs {
            let item = trie.get(x).unwrap();
            assert_eq!(item, Some(x.clone()));
        }
    }

    #[test]
    fn get_insert_e() {
        let mut trie = Trie::new_temp();
        trie.insert(vec![0x00], vec![0x00]).unwrap();
        trie.insert(vec![0xC8], vec![0xC8]).unwrap();
        trie.insert(vec![0xC8, 0x00], vec![0xC8, 0x00]).unwrap();

        assert_eq!(trie.get(&vec![0x00]).unwrap(), Some(vec![0x00]));
        assert_eq!(trie.get(&vec![0xC8]).unwrap(), Some(vec![0xC8]));
        assert_eq!(trie.get(&vec![0xC8, 0x00]).unwrap(), Some(vec![0xC8, 0x00]));
    }

    #[test]
    fn get_insert_f() {
        let mut trie = Trie::new_temp();
        trie.insert(vec![0x00], vec![0x00]).unwrap();
        trie.insert(vec![0x01], vec![0x01]).unwrap();
        trie.insert(vec![0x10], vec![0x10]).unwrap();
        trie.insert(vec![0x19], vec![0x19]).unwrap();
        trie.insert(vec![0x19, 0x00], vec![0x19, 0x00]).unwrap();
        trie.insert(vec![0x1A], vec![0x1A]).unwrap();

        assert_eq!(trie.get(&vec![0x00]).unwrap(), Some(vec![0x00]));
        assert_eq!(trie.get(&vec![0x01]).unwrap(), Some(vec![0x01]));
        assert_eq!(trie.get(&vec![0x10]).unwrap(), Some(vec![0x10]));
        assert_eq!(trie.get(&vec![0x19]).unwrap(), Some(vec![0x19]));
        assert_eq!(trie.get(&vec![0x19, 0x00]).unwrap(), Some(vec![0x19, 0x00]));
        assert_eq!(trie.get(&vec![0x1A]).unwrap(), Some(vec![0x1A]));
    }

    #[test]
    fn get_insert_remove_a() {
        let mut trie = Trie::new_temp();
        trie.insert(b"do".to_vec(), b"verb".to_vec()).unwrap();
        trie.insert(b"horse".to_vec(), b"stallion".to_vec())
            .unwrap();
        trie.insert(b"doge".to_vec(), b"coin".to_vec()).unwrap();
        trie.remove(b"horse".to_vec()).unwrap();
        assert_eq!(trie.get(&b"do".to_vec()).unwrap(), Some(b"verb".to_vec()));
        assert_eq!(trie.get(&b"doge".to_vec()).unwrap(), Some(b"coin".to_vec()));
    }

    #[test]
    fn get_insert_remove_b() {
        let mut trie = Trie::new_temp();
        trie.insert(vec![185], vec![185]).unwrap();
        trie.insert(vec![185, 0], vec![185, 0]).unwrap();
        trie.insert(vec![185, 1], vec![185, 1]).unwrap();
        trie.remove(vec![185, 1]).unwrap();
        assert_eq!(trie.get(&vec![185, 0]).unwrap(), Some(vec![185, 0]));
        assert_eq!(trie.get(&vec![185]).unwrap(), Some(vec![185]));
        assert!(trie.get(&vec![185, 1]).unwrap().is_none());
    }

    #[test]
    fn compute_hash_a() {
        let mut trie = Trie::new_temp();
        trie.insert(b"do".to_vec(), b"verb".to_vec()).unwrap();
        trie.insert(b"horse".to_vec(), b"stallion".to_vec())
            .unwrap();
        trie.insert(b"doge".to_vec(), b"coin".to_vec()).unwrap();
        trie.insert(b"dog".to_vec(), b"puppy".to_vec()).unwrap();

        assert_eq!(
            trie.hash().unwrap().0.as_slice(),
            hex!("5991bb8c6514148a29db676a14ac506cd2cd5775ace63c30a4fe457715e9ac84").as_slice()
        );
    }

    #[test]
    fn compute_hash_b() {
        let mut trie = Trie::new_temp();
        assert_eq!(
            trie.hash().unwrap().0.as_slice(),
            hex!("56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421").as_slice(),
        );
    }

    #[test]
    fn compute_hash_c() {
        let mut trie = Trie::new_temp();
        let data = [
            (
                hex!("0000000000000000000000000000000000000000000000000000000000000045").to_vec(),
                hex!("22b224a1420a802ab51d326e29fa98e34c4f24ea").to_vec(),
            ),
            (
                hex!("0000000000000000000000000000000000000000000000000000000000000046").to_vec(),
                hex!("67706c2076330000000000000000000000000000000000000000000000000000").to_vec(),
            ),
            (
                hex!("000000000000000000000000697c7b8c961b56f675d570498424ac8de1a918f6").to_vec(),
                hex!("1234567890").to_vec(),
            ),
            (
                hex!("0000000000000000000000007ef9e639e2733cb34e4dfc576d4b23f72db776b2").to_vec(),
                hex!("4655474156000000000000000000000000000000000000000000000000000000").to_vec(),
            ),
            (
                hex!("000000000000000000000000ec4f34c97e43fbb2816cfd95e388353c7181dab1").to_vec(),
                hex!("4e616d6552656700000000000000000000000000000000000000000000000000").to_vec(),
            ),
            (
                hex!("4655474156000000000000000000000000000000000000000000000000000000").to_vec(),
                hex!("7ef9e639e2733cb34e4dfc576d4b23f72db776b2").to_vec(),
            ),
            (
                hex!("4e616d6552656700000000000000000000000000000000000000000000000000").to_vec(),
                hex!("ec4f34c97e43fbb2816cfd95e388353c7181dab1").to_vec(),
            ),
            (
                hex!("000000000000000000000000697c7b8c961b56f675d570498424ac8de1a918f6").to_vec(),
                hex!("6f6f6f6820736f2067726561742c207265616c6c6c793f000000000000000000").to_vec(),
            ),
            (
                hex!("6f6f6f6820736f2067726561742c207265616c6c6c793f000000000000000000").to_vec(),
                hex!("697c7b8c961b56f675d570498424ac8de1a918f6").to_vec(),
            ),
        ];

        for (path, value) in data {
            trie.insert(path, value).unwrap();
        }

        assert_eq!(
            trie.hash().unwrap().0.as_slice(),
            hex!("9f6221ebb8efe7cff60a716ecb886e67dd042014be444669f0159d8e68b42100").as_slice(),
        );
    }

    #[test]
    fn compute_hash_d() {
        let mut trie = Trie::new_temp();

        let data = [
            (
                b"key1aa".to_vec(),
                b"0123456789012345678901234567890123456789xxx".to_vec(),
            ),
            (
                b"key1".to_vec(),
                b"0123456789012345678901234567890123456789Very_Long".to_vec(),
            ),
            (b"key2bb".to_vec(), b"aval3".to_vec()),
            (b"key2".to_vec(), b"short".to_vec()),
            (b"key3cc".to_vec(), b"aval3".to_vec()),
            (
                b"key3".to_vec(),
                b"1234567890123456789012345678901".to_vec(),
            ),
        ];

        for (path, value) in data {
            trie.insert(path, value).unwrap();
        }

        assert_eq!(
            trie.hash().unwrap().0.as_slice(),
            hex!("cb65032e2f76c48b82b5c24b3db8f670ce73982869d38cd39a624f23d62a9e89").as_slice(),
        );
    }

    #[test]
    fn compute_hash_e() {
        let mut trie = Trie::new_temp();
        trie.insert(b"abc".to_vec(), b"123".to_vec()).unwrap();
        trie.insert(b"abcd".to_vec(), b"abcd".to_vec()).unwrap();
        trie.insert(b"abc".to_vec(), b"abc".to_vec()).unwrap();

        assert_eq!(
            trie.hash().unwrap().0.as_slice(),
            hex!("7a320748f780ad9ad5b0837302075ce0eeba6c26e3d8562c67ccc0f1b273298a").as_slice(),
        );
    }

    fn print_trie(t: Trie) {
        t.into_iter().for_each(|(path, node)| match node {
            Node::Leaf(leaf) => {
                eprintln!(
                    "PATH: {} => LEAF: PARTIAL: {} VALUE: {}",
                    hex::encode(path.encode_compact()),
                    hex::encode(leaf.partial.encode_compact()),
                    hex::encode(leaf.value)
                )
            }
            Node::Extension(ext) => {
                eprintln!(
                    "PATH: {} => EXTENSION: PREFIX: {} CHILD: {}",
                    hex::encode(path.encode_compact()),
                    hex::encode(ext.prefix.encode_compact()),
                    hex::encode(ext.child)
                )
            }
            Node::Branch(branch) => {
                let encoded_choices: Vec<_> = branch
                    .choices
                    .iter()
                    .enumerate()
                    .map(|(i, c)| format!("BR{i}: {}", hex::encode(c)))
                    .collect();
                eprintln!(
                    "PATH: {} => BRANCH: CHILDREN: [{}] VALUE: {}",
                    hex::encode(path.encode_compact()),
                    encoded_choices.join(", "),
                    hex::encode(branch.value)
                )
            }
        })
    }

    #[test]
    fn compute_hash_from_compact_iter_matches_tree_for_small_values() {
        use ethrex_rlp::encode::RLPEncode;
        let mut trie = Trie::stateless();
        let leaves: Vec<_> = (0u32..4096).map(|_| [0xffu8]).collect();
        assert_eq!(
            *EMPTY_TRIE_HASH,
            Trie::compute_hash_from_compact_iter(&leaves[..0], |v, b| v.encode(b))
        );
        for i in 1..4096 {
            trie.insert((i - 1).encode_to_vec(), leaves[i - 1].encode_to_vec())
                .unwrap();
            if i == 130 {
                assert_eq!(
                    trie.root.clone().map(|r| r.finalize()).unwrap(),
                    Trie::compute_hash_from_compact_iter(&leaves[..i], |v, b| v.encode(b)),
                    "assertion failed for length {i}"
                );
            }
        }
    }

    #[test]
    fn compute_hash_from_compact_iter_matches_tree_for_large_values() {
        use ethrex_rlp::encode::RLPEncode;
        let mut trie = Trie::stateless();
        let leaves: Vec<_> = (0u32..(1 << 12)).map(|_| vec![0xffffu32; 32]).collect();
        assert_eq!(
            *EMPTY_TRIE_HASH,
            Trie::compute_hash_from_compact_iter(&leaves[..0], |v, b| v.encode(b))
        );
        /*
        for i in 1..4096 {
            eprintln!("START TRIE FOR LEN: {i}");
            let mut trie_iter = Trie::stateless();
            for j in 1..=i {
                trie_iter
                    .insert((j - 1).encode_to_vec(), leaves[j - 1].encode_to_vec())
                    .unwrap();
            }
            print_trie(trie_iter);
            eprintln!("FINISHED TRIE FOR LEN: {i}");
        }
        assert!(false);
        */
        let mut res = Vec::with_capacity(1 << 12);
        for i in 1..(1 << 12) {
            trie.insert((i - 1).encode_to_vec(), leaves[i - 1].encode_to_vec())
                .unwrap();
            // assert_eq!(
            //     trie.root.clone().map(|r| r.finalize()).unwrap(),
            //     Trie::compute_hash_from_compact_iter(&leaves[..i], |v, b| v.encode(b)),
            //     "assertion failed for length {i}"
            // );
            res.push(
                trie.root.clone().map(|r| r.finalize()).unwrap()
                    == Trie::compute_hash_from_compact_iter(&leaves[..i], |v, b| v.encode(b)),
            );
        }
        for i in 1..(1 << 12) {
            eprintln!(
                "LEN: {i} MAX: {:x} MAXKEY: {} MATCH: {}",
                i - 1,
                hex::encode((i - 1).encode_to_vec()),
                res[i - 1]
            );
        }
        assert!(false);
    }

    // Proptests
    proptest! {
        #[test]
        fn proptest_get_insert(data in btree_set(vec(any::<u8>(), 1..100), 1..100)) {
            let mut trie = Trie::new_temp();

            for val in data.iter(){
                trie.insert(val.clone(), val.clone()).unwrap();
            }

            for val in data.iter() {
                let item = trie.get(val).unwrap();
                prop_assert!(item.is_some());
                prop_assert_eq!(&item.unwrap(), val);
            }
        }

        #[test]
        fn proptest_get_insert_with_removals(mut data in vec((vec(any::<u8>(), 5..100), any::<bool>()), 1..100)) {
            let mut trie = Trie::new_temp();
            // Remove duplicate values with different expected status
            data.sort_by_key(|(val, _)| val.clone());
            data.dedup_by_key(|(val, _)| val.clone());
            // Insertions
            for (val, _) in data.iter() {
                trie.insert(val.clone(), val.clone()).unwrap();
            }
            // Removals
            for (val, should_remove) in data.iter() {
                if *should_remove {
                    let removed = trie.remove(val.clone()).unwrap();
                    prop_assert_eq!(removed, Some(val.clone()));
                }
            }
            // Check trie values
            for (val, removed) in data.iter() {
                let item = trie.get(val).unwrap();
                if !removed {
                    prop_assert_eq!(item, Some(val.clone()));
                } else {
                    prop_assert!(item.is_none());
                }
            }
        }

        #[test]
        // The previous test needs to sort the input values in order to get rid of duplicate entries, leading to ordered insertions
        // This check has a fixed way of determining wether a value should be removed but doesn't require ordered insertions
        fn proptest_get_insert_with_removals_unsorted(data in btree_set(vec(any::<u8>(), 5..100), 1..100)) {
            let mut trie = Trie::new_temp();
            // Remove all values that have an odd first value
            let remove = |value: &Vec<u8>| -> bool {
                value.first().is_some_and(|v| v % 2 != 0)
            };
            // Insertions
            for val in data.iter() {
                trie.insert(val.clone(), val.clone()).unwrap();
            }
            // Removals
            for val in data.iter() {
                if remove(val) {
                    let removed = trie.remove(val.clone()).unwrap();
                    prop_assert_eq!(removed, Some(val.clone()));
                }
            }
            // Check trie values
            for val in data.iter() {
                let item = trie.get(val).unwrap();
                if !remove(val) {
                    prop_assert_eq!(item, Some(val.clone()));
                } else {
                    prop_assert!(item.is_none());
                }
            }
        }


        #[test]
        fn proptest_compare_hash(data in btree_set(vec(any::<u8>(), 1..100), 1..100)) {
            let mut trie = Trie::new_temp();
            let mut cita_trie = cita_trie();

            for val in data.iter(){
                trie.insert(val.clone(), val.clone()).unwrap();
                cita_trie.insert(val.clone(), val.clone()).unwrap();
            }

            let hash = trie.hash().unwrap().0.to_vec();
            let cita_hash = cita_trie.root().unwrap();
            prop_assert_eq!(hash, cita_hash);
        }

        #[test]
        fn proptest_compare_hash_with_removals(mut data in vec((vec(any::<u8>(), 5..100), any::<bool>()), 1..100)) {
            let mut trie = Trie::new_temp();
            let mut cita_trie = cita_trie();
            // Remove duplicate values with different expected status
            data.sort_by_key(|(val, _)| val.clone());
            data.dedup_by_key(|(val, _)| val.clone());
            // Insertions
            for (val, _) in data.iter() {
                trie.insert(val.clone(), val.clone()).unwrap();
                cita_trie.insert(val.clone(), val.clone()).unwrap();
            }
            // Removals
            for (val, should_remove) in data.iter() {
                if *should_remove {
                    trie.remove(val.clone()).unwrap();
                    cita_trie.remove(val).unwrap();
                }
            }
            // Compare hashes
            let hash = trie.hash().unwrap().0.to_vec();
            let cita_hash = cita_trie.root().unwrap();
            prop_assert_eq!(hash, cita_hash);
        }

        #[test]
        // The previous test needs to sort the input values in order to get rid of duplicate entries, leading to ordered insertions
        // This check has a fixed way of determining wether a value should be removed but doesn't require ordered insertions
        fn proptest_compare_hash_with_removals_unsorted(data in btree_set(vec(any::<u8>(), 5..100), 1..100)) {
            let mut trie = Trie::new_temp();
            let mut cita_trie = cita_trie();
            // Remove all values that have an odd first value
            let remove = |value: &Vec<u8>| -> bool {
                value.first().is_some_and(|v| v % 2 != 0)
            };
            // Insertions
            for val in data.iter() {
                trie.insert(val.clone(), val.clone()).unwrap();
                cita_trie.insert(val.clone(), val.clone()).unwrap();
            }
            // Removals
            for val in data.iter() {
                if remove(val) {
                    trie.remove(val.clone()).unwrap();
                    cita_trie.remove(val).unwrap();
                }
            }
            // Compare hashes
            let hash = trie.hash().unwrap().0.to_vec();
            let cita_hash = cita_trie.root().unwrap();
            prop_assert_eq!(hash, cita_hash);
        }

        #[test]
        fn proptest_compare_hash_between_inserts(data in btree_set(vec(any::<u8>(), 1..100), 1..100)) {
            let mut trie = Trie::new_temp();
            let mut cita_trie = cita_trie();

            for val in data.iter(){
                trie.insert(val.clone(), val.clone()).unwrap();
                cita_trie.insert(val.clone(), val.clone()).unwrap();
                let hash = trie.hash().unwrap().0.to_vec();
                let cita_hash = cita_trie.root().unwrap();
                prop_assert_eq!(hash, cita_hash);
            }

        }

        #[test]
        fn proptest_compare_proof(data in btree_set(vec(any::<u8>(), 1..100), 1..100)) {
            let mut trie = Trie::new_temp();
            let mut cita_trie = cita_trie();

            for val in data.iter(){
                trie.insert(val.clone(), val.clone()).unwrap();
                cita_trie.insert(val.clone(), val.clone()).unwrap();
            }
            let _ = cita_trie.root();
            for val in data.iter(){
                let proof = trie.get_proof(val).unwrap();
                let cita_proof = cita_trie.get_proof(val).unwrap();
                prop_assert_eq!(proof, cita_proof);
            }
        }

        #[test]
        fn proptest_compare_proof_with_removals(mut data in vec((vec(any::<u8>(), 5..100), any::<bool>()), 1..100)) {
            let mut trie = Trie::new_temp();
            let mut cita_trie = cita_trie();
            // Remove duplicate values with different expected status
            data.sort_by_key(|(val, _)| val.clone());
            data.dedup_by_key(|(val, _)| val.clone());
            // Insertions
            for (val, _) in data.iter() {
                trie.insert(val.clone(), val.clone()).unwrap();
                cita_trie.insert(val.clone(), val.clone()).unwrap();
            }
            // Removals
            for (val, should_remove) in data.iter() {
                if *should_remove {
                    trie.remove(val.clone()).unwrap();
                    cita_trie.remove(val).unwrap();
                }
            }
            // Compare proofs
            let _ = cita_trie.root();
            for (val, _) in data.iter() {
                let proof = trie.get_proof(val).unwrap();
                let cita_proof = cita_trie.get_proof(val).unwrap();
                prop_assert_eq!(proof, cita_proof);
            }
        }


        #[test]
        // The previous test needs to sort the input values in order to get rid of duplicate entries, leading to ordered insertions
        // This check has a fixed way of determining wether a value should be removed but doesn't require ordered insertions
        fn proptest_compare_proof_with_removals_unsorted(data in btree_set(vec(any::<u8>(), 5..100), 1..100)) {
            let mut trie = Trie::new_temp();
            let mut cita_trie = cita_trie();
            // Remove all values that have an odd first value
            let remove = |value: &Vec<u8>| -> bool {
                value.first().is_some_and(|v| v % 2 != 0)
            };
            // Insertions
            for val in data.iter() {
                trie.insert(val.clone(), val.clone()).unwrap();
                cita_trie.insert(val.clone(), val.clone()).unwrap();
            }
            // Removals
            for val in data.iter() {
                if remove(val) {
                    trie.remove(val.clone()).unwrap();
                    cita_trie.remove(val).unwrap();
                }
            }
            // Compare proofs
            let _ = cita_trie.root();
            for val in data.iter() {
                let proof = trie.get_proof(val).unwrap();
                let cita_proof = cita_trie.get_proof(val).unwrap();
                prop_assert_eq!(proof, cita_proof);
            }
        }

    }

    fn cita_trie() -> CitaTrie<CitaMemoryDB, HasherKeccak> {
        let memdb = Arc::new(CitaMemoryDB::new(true));
        let hasher = Arc::new(HasherKeccak::new());

        CitaTrie::new(Arc::clone(&memdb), Arc::clone(&hasher))
    }

    #[test]
    fn get_proof_one_leaf() {
        // Trie -> Leaf["duck"]
        let mut cita_trie = cita_trie();
        let mut trie = Trie::new_temp();
        cita_trie
            .insert(b"duck".to_vec(), b"duckling".to_vec())
            .unwrap();
        trie.insert(b"duck".to_vec(), b"duckling".to_vec()).unwrap();
        let cita_proof = cita_trie.get_proof(b"duck".as_ref()).unwrap();
        let trie_proof = trie.get_proof(&b"duck".to_vec()).unwrap();
        assert_eq!(cita_proof, trie_proof);
    }

    #[test]
    fn get_proof_two_leaves() {
        // Trie -> Extension[Branch[Leaf["duck"] Leaf["goose"]]]
        let mut cita_trie = cita_trie();
        let mut trie = Trie::new_temp();
        cita_trie
            .insert(b"duck".to_vec(), b"duck".to_vec())
            .unwrap();
        cita_trie
            .insert(b"goose".to_vec(), b"goose".to_vec())
            .unwrap();
        trie.insert(b"duck".to_vec(), b"duck".to_vec()).unwrap();
        trie.insert(b"goose".to_vec(), b"goose".to_vec()).unwrap();
        let _ = cita_trie.root();
        let cita_proof = cita_trie.get_proof(b"duck".as_ref()).unwrap();
        let trie_proof = trie.get_proof(&b"duck".to_vec()).unwrap();
        assert_eq!(cita_proof, trie_proof);
    }

    #[test]
    fn get_proof_one_big_leaf() {
        // Trie -> Leaf[[0,0,0,0,0,0,0,0,0,0,0,0,0,0]]
        let val = vec![0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let mut cita_trie = cita_trie();
        let mut trie = Trie::new_temp();
        cita_trie.insert(val.clone(), val.clone()).unwrap();
        trie.insert(val.clone(), val.clone()).unwrap();
        let _ = cita_trie.root();
        let cita_proof = cita_trie.get_proof(&val).unwrap();
        let trie_proof = trie.get_proof(&val).unwrap();
        assert_eq!(cita_proof, trie_proof);
    }

    #[test]
    fn get_proof_path_in_branch() {
        // Trie -> Extension[Branch[ [Leaf[[183,0,0,0,0,0]]], [183]]]
        let mut cita_trie = cita_trie();
        let mut trie = Trie::new_temp();
        cita_trie.insert(vec![183], vec![183]).unwrap();
        cita_trie
            .insert(vec![183, 0, 0, 0, 0, 0], vec![183, 0, 0, 0, 0, 0])
            .unwrap();
        trie.insert(vec![183], vec![183]).unwrap();
        trie.insert(vec![183, 0, 0, 0, 0, 0], vec![183, 0, 0, 0, 0, 0])
            .unwrap();
        let _ = cita_trie.root();
        let cita_proof = cita_trie.get_proof(&[183]).unwrap();
        let trie_proof = trie.get_proof(&vec![183]).unwrap();
        assert_eq!(cita_proof, trie_proof);
    }

    #[test]
    fn get_proof_removed_value() {
        let a = vec![5, 0, 0, 0, 0];
        let b = vec![6, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let mut cita_trie = cita_trie();
        let mut trie = Trie::new_temp();
        cita_trie.insert(a.clone(), a.clone()).unwrap();
        cita_trie.insert(b.clone(), b.clone()).unwrap();
        trie.insert(a.clone(), a.clone()).unwrap();
        trie.insert(b.clone(), b.clone()).unwrap();
        trie.remove(a.clone()).unwrap();
        cita_trie.remove(&a).unwrap();
        let _ = cita_trie.root();
        let cita_proof = cita_trie.get_proof(&a).unwrap();
        let trie_proof = trie.get_proof(&a).unwrap();
        assert_eq!(cita_proof, trie_proof);
    }
}
