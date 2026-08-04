//! [`BinaryTrieDB`] over ethrex's storage backend: the EIP-8297 binary
//! trie's nodes on disk, in the [`BINARY_TRIE_NODES`] column family.
//!
//! Deliberately separate from [`BackendTrieDB`], which does the same job
//! for the MPT. The two trie traits are separate by design, and the MPT
//! path carries machinery — an address prefix for per-account storage
//! subtries, and the flat key-value tables the trie keys dispatch
//! between — that has no counterpart here: the binary trie is one
//! unified tree, so a node's bit path is the whole key and there is one
//! table to read.
//!
//! [`BackendTrieDB`]: crate::trie::BackendTrieDB

use crate::api::tables::BINARY_TRIE_NODES;
use crate::api::{StorageBackend, StorageReadView};
use crate::error::StoreError;
use ethrex_binary_trie::BinaryTrieError;
use ethrex_binary_trie::trie::{BinaryTrieDB, BitPath};
use std::sync::Arc;

/// [`BinaryTrieDB`] holding a pre-acquired read view for a whole trie
/// traversal, so a descent costs one lock acquisition rather than one
/// per node — the same arrangement [`BackendTrieDB`] uses.
///
/// The view is a point-in-time snapshot on some backends (the in-memory
/// one, notably), so a handle does not necessarily see writes made
/// through it after it was constructed. That is what the trie wants: it
/// opens a handle at a root, reads the state that root addresses, and
/// commits once. A reader that must see a newer commit takes a new
/// handle.
///
/// [`BackendTrieDB`]: crate::trie::BackendTrieDB
pub struct BackendBinaryTrieDB {
    /// The storage backend, used only for writes.
    db: Arc<dyn StorageBackend>,
    /// Pre-acquired read view, held for this handle's lifetime.
    read_view: Arc<dyn StorageReadView>,
}

impl BackendBinaryTrieDB {
    /// A handle on `db`, acquiring its read view now.
    pub fn new(db: Arc<dyn StorageBackend>) -> Result<Self, StoreError> {
        let read_view = db.begin_read()?;
        Ok(Self::with_view(db, read_view))
    }

    /// A handle on `db` sharing an existing read view, so several
    /// handles used in one query read one consistent snapshot.
    pub fn with_view(db: Arc<dyn StorageBackend>, read_view: Arc<dyn StorageReadView>) -> Self {
        Self { db, read_view }
    }
}

impl BinaryTrieDB for BackendBinaryTrieDB {
    fn get(&self, path: &BitPath) -> Result<Option<Vec<u8>>, BinaryTrieError> {
        self.read_view
            .get(BINARY_TRIE_NODES, &path.to_db_key())
            .map_err(backend_error)
    }

    /// Writes every entry in one transaction, so a commit either lands
    /// whole or not at all.
    ///
    /// An empty value is a tombstone, not a node: it deletes the key
    /// rather than storing zero bytes at it, so a later [`get`] answers
    /// `None`. Storing the empty value instead would make the path read
    /// back as a node the trie never wrote, and decoding would fail.
    ///
    /// [`get`]: BinaryTrieDB::get
    fn put_batch(&self, entries: Vec<(BitPath, Vec<u8>)>) -> Result<(), BinaryTrieError> {
        let mut tx = self.db.begin_write().map_err(backend_error)?;
        let mut writes = Vec::with_capacity(entries.len());
        for (path, encoded) in entries {
            let key = path.to_db_key();
            if encoded.is_empty() {
                tx.delete(BINARY_TRIE_NODES, &key).map_err(backend_error)?;
            } else {
                writes.push((key, encoded));
            }
        }
        tx.put_batch(BINARY_TRIE_NODES, writes)
            .map_err(backend_error)?;
        tx.commit().map_err(backend_error)
    }
}

/// A storage failure as the trie's backend error, the variant that
/// exists for exactly this.
fn backend_error(e: StoreError) -> BinaryTrieError {
    BinaryTrieError::Backend(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::StorageBackend;
    use crate::api::tables::{BINARY_TRIE_NODES, TABLES};
    use crate::backend::in_memory::InMemoryBackend;
    use ethrex_binary_trie::trie::{BinaryTrie, BinaryTrieDB, BitPath};
    use std::sync::Arc;

    fn backend() -> Arc<dyn StorageBackend> {
        Arc::new(InMemoryBackend::open().expect("in-memory backend opens"))
    }

    /// A fresh handle on `db`.
    ///
    /// Reads go through a read view acquired at construction, so every
    /// test that writes and then reads takes a new handle rather than
    /// reusing the writing one — the in-memory backend's read view is a
    /// point-in-time snapshot.
    fn handle(db: &Arc<dyn StorageBackend>) -> BackendBinaryTrieDB {
        BackendBinaryTrieDB::new(Arc::clone(db)).expect("read view opens")
    }

    /// Four keys in the 34-byte shape the embedding produces, pairwise
    /// prefix-free, differing in their first byte.
    fn sample_entries() -> Vec<(Vec<u8>, [u8; 32])> {
        [0x00u8, 0x01, 0x80, 0xff]
            .into_iter()
            .enumerate()
            .map(|(i, first)| {
                let mut key = vec![0x00; 34];
                key[0] = first;
                (key, [i as u8; 32])
            })
            .collect()
    }

    #[test]
    fn binary_trie_nodes_is_a_registered_table() {
        // Unregistered column families are dropped at startup by
        // `drop_obsolete_cfs`, so a node table missing from `TABLES`
        // would lose the whole trie on the next boot.
        assert!(TABLES.contains(&BINARY_TRIE_NODES));
    }

    #[test]
    fn round_trips_through_the_backend() {
        let db = backend();
        let root_path = BitPath::new();
        let left = BitPath::from_bits(&[0, 1, 0]);
        let deep = BitPath::from_bits(&[1; 17]);

        assert_eq!(handle(&db).get(&root_path).unwrap(), None);

        handle(&db)
            .put_batch(vec![
                (root_path.clone(), vec![0x01, 0x02]),
                (left.clone(), vec![0x03]),
                (deep.clone(), vec![0x04, 0x05, 0x06]),
            ])
            .unwrap();

        let reader = handle(&db);
        assert_eq!(reader.get(&root_path).unwrap(), Some(vec![0x01, 0x02]));
        assert_eq!(reader.get(&left).unwrap(), Some(vec![0x03]));
        assert_eq!(reader.get(&deep).unwrap(), Some(vec![0x04, 0x05, 0x06]));
        assert_eq!(reader.get(&BitPath::from_bits(&[1, 1, 1])).unwrap(), None);

        // Single-version storage: writing a path again overwrites it.
        handle(&db)
            .put_batch(vec![(left.clone(), vec![0x07])])
            .unwrap();
        assert_eq!(handle(&db).get(&left).unwrap(), Some(vec![0x07]));
    }

    #[test]
    fn an_empty_value_is_a_tombstone() {
        let db = backend();
        let path = BitPath::from_bits(&[1, 0, 1]);

        handle(&db)
            .put_batch(vec![(path.clone(), vec![0xaa, 0xbb])])
            .unwrap();
        assert_eq!(handle(&db).get(&path).unwrap(), Some(vec![0xaa, 0xbb]));

        handle(&db).put_batch(vec![(path.clone(), vec![])]).unwrap();
        // `None`, not `Some(vec![])`: the node left the tree, and a
        // zero-byte value would decode as a malformed node.
        assert_eq!(handle(&db).get(&path).unwrap(), None);

        // Tombstoning a path that was never written is not an error.
        handle(&db)
            .put_batch(vec![(BitPath::from_bits(&[0]), vec![])])
            .unwrap();
        assert_eq!(handle(&db).get(&BitPath::from_bits(&[0])).unwrap(), None);
    }

    #[test]
    fn trailing_zero_bits_do_not_collide() {
        // The injectivity `BitPath::to_db_key`'s explicit bit count
        // exists for, checked through the real key encoding and a real
        // column family: without the count these paths pack to the same
        // bytes and one node silently overwrites the other.
        let db = backend();
        let short = BitPath::from_bits(&[1]);
        let long = BitPath::from_bits(&[1, 0]);
        let longer = BitPath::from_bits(&[1, 0, 0]);
        let root = BitPath::new();
        let zero = BitPath::from_bits(&[0]);

        handle(&db)
            .put_batch(vec![
                (short.clone(), vec![0x01]),
                (long.clone(), vec![0x02]),
                (longer.clone(), vec![0x03]),
                (root.clone(), vec![0x04]),
                (zero.clone(), vec![0x05]),
            ])
            .unwrap();

        let reader = handle(&db);
        assert_eq!(reader.get(&short).unwrap(), Some(vec![0x01]));
        assert_eq!(reader.get(&long).unwrap(), Some(vec![0x02]));
        assert_eq!(reader.get(&longer).unwrap(), Some(vec![0x03]));
        assert_eq!(reader.get(&root).unwrap(), Some(vec![0x04]));
        assert_eq!(reader.get(&zero).unwrap(), Some(vec![0x05]));

        // And a tombstone at one of them leaves its neighbours alone.
        handle(&db).put_batch(vec![(long.clone(), vec![])]).unwrap();
        let reader = handle(&db);
        assert_eq!(reader.get(&long).unwrap(), None);
        assert_eq!(reader.get(&short).unwrap(), Some(vec![0x01]));
        assert_eq!(reader.get(&longer).unwrap(), Some(vec![0x03]));
    }

    #[test]
    fn a_trie_reopens_over_the_same_database() {
        let db = backend();
        let entries = sample_entries();

        let mut trie = BinaryTrie::new(Box::new(handle(&db)));
        for (key, value) in &entries {
            trie.insert(key.clone(), *value).unwrap();
        }
        let root = trie.commit().unwrap();
        drop(trie);

        // A fresh handle on the same database, so nothing is inherited
        // but the bytes on disk.
        let mut reopened = BinaryTrie::open(Box::new(handle(&db)), root);
        for (key, value) in &entries {
            assert_eq!(reopened.get(key).unwrap(), Some(*value), "key {key:?}");
        }
        assert_eq!(reopened.root(), root);

        // The same set built with no database at all commits to the
        // same root, so nothing about the storage round trip perturbed
        // the structure.
        let mut fresh = BinaryTrie::new_temp();
        for (key, value) in &entries {
            fresh.insert(key.clone(), *value).unwrap();
        }
        assert_eq!(fresh.commit().unwrap(), root);
    }

    #[test]
    fn a_removal_survives_a_reopen() {
        let db = backend();
        let entries = sample_entries();
        let (removed_key, _) = entries[1].clone();

        let mut trie = BinaryTrie::new(Box::new(handle(&db)));
        for (key, value) in &entries {
            trie.insert(key.clone(), *value).unwrap();
        }
        let full_root = trie.commit().unwrap();
        drop(trie);

        let mut trie = BinaryTrie::open(Box::new(handle(&db)), full_root);
        assert!(trie.remove(&removed_key).unwrap().is_some());
        let pruned_root = trie.commit().unwrap();
        drop(trie);

        // Reopening reads the collapsed structure back, which only
        // works if the tombstones written by the removal actually
        // deleted their keys.
        let mut reopened = BinaryTrie::open(Box::new(handle(&db)), pruned_root);
        assert_eq!(reopened.get(&removed_key).unwrap(), None);
        for (key, value) in entries.iter().filter(|(key, _)| key != &removed_key) {
            assert_eq!(reopened.get(key).unwrap(), Some(*value), "key {key:?}");
        }

        // Canonical form: the pruned trie is the trie of the remaining
        // keys, not merely a trie that answers the same queries.
        let mut fresh = BinaryTrie::new_temp();
        for (key, value) in entries.iter().filter(|(key, _)| key != &removed_key) {
            fresh.insert(key.clone(), *value).unwrap();
        }
        assert_eq!(pruned_root, fresh.commit().unwrap());
        assert_ne!(pruned_root, full_root);

        // The removal's tombstones deleted their keys rather than
        // storing zero bytes at them, so the column family holds no
        // empty value anywhere.
        let read_view = db.begin_read().unwrap();
        let empties: Vec<Vec<u8>> = read_view
            .prefix_iterator(BINARY_TRIE_NODES, &[])
            .unwrap()
            .map(|entry| entry.unwrap())
            .filter(|(_, value)| value.is_empty())
            .map(|(key, _)| key.into_vec())
            .collect();
        assert!(
            empties.is_empty(),
            "tombstoned keys left behind: {empties:?}"
        );

        // Reinserting reaches the original trie again, which means the
        // paths the removal tombstoned are readable as absent rather
        // than as a node the tree never wrote.
        let mut reinserted = BinaryTrie::open(Box::new(handle(&db)), pruned_root);
        reinserted
            .insert(removed_key.clone(), entries[1].1)
            .unwrap();
        assert_eq!(reinserted.commit().unwrap(), full_root);
    }
}
