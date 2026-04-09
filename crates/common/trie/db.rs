use alloc::{boxed::Box, vec, vec::Vec};
use ethrex_rlp::encode::RLPEncode;

use crate::{Nibbles, Node, error::TrieError};

/// Conditional Send + Sync bound: required in std (for thread-pool trie
/// generation in snap sync), not required in no_std (single-threaded guest
/// programs).
#[cfg(feature = "std")]
pub trait MaybeSendSync: Send + Sync {}
#[cfg(feature = "std")]
impl<T: Send + Sync> MaybeSendSync for T {}

#[cfg(not(feature = "std"))]
pub trait MaybeSendSync {}
#[cfg(not(feature = "std"))]
impl<T> MaybeSendSync for T {}

pub trait TrieDB: MaybeSendSync {
    fn get(&self, key: Nibbles) -> Result<Option<Vec<u8>>, TrieError>;
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
    /// Commits any pending changes to the underlying storage
    /// For read-only or in-memory implementations, this is a no-op
    fn commit(&self) -> Result<(), TrieError> {
        Ok(())
    }

    fn flatkeyvalue_computed(&self, _key: Nibbles) -> bool {
        false
    }
}

// std: InMemoryTrieDB backed by Arc<Mutex<BTreeMap>> for thread safety
#[cfg(feature = "std")]
mod in_memory_std {
    use alloc::{collections::BTreeMap, sync::Arc, vec};
    use ethereum_types::H256;
    use std::sync::Mutex;

    use super::TrieDB;
    use crate::{Nibbles, Node, Trie, error::TrieError};

    // TODO: we should replace this with BackendTrieDB
    /// InMemory implementation for the TrieDB trait, with get and put operations.
    pub struct InMemoryTrieDB {
        inner: Arc<Mutex<BTreeMap<Vec<u8>, Vec<u8>>>>,
        prefix: Option<Nibbles>,
    }

    impl Default for InMemoryTrieDB {
        fn default() -> Self {
            Self {
                inner: Arc::new(Mutex::new(BTreeMap::new())),
                prefix: None,
            }
        }
    }

    impl InMemoryTrieDB {
        pub fn new(map: BTreeMap<Vec<u8>, Vec<u8>>) -> Self {
            Self {
                inner: Arc::new(Mutex::new(map)),
                prefix: None,
            }
        }

        pub fn from_nodes(
            root_hash: H256,
            state_nodes: &BTreeMap<H256, Node>,
        ) -> Result<Self, TrieError> {
            let mut embedded_root = Trie::get_embedded_root(state_nodes, root_hash)?;
            let mut hashed_nodes = vec![];
            embedded_root.commit(
                Nibbles::default(),
                &mut hashed_nodes,
                &ethrex_crypto::NativeCrypto,
            );

            let hashed_nodes = hashed_nodes
                .into_iter()
                .map(|(k, v)| (k.into_vec(), v))
                .collect();

            Ok(Self::new(hashed_nodes))
        }

        fn apply_prefix(&self, path: Nibbles) -> Nibbles {
            match &self.prefix {
                Some(prefix) => prefix.concat(&path),
                None => path,
            }
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
}

// no_std: InMemoryTrieDB backed by RefCell<BTreeMap> (single-threaded)
#[cfg(not(feature = "std"))]
mod in_memory_nostd {
    use alloc::{collections::BTreeMap, vec::Vec};
    use core::cell::RefCell;

    use super::TrieDB;
    use crate::{Nibbles, error::TrieError};

    /// InMemory implementation for the TrieDB trait, for single-threaded
    /// no_std environments (guest programs).
    pub struct InMemoryTrieDB {
        inner: RefCell<BTreeMap<Vec<u8>, Vec<u8>>>,
        prefix: Option<Nibbles>,
    }

    impl Default for InMemoryTrieDB {
        fn default() -> Self {
            Self {
                inner: RefCell::new(BTreeMap::new()),
                prefix: None,
            }
        }
    }

    impl InMemoryTrieDB {
        pub fn new(map: BTreeMap<Vec<u8>, Vec<u8>>) -> Self {
            Self {
                inner: RefCell::new(map),
                prefix: None,
            }
        }

        fn apply_prefix(&self, path: Nibbles) -> Nibbles {
            match &self.prefix {
                Some(prefix) => prefix.concat(&path),
                None => path,
            }
        }
    }

    impl TrieDB for InMemoryTrieDB {
        fn get(&self, key: Nibbles) -> Result<Option<Vec<u8>>, TrieError> {
            Ok(self
                .inner
                .borrow()
                .get(self.apply_prefix(key).as_ref())
                .cloned())
        }

        fn put_batch(&self, key_values: Vec<(Nibbles, Vec<u8>)>) -> Result<(), TrieError> {
            let mut db = self.inner.borrow_mut();

            for (key, value) in key_values {
                let prefixed_key = self.apply_prefix(key);
                db.insert(prefixed_key.into_vec(), value);
            }

            Ok(())
        }
    }
}

#[cfg(not(feature = "std"))]
pub use in_memory_nostd::InMemoryTrieDB;
#[cfg(feature = "std")]
pub use in_memory_std::InMemoryTrieDB;
