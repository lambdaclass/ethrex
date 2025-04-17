use crate::{node_hash::NodeHash, Node, PathRLP};
use slab::Slab;
use std::{
    collections::{hash_map::Entry, HashMap},
    mem,
    ops::Index,
};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct CacheKey(usize);

impl CacheKey {
    pub const INVALID: Self = Self(usize::MAX);

    pub fn is_valid(self) -> bool {
        self != Self::INVALID
    }
}

#[derive(Debug, Default)]
pub struct StateCache {
    storage: Slab<CacheNode>,
    by_path: HashMap<PathRLP, usize>,
}

impl StateCache {
    pub fn get(&self, path: impl AsRef<[u8]>) -> Option<&Node> {
        self.by_path
            .get(path.as_ref())
            .map(|&index| &self.storage[index].value)
    }

    pub fn insert(&mut self, path: PathRLP, value: Node) -> (CacheKey, Option<Node>) {
        let node = CacheNode {
            value,
            hash: NodeHash::default(),
        };

        match self.by_path.entry(path) {
            Entry::Occupied(entry) => (
                CacheKey(*entry.get()),
                Some(mem::replace(&mut self.storage[*entry.get()], node).value),
            ),
            Entry::Vacant(entry) => (CacheKey(*entry.insert(self.storage.insert(node))), None),
        }
    }

    pub fn remove(&mut self, path: &PathRLP) -> Option<Node> {
        self.by_path
            .remove(path)
            .map(|index| self.storage.remove(index).value)
    }

    pub fn clear(&mut self) {
        todo!()
    }
}

impl Index<CacheKey> for StateCache {
    type Output = Node;

    fn index(&self, index: CacheKey) -> &Self::Output {
        &self.storage[index.0].value
    }
}

#[derive(Debug)]
struct CacheNode {
    pub value: Node,
    pub hash: NodeHash,
}
