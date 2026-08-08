//! The storage backend nodes are read from and written to.
//!
//! Deliberately a trait with an in-memory implementation and nothing
//! else: the real database lives in `crates/storage`, so this crate
//! stays free of a database dependency. The in-memory implementation
//! is permanent, not scaffolding — it is what tests and any
//! database-less consumer run against, mirroring `InMemoryTrieDB` on
//! the MPT side.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};

use crate::error::BinaryTrieError;

use super::path::BitPath;

/// Encoded nodes keyed by [`BitPath::to_db_key`].
type Nodes = BTreeMap<Vec<u8>, Vec<u8>>;

/// A set of nodes, shareable so several handles can address one set.
pub type NodeMap = Arc<Mutex<Nodes>>;

/// Node storage: path-keyed, single-version, overwriting in place —
/// the same model `TrieDB` uses for the MPT.
pub trait BinaryTrieDB: Send + Sync {
    /// Encoded node stored at `path`, or `None` if there is none.
    fn get(&self, path: &BitPath) -> Result<Option<Vec<u8>>, BinaryTrieError>;

    /// Write every entry, replacing whatever was at each path.
    ///
    /// An empty value is a tombstone: the node at that path left the
    /// tree and the implementation must delete it, so a later [`get`]
    /// answers `None`. No node ever encodes to zero bytes — a node
    /// encoding starts with a tag — so the two cannot be confused.
    /// Removal rides in the write batch rather than in a method of its
    /// own so that one atomic batch carries a whole commit, the same
    /// arrangement the MPT's `TrieDB` uses.
    ///
    /// [`get`]: BinaryTrieDB::get
    fn put_batch(&self, entries: Vec<(BitPath, Vec<u8>)>) -> Result<(), BinaryTrieError>;

    /// Whether a read of `key` may be answered from the flat leaf mirror
    /// instead of by descending the tree.
    ///
    /// **A coverage question, and only that.** `true` promises that
    /// [`binary_flat_get`] is *authoritative for this key in both
    /// directions*: it returns the leaf's value if the tree holds it, and
    /// `None` if the tree does not. A `None` under a `true` answer is
    /// therefore a definitive absence and [`BinaryTrie::get`] does **not**
    /// fall back to a descent — which is why an implementation that is
    /// merely *usually* right must answer `false`. A mirror that is a subset
    /// of the tree makes reads lose live state; a mirror that is a superset
    /// makes them invent it.
    ///
    /// Defaults to `false`, so an implementation that keeps no mirror — the
    /// in-memory one, every test that does not care — is unaffected and
    /// every read descends. The MPT's `TrieDB::flatkeyvalue_computed` has
    /// exactly this default for exactly this reason.
    ///
    /// Note this is *not* the same question as "who is responsible for
    /// writing this key's mirror row". A backfill generator that has not
    /// started owns nothing, so the commit path owns the whole keyspace —
    /// and yet no reader may trust the mirror, because nothing has populated
    /// it. The two predicates are separate on purpose and answer an absent
    /// frontier oppositely.
    ///
    /// [`binary_flat_get`]: BinaryTrieDB::binary_flat_get
    /// [`BinaryTrie::get`]: super::BinaryTrie::get
    fn binary_flat_computed(&self, _key: &[u8]) -> bool {
        false
    }

    /// The flat mirror's value for `key`, consulted only when
    /// [`binary_flat_computed`] answered `true` for it.
    ///
    /// Defaults to `Ok(None)`, which is unreachable behind the `false`
    /// default above and would be read as a definitive absence if it ever
    /// were reached — so an implementation overriding one of the two must
    /// override both.
    ///
    /// [`binary_flat_computed`]: BinaryTrieDB::binary_flat_computed
    fn binary_flat_get(&self, _key: &[u8]) -> Result<Option<[u8; 32]>, BinaryTrieError> {
        Ok(None)
    }
}

/// In-memory [`BinaryTrieDB`], backed by a map that clones and
/// [`InMemoryBinaryTrieDB::new`] share rather than copy.
#[derive(Clone, Default)]
pub struct InMemoryBinaryTrieDB {
    inner: NodeMap,
}

impl InMemoryBinaryTrieDB {
    /// A handle on an existing set of nodes.
    pub const fn new(map: NodeMap) -> Self {
        Self { inner: map }
    }

    /// A handle on a fresh, empty set of nodes.
    pub fn new_empty() -> Self {
        Self::default()
    }

    /// The underlying map, for opening a second handle on the same
    /// nodes.
    pub fn inner(&self) -> NodeMap {
        Arc::clone(&self.inner)
    }

    fn lock_inner(&self) -> Result<MutexGuard<'_, Nodes>, BinaryTrieError> {
        self.inner
            .lock()
            .map_err(|_| BinaryTrieError::Backend("node map mutex poisoned".into()))
    }
}

impl BinaryTrieDB for InMemoryBinaryTrieDB {
    fn get(&self, path: &BitPath) -> Result<Option<Vec<u8>>, BinaryTrieError> {
        Ok(self.lock_inner()?.get(&path.to_db_key()).cloned())
    }

    fn put_batch(&self, entries: Vec<(BitPath, Vec<u8>)>) -> Result<(), BinaryTrieError> {
        let mut nodes = self.lock_inner()?;
        for (path, encoded) in entries {
            if encoded.is_empty() {
                nodes.remove(&path.to_db_key());
            } else {
                nodes.insert(path.to_db_key(), encoded);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trie::path::BitPath;

    #[test]
    fn in_memory_round_trips() {
        let db = InMemoryBinaryTrieDB::new_empty();
        let root_path = BitPath::new();
        let left = BitPath::from_bits(&[1, 0, 1]);

        assert_eq!(db.get(&root_path).unwrap(), None);
        db.put_batch(vec![
            (root_path.clone(), vec![0x01, 0x02]),
            (left.clone(), vec![0x03]),
        ])
        .unwrap();

        assert_eq!(db.get(&root_path).unwrap(), Some(vec![0x01, 0x02]));
        assert_eq!(db.get(&left).unwrap(), Some(vec![0x03]));
        assert_eq!(db.get(&BitPath::from_bits(&[1, 1, 1])).unwrap(), None);

        // Writing the same path again overwrites in place, as the
        // single-version storage model requires.
        db.put_batch(vec![(left.clone(), vec![0x04])]).unwrap();
        assert_eq!(db.get(&left).unwrap(), Some(vec![0x04]));
    }

    #[test]
    fn handles_share_one_node_map() {
        let db = InMemoryBinaryTrieDB::new_empty();
        let other = InMemoryBinaryTrieDB::new(db.inner());
        db.put_batch(vec![(BitPath::from_bits(&[0]), vec![0xaa])])
            .unwrap();
        assert_eq!(
            other.get(&BitPath::from_bits(&[0])).unwrap(),
            Some(vec![0xaa])
        );
    }

    #[test]
    fn db_keys_distinguish_trailing_zero_bits() {
        // The same injectivity `encode_bit_prefix` has: without the bit
        // count, these two paths would pack to identical bytes and one
        // node would overwrite the other.
        assert_ne!(
            BitPath::from_bits(&[1]).to_db_key(),
            BitPath::from_bits(&[1, 0]).to_db_key()
        );
        assert_ne!(
            BitPath::new().to_db_key(),
            BitPath::from_bits(&[0]).to_db_key()
        );

        let db = InMemoryBinaryTrieDB::new_empty();
        db.put_batch(vec![
            (BitPath::from_bits(&[1]), vec![0x01]),
            (BitPath::from_bits(&[1, 0]), vec![0x02]),
        ])
        .unwrap();
        assert_eq!(db.get(&BitPath::from_bits(&[1])).unwrap(), Some(vec![0x01]));
        assert_eq!(
            db.get(&BitPath::from_bits(&[1, 0])).unwrap(),
            Some(vec![0x02])
        );
    }
}
