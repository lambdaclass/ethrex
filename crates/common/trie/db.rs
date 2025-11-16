use ethereum_types::H256;
use ethrex_rlp::encode::RLPEncode;

use crate::{Nibbles, Node, NodeRLP, Trie, error::TrieError};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicPtr, AtomicUsize},
    },
};

// Nibbles -> encoded node
pub type NodeMap = Arc<Mutex<BTreeMap<Vec<u8>, Vec<u8>>>>;

pub trait TrieDB: Send + Sync {
    fn get(&self, key: Nibbles) -> Result<Option<Vec<u8>>, TrieError>;
    fn get_nodes_in_path(
        &self,
        key: Nibbles,
        start: usize,
        count: usize,
    ) -> Result<Vec<Option<Vec<u8>>>, TrieError> {
        let keys = (start..start + count).map(|i| key.slice(0, i));
        let mut values = Vec::with_capacity(count);
        for key in keys {
            values.push(self.get(key)?);
        }
        Ok(values)
    }
    fn put_batch(&self, key_values: Vec<(Nibbles, Vec<u8>)>) -> Result<(), TrieError>;
    // TODO: replace putbatch with this function.
    fn put_batch_no_alloc(&self, key_values: &[(Nibbles, Node)]) -> Result<(), TrieError> {
        self.put_batch(
            key_values
                .iter()
                .map(|node| (node.0.clone(), node.1.encode_to_vec()))
                .collect(),
        )
    }
    fn put(&self, key: Nibbles, value: Vec<u8>) -> Result<(), TrieError> {
        self.put_batch(vec![(key, value)])
    }
    fn flatkeyvalue_computed(&self, _key: Nibbles) -> bool {
        false
    }
}

pub(crate) struct BulkTrieDB<'a> {
    db: &'a dyn TrieDB,
    path: Nibbles,
    nodes: AtomicPtr<Option<Vec<u8>>>,
    nodes_count: AtomicUsize,
    nodes_cap: AtomicUsize,
    first_idx: AtomicUsize,
}

impl<'a> BulkTrieDB<'a> {
    pub fn new(db: &'a dyn TrieDB, path: Nibbles) -> Self {
        Self {
            db,
            path,
            // NOTE: in normal usage, none of these atomics will be contended,
            // they were chosen just to avoid playing with `UnsafeCell` while
            // meeting the trait requirements of `Send + Sync`.
            nodes: AtomicPtr::default(),
            nodes_count: AtomicUsize::default(),
            // NOTE: needed to meet the invariants for freeing
            nodes_cap: AtomicUsize::default(),
            first_idx: AtomicUsize::default(),
        }
    }

    fn get_nodes(&self, first: usize, count: usize) -> Result<&'a [Option<Vec<u8>>], TrieError> {
        // NOTE: in theory, `leak` could produce a `NULL` pointer if the vector
        // is empty. Using `with_capacity` guarantees it's not `NULL` because it
        // forces preallocation. So, in this initial version that call to
        // `with_capacity` has semantic relevance and is not just an optimization.
        use std::sync::atomic::Ordering::Relaxed;
        let nodes_ptr = self.nodes.load(Relaxed);
        if !nodes_ptr.is_null() {
            let count = self.nodes_count.load(Relaxed);
            let nodes = unsafe { std::slice::from_raw_parts(nodes_ptr, count) };
            return Ok(nodes);
        }
        let encoded_nodes = self.db.get_nodes_in_path(self.path.clone(), first, count)?;
        let cap = encoded_nodes.capacity();
        let encoded_nodes = encoded_nodes.leak();
        self.nodes_count.store(encoded_nodes.len(), Relaxed);
        self.nodes_cap.store(cap, Relaxed);
        self.nodes.store(encoded_nodes.as_ptr().cast_mut(), Relaxed);
        self.first_idx.store(first, Relaxed);
        Ok(encoded_nodes)
    }
}
impl<'a> Drop for BulkTrieDB<'a> {
    fn drop(&mut self) {
        use std::sync::atomic::Ordering::Relaxed;
        let ptr = self.nodes.load(Relaxed);
        if ptr.is_null() {
            return;
        }
        let len = self.nodes_count.load(Relaxed);
        let cap = self.nodes_cap.load(Relaxed);
        unsafe { Vec::from_raw_parts(ptr, len, cap) };
    }
}
impl<'a> TrieDB for BulkTrieDB<'a> {
    fn get(&self, key: Nibbles) -> Result<Option<Vec<u8>>, TrieError> {
        if !self.path.as_ref().starts_with(key.as_ref()) {
            // key not in path
            return Ok(None);
        }
        let count = 14; //self.path.len().saturating_sub(key.len()).min(14);
        let nodes = self.get_nodes(key.len(), count)?;
        // Because we skip some nodes, we need to offset the relative position
        // by the difference between the full path and what we actually have.
        let index = key.len() - self.first_idx.load(std::sync::atomic::Ordering::Relaxed);
        Ok(nodes.get(index).cloned().flatten())
    }
    fn put_batch(&self, key_values: Vec<(Nibbles, Vec<u8>)>) -> Result<(), TrieError> {
        unimplemented!()
    }
}
/// InMemory implementation for the TrieDB trait, with get and put operations.
#[derive(Default)]
pub struct InMemoryTrieDB {
    inner: NodeMap,
    prefix: Option<Nibbles>,
}

impl InMemoryTrieDB {
    pub const fn new(map: NodeMap) -> Self {
        Self {
            inner: map,
            prefix: None,
        }
    }

    pub const fn new_with_prefix(map: NodeMap, prefix: Nibbles) -> Self {
        Self {
            inner: map,
            prefix: Some(prefix),
        }
    }

    pub fn new_empty() -> Self {
        Self {
            inner: Default::default(),
            prefix: None,
        }
    }

    // Do not remove or make private as we use this in ethrex-replay
    pub fn from_nodes(
        root_hash: H256,
        state_nodes: &BTreeMap<H256, NodeRLP>,
    ) -> Result<Self, TrieError> {
        let mut embedded_root = Trie::get_embedded_root(state_nodes, root_hash)?;
        let mut hashed_nodes = vec![];
        embedded_root.commit(Nibbles::default(), &mut hashed_nodes);

        let hashed_nodes = hashed_nodes
            .into_iter()
            .map(|(k, v)| (k.into_vec(), v))
            .collect();

        let in_memory_trie = Arc::new(Mutex::new(hashed_nodes));
        Ok(Self::new(in_memory_trie))
    }

    fn apply_prefix(&self, path: Nibbles) -> Nibbles {
        match &self.prefix {
            Some(prefix) => prefix.concat(&path),
            None => path,
        }
    }

    // Do not remove or make private as we use this in ethrex-replay
    pub fn inner(&self) -> NodeMap {
        Arc::clone(&self.inner)
    }
}

impl TrieDB for InMemoryTrieDB {
    fn get(&self, key: Nibbles) -> Result<Option<Vec<u8>>, TrieError> {
        Ok(self
            .inner
            .lock()
            .map_err(|_| TrieError::LockError)?
            .get(self.apply_prefix(key).as_ref())
            .cloned())
    }

    fn put_batch(&self, key_values: Vec<(Nibbles, Vec<u8>)>) -> Result<(), TrieError> {
        let mut db = self.inner.lock().map_err(|_| TrieError::LockError)?;

        for (key, value) in key_values {
            let prefixed_key = self.apply_prefix(key);
            db.insert(prefixed_key.into_vec(), value);
        }

        Ok(())
    }
}
