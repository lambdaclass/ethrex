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

use super::group::{DEFAULT_GROUP_DEPTH, GroupDepth};
use super::path::BitPath;

/// Encoded group rows keyed by [`BitPath::to_db_key`] of the group root.
type Rows = BTreeMap<Vec<u8>, Vec<u8>>;

/// A set of rows, shareable so several handles can address one set.
pub type NodeMap = Arc<Mutex<Rows>>;

/// Row storage: path-keyed, single-version, overwriting in place —
/// the same model `TrieDB` uses for the MPT.
///
/// **The unit is a group row, not a node.** One row holds every node of
/// one group, encoded verbatim by [`GroupRow`](super::group::GroupRow);
/// the key is [`BitPath::to_db_key`] of the group *root*, so a key's
/// shape is unchanged and only the bit counts that can appear shrink to
/// the multiples of [`BinaryTrieDB::group_depth`]. A backend stores
/// bytes at keys and never looks inside a row.
pub trait BinaryTrieDB: Send + Sync {
    /// How many levels of tree one row of this store spans.
    ///
    /// Answered by the *backend* because the depth is a property of the
    /// datadir the rows were written under, not of the trie reading them:
    /// rows written at one depth and read at another address different
    /// groups, and the reader would silently miss members.
    ///
    /// Defaults to [`DEFAULT_GROUP_DEPTH`] so a backend that has no
    /// datadir to consult — the in-memory one, every test that does not
    /// care — needs no wiring. **A wrapper delegating to another backend
    /// must forward this**: taking the default while the wrapped store
    /// was built at a different depth is a silent disagreement, and the
    /// only symptom is a read that finds no member where the parent's
    /// hash says one exists.
    fn group_depth(&self) -> GroupDepth {
        DEFAULT_GROUP_DEPTH
    }

    /// Encoded row of the group rooted at `group_root`, or `None` if
    /// there is none.
    ///
    /// `group_root` is a group root — its bit length is a multiple of
    /// [`group_depth`] — which is what a caller gets from
    /// [`group_root`](super::group::group_root). Passing a node path
    /// straight through addresses a different row and finds nothing.
    ///
    /// [`group_depth`]: BinaryTrieDB::group_depth
    fn get_group(&self, group_root: &BitPath) -> Result<Option<Vec<u8>>, BinaryTrieError>;

    /// Write every row, replacing whatever was at each row key.
    ///
    /// An empty value is a tombstone: the group lost its **last** member
    /// and the implementation must delete the row, so a later
    /// [`get_group`] answers `None`. A group that lost *some* of its
    /// members arrives here as an ordinary rewrite carrying the
    /// survivors — that distinction is the single place a live node is
    /// easiest to lose. No row ever encodes to zero bytes — a row starts
    /// with a version byte — so a tombstone cannot be confused for one.
    /// Removal rides in the write batch rather than in a method of its
    /// own so that one atomic batch carries a whole commit, the same
    /// arrangement the MPT's `TrieDB` uses.
    ///
    /// [`get_group`]: BinaryTrieDB::get_group
    fn put_groups(&self, rows: Vec<(BitPath, Vec<u8>)>) -> Result<(), BinaryTrieError>;

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
    /// `None` means [`DEFAULT_GROUP_DEPTH`]; set explicitly by
    /// [`InMemoryBinaryTrieDB::at_group_depth`] so a test can run the
    /// whole trie at any depth 1..=8 rather than only at the default.
    group_depth: Option<GroupDepth>,
}

impl InMemoryBinaryTrieDB {
    /// A handle on an existing set of rows, at the default group depth.
    pub const fn new(map: NodeMap) -> Self {
        Self {
            inner: map,
            group_depth: None,
        }
    }

    /// A handle on a fresh, empty set of rows.
    pub fn new_empty() -> Self {
        Self::default()
    }

    /// The same handle, reading and writing rows of `depth` levels.
    pub const fn at_group_depth(mut self, depth: GroupDepth) -> Self {
        self.group_depth = Some(depth);
        self
    }

    /// The underlying map, for opening a second handle on the same
    /// rows.
    pub fn inner(&self) -> NodeMap {
        Arc::clone(&self.inner)
    }

    fn lock_inner(&self) -> Result<MutexGuard<'_, Rows>, BinaryTrieError> {
        self.inner
            .lock()
            .map_err(|_| BinaryTrieError::Backend("node map mutex poisoned".into()))
    }
}

impl BinaryTrieDB for InMemoryBinaryTrieDB {
    fn group_depth(&self) -> GroupDepth {
        self.group_depth.unwrap_or(DEFAULT_GROUP_DEPTH)
    }

    fn get_group(&self, group_root: &BitPath) -> Result<Option<Vec<u8>>, BinaryTrieError> {
        Ok(self.lock_inner()?.get(&group_root.to_db_key()).cloned())
    }

    fn put_groups(&self, rows: Vec<(BitPath, Vec<u8>)>) -> Result<(), BinaryTrieError> {
        let mut stored = self.lock_inner()?;
        for (group_root, encoded) in rows {
            if encoded.is_empty() {
                stored.remove(&group_root.to_db_key());
            } else {
                stored.insert(group_root.to_db_key(), encoded);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trie::group::{GroupRow, MAX_GROUP_DEPTH, group_root};
    use crate::trie::path::BitPath;

    fn depth(levels: usize) -> GroupDepth {
        GroupDepth::new(levels).expect("valid group depth")
    }

    /// A row holding one member at `relative`, as bytes.
    fn row(relative: &[u8], node: &[u8], g: GroupDepth) -> Vec<u8> {
        let mut row = GroupRow::new();
        row.push(relative, node.to_vec(), g).expect("valid member");
        row.encode()
    }

    #[test]
    fn in_memory_round_trips_rows() {
        let g = depth(5);
        let db = InMemoryBinaryTrieDB::new_empty();
        let root_group = BitPath::new();
        // Depth 5 starts a group of its own; depth 3 lives in the root
        // group. Both are addressed by their *group root*.
        let second_group = BitPath::from_bits(&[1, 0, 1, 0, 1]);

        assert_eq!(db.get_group(&root_group).unwrap(), None);
        db.put_groups(vec![
            (root_group.clone(), row(&[1, 0, 1], b"root-group", g)),
            (second_group.clone(), row(&[], b"second-group", g)),
        ])
        .unwrap();

        assert_eq!(
            GroupRow::decode(&db.get_group(&root_group).unwrap().unwrap())
                .unwrap()
                .get(&[1, 0, 1]),
            Some(&b"root-group"[..])
        );
        assert_eq!(
            GroupRow::decode(&db.get_group(&second_group).unwrap().unwrap())
                .unwrap()
                .get(&[]),
            Some(&b"second-group"[..])
        );
        // A group nothing was written to has no row at all, which is how
        // a reader tells "no group here" from "a group with one member".
        assert_eq!(
            db.get_group(&BitPath::from_bits(&[1, 1, 1, 1, 1])).unwrap(),
            None
        );

        // Writing the same row key again replaces the whole row, as the
        // single-version storage model requires.
        db.put_groups(vec![(second_group.clone(), row(&[], b"replaced", g))])
            .unwrap();
        assert_eq!(
            GroupRow::decode(&db.get_group(&second_group).unwrap().unwrap())
                .unwrap()
                .get(&[]),
            Some(&b"replaced"[..])
        );

        // An empty value deletes the row: the group lost its last member.
        db.put_groups(vec![(second_group.clone(), Vec::new())])
            .unwrap();
        assert_eq!(db.get_group(&second_group).unwrap(), None);
    }

    #[test]
    fn handles_share_one_row_map() {
        let g = depth(4);
        let db = InMemoryBinaryTrieDB::new_empty();
        let other = InMemoryBinaryTrieDB::new(db.inner());
        db.put_groups(vec![(BitPath::new(), row(&[0], b"x", g))])
            .unwrap();
        assert_eq!(
            other.get_group(&BitPath::new()).unwrap(),
            Some(row(&[0], b"x", g))
        );
    }

    #[test]
    fn row_keys_distinguish_trailing_zero_bits() {
        // The same injectivity `encode_bit_prefix` has, now at row
        // granularity: `[1]` and `[1, 0]` pack to identical bytes, so
        // without the bit count in front one group root would address
        // the other's row. Group depth 1 is the depth at which both are
        // group roots, so it is the depth where the collision is live.
        let g = depth(1);
        let shallow = BitPath::from_bits(&[1]);
        let deeper = BitPath::from_bits(&[1, 0]);
        assert_eq!(group_root(&shallow, g), shallow);
        assert_eq!(group_root(&deeper, g), deeper);
        assert_ne!(shallow.to_db_key(), deeper.to_db_key());

        let db = InMemoryBinaryTrieDB::new_empty();
        db.put_groups(vec![
            (shallow.clone(), row(&[], b"shallow", g)),
            (deeper.clone(), row(&[], b"deeper", g)),
        ])
        .unwrap();
        assert_eq!(
            db.get_group(&shallow).unwrap(),
            Some(row(&[], b"shallow", g))
        );
        assert_eq!(db.get_group(&deeper).unwrap(), Some(row(&[], b"deeper", g)));
    }

    #[test]
    fn the_group_depth_defaults_and_can_be_set() {
        // The default exists so a backend with no datadir to consult
        // needs no wiring; the setter exists so a test can drive the
        // whole trie at every depth rather than only at the default.
        assert_eq!(
            InMemoryBinaryTrieDB::new_empty().group_depth(),
            DEFAULT_GROUP_DEPTH
        );
        for levels in 1..=MAX_GROUP_DEPTH {
            let g = depth(levels);
            assert_eq!(
                InMemoryBinaryTrieDB::new_empty()
                    .at_group_depth(g)
                    .group_depth(),
                g
            );
        }
    }
}
