//! Node-level views of the trie, for producing and consuming a witness.
//!
//! # The unit is a node encoding, never a stored row
//!
//! `BINARY_TRIE_NODES` (in `crates/storage`) stores a whole subtree band per
//! row, at a group depth that is a property of the *datadir* (see
//! [`BinaryTrieDB::group_depth`]). A
//! witness made of rows would be tied to whatever depth its producer happened
//! to run at, and would address different groups — and so silently miss
//! members — on a node running another. That depth is also still unsettled.
//!
//! Node encodings have no such problem. A node's stored bytes *are* its BLAKE3
//! preimage (see [`hash_stored_node`] and `node.rs`), so they are
//! consensus-visible and identical at every group depth; a row is a storage
//! container around them and is not. Both types here therefore speak node
//! encodings, and both present themselves at **group depth 1**, where a group
//! is exactly one node and the row framing collapses to a wrapper.
//!
//! That is what makes the two halves compose. [`RecordingBinaryTrieDB`] wraps a
//! store's backend at *its* depth and re-presents it one node at a time, so
//! what it records is exactly the nodes a descent touched — not the up to
//! `2^g - 1` neighbours that happened to share their row. Feed those encodings
//! to [`WitnessBinaryTrieDB`] and the trie reads back identically, whatever
//! depth either side runs at.
//!
//! # What the consumer checks
//!
//! [`WitnessBinaryTrieDB::new`] does not trust the list it is given. It walks
//! from `root`, and a node is only ever reached through the commitment that
//! named it — the root, or a parent's child pointer — so every node it installs
//! is one the tree really holds at the path it installs it at. Anything left
//! over is [`WitnessError::ExtraneousNode`]: a witness may stop early (that is
//! its frontier, and a descent that reaches one fails then), but it must be
//! connected downward from the root.
//!
//! [`hash_stored_node`]: super::hash_stored_node

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex};

use ethereum_types::H256;
use thiserror::Error;

use super::db::BinaryTrieDB;
use super::group::{GroupDepth, GroupRow, group_root, relative_bits};
use super::hash_stored_node;
use super::node::{EMPTY_TRIE_ROOT, StoredNode, decode};
use super::path::BitPath;
use crate::error::BinaryTrieError;

/// One node per row: the depth both types here present themselves at.
const ONE_NODE_PER_ROW: GroupDepth = match GroupDepth::new(1) {
    Some(depth) => depth,
    None => panic!("1 is a valid group depth"),
};

/// Node encodings keyed by the commitment they hash to.
///
/// Ordered so a witness serializes deterministically: the same block witnessed
/// twice must produce the same bytes, or nothing downstream can be compared.
pub type WitnessNodes = BTreeMap<H256, Vec<u8>>;

/// Why a set of node encodings is not a witness for a given root.
#[derive(Debug, Error, PartialEq, Eq, Clone)]
pub enum WitnessError {
    /// A node's bytes are not a node.
    #[error("witness contains a malformed node")]
    MalformedNode,
    /// The witness carries a node nothing in it names.
    ///
    /// A witness must be **connected downward from the root**: every node it
    /// carries has to be reachable from `root` through child pointers of nodes
    /// it also carries. Stopping early is fine — that is the frontier, and a
    /// descent that reaches one fails there. Carrying a node hanging off
    /// nothing is not, and it is the shape a forgery, a node from another
    /// block, and a corrupted node (whose hash no longer matches the pointer
    /// that named it) all take.
    ///
    /// Note this is stricter than "belongs to the tree": a node that is
    /// genuinely part of the tree but sits under a branch the witness omitted
    /// is unreachable *here*, and rejected. That is the useful rule — such a
    /// node can never be resolved by a descent anyway, since a descent reaches
    /// nodes top-down and would have failed at the omitted branch first.
    #[error("witness carries a node that hashes to {0:#x}, which nothing in it names")]
    ExtraneousNode(H256),
    /// The same commitment was reached at two different paths.
    ///
    /// Impossible in a well-formed tree — every leaf carries its whole key, so
    /// no two positions hold identical subtrees — and refusing it bounds the
    /// walk.
    #[error("witness reaches the node {0:#x} at more than one path")]
    RepeatedNode(H256),
    /// The empty tree has no nodes, so it admits only the empty witness.
    #[error("the empty root admits only an empty witness")]
    EmptyRootConflict,
}

impl From<WitnessError> for BinaryTrieError {
    fn from(error: WitnessError) -> Self {
        // A witness fault reaches the trie as a backend fault: it is the store
        // behind this trie that could not answer.
        BinaryTrieError::Backend(error.to_string())
    }
}

/// Wrap `encoded` in the one-member row a group depth of 1 expects.
fn single_member_row(encoded: &[u8]) -> Result<Vec<u8>, BinaryTrieError> {
    let mut row = GroupRow::new();
    row.push(&[], encoded.to_vec(), ONE_NODE_PER_ROW)?;
    Ok(row.encode())
}

// ---------------------------------------------------------------------------
// Producing
// ---------------------------------------------------------------------------

/// A read-only view of another backend, one node at a time, recording every
/// node it serves.
///
/// # Why it re-presents the depth rather than forwarding it
///
/// Forwarding `group_depth` would make [`BinaryTrie::resolve`] install a whole
/// row's worth of nodes per read, and this type cannot tell which of them the
/// descent actually wanted — so a recording wrapper that forwarded would
/// capture the producer's group depth in the *contents* of every witness it
/// made. Answering 1 instead costs one inner row read per node and buys a
/// witness that is a function of the block alone.
///
/// # It does not forward the flat mirror either
///
/// [`BinaryTrieDB::binary_flat_computed`] defaults to `false` here, and must.
/// A backend whose flat leaf mirror is authoritative answers reads without
/// descending the tree at all — correct for a node serving queries, and fatal
/// for a witness, which would come back empty for reads that succeeded locally
/// and then fail for everyone else.
///
/// [`BinaryTrie::resolve`]: super::BinaryTrie
pub struct RecordingBinaryTrieDB {
    inner: Box<dyn BinaryTrieDB>,
    recorded: Arc<Mutex<WitnessNodes>>,
}

impl RecordingBinaryTrieDB {
    pub fn new(inner: Box<dyn BinaryTrieDB>) -> Self {
        Self {
            inner,
            recorded: Arc::new(Mutex::new(WitnessNodes::new())),
        }
    }

    /// A handle on the nodes recorded so far, shared with this database rather
    /// than copied — read it after the traversal that fills it.
    pub fn recorded(&self) -> Arc<Mutex<WitnessNodes>> {
        Arc::clone(&self.recorded)
    }
}

impl BinaryTrieDB for RecordingBinaryTrieDB {
    fn group_depth(&self) -> GroupDepth {
        ONE_NODE_PER_ROW
    }

    fn get_group(&self, group_root_path: &BitPath) -> Result<Option<Vec<u8>>, BinaryTrieError> {
        // At depth 1 every path is its own group root, so `group_root_path` is
        // the node's own path and its relative bits are empty.
        let inner_depth = self.inner.group_depth();
        let Some(row) = self
            .inner
            .get_group(&group_root(group_root_path, inner_depth))?
        else {
            return Ok(None);
        };
        let row = GroupRow::decode(&row)?;
        let Some(encoded) = row.get(relative_bits(group_root_path, inner_depth)) else {
            return Ok(None);
        };
        self.recorded
            .lock()
            .map_err(|_| BinaryTrieError::Backend("witness recorder mutex poisoned".into()))?
            .insert(hash_stored_node(encoded), encoded.to_vec());
        Ok(Some(single_member_row(encoded)?))
    }

    /// Refused rather than forwarded or dropped.
    ///
    /// Witness generation reads; a caller that reaches this has asked a
    /// recording view to mutate the store it is reading, and silently
    /// discarding the write would leave it with a trie whose root does not
    /// match anything it can read back.
    fn put_groups(&self, _rows: Vec<(BitPath, Vec<u8>)>) -> Result<(), BinaryTrieError> {
        Err(BinaryTrieError::Backend(
            "a recording trie view is read-only".into(),
        ))
    }
}

// ---------------------------------------------------------------------------
// Consuming
// ---------------------------------------------------------------------------

/// A trie backend over the node encodings of a witness, and nothing else.
///
/// Holds no database and reads nothing. Every node it serves was reached from
/// the root it was built for, through the child pointer that named it, so a
/// [`BinaryTrie`] opened over it reads exactly the state the witness proves and
/// fails at the witness's frontier rather than reading past it.
///
/// [`BinaryTrie`]: super::BinaryTrie
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WitnessBinaryTrieDB {
    /// Node encodings keyed by [`BitPath::to_db_key`] of their own path — the
    /// key a group depth of 1 asks for.
    by_path: HashMap<Vec<u8>, Vec<u8>>,
}

impl WitnessBinaryTrieDB {
    /// Index `nodes` against `root`, rejecting anything the root does not
    /// reach.
    ///
    /// # Errors
    ///
    /// Every [`WitnessError`]; see that type for what each one means. A witness
    /// that merely *stops* somewhere is accepted — the descent that reaches the
    /// hole is what fails, and it fails at the point the missing node was
    /// needed.
    pub fn new(root: H256, nodes: &[Vec<u8>]) -> Result<Self, WitnessError> {
        let mut by_hash: HashMap<H256, &[u8]> = HashMap::with_capacity(nodes.len());
        for encoded in nodes {
            by_hash.insert(hash_stored_node(encoded), encoded.as_slice());
        }

        if root == EMPTY_TRIE_ROOT {
            return if by_hash.is_empty() {
                Ok(Self {
                    by_path: HashMap::new(),
                })
            } else {
                Err(WitnessError::EmptyRootConflict)
            };
        }

        let mut by_path = HashMap::with_capacity(by_hash.len());
        let mut reached: HashSet<H256> = HashSet::with_capacity(by_hash.len());
        // Root first, then whichever child pointers the nodes we accept name.
        let mut frontier = vec![(BitPath::new(), root)];
        while let Some((path, expected)) = frontier.pop() {
            let Some(encoded) = by_hash.get(&expected) else {
                // The witness stops here. Legitimate: it only carries what the
                // block touched.
                continue;
            };
            if !reached.insert(expected) {
                return Err(WitnessError::RepeatedNode(expected));
            }
            let node = decode(encoded).map_err(|_| WitnessError::MalformedNode)?;
            by_path.insert(path.to_db_key(), encoded.to_vec());
            if let StoredNode::Branch {
                prefix,
                left,
                right,
            } = node
            {
                frontier.push((path.child(&prefix, 0), left));
                frontier.push((path.child(&prefix, 1), right));
            }
        }

        if let Some(unreached) = by_hash.keys().find(|hash| !reached.contains(*hash)) {
            return Err(WitnessError::ExtraneousNode(*unreached));
        }

        Ok(Self { by_path })
    }

    /// How many nodes this witness holds.
    pub fn len(&self) -> usize {
        self.by_path.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_path.is_empty()
    }
}

impl BinaryTrieDB for WitnessBinaryTrieDB {
    fn group_depth(&self) -> GroupDepth {
        ONE_NODE_PER_ROW
    }

    fn get_group(&self, group_root_path: &BitPath) -> Result<Option<Vec<u8>>, BinaryTrieError> {
        match self.by_path.get(&group_root_path.to_db_key()) {
            None => Ok(None),
            Some(encoded) => Ok(Some(single_member_row(encoded)?)),
        }
    }

    /// Accepted and discarded, unlike [`RecordingBinaryTrieDB::put_groups`].
    ///
    /// A verifier applies updates to this trie and asks for the resulting root;
    /// `BinaryTrie::root` computes hashes without writing, but a caller that
    /// commits instead must not be told the store is broken — there is simply
    /// nowhere to persist to, and the root it gets is the same either way.
    fn put_groups(&self, _rows: Vec<(BitPath, Vec<u8>)>) -> Result<(), BinaryTrieError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trie::{BinaryTrie, InMemoryBinaryTrieDB, MAX_GROUP_DEPTH};

    fn depth(levels: usize) -> GroupDepth {
        GroupDepth::new(levels).expect("valid group depth")
    }

    /// A trie over `count` keys, stored at `group_depth`.
    fn trie_at(group_depth: GroupDepth, count: u8) -> (InMemoryBinaryTrieDB, H256) {
        let db = InMemoryBinaryTrieDB::new_empty().at_group_depth(group_depth);
        let mut trie = BinaryTrie::new(Box::new(db.clone()));
        for i in 0..count {
            trie.insert(vec![i, i.wrapping_mul(31), 7], [i; 32])
                .unwrap();
        }
        let root = trie.commit().unwrap().root;
        (db, root)
    }

    fn key(i: u8) -> Vec<u8> {
        vec![i, i.wrapping_mul(31), 7]
    }

    /// Read `keys` through a recorder over `db`, returning the nodes touched.
    fn record_reads(db: InMemoryBinaryTrieDB, root: H256, keys: &[u8]) -> Vec<Vec<u8>> {
        let recorder = RecordingBinaryTrieDB::new(Box::new(db));
        let recorded = recorder.recorded();
        let mut trie = BinaryTrie::open(Box::new(recorder), root);
        for i in keys {
            trie.get(&key(*i)).unwrap();
        }
        let nodes = recorded.lock().unwrap();
        nodes.values().cloned().collect()
    }

    #[test]
    fn a_recorded_witness_serves_the_reads_that_produced_it() {
        let (db, root) = trie_at(depth(4), 16);
        let nodes = record_reads(db, root, &[3, 9]);
        assert!(!nodes.is_empty());

        let witness = WitnessBinaryTrieDB::new(root, &nodes).expect("witness indexes");
        let mut trie = BinaryTrie::open(Box::new(witness), root);
        assert_eq!(trie.get(&key(3)).unwrap(), Some([3u8; 32]));
        assert_eq!(trie.get(&key(9)).unwrap(), Some([9u8; 32]));
    }

    /// The independence claim, checked rather than asserted: a witness recorded
    /// against a store at one group depth is byte-identical to one recorded at
    /// any other, and verifies against all of them.
    #[test]
    fn the_witness_is_the_same_at_every_group_depth() {
        let mut baseline: Option<(H256, Vec<Vec<u8>>)> = None;
        for levels in 1..=MAX_GROUP_DEPTH {
            let (db, root) = trie_at(depth(levels), 16);
            let mut nodes = record_reads(db, root, &[3, 9]);
            nodes.sort();
            match &baseline {
                None => baseline = Some((root, nodes)),
                Some((first_root, first_nodes)) => {
                    assert_eq!(
                        root, *first_root,
                        "the root is a property of the state, not of the row layout"
                    );
                    assert_eq!(
                        nodes, *first_nodes,
                        "depth {levels} recorded a different set of node encodings"
                    );
                }
            }
        }
        let (root, nodes) = baseline.expect("at least one depth");
        assert!(
            nodes.len() > 1,
            "the reads must have descended, not sat at the root"
        );
        let witness = WitnessBinaryTrieDB::new(root, &nodes).expect("witness indexes");
        let mut trie = BinaryTrie::open(Box::new(witness), root);
        assert_eq!(trie.get(&key(3)).unwrap(), Some([3u8; 32]));
    }

    #[test]
    fn a_missing_node_fails_where_it_is_needed() {
        let (db, root) = trie_at(depth(4), 16);
        let nodes = record_reads(db, root, &[3]);
        assert!(nodes.len() > 1);

        // Drop each node in turn; the root's absence is caught at the first
        // read, any other at the descent that reaches it. Either way the read
        // must not answer.
        for drop in 0..nodes.len() {
            let mut short = nodes.clone();
            short.remove(drop);
            let Ok(witness) = WitnessBinaryTrieDB::new(root, &short) else {
                continue; // dropping an interior node orphans its subtree
            };
            let mut trie = BinaryTrie::open(Box::new(witness), root);
            assert!(
                trie.get(&key(3)).is_err(),
                "dropping node {drop} still answered the read"
            );
        }
    }

    #[test]
    fn a_corrupted_node_is_rejected_as_extraneous() {
        let (db, root) = trie_at(depth(4), 16);
        let nodes = record_reads(db, root, &[3]);
        for index in 0..nodes.len() {
            for byte in 0..nodes[index].len() {
                let mut tampered = nodes.clone();
                tampered[index][byte] ^= 1;
                // Flipping a byte changes the node's hash, so nothing names it
                // any more — and the node it replaced is now missing.
                match WitnessBinaryTrieDB::new(root, &tampered) {
                    Err(_) => {}
                    Ok(witness) => {
                        let mut trie = BinaryTrie::open(Box::new(witness), root);
                        assert!(
                            trie.get(&key(3)).is_err(),
                            "tampering with byte {byte} of node {index} was accepted"
                        );
                    }
                }
            }
        }
    }

    /// A trie over key material that shares no subtree with [`trie_at`]'s: a
    /// different key length and a different value, so no node of one can be a
    /// node of the other.
    fn foreign_trie() -> (Vec<Vec<u8>>, H256) {
        let db = InMemoryBinaryTrieDB::new_empty();
        let mut trie = BinaryTrie::new(Box::new(db.clone()));
        for i in 0u8..8 {
            trie.insert(vec![0xf0 | i, 0xa5, 0x11, 0x22], [0xee; 32])
                .unwrap();
        }
        let root = trie.commit().unwrap().root;
        let recorder = RecordingBinaryTrieDB::new(Box::new(db));
        let recorded = recorder.recorded();
        let mut reader = BinaryTrie::open(Box::new(recorder), root);
        reader.get(&[0xf3, 0xa5, 0x11, 0x22]).unwrap();
        let nodes = recorded.lock().unwrap().values().cloned().collect();
        (nodes, root)
    }

    #[test]
    fn a_node_the_root_does_not_reach_is_rejected() {
        let (db, root) = trie_at(depth(4), 16);
        let nodes = record_reads(db, root, &[3]);
        let (foreign, foreign_root) = foreign_trie();
        assert_ne!(root, foreign_root);

        let stranger = foreign
            .iter()
            .find(|node| !nodes.contains(node))
            .expect("the two tries share nothing")
            .clone();
        let mut padded = nodes;
        padded.push(stranger.clone());
        assert_eq!(
            WitnessBinaryTrieDB::new(root, &padded),
            Err(WitnessError::ExtraneousNode(hash_stored_node(&stranger)))
        );
    }

    /// The converse, and the reason [`WitnessError::ExtraneousNode`] is a
    /// *reachability* rule and not a minimality one: an untouched node that the
    /// witness's own nodes do name is accepted. It cannot change an answer —
    /// reads follow hash pointers, so a node no descent enters is inert — and
    /// rejecting it would make a witness's validity depend on the producer's
    /// read pattern rather than on the tree's shape.
    #[test]
    fn an_untouched_but_named_node_is_accepted() {
        let (db, root) = trie_at(depth(4), 16);
        let narrow = record_reads(db.clone(), root, &[3]);
        let wide = record_reads(db, root, &[3, 9]);

        // The siblings the narrow walk's own branches point at but did not
        // enter: reachable by construction, and carried by no minimal witness.
        let mut named = HashSet::new();
        for node in &narrow {
            if let Ok(StoredNode::Branch { left, right, .. }) = decode(node) {
                named.insert(left);
                named.insert(right);
            }
        }
        let carried: HashSet<H256> = narrow.iter().map(|n| hash_stored_node(n)).collect();
        let spare = wide
            .iter()
            .find(|node| {
                let hash = hash_stored_node(node);
                named.contains(&hash) && !carried.contains(&hash)
            })
            .expect("the narrow walk names siblings it did not enter")
            .clone();

        let mut padded = narrow;
        padded.push(spare);
        let witness = WitnessBinaryTrieDB::new(root, &padded)
            .expect("a node the witness's own branches name is not extraneous");
        let mut trie = BinaryTrie::open(Box::new(witness), root);
        assert_eq!(trie.get(&key(3)).unwrap(), Some([3u8; 32]));
    }

    #[test]
    fn a_witness_for_another_root_does_not_index() {
        let (db, root) = trie_at(depth(4), 16);
        let nodes = record_reads(db, root, &[3]);
        let (other_db, other_root) = trie_at(depth(4), 8);
        let _ = other_db;
        assert_ne!(root, other_root);

        // Nothing under the other root names these nodes, so every one of them
        // is extraneous.
        assert!(matches!(
            WitnessBinaryTrieDB::new(other_root, &nodes),
            Err(WitnessError::ExtraneousNode(_))
        ));
    }

    #[test]
    fn the_empty_root_admits_only_the_empty_witness() {
        assert!(WitnessBinaryTrieDB::new(EMPTY_TRIE_ROOT, &[]).is_ok());
        assert_eq!(
            WitnessBinaryTrieDB::new(EMPTY_TRIE_ROOT, &[vec![0u8; 4]]),
            Err(WitnessError::EmptyRootConflict)
        );
    }

    #[test]
    fn a_recording_view_refuses_to_write() {
        let (db, _) = trie_at(depth(4), 4);
        let recorder = RecordingBinaryTrieDB::new(Box::new(db));
        assert!(
            recorder
                .put_groups(vec![(BitPath::new(), vec![1])])
                .is_err()
        );
    }
}
