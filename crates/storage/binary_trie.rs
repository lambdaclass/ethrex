//! [`BinaryTrieDB`] over ethrex's storage backend: the EIP-8297 binary
//! trie's nodes on disk, in the [`BINARY_TRIE_NODES`] column family, and
//! the flat mirror of its leaves in [`BINARY_FLATKEYVALUE`].
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

use crate::api::tables::{BINARY_FLATKEYVALUE, BINARY_TRIE_NODES};
use crate::api::{StorageBackend, StorageLockedView, StorageReadView};
use crate::error::StoreError;
use crate::layering::TrieLayerCache;
use ethrex_binary_trie::BinaryTrieError;
use ethrex_binary_trie::trie::{BinaryTrieDB, BitPath};
use ethrex_common::H256;
use std::sync::{Arc, Mutex};

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
    fn get_group(&self, group_root: &BitPath) -> Result<Option<Vec<u8>>, BinaryTrieError> {
        self.read_view
            .get(BINARY_TRIE_NODES, &group_root.to_db_key())
            .map_err(backend_error)
    }

    /// Writes every row in one transaction, so a commit either lands
    /// whole or not at all.
    ///
    /// An empty value is a tombstone, not a row: it deletes the key
    /// rather than storing zero bytes at it, so a later [`get_group`]
    /// answers `None`. Storing the empty value instead would make the
    /// row read back as a group the trie never wrote, and decoding would
    /// fail.
    ///
    /// [`get_group`]: BinaryTrieDB::get_group
    fn put_groups(&self, rows: Vec<(BitPath, Vec<u8>)>) -> Result<(), BinaryTrieError> {
        let mut tx = self.db.begin_write().map_err(backend_error)?;
        let mut writes = Vec::with_capacity(rows.len());
        for (group_root, encoded) in rows {
            let key = group_root.to_db_key();
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

/// Binary-trie **group row** writes as they are stored and flushed:
/// `BINARY_TRIE_NODES` key/value pairs, with an **empty value meaning "delete
/// this row"** per [`BinaryTrieDB::put_groups`]'s tombstone convention.
///
/// A key is the DB key of a group *root* and a value is a whole `GroupRow`, so
/// one entry here is a group of up to `2^g - 1` nodes rather than one node.
///
/// **Named `Rows`, not `Nodes`, deliberately.** It was `BinaryTrieNodes` while a
/// row held exactly one node, and keeping that name once the container changed
/// would have left `len()` reading as a node count at every call site that took
/// it for one. A `Vec<(Vec<u8>, Vec<u8>)>` carries no evidence either way, so
/// the alias is the only place a reader is told.
pub type BinaryTrieRows = Vec<(Vec<u8>, Vec<u8>)>;

/// Flat-mirror writes as they are staged and flushed: [`BINARY_FLATKEYVALUE`]
/// key/value pairs, with an **empty value meaning "delete this row"** per
/// [`BackendBinaryFlatDB::put_batch`]'s tombstone convention.
///
/// Byte-identical in shape to [`BinaryTrieRows`] and deliberately a distinct
/// alias: the two key spaces overlap exactly (a 34-byte `BitPath` DB key and a
/// 34-byte account-zone tree key are indistinguishable), so the type name is the
/// only place a reader is told which table a `Vec<(Vec<u8>, Vec<u8>)>` is bound
/// for.
pub type BinaryFlatWrites = Vec<(Vec<u8>, Vec<u8>)>;

/// One leaf as an ordered range hands it back: the tree key, 34 or 66 bytes,
/// and its 32-byte value.
pub type BinaryFlatLeaf = (Vec<u8>, [u8; 32]);

/// One row of [`BINARY_FLATKEYVALUE`] as a scan yields it: a leaf, or the read
/// failure that stopped the scan.
pub type BinaryFlatEntry = Result<BinaryFlatLeaf, StoreError>;

/// Length of every leaf value in the binary trie, and therefore of every value
/// in [`BINARY_FLATKEYVALUE`]. Fixed, which is why the flat encoding needs no
/// tag and no length prefix.
pub const BINARY_FLAT_VALUE_LENGTH: usize = 32;

/// The [`BINARY_FLATKEYVALUE`] column family: the binary trie's leaves keyed by
/// their own tree key, one row per live leaf.
///
/// A **mirror**, not the state. The trie is authoritative; every row here is
/// produced by a change the trie was already told about, and the whole table can
/// be dropped and rebuilt from it. What it buys is a leaf read in one lookup
/// instead of a ~256-bit descent, and — the thing the node table cannot do at any
/// price — enumeration **in leaf order**, because a tree key's bytewise order is
/// its bit order and [`BINARY_TRIE_NODES`] is keyed by bit path behind a bit
/// *count*, so it sorts breadth-first instead.
///
/// Deliberately a separate type from [`BackendBinaryTrieDB`] rather than another
/// method on it, and not a [`BinaryTrieDB`] implementation at all: the trait is
/// keyed by [`BitPath`], and a flat row is keyed by a tree key. The two key
/// spaces are also not distinguishable by length — a node at bit-depth 240 has a
/// 34-byte DB key, exactly like an account-zone tree key — so sharing one handle
/// would put two unrelated key spaces one typo apart.
pub struct BackendBinaryFlatDB {
    /// The storage backend, used only for writes.
    db: Arc<dyn StorageBackend>,
    /// Pre-acquired read view, held for this handle's lifetime.
    read_view: Arc<dyn StorageReadView>,
}

impl BackendBinaryFlatDB {
    /// A handle on `db`, acquiring its read view now.
    pub fn new(db: Arc<dyn StorageBackend>) -> Result<Self, StoreError> {
        let read_view = db.begin_read()?;
        Ok(Self::with_view(db, read_view))
    }

    /// A handle on `db` sharing an existing read view, so a flat read and a
    /// node read taken in one query see one consistent snapshot.
    pub fn with_view(db: Arc<dyn StorageBackend>, read_view: Arc<dyn StorageReadView>) -> Self {
        Self { db, read_view }
    }

    /// The leaf value stored under `key`, or `None` if the trie does not hold
    /// that key.
    ///
    /// Absence is absence: a key with no row is a key the trie does not hold,
    /// which is why a caller must know the mirror covers the key before reading
    /// it. That coverage question is not answered here.
    ///
    /// # Errors
    ///
    /// [`StoreError::Custom`] if a row is not exactly
    /// [`BINARY_FLAT_VALUE_LENGTH`] bytes. Nothing should ever write one that
    /// is not, and a short or long row means the table was written by something
    /// that does not share this encoding — worth failing on rather than
    /// padding into a plausible-looking leaf value.
    pub fn get(&self, key: &[u8]) -> Result<Option<[u8; 32]>, StoreError> {
        let Some(value) = self.read_view.get(BINARY_FLATKEYVALUE, key)? else {
            return Ok(None);
        };
        value.as_slice().try_into().map(Some).map_err(|_| {
            StoreError::Custom(format!(
                "binary flat value for key {key:?} is {} bytes, not {BINARY_FLAT_VALUE_LENGTH}",
                value.len()
            ))
        })
    }

    /// Write every entry in one transaction, so a batch lands whole or not at
    /// all.
    ///
    /// **Two kinds of zero, and they mean opposite things.** An *empty* value is
    /// a tombstone: the key left the tree, so the row is deleted and a later
    /// [`get`] answers `None` — the same convention
    /// [`BinaryTrieDB::put_groups`] uses for rows. A value of *32 zero bytes* is
    /// an invariant violation and is refused: the state embedding resolves a
    /// zero-valued leaf to a removal ("zero means absent"), so the trie never
    /// holds one, and storing it here would put a row in the mirror for a key
    /// the trie's root does not commit to. A range served from this table and
    /// proved against that root would then fail on it.
    ///
    /// The check lives at the writer because that is where the invariant is:
    /// a reader finding a zero row can only report that something already went
    /// wrong, and by then the mirror and the trie already disagree.
    ///
    /// # Errors
    ///
    /// [`StoreError::Custom`] if any value is neither empty nor exactly
    /// [`BINARY_FLAT_VALUE_LENGTH`] bytes, or if any value is 32 zero bytes.
    /// Nothing is written when an entry is refused: the check runs over the
    /// whole batch before the transaction is opened.
    ///
    /// [`get`]: BackendBinaryFlatDB::get
    pub fn put_batch(&self, entries: Vec<(Vec<u8>, Vec<u8>)>) -> Result<(), StoreError> {
        for (key, value) in &entries {
            if value.is_empty() {
                continue;
            }
            if value.len() != BINARY_FLAT_VALUE_LENGTH {
                return Err(StoreError::Custom(format!(
                    "binary flat value for key {key:?} is {} bytes, not {BINARY_FLAT_VALUE_LENGTH} \
                     (an empty value is the tombstone; there is no other short form)",
                    value.len()
                )));
            }
            if value.iter().all(|byte| *byte == 0) {
                return Err(StoreError::Custom(format!(
                    "refusing to store a 32-zero-byte binary flat value for key {key:?}: zero \
                     means absent, so the trie removed this leaf and the row must be deleted \
                     with an empty value, not written as zeros"
                )));
            }
        }

        let mut tx = self.db.begin_write()?;
        let mut writes = Vec::with_capacity(entries.len());
        for (key, value) in entries {
            if value.is_empty() {
                tx.delete(BINARY_FLATKEYVALUE, &key)?;
            } else {
                writes.push((key, value));
            }
        }
        tx.put_batch(BINARY_FLATKEYVALUE, writes)?;
        tx.commit()
    }

    /// Pin [`BINARY_FLATKEYVALUE`] for an ordered scan.
    ///
    /// The returned view is a snapshot taken now and held until it is dropped.
    /// Reads go through it, not through this handle's `read_view`, so the whole
    /// scan sees one point in time even though the persist worker is flushing
    /// committed layers into the table concurrently.
    ///
    /// Deliberately a separate object the caller must keep alive rather than a
    /// `range_from` method here. An iterator that opened its own view and
    /// dropped it per call would be pinned only within one `next`, which is
    /// exactly the hazard; making the view a value the caller holds turns
    /// "keep it open for the whole merge" into something the borrow checker
    /// enforces.
    pub fn begin_locked(&self) -> Result<BinaryFlatLockedView, StoreError> {
        Ok(BinaryFlatLockedView {
            view: self.db.begin_locked(BINARY_FLATKEYVALUE)?,
        })
    }
}

/// A pinned read view of [`BINARY_FLATKEYVALUE`], the disk half of an ordered
/// range scan.
///
/// Held open for the length of a scan. See
/// [`BackendBinaryFlatDB::begin_locked`] for why it is a value rather than a
/// method.
pub struct BinaryFlatLockedView {
    view: Box<dyn StorageLockedView>,
}

impl BinaryFlatLockedView {
    /// Every row with a key at or after `origin`, in ascending key order —
    /// which, by the ordering property this table exists for, is leaf order.
    ///
    /// A real seek, off [`StorageLockedView::range_from`]: no `O(n)` walk to
    /// reach the origin, and no `prefix_iterator`, whose two backend
    /// implementations disagree about what a non-empty prefix means and so
    /// cannot be used as a seek at all.
    ///
    /// # Errors
    ///
    /// [`StoreError::Custom`] if a row is not exactly
    /// [`BINARY_FLAT_VALUE_LENGTH`] bytes, reported at the row rather than up
    /// front — the scan is lazy, so a malformed row stops it where it is found.
    pub fn range_from<'a>(
        &'a self,
        origin: &[u8],
    ) -> Result<impl Iterator<Item = BinaryFlatEntry> + 'a, StoreError> {
        Ok(self.view.range_from(origin)?.map(|entry| {
            let (key, value) = entry?;
            let value: [u8; 32] = value.as_ref().try_into().map_err(|_| {
                StoreError::Custom(format!(
                    "binary flat value for key {key:?} is {} bytes, not \
                     {BINARY_FLAT_VALUE_LENGTH}",
                    value.len()
                ))
            })?;
            Ok((key.into_vec(), value))
        }))
    }

    /// The leaf value stored under `key` as of the pinned snapshot.
    ///
    /// Same contract as [`BackendBinaryFlatDB::get`], read through this view so
    /// a point read taken during a scan agrees with the scan.
    pub fn get(&self, key: &[u8]) -> Result<Option<[u8; 32]>, StoreError> {
        let Some(value) = self.view.get(key)? else {
            return Ok(None);
        };
        value.as_slice().try_into().map(Some).map_err(|_| {
            StoreError::Custom(format!(
                "binary flat value for key {key:?} is {} bytes, not {BINARY_FLAT_VALUE_LENGTH}",
                value.len()
            ))
        })
    }
}

/// How much of the keyspace a **reader** may trust [`BINARY_FLATKEYVALUE`]
/// for, parsed from the durable backfill frontier marker.
///
/// **The read-side counterpart of `store::binary_flat_frontier_covers`, and
/// deliberately not the same predicate.** That one asks *who writes this row*
/// and answers "this commit path" when the marker is absent, because a
/// generator that has never run owns nothing. This one asks *may a read trust
/// the mirror here* and answers `false` for the same absent marker, because
/// nothing has populated the mirror yet and Decision 1 forbids a coverage
/// predicate returning `true` for a key whose row might legitimately be
/// missing. Two questions, opposite answers to the same input; folding them
/// into one function would have to pick one and be wrong about the other.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BinaryFlatCoverage {
    /// Marker absent (or empty): no genesis seed and no generator sweep, so
    /// the mirror is a strict subset of the trie and covers nothing.
    Nothing,
    /// The sweep has reached this tree key inclusively; keys at or below it
    /// are covered, keys above it are not.
    UpTo(Vec<u8>),
    /// The `[0xff]` sentinel: the whole keyspace.
    Everything,
}

impl BinaryFlatCoverage {
    /// Read the three-state durable marker.
    ///
    /// The `[0xff]` sentinel is unambiguous because no tree key is one byte
    /// long — they are 34 or 66 — so it can never collide with a real
    /// frontier.
    pub fn from_marker(marker: Option<&[u8]>) -> Self {
        match marker {
            None | Some([]) => Self::Nothing,
            Some(marker) if marker == BINARY_FLAT_FRONTIER_COMPLETE => Self::Everything,
            Some(marker) => Self::UpTo(marker.to_vec()),
        }
    }

    /// Whether a read of `key` may be answered from the mirror.
    ///
    /// The comparison is inclusive and over whole tree keys: this trie has one
    /// keyspace and one linear sweep, so a single `key <= frontier` suffices.
    /// The MPT's second, stricter predicate exists only because its generator
    /// is a nested account/storage sweep, and there is nothing here for it to
    /// mean (plan Decision 2).
    pub fn covers(&self, key: &[u8]) -> bool {
        match self {
            Self::Nothing => false,
            Self::UpTo(frontier) => key <= frontier.as_slice(),
            Self::Everything => true,
        }
    }

    /// Whether the sweep is finished — the gate range serving and deep-reorg
    /// deferral consult, and the analogue of `Store::flatkeyvalue_fully_generated`.
    pub fn is_complete(&self) -> bool {
        matches!(self, Self::Everything)
    }
}

/// The `[0xff]` completion sentinel stored under the mirror's frontier key.
///
/// Defined here, next to the coverage predicate that reads it, and re-exported
/// by `store` alongside the `MISC_VALUES` key it is stored under.
pub const BINARY_FLAT_FRONTIER_COMPLETE: &[u8] = &[0xff];

/// Shared buffer a [`LayeredBinaryTrieDB`] writes into. The trie owns its
/// `Box<dyn BinaryTrieDB>`, so the caller keeps a handle on the buffer to
/// collect the staged writes after committing.
///
/// **Rows, not nodes**, for the reason [`BinaryTrieRows`] gives: what a commit
/// deposits here is one entry per group touched, and a caller reading `len()`
/// as a node count reads it low by roughly the group's occupancy.
pub type StagedBinaryRows = Arc<Mutex<BinaryTrieRows>>;

/// [`BinaryTrieDB`] that reads through the in-memory diff-layer chain before
/// disk, and **stages** its writes into a buffer instead of writing them.
///
/// The binary-trie counterpart of [`TrieWrapper`]: nodes for recently imported
/// blocks live in [`TrieLayerCache`] until the commit gate says a layer is deep
/// enough to be safe, so a reader that went straight to disk would not see the
/// state of the block it is executing on. Reads therefore cascade layer chain
/// -> disk, and writes never reach disk here at all — the layer they are staged
/// into is flushed by `commit_to_disk`, in the same write batch as the same
/// block's MPT nodes.
///
/// Staging rather than writing is not an optimisation. The binary trie is
/// path-keyed and single-version: a block that writes through has no second
/// version to fall back on, so a reorg would strand the abandoned branch's
/// nodes on disk and two blocks at one height would overwrite each other at
/// shared paths.
///
/// [`TrieWrapper`]: crate::layering::TrieWrapper
pub struct LayeredBinaryTrieDB {
    /// Binary-trie root this handle reads at; the entry point for the
    /// layer-chain walk.
    binary_root: H256,
    /// Snapshot of the layer cache, taken once for the whole traversal.
    cache: Arc<TrieLayerCache>,
    /// The on-disk trie, consulted only when the layer chain misses.
    db: BackendBinaryTrieDB,
    /// The on-disk leaf mirror, read only for keys [`Self::coverage`] covers.
    flat_db: BackendBinaryFlatDB,
    /// How much of the keyspace this handle's reads may trust the mirror for.
    ///
    /// A snapshot taken when the handle was built, like the layer cache
    /// snapshot beside it. A frontier that advanced since then makes this
    /// handle under-claim coverage and descend where it could have looked up,
    /// which is the harmless direction; the other direction would read a row
    /// the generator has not written.
    coverage: BinaryFlatCoverage,
    /// Where [`BinaryTrieDB::put_groups`] deposits this block's row writes.
    staged: StagedBinaryRows,
}

impl LayeredBinaryTrieDB {
    /// A handle reading at `binary_root` through `cache`, falling back to `db`
    /// for nodes and `flat_db` for mirrored leaves within `coverage`, and
    /// staging writes into `staged`.
    pub fn new(
        binary_root: H256,
        cache: Arc<TrieLayerCache>,
        db: BackendBinaryTrieDB,
        flat_db: BackendBinaryFlatDB,
        coverage: BinaryFlatCoverage,
        staged: StagedBinaryRows,
    ) -> Self {
        Self {
            binary_root,
            cache,
            db,
            flat_db,
            coverage,
            staged,
        }
    }

    /// A fresh, empty staging buffer.
    pub fn staging_buffer() -> StagedBinaryRows {
        Arc::new(Mutex::new(Vec::new()))
    }
}

impl BinaryTrieDB for LayeredBinaryTrieDB {
    /// Read cascade: layer chain, then the deep-reorg overlay if one is
    /// installed and serves this root, then disk. The binary mirror of
    /// [`TrieWrapper::get`](crate::layering::TrieWrapper), including the
    /// precedence: a layer write supersedes the pivot value the overlay holds,
    /// and an overlay hit supersedes disk, which during a deep reorg still
    /// reflects the chain being abandoned.
    ///
    /// A layer hit is authoritative in both directions. `Some(None)` is a
    /// tombstone — the node left the tree in one of these blocks — and must
    /// answer `None` *without* falling through, because the single-version
    /// on-disk trie still holds the node this block removed. The overlay's
    /// `Some(None)` means the same thing one level down: the node did not exist
    /// at the pivot, so disk must not be consulted for it either.
    fn get_group(&self, group_root: &BitPath) -> Result<Option<Vec<u8>>, BinaryTrieError> {
        let key = group_root.to_db_key();
        if let Some(value) = self.cache.binary_get(self.binary_root, &key) {
            return Ok(value);
        }
        // Gated on the binary root, not the header state root: before activation
        // those differ, and this reader only ever holds the binary one. See
        // `TrieLayerCache::overlay_serves_binary`.
        if self.cache.overlay_serves_binary(self.binary_root)
            && let Some(value) = self.cache.lookup_binary_overlay(&key)
        {
            return Ok(value);
        }
        self.db.get_group(group_root)
    }

    /// Stages every row, writing nothing. Tombstones are staged verbatim as
    /// empty values so the layer represents "this key is deleted" faithfully —
    /// both for a reader walking the chain and for the eventual disk flush,
    /// which turns an empty value back into a `delete`.
    ///
    /// **Reads do not see what this staged.** A handle stages a whole commit
    /// and is then dropped, so nothing reads back through it; a caller that
    /// committed twice through one handle would have the second commit read a
    /// pre-first-commit picture of any row it had to rebuild from the store.
    /// `BinaryTrie::commit` supplies rows from memory instead of re-reading
    /// them, which is what keeps that from mattering.
    fn put_groups(&self, rows: Vec<(BitPath, Vec<u8>)>) -> Result<(), BinaryTrieError> {
        let mut staged = self
            .staged
            .lock()
            .map_err(|_| BinaryTrieError::Backend("binary staging buffer poisoned".to_string()))?;
        staged.reserve(rows.len());
        for (group_root, encoded) in rows {
            staged.push((group_root.to_db_key(), encoded));
        }
        Ok(())
    }

    /// Whether a read at this handle's root may be answered from the mirror.
    ///
    /// Two gates, and the second is the one that is easy to forget.
    ///
    /// **The frontier** is the coverage snapshot taken when this handle was
    /// built: the mirror covers a key only once a genesis seed or a backfill
    /// sweep has written its row.
    ///
    /// **A deep-reorg overlay turns the mirror off entirely**, for the reason
    /// spelled out on the MPT's `TrieWrapper::flatkeyvalue_computed`: journal
    /// entries written while a generator was running are permanently missing
    /// pre-images past the frontier, so an unwind cannot restore those rows,
    /// and disk may still hold the abandoned chain's value for them. Nodes are
    /// always journaled and always unwind, so the descent stays correct where
    /// the mirror does not — which is exactly what returning `false` here buys.
    fn binary_flat_computed(&self, key: &[u8]) -> bool {
        if self.cache.overlay_serves_binary(self.binary_root) {
            return false;
        }
        self.coverage.covers(key)
    }

    /// Read cascade for a mirrored leaf: layer chain, then disk.
    ///
    /// **No overlay step, and that is not an omission.** This method is
    /// reached only when [`binary_flat_computed`] said `true`, and that gate
    /// answers `false` whenever an overlay serves this root — so an overlay
    /// branch here would be code that cannot run, standing in for a safety
    /// property enforced one level up.
    ///
    /// A layer hit is authoritative in both directions, exactly as it is for
    /// nodes: `Some(None)` is a tombstone — the leaf left the tree in one of
    /// these blocks — and must answer `None` without falling through, because
    /// the single-version mirror on disk still holds the row this block
    /// deleted.
    ///
    /// [`binary_flat_computed`]: BinaryTrieDB::binary_flat_computed
    fn binary_flat_get(&self, key: &[u8]) -> Result<Option<[u8; 32]>, BinaryTrieError> {
        if let Some(value) = self.cache.binary_flat_get(self.binary_root, key) {
            return value
                .map(|value| {
                    <[u8; 32]>::try_from(value.as_slice()).map_err(|_| {
                        BinaryTrieError::Backend(format!(
                            "staged binary flat value for key {key:?} is {} bytes, not \
                             {BINARY_FLAT_VALUE_LENGTH}",
                            value.len()
                        ))
                    })
                })
                .transpose();
        }
        self.flat_db.get(key).map_err(backend_error)
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

        assert_eq!(handle(&db).get_group(&root_path).unwrap(), None);

        handle(&db)
            .put_groups(vec![
                (root_path.clone(), vec![0x01, 0x02]),
                (left.clone(), vec![0x03]),
                (deep.clone(), vec![0x04, 0x05, 0x06]),
            ])
            .unwrap();

        let reader = handle(&db);
        assert_eq!(
            reader.get_group(&root_path).unwrap(),
            Some(vec![0x01, 0x02])
        );
        assert_eq!(reader.get_group(&left).unwrap(), Some(vec![0x03]));
        assert_eq!(
            reader.get_group(&deep).unwrap(),
            Some(vec![0x04, 0x05, 0x06])
        );
        assert_eq!(
            reader.get_group(&BitPath::from_bits(&[1, 1, 1])).unwrap(),
            None
        );

        // Single-version storage: writing a path again overwrites it.
        handle(&db)
            .put_groups(vec![(left.clone(), vec![0x07])])
            .unwrap();
        assert_eq!(handle(&db).get_group(&left).unwrap(), Some(vec![0x07]));
    }

    #[test]
    fn an_empty_value_is_a_tombstone() {
        let db = backend();
        let path = BitPath::from_bits(&[1, 0, 1]);

        handle(&db)
            .put_groups(vec![(path.clone(), vec![0xaa, 0xbb])])
            .unwrap();
        assert_eq!(
            handle(&db).get_group(&path).unwrap(),
            Some(vec![0xaa, 0xbb])
        );

        handle(&db)
            .put_groups(vec![(path.clone(), vec![])])
            .unwrap();
        // `None`, not `Some(vec![])`: the group lost its last member, and
        // a zero-byte value would decode as a malformed row.
        assert_eq!(handle(&db).get_group(&path).unwrap(), None);

        // Tombstoning a path that was never written is not an error.
        handle(&db)
            .put_groups(vec![(BitPath::from_bits(&[0]), vec![])])
            .unwrap();
        assert_eq!(
            handle(&db).get_group(&BitPath::from_bits(&[0])).unwrap(),
            None
        );
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
            .put_groups(vec![
                (short.clone(), vec![0x01]),
                (long.clone(), vec![0x02]),
                (longer.clone(), vec![0x03]),
                (root.clone(), vec![0x04]),
                (zero.clone(), vec![0x05]),
            ])
            .unwrap();

        let reader = handle(&db);
        assert_eq!(reader.get_group(&short).unwrap(), Some(vec![0x01]));
        assert_eq!(reader.get_group(&long).unwrap(), Some(vec![0x02]));
        assert_eq!(reader.get_group(&longer).unwrap(), Some(vec![0x03]));
        assert_eq!(reader.get_group(&root).unwrap(), Some(vec![0x04]));
        assert_eq!(reader.get_group(&zero).unwrap(), Some(vec![0x05]));

        // And a tombstone at one of them leaves its neighbours alone.
        handle(&db)
            .put_groups(vec![(long.clone(), vec![])])
            .unwrap();
        let reader = handle(&db);
        assert_eq!(reader.get_group(&long).unwrap(), None);
        assert_eq!(reader.get_group(&short).unwrap(), Some(vec![0x01]));
        assert_eq!(reader.get_group(&longer).unwrap(), Some(vec![0x03]));
    }

    #[test]
    fn a_trie_reopens_over_the_same_database() {
        let db = backend();
        let entries = sample_entries();

        let mut trie = BinaryTrie::new(Box::new(handle(&db)));
        for (key, value) in &entries {
            trie.insert(key.clone(), *value).unwrap();
        }
        let root = trie.commit().unwrap().root;
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
        assert_eq!(fresh.commit().unwrap().root, root);
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
        let full_root = trie.commit().unwrap().root;
        drop(trie);

        let mut trie = BinaryTrie::open(Box::new(handle(&db)), full_root);
        assert!(trie.remove(&removed_key).unwrap().is_some());
        let pruned_root = trie.commit().unwrap().root;
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
        assert_eq!(pruned_root, fresh.commit().unwrap().root);
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
        assert_eq!(reinserted.commit().unwrap().root, full_root);
    }

    // ---- The flat mirror ----------------------------------------------

    mod flat {
        use super::*;
        use crate::api::tables::BINARY_FLATKEYVALUE;
        use ethrex_binary_trie::embedding::{
            address20_to_address32, get_tree_key_for_basic_data, get_tree_key_for_code_chunk,
            get_tree_key_for_storage_slot,
        };
        use ethrex_common::{H160, U256};

        fn flat_handle(db: &Arc<dyn StorageBackend>) -> BackendBinaryFlatDB {
            BackendBinaryFlatDB::new(Arc::clone(db)).expect("read view opens")
        }

        /// One real key from each zone: a 34-byte account header key, a
        /// 34-byte code chunk key, and a 66-byte overflow storage key.
        fn one_key_per_zone() -> Vec<Vec<u8>> {
            let address = address20_to_address32(H160::from_low_u64_be(1));
            vec![
                get_tree_key_for_basic_data(&address),
                get_tree_key_for_code_chunk(&[3u8; 32], 0),
                get_tree_key_for_storage_slot(&address, U256::from(1_000u64)),
            ]
        }

        #[test]
        fn binary_flatkeyvalue_is_a_registered_table() {
            // Unregistered column families are dropped at startup by
            // `drop_obsolete_cfs`. A mirror missing from `TABLES` would be
            // silently wiped on every boot — and because absence in the
            // mirror is read as absence in the trie, a reader below the
            // coverage frontier would then answer `None` for live state.
            assert!(TABLES.contains(&BINARY_FLATKEYVALUE));
        }

        #[test]
        fn round_trips_a_key_from_every_zone() {
            let db = backend();
            let keys = one_key_per_zone();
            assert_eq!(
                keys.iter().map(Vec::len).collect::<Vec<_>>(),
                vec![34, 34, 66],
                "one 66-byte key and two 34-byte ones, coexisting in one table"
            );

            assert_eq!(flat_handle(&db).get(&keys[0]).unwrap(), None);

            flat_handle(&db)
                .put_batch(
                    keys.iter()
                        .enumerate()
                        .map(|(i, key)| (key.clone(), vec![i as u8 + 1; 32]))
                        .collect(),
                )
                .unwrap();

            let reader = flat_handle(&db);
            for (i, key) in keys.iter().enumerate() {
                assert_eq!(reader.get(key).unwrap(), Some([i as u8 + 1; 32]), "{key:?}");
            }
            // A key the mirror does not hold reads as absent, not as zeros.
            assert_eq!(reader.get(&[0x7fu8; 34]).unwrap(), None);

            // Single-version storage: writing a key again overwrites it.
            flat_handle(&db)
                .put_batch(vec![(keys[0].clone(), vec![0xee; 32])])
                .unwrap();
            assert_eq!(flat_handle(&db).get(&keys[0]).unwrap(), Some([0xee; 32]));
        }

        #[test]
        fn an_empty_value_is_a_tombstone() {
            let db = backend();
            let key = one_key_per_zone()[0].clone();

            flat_handle(&db)
                .put_batch(vec![(key.clone(), vec![0xaa; 32])])
                .unwrap();
            assert_eq!(flat_handle(&db).get(&key).unwrap(), Some([0xaa; 32]));

            flat_handle(&db)
                .put_batch(vec![(key.clone(), Vec::new())])
                .unwrap();
            // `None`, and the row is gone rather than holding zero bytes:
            // absence in the mirror is read as absence in the trie.
            assert_eq!(flat_handle(&db).get(&key).unwrap(), None);
            let read_view = db.begin_read().unwrap();
            assert_eq!(read_view.get(BINARY_FLATKEYVALUE, &key).unwrap(), None);

            // Tombstoning a key that was never written is not an error.
            flat_handle(&db)
                .put_batch(vec![(vec![0x11u8; 34], Vec::new())])
                .unwrap();
        }

        #[test]
        fn a_32_zero_byte_value_never_reaches_the_table() {
            // The invariant that keeps the mirror from becoming a superset
            // of the trie: "zero means absent", so a leaf whose value is 32
            // zero bytes was *removed*, and writing it would put a row in
            // the mirror for a key the root does not commit to. A range
            // served from the mirror and proved against that root fails on
            // exactly that row.
            let db = backend();
            let key = one_key_per_zone()[0].clone();

            let refused = flat_handle(&db).put_batch(vec![(key.clone(), vec![0u8; 32])]);
            assert!(refused.is_err(), "a 32-zero-byte value must be refused");
            assert_eq!(flat_handle(&db).get(&key).unwrap(), None);

            // The refusal is per batch, not per entry: nothing lands when
            // one entry is bad, so a caller cannot half-apply a block.
            let other = one_key_per_zone()[1].clone();
            let refused = flat_handle(&db).put_batch(vec![
                (other.clone(), vec![0xcc; 32]),
                (key.clone(), vec![0u8; 32]),
            ]);
            assert!(refused.is_err());
            assert_eq!(flat_handle(&db).get(&other).unwrap(), None);

            // The empty value — the tombstone — is a different thing and is
            // accepted. These two are one typo apart and mean opposite
            // things.
            flat_handle(&db).put_batch(vec![(key, Vec::new())]).unwrap();
        }

        #[test]
        fn a_value_of_the_wrong_length_is_refused() {
            // Neither empty nor 32 bytes: there is no third form, and
            // accepting one would put a row in the table that `get` cannot
            // decode.
            let db = backend();
            let key = one_key_per_zone()[0].clone();
            for value in [vec![0xaau8; 31], vec![0xaau8; 33], vec![0xaau8; 1]] {
                assert!(
                    flat_handle(&db)
                        .put_batch(vec![(key.clone(), value.clone())])
                        .is_err(),
                    "{} bytes must be refused",
                    value.len()
                );
            }
        }

        #[test]
        fn the_table_iterates_in_tree_key_order() {
            // The property the whole mirror rests on, at the column family:
            // its bytewise order is the tree's leaf order, so an ordered
            // scan of the table is an ordered scan of the trie. Pinned in
            // the binary-trie crate as "sorting keys as bytes gives the
            // same sequence as a depth-first walk"; asserted here of the
            // storage backend, which is the half that could regress on its
            // own.
            let db = backend();
            let mut keys: Vec<Vec<u8>> = Vec::new();
            for i in 0..8u64 {
                let address = address20_to_address32(H160::from_low_u64_be(i + 1));
                keys.push(get_tree_key_for_basic_data(&address));
                keys.push(get_tree_key_for_storage_slot(&address, U256::from(500u64)));
                keys.push(get_tree_key_for_code_chunk(&[i as u8; 32], 0));
            }
            keys.sort();
            keys.dedup();

            // Written in an order that is not the key order, so the table's
            // own sort is what produces the sequence read back.
            let mut shuffled: Vec<(Vec<u8>, Vec<u8>)> = keys
                .iter()
                .enumerate()
                .map(|(i, key)| (key.clone(), vec![(i % 251) as u8 + 1; 32]))
                .collect();
            shuffled.reverse();
            flat_handle(&db).put_batch(shuffled).unwrap();

            let reader = flat_handle(&db).begin_locked().unwrap();
            let scanned: Vec<Vec<u8>> = reader
                .range_from(&[])
                .unwrap()
                .map(|entry| entry.unwrap().0)
                .collect();
            assert_eq!(scanned, keys);

            // The three zones come out contiguous and in zone order, which
            // is what lets a range request span them without a seam.
            let zones: Vec<u8> = scanned.iter().map(|key| key[0]).collect();
            let mut sorted_zones = zones.clone();
            sorted_zones.sort_unstable();
            assert_eq!(zones, sorted_zones);
            assert!(zones.contains(&0) && zones.contains(&1) && zones.contains(&255));
        }

        #[test]
        fn a_range_starts_at_the_first_key_at_or_after_the_origin() {
            let db = backend();
            let mut keys: Vec<Vec<u8>> = (0..8u64)
                .map(|i| {
                    get_tree_key_for_basic_data(&address20_to_address32(H160::from_low_u64_be(
                        i + 1,
                    )))
                })
                .collect();
            keys.sort();
            flat_handle(&db)
                .put_batch(
                    keys.iter()
                        .map(|key| (key.clone(), vec![0x01; 32]))
                        .collect(),
                )
                .unwrap();

            let reader = flat_handle(&db).begin_locked().unwrap();
            let from = |origin: &[u8]| -> Vec<Vec<u8>> {
                reader
                    .range_from(origin)
                    .unwrap()
                    .map(|entry| entry.unwrap().0)
                    .collect()
            };

            // An origin equal to a key returns that key.
            assert_eq!(from(&keys[3]), keys[3..].to_vec());
            // An origin one past a key returns the successor: the keys are
            // hash-derived so no two are adjacent, and bumping the last
            // byte lands strictly inside the gap.
            let mut between = keys[3].clone();
            let last = between.len() - 1;
            between[last] = between[last].wrapping_add(1);
            assert_eq!(from(&between), keys[4..].to_vec());
            // An origin above every key returns nothing.
            assert!(from(&[0xffu8; 66]).is_empty());
            // The empty origin returns everything.
            assert_eq!(from(&[]), keys);
        }

        #[test]
        fn a_range_over_an_empty_table_is_empty() {
            let db = backend();
            let reader = flat_handle(&db).begin_locked().unwrap();
            assert_eq!(reader.range_from(&[]).unwrap().count(), 0);
            assert_eq!(reader.range_from(&[0x00u8; 34]).unwrap().count(), 0);
        }
    }

    // ---- Reading through the mirror ------------------------------------

    mod coverage {
        use super::*;
        use crate::api::tables::{BINARY_FLATKEYVALUE, STATE_HISTORY};
        use crate::journal::JournalEntry;
        use crate::layering::{BinaryLayerUpdate, Overlay, TrieLayerCache};
        use ethrex_binary_trie::embedding::{
            address20_to_address32, get_tree_key_for_basic_data, get_tree_key_for_storage_slot,
        };
        use ethrex_common::{H160, U256};

        fn account_key(i: u64) -> Vec<u8> {
            get_tree_key_for_basic_data(&address20_to_address32(H160::from_low_u64_be(i)))
        }

        fn storage_key(i: u64) -> Vec<u8> {
            get_tree_key_for_storage_slot(
                &address20_to_address32(H160::from_low_u64_be(i)),
                U256::from(4_096u64),
            )
        }

        #[test]
        fn an_absent_marker_covers_nothing() {
            // **The seam.** The write-side `binary_flat_frontier_covers` reads
            // an absent marker as `true` — no generator exists, so the commit
            // path owns the whole keyspace. That is a write-ownership answer.
            // The read gate asks a different question and must answer `false`:
            // an absent marker means nothing has seeded the mirror, so it is a
            // strict subset of the trie and a miss would be read as an absence
            // the trie does not agree with.
            for marker in [None, Some([].as_slice())] {
                let coverage = BinaryFlatCoverage::from_marker(marker);
                assert_eq!(coverage, BinaryFlatCoverage::Nothing);
                assert!(!coverage.covers(&account_key(1)));
                assert!(!coverage.covers(&storage_key(1)));
                assert!(!coverage.covers(&[]));
                assert!(!coverage.is_complete());
            }
        }

        #[test]
        fn the_completion_sentinel_covers_every_zone() {
            // Including the overflow-storage zone, whose keys *begin* with
            // `0xff` and are therefore lexicographically greater than the
            // 1-byte `[0xff]` sentinel. Comparing the sentinel as an ordinary
            // frontier would exclude every storage leaf from a mirror that is
            // complete — the same trap the write-side predicate documents.
            let coverage = BinaryFlatCoverage::from_marker(Some(&[0xff]));
            assert_eq!(coverage, BinaryFlatCoverage::Everything);
            assert!(coverage.is_complete());
            assert!(coverage.covers(&account_key(1)));
            assert!(coverage.covers(&storage_key(1)));
            assert!(coverage.covers(&[0xff; 66]));
        }

        #[test]
        fn a_partial_frontier_is_inclusive_and_stops_above_itself() {
            let mut keys = [account_key(1), account_key(2), storage_key(3)];
            keys.sort();
            let coverage = BinaryFlatCoverage::from_marker(Some(&keys[1]));
            assert!(coverage.covers(&keys[0]));
            assert!(coverage.covers(&keys[1]), "the frontier key itself");
            assert!(!coverage.covers(&keys[2]));
            assert!(!coverage.is_complete());
        }

        /// A layered handle over `backend` at `binary_root`, with `cache` and
        /// `coverage`.
        fn layered(
            db: &Arc<dyn StorageBackend>,
            cache: Arc<TrieLayerCache>,
            binary_root: H256,
            coverage: BinaryFlatCoverage,
        ) -> LayeredBinaryTrieDB {
            LayeredBinaryTrieDB::new(
                binary_root,
                cache,
                BackendBinaryTrieDB::new(Arc::clone(db)).unwrap(),
                BackendBinaryFlatDB::new(Arc::clone(db)).unwrap(),
                coverage,
                LayeredBinaryTrieDB::staging_buffer(),
            )
        }

        #[test]
        fn a_mirrored_read_cascades_layers_then_disk() {
            let db = backend();
            let on_disk = account_key(1);
            let staged = account_key(2);
            let overwritten = account_key(3);
            let deleted = account_key(4);
            BackendBinaryFlatDB::new(Arc::clone(&db))
                .unwrap()
                .put_batch(vec![
                    (on_disk.clone(), vec![0x11; 32]),
                    (overwritten.clone(), vec![0x22; 32]),
                    (deleted.clone(), vec![0x33; 32]),
                ])
                .unwrap();

            let root = H256::repeat_byte(0xb1);
            let mut cache = TrieLayerCache::default();
            cache.put_batch_with_binary(
                H256::zero(),
                H256::repeat_byte(0x01),
                1,
                H256::repeat_byte(0x01),
                vec![],
                BinaryLayerUpdate {
                    root,
                    parent_root: H256::zero(),
                    rows: vec![],
                    flat: vec![
                        (staged.clone(), vec![0x44; 32]),
                        (overwritten.clone(), vec![0x55; 32]),
                        // Empty value: the leaf left the tree in this block.
                        (deleted.clone(), vec![]),
                    ],
                },
            );
            let handle = layered(
                &db,
                Arc::new(cache),
                root,
                BinaryFlatCoverage::from_marker(Some(&[0xff])),
            );

            assert_eq!(handle.binary_flat_get(&on_disk).unwrap(), Some([0x11; 32]));
            assert_eq!(handle.binary_flat_get(&staged).unwrap(), Some([0x44; 32]));
            assert_eq!(
                handle.binary_flat_get(&overwritten).unwrap(),
                Some([0x55; 32]),
                "the layer's value supersedes the row on disk"
            );
            assert_eq!(
                handle.binary_flat_get(&deleted).unwrap(),
                None,
                "a layer tombstone must not fall through to the surviving disk row"
            );
            assert_eq!(handle.binary_flat_get(&account_key(9)).unwrap(), None);
        }

        #[test]
        fn an_overlay_turns_the_mirror_off_even_with_a_complete_frontier() {
            // The MPT's `TrieWrapper::flatkeyvalue_computed` does exactly this
            // and says why: journal entries written while a generator was
            // running are permanently missing pre-images past the frontier, so
            // an unwind cannot restore those rows and disk may still hold the
            // abandoned chain's value. Nodes are always journaled, so the
            // descent stays correct where the mirror does not.
            let db = backend();
            let key = account_key(1);
            let pivot = H256::repeat_byte(0xb1);

            let entry = JournalEntry {
                block_hash: H256::repeat_byte(1),
                parent_state_root: H256::repeat_byte(0xf0),
                parent_binary_root: pivot,
                account_trie_diff: vec![],
                storage_trie_diff: vec![],
                account_flat_diff: vec![],
                storage_flat_diff: vec![],
                binary_trie_diff: vec![],
                binary_flat_diff: vec![(key.clone(), Some(vec![0x99; 32]))],
            };
            let mut tx = db.begin_write().unwrap();
            tx.put(STATE_HISTORY, &1u64.to_be_bytes(), &entry.encode())
                .unwrap();
            tx.commit().unwrap();
            let overlay = Overlay::from_journal(db.as_ref(), 1, 1, |_| None).unwrap();

            let mut cache = TrieLayerCache::default();
            cache.set_overlay(Arc::new(overlay));
            let cache = Arc::new(cache);
            let complete = BinaryFlatCoverage::from_marker(Some(&[0xff]));

            assert!(
                !layered(&db, cache.clone(), pivot, complete.clone()).binary_flat_computed(&key),
                "the mirror is off at a root the overlay serves"
            );
            // A root the overlay does *not* serve is unaffected: it reads
            // disk, which still holds that root's state.
            assert!(
                layered(&db, cache, H256::repeat_byte(0xee), complete).binary_flat_computed(&key),
            );
        }

        #[test]
        fn the_frontier_gates_the_read_key_by_key() {
            let db = backend();
            let mut keys = [account_key(1), account_key(2), storage_key(3)];
            keys.sort();
            let cache = Arc::new(TrieLayerCache::default());
            let handle = layered(
                &db,
                cache,
                H256::repeat_byte(0xb1),
                BinaryFlatCoverage::from_marker(Some(&keys[1])),
            );
            assert!(handle.binary_flat_computed(&keys[0]));
            assert!(handle.binary_flat_computed(&keys[1]));
            assert!(!handle.binary_flat_computed(&keys[2]));
        }

        #[test]
        fn a_trie_over_the_layered_handle_agrees_with_the_mirror_on_and_off() {
            // The composition that matters: the same state read twice, once
            // through the descent and once through the mirror, must give the
            // same answers — for present keys, absent keys, and both key
            // lengths.
            let db = backend();
            let entries: Vec<(Vec<u8>, [u8; 32])> = (1..=6u64)
                .flat_map(|i| {
                    [
                        (account_key(i), [i as u8; 32]),
                        (storage_key(i), [i as u8 + 100; 32]),
                    ]
                })
                .collect();

            let mut trie =
                BinaryTrie::new(Box::new(BackendBinaryTrieDB::new(Arc::clone(&db)).unwrap()));
            for (key, value) in &entries {
                trie.insert(key.clone(), *value).unwrap();
            }
            let committed = trie.commit().unwrap();
            drop(trie);

            // The mirror, written from the changelog exactly as the commit
            // path writes it.
            BackendBinaryFlatDB::new(Arc::clone(&db))
                .unwrap()
                .put_batch(
                    committed
                        .leaves
                        .iter()
                        .map(|(key, value)| {
                            (key.clone(), value.map(|v| v.to_vec()).unwrap_or_default())
                        })
                        .collect(),
                )
                .unwrap();

            let absent = vec![account_key(99), storage_key(99)];
            for coverage in [
                BinaryFlatCoverage::Nothing,
                BinaryFlatCoverage::from_marker(Some(&[0xff])),
            ] {
                let mut trie = BinaryTrie::open(
                    Box::new(layered(
                        &db,
                        Arc::new(TrieLayerCache::default()),
                        committed.root,
                        coverage.clone(),
                    )),
                    committed.root,
                );
                for (key, value) in &entries {
                    assert_eq!(
                        trie.get(key).unwrap(),
                        Some(*value),
                        "key {key:?} under {coverage:?}"
                    );
                }
                for key in &absent {
                    assert_eq!(trie.get(key).unwrap(), None, "{key:?} under {coverage:?}");
                }
            }
        }

        #[test]
        fn the_mirror_is_actually_on_the_layered_read_path() {
            // The storage-side counterpart of the crate's node-read counter:
            // wipe the *node* table, leave the mirror, and a covered read
            // still answers. Nothing but the mirror can be serving it.
            let db = backend();
            let key = account_key(1);
            let mut trie =
                BinaryTrie::new(Box::new(BackendBinaryTrieDB::new(Arc::clone(&db)).unwrap()));
            trie.insert(key.clone(), [0x77; 32]).unwrap();
            let committed = trie.commit().unwrap();
            drop(trie);
            BackendBinaryFlatDB::new(Arc::clone(&db))
                .unwrap()
                .put_batch(vec![(key.clone(), vec![0x77; 32])])
                .unwrap();
            db.clear_table(BINARY_TRIE_NODES).unwrap();

            let mut covered = BinaryTrie::open(
                Box::new(layered(
                    &db,
                    Arc::new(TrieLayerCache::default()),
                    committed.root,
                    BinaryFlatCoverage::from_marker(Some(&[0xff])),
                )),
                committed.root,
            );
            assert_eq!(covered.get(&key).unwrap(), Some([0x77; 32]));

            // Without coverage the same read has to descend, and there is
            // nothing left to descend into.
            let mut uncovered = BinaryTrie::open(
                Box::new(layered(
                    &db,
                    Arc::new(TrieLayerCache::default()),
                    committed.root,
                    BinaryFlatCoverage::Nothing,
                )),
                committed.root,
            );
            assert!(
                uncovered.get(&key).is_err(),
                "the node table is gone, so an uncovered read cannot succeed"
            );
            // And the table really is empty, so the assertion above is not
            // passing on a stale read view.
            assert_eq!(
                db.begin_read()
                    .unwrap()
                    .prefix_iterator(BINARY_FLATKEYVALUE, &[])
                    .unwrap()
                    .count(),
                1
            );
        }
    }
}
