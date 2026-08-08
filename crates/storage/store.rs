#[cfg(feature = "rocksdb")]
use crate::backend::rocksdb::RocksDBBackend;
use crate::{
    STORE_METADATA_FILENAME, STORE_SCHEMA_VERSION,
    api::{
        StorageBackend, StorageReadView, StorageWriteBatch,
        tables::{
            ACCOUNT_CODE_METADATA, ACCOUNT_CODES, ACCOUNT_FLATKEYVALUE, ACCOUNT_TRIE_NODES,
            BAD_BLOCKS, BINARY_FLATKEYVALUE, BINARY_TRIE_NODES, BINARY_TRIE_ROOTS,
            BLOCK_ACCESS_LISTS, BLOCK_NUMBERS, BODIES, CANONICAL_BLOCK_HASHES, CHAIN_DATA,
            EXECUTION_WITNESSES, FULLSYNC_HEADERS, HEADERS, INVALID_CHAINS, MISC_VALUES,
            PENDING_BLOCKS, RECEIPTS_V2, SNAP_STATE, STATE_HISTORY, STORAGE_FLATKEYVALUE,
            STORAGE_TRIE_NODES, TRANSACTION_LOCATIONS,
        },
    },
    apply_prefix,
    backend::in_memory::InMemoryBackend,
    binary_trie::{
        BINARY_FLAT_FRONTIER_COMPLETE, BackendBinaryFlatDB, BackendBinaryTrieDB,
        BinaryFlatCoverage, BinaryFlatWrites, BinaryTrieNodes, LayeredBinaryTrieDB,
        StagedBinaryNodes,
    },
    block_data_buffer::BlockDataBuffer,
    error::StoreError,
    journal::{FlatDiff, JournalEntry},
    layering::{BinaryLayerUpdate, Overlay, TrieLayerCache, TrieWrapper},
    rlp::{BlockBodyRLP, BlockHeaderRLP, BlockRLP},
    trie::{BackendTrieDB, BackendTrieDBLocked, classify_trie_key},
    utils::{ChainDataIndex, SnapStateIndex},
};

use ethrex_binary_trie::trie::{
    BinaryTrie, BinaryTrieDB, BitPath, EMPTY_TRIE_ROOT as BINARY_EMPTY_TRIE_ROOT, LeafChangelog,
    hash_stored_node,
};
use ethrex_common::{
    Address, H256, U256,
    types::{
        AccountInfo, AccountState, AccountUpdate, Block, BlockBody, BlockHash, BlockHeader,
        BlockNumber, ChainConfig, Code, CodeMetadata, ForkId, Genesis, GenesisAccount, Index,
        Receipt, Transaction,
        block_access_list::BlockAccessList,
        block_execution_witness::{ExecutionWitness, RpcExecutionWitness},
        pbt_state::{self, BinaryAccount, apply_account_updates},
    },
    utils::keccak,
};
use ethrex_crypto::{NativeCrypto, keccak::keccak_hash};
use ethrex_rlp::{
    decode::{RLPDecode, decode_bytes},
    encode::RLPEncode,
};
use ethrex_trie::{EMPTY_TRIE_HASH, Nibbles, Trie, TrieLogger, TrieNode, TrieWitness};
use ethrex_trie::{Node, NodeRLP};
use lru::LruCache;
use rayon::prelude::*;
use rustc_hash::FxBuildHasher;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, HashSet, hash_map::Entry},
    fmt::Debug,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex, RwLock,
        atomic::{AtomicUsize, Ordering},
        mpsc::{SyncSender, TryRecvError, sync_channel},
    },
    thread::JoinHandle,
};
use tracing::{debug, error, info, warn};

/// Maximum number of execution witnesses to keep in the database
pub const MAX_WITNESSES: u64 = 128;

// We use one constant for in-memory and another for on-disk backends.
// This is due to tests requiring state older than 128 blocks.
// TODO: unify these
pub const DB_COMMIT_THRESHOLD: usize = 128;
const IN_MEMORY_COMMIT_THRESHOLD: usize = 10000;

/// Depth-only commit threshold for batch execution (full sync / block import). Each batch
/// layer holds ~1024 blocks of trie diffs (~1 GB), so we flush after a few layers to bound
/// memory. The canonical `head - DB_COMMIT_THRESHOLD` safe-commit root never lands on a batch
/// layer boundary, so batch mode commits by depth instead; this is sound because full sync and
/// import only ever extend a single canonical chain (no competing forks to mis-commit).
pub const BATCH_COMMIT_THRESHOLD: usize = 4;

/// Default size in bytes of the RocksDB shared block cache: 12 GiB.
///
/// This cache holds both data blocks AND the index/bloom-filter blocks for every
/// open SST file (because we enable `cache_index_and_filter_blocks`), so its size
/// is the effective upper bound on RocksDB's resident memory footprint. 12 GiB
/// keeps the filter/index working set resident plus hot EVM state; a sweep on a
/// synced mainnet node (32 GiB cap) found 8-16 GiB all keep up with head-following,
/// with larger giving no gain (the OS page cache backstops the uncompressed state
/// CFs) and ~8 GiB the floor where the filter set starts to thrash.
pub const DEFAULT_ROCKSDB_BLOCK_CACHE_SIZE_BYTES: usize = 12 * 1024 * 1024 * 1024;

/// Tunable configuration for [`Store::new_with_config`] and related constructors.
///
/// Use [`StoreConfig::default()`] for production-tuned defaults; callers that
/// don't need to override anything should keep calling [`Store::new`] directly.
#[derive(Debug, Clone, Copy)]
pub struct StoreConfig {
    /// Size in bytes of the RocksDB shared block cache. With
    /// `cache_index_and_filter_blocks` enabled (the ethrex default), this is
    /// the effective ceiling on RocksDB's resident memory. Ignored for
    /// in-memory backends.
    pub rocksdb_block_cache_size: usize,
    /// Bound on the persist worker's channel: number of staged (acked) live
    /// messages whose flush may still be in flight. Once full, the next send
    /// blocks — that is the backpressure that throttles `newPayload`.
    /// Clamped to `max(1)` at construction (0 would make a rendezvous channel).
    pub persist_channel_capacity: usize,
}

impl Default for StoreConfig {
    fn default() -> Self {
        Self {
            rocksdb_block_cache_size: DEFAULT_ROCKSDB_BLOCK_CACHE_SIZE_BYTES,
            persist_channel_capacity: DEFAULT_PERSIST_CHANNEL_CAPACITY,
        }
    }
}

/// Control messages for the FlatKeyValue generator
#[derive(Debug, PartialEq)]
enum FKVGeneratorControlMessage {
    Stop,
    Continue,
}

// 64mb
const CODE_CACHE_MAX_SIZE: u64 = 64 * 1024 * 1024;

/// Key used to persist the `flushed_upto` block number in `MISC_VALUES`.
const FLUSHED_UPTO_KEY: &[u8] = b"bodies_flushed_upto";

/// Single key under which the bounded list of bad blocks is stored in `BAD_BLOCKS`.
const BAD_BLOCKS_KEY: &[u8] = b"bad_blocks";

/// `MISC_VALUES` key holding the binary flat mirror's durable backfill frontier.
///
/// A named constant rather than a repeated literal, which is where the MPT's
/// equivalent went wrong: it spells `"last_written"` inline at eight sites and
/// has no constant, unlike [`FLUSHED_UPTO_KEY`].
///
/// Three durable states, per the plan's Decision 2:
/// - **absent** — no backfill generator has ever run;
/// - **a real tree key** — the generator has swept up to and including it;
/// - **the 1-byte sentinel `[0xff]`** — complete. Unambiguous because no tree
///   key is one byte long; they are 34 or 66.
pub(crate) const BINARY_LAST_WRITTEN_KEY: &[u8] = b"binary_last_written";

/// Whether `commit_to_disk` is the writer responsible for `key` — i.e. whether
/// it should both write the mirror row and journal its pre-image.
///
/// This is the binary counterpart of the MPT's frontier skip
/// (`key > last_written` -> skip both the write and the journal push, because
/// "keys past the frontier aren't written to disk yet, so they must not be
/// journaled either": a `Some(None)` pre-image recorded for a key that was
/// never put would make a rollback delete a row that never existed).
///
/// **This is a *write-ownership* question, not a coverage claim, and the
/// distinction is deliberate.** Task 9 of the plan builds the backfill
/// generator that makes this marker non-absent; until then the marker is always
/// absent, and the three cases resolve as:
///
/// - **absent** — no generator exists, so no generator owns any part of the
///   keyspace, so this write path owns all of it. Skip nothing. That is the
///   honest answer to "who writes this row", and it is *not* the answer to "may
///   a reader trust the mirror here", which is `false`: nothing has seeded the
///   genesis alloc leaves (that is Task 8), so the mirror is a strict subset of
///   the trie until a generator or a genesis seed says otherwise. Task 7's
///   `binary_flat_computed` must answer that second question separately and must
///   **not** reuse this function — Decision 1 is explicit that a coverage
///   predicate must never return `true` for a key whose row might be missing.
/// - **a real tree key** — the live case once Task 9 lands. Keys at or below the
///   frontier are ours; keys above it belong to the generator, which has not
///   written them yet. This arm needs no further change when Task 9 lands.
/// - **`[0xff]`** — complete; the generator owns nothing, we own everything.
///
/// The `[0xff]` case cannot be folded into the ordinary comparison, and the
/// explicit arm is load-bearing rather than tidy: a storage-zone tree key begins
/// with the zone byte `0xff` and continues, so `[0xff, ..] > [0xff]`
/// lexicographically and every overflow-storage leaf would be skipped on a
/// *complete* mirror.
/// Fold a [`BinaryTrie::commit`] changelog into [`BINARY_FLATKEYVALUE`] inside
/// an existing write batch: `Some` writes the 32-byte value, `None` deletes
/// the row.
///
/// Takes the batch rather than opening one so a caller can land the rows in the
/// same transaction as whatever else the commit implies — for genesis, the
/// completion marker.
///
/// # Errors
///
/// [`StoreError::Custom`] on a 32-zero-byte value, the same refusal
/// [`BackendBinaryFlatDB::put_batch`] and `commit_to_disk` make and for the same
/// reason: "zero means absent", so the trie removed that leaf, and a row
/// written for it would put a key in the mirror the trie's root does not commit
/// to. Nothing is written for that entry, and the caller's batch is expected to
/// be dropped rather than committed.
fn stage_binary_flat_leaves(
    tx: &mut Box<dyn crate::api::StorageWriteBatch>,
    leaves: &LeafChangelog,
) -> Result<(), StoreError> {
    for (key, value) in leaves {
        match value {
            Some(value) if value.iter().all(|byte| *byte == 0) => {
                return Err(StoreError::Custom(format!(
                    "refusing to seed a 32-zero-byte binary flat value for key {key:?}: zero \
                     means absent, so the trie holds no such leaf"
                )));
            }
            Some(value) => tx.put(BINARY_FLATKEYVALUE, key, value)?,
            None => tx.delete(BINARY_FLATKEYVALUE, key)?,
        }
    }
    Ok(())
}

fn binary_flat_frontier_covers(frontier: &[u8], key: &[u8]) -> bool {
    if frontier.is_empty() || frontier == BINARY_FLAT_FRONTIER_COMPLETE {
        return true;
    }
    key <= frontier
}

/// Maximum number of bad blocks retained for `debug_getBadBlocks`.
const MAX_BAD_BLOCKS: usize = 16;

#[derive(Debug)]
struct CodeCache {
    inner_cache: LruCache<H256, Code, FxBuildHasher>,
    cache_size: u64,
}

impl Default for CodeCache {
    fn default() -> Self {
        Self {
            inner_cache: LruCache::unbounded_with_hasher(FxBuildHasher),
            cache_size: 0,
        }
    }
}

impl CodeCache {
    fn get(&mut self, code_hash: &H256) -> Result<Option<Code>, StoreError> {
        Ok(self.inner_cache.get(code_hash).cloned())
    }

    fn insert(&mut self, code: &Code) -> Result<(), StoreError> {
        let code_size = code.size();
        let cache_len = self.inner_cache.len() + 1;
        self.cache_size += code_size as u64;
        let current_size = self.cache_size;
        debug!(
            "[ACCOUNT CODE CACHE] cache elements (): {cache_len}, total size: {current_size} bytes"
        );

        while self.cache_size > CODE_CACHE_MAX_SIZE {
            if let Some((_, code)) = self.inner_cache.pop_lru() {
                self.cache_size -= code.size() as u64;
            } else {
                break;
            }
        }

        self.inner_cache.get_or_insert(code.hash, || code.clone());
        Ok(())
    }
}

/// Main storage interface for the ethrex client.
///
/// `Store` is `Clone` and thread-safe; all clones share the same backend and
/// caches via `Arc`. Reads consult an in-memory block-data buffer before disk
/// so not-yet-flushed blocks are always visible.
#[derive(Debug, Clone)]
pub struct Store {
    /// Path to the database directory.
    db_path: PathBuf,
    /// Storage backend (InMemory or RocksDB).
    backend: Arc<dyn StorageBackend>,
    /// Chain configuration (fork schedule, chain ID, etc.).
    chain_config: ChainConfig,
    /// Cache for trie nodes from recent blocks.
    trie_cache: Arc<RwLock<Arc<TrieLayerCache>>>,
    /// Channel for controlling the FlatKeyValue generator background task.
    flatkeyvalue_control_tx: std::sync::mpsc::SyncSender<FKVGeneratorControlMessage>,
    /// In-memory overlay of block data not yet flushed to disk.
    block_data_buffer: Arc<RwLock<Arc<BlockDataBuffer>>>,
    /// Channel to the single persist worker (`apply_updates` → `PersistMessage::Block`,
    /// `wait_for_persistence_idle` → `PersistMessage::Ping`). The worker is the
    /// sole mutator of `block_data_buffer` in production.
    persist_tx: std::sync::mpsc::SyncSender<PersistMessage>,
    /// Roots whose trie diff-layer is being built but not yet installed in
    /// `trie_cache`. Trie opens block on these so a just-added block's state is
    /// never read as stale before its layer lands.
    pending_trie_roots: Arc<PendingTrieRoots>,
    /// Cached latest canonical block header. May be slightly stale, which is
    /// acceptable for RPC "latest" queries and sync operations.
    latest_block_header: LatestBlockHeaderCache,
    /// Last computed FlatKeyValue for incremental updates.
    last_computed_flatkeyvalue: Arc<RwLock<Vec<u8>>>,

    /// In-memory copy of the binary flat mirror's backfill frontier, in the
    /// **durable** three-state shape (absent / a tree key / `[0xff]`) rather
    /// than the MPT's padded in-memory form — the marker's states are already
    /// distinguishable here, and padding a 34- or 66-byte keyspace to one
    /// length would make an all-zero pad compare as *covering* every account
    /// key, which is the wrong direction to be wrong in.
    ///
    /// Kept beside `last_computed_flatkeyvalue` and read the same way: the
    /// generator advances it at batch commit, so it lags the durable marker
    /// and readers under-claim coverage.
    binary_last_computed: Arc<RwLock<Vec<u8>>>,

    /// Cache for account bytecodes, keyed by the bytecode hash.
    /// Note that we don't remove entries on account code changes, since
    /// those changes already affect the code hash stored in the account, and only
    /// may result in this cache having useless data.
    account_code_cache: Arc<Mutex<CodeCache>>,

    /// Cache for code metadata (code length), keyed by the bytecode hash.
    /// Uses FxHashMap for efficient lookups, much smaller than code cache.
    code_metadata_cache: Arc<Mutex<rustc_hash::FxHashMap<H256, CodeMetadata>>>,

    /// Serializes concurrent `forkchoice_update` callers so that the cache
    /// update and the DB write transaction remain mutually ordered.
    fcu_lock: Arc<tokio::sync::Mutex<()>>,

    /// Canonical safe-commit state root, computed after each forkchoice update.
    ///
    /// Shared with the [`TrieLayerCache`] so that the Store can update the cell without
    /// replacing the cache Arc. `H256::zero()` means "no safe commit point yet".
    /// Cloning `Store` shares this cell across all clones, which is required and correct.
    safe_commit_root: Arc<RwLock<H256>>,

    /// While set, `forkchoice_update_inner` skips STATE_HISTORY pruning (the
    /// finalized-number update still lands). Set by `Blockchain::enter_reorg` for
    /// the duration of a deep-reorg apply pass: `Overlay::from_journal` reads
    /// journal entries one by one with no snapshot isolation, and syncer-driven
    /// forkchoice updates are not gated by the reorg mutex, so a concurrent
    /// finality advance could otherwise prune entries out from under overlay
    /// construction (spurious `MissingEntry`) or between a case-1 attempt and its
    /// retry. Pruning catches up on the first finality advance after the pass
    /// ends (`delete_range` is cumulative).
    journal_pruning_paused: Arc<std::sync::atomic::AtomicBool>,

    background_threads: Arc<ThreadList>,
}

#[derive(Debug, Default)]
struct ThreadList {
    list: Vec<JoinHandle<()>>,
}

impl Drop for ThreadList {
    fn drop(&mut self) {
        for handle in self.list.drain(..) {
            let _ = handle.join();
        }
    }
}

/// Storage trie nodes grouped by account address hash.
///
/// Each entry contains the hashed account address and the trie nodes
/// for that account's storage trie.
pub type StorageTrieNodes = Vec<(H256, Vec<(Nibbles, Vec<u8>)>)>;
type StorageTries = HashMap<Address, (TrieWitness, Trie)>;

/// Storage backend type selection.
///
/// Used when creating a new [`Store`] to specify which backend to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineType {
    /// In-memory storage, non-persistent. Suitable for testing.
    InMemory,
    /// RocksDB storage, persistent. Suitable for production.
    #[cfg(feature = "rocksdb")]
    RocksDB,
}

/// Whether a root-addressed read must first prove that this node actually holds
/// the state behind that root.
///
/// Trie nodes are keyed by path, so a read at a root the store no longer holds
/// silently falls through to whatever version the on-disk trie currently is.
/// See [`Store::ensure_trie_holds_state_root`].
#[derive(Clone, Copy)]
enum RootCheck {
    /// Verify the root, naming the given block in the error when it is known.
    Verify(Option<BlockNumber>),
    /// Skip the check: the caller has already verified the root, or deliberately
    /// addresses state by root with no block behind it.
    Skip,
}

/// Batch of updates to apply to the store atomically.
///
/// Used during block execution to collect all state changes before
/// committing them to the database in a single transaction.
pub struct UpdateBatch {
    /// New nodes to add to the state trie.
    pub account_updates: Vec<TrieNode>,
    /// Storage trie updates per account (keyed by hashed address).
    pub storage_updates: Vec<(H256, Vec<TrieNode>)>,
    /// Blocks to store.
    pub blocks: Vec<Block>,
    /// Receipts to store, grouped by block hash.
    pub receipts: Vec<(H256, Vec<Receipt>)>,
    /// Contract code updates (code hash -> bytecode).
    pub code_updates: Vec<(H256, Code)>,
    /// This block's EIP-8297 binary-trie advance, staged rather than written.
    ///
    /// `None` on every chain that does not schedule `binaryTreeTime`, and on
    /// the re-store paths that do not re-advance the trie (witness
    /// regeneration), where the nodes are already staged or on disk.
    ///
    /// Carried in the *same* batch as the MPT's nodes so both land in one
    /// diff layer and one disk write. See
    /// [`Store::advance_binary_trie_for_block`].
    pub binary_update: Option<BinaryTrieAdvance>,
    /// Commit gate for this batch's trie layers (independent of `wait_for_flush`).
    ///
    /// - `None`: live path (`newPayload`). The persist worker commits by the canonical
    ///   `head - DB_COMMIT_THRESHOLD` safe-commit root.
    /// - `Some(depth)`: single-canonical-chain execution (batch import, full sync, startup
    ///   state regeneration). The persist worker commits every layer deeper than `depth`
    ///   (see [`Trie::get_commitable_by_depth`]), which bounds resident trie layers to ~`depth`.
    pub commit_depth: Option<usize>,
    /// When the persist worker acks (independent of `commit_depth`).
    ///
    /// - `false`: ack after staging, so the caller's next-block execution overlaps this
    ///   block's disk flush. Used by the live path and the per-block re-execution paths
    ///   (regen / full-sync fallback / import tail) — their memory is already bounded by
    ///   `commit_depth`'s depth gate and the persist channel capacity, so waiting per block
    ///   would only serialize CPU and I/O for no benefit.
    /// - `true`: ack after flush, bounding in-flight work to ~1 message. Used by the bespoke
    ///   batch path, where a single message carries ~1024 blocks (~1 GB of trie diff) and two
    ///   in flight would be a real memory cost.
    pub wait_for_flush: bool,
}

/// One block's EIP-8297 binary-trie advance: the root it reached and the node
/// writes that got it there, **staged** rather than written.
///
/// Produced by [`Store::advance_binary_trie_for_block`] and carried through
/// [`UpdateBatch`] into the block's diff layer. Nothing here has touched disk;
/// dropping it (a rejected block, an abandoned branch) discards the block's
/// binary state completely, which is the property the whole staging exercise
/// exists for.
#[derive(Debug, Clone)]
pub struct BinaryTrieAdvance {
    /// Binary-trie root after this block's updates. Recorded in
    /// `BINARY_TRIE_ROOTS` by the producer, and the key the layer's secondary
    /// index uses.
    pub root: H256,
    /// The parent block's binary-trie root, i.e. the root these updates were
    /// applied on top of.
    ///
    /// Carried alongside the new root because the reverse-diff journal has to
    /// record it: unwinding this block's nodes returns the trie to *this* root,
    /// and nothing else in the layer knows what it is (before activation the
    /// parent's header carries an MPT root, and the parent layer is usually
    /// already on disk).
    pub parent_root: H256,
    /// `(BINARY_TRIE_NODES key, encoded node)` pairs; an empty value is a
    /// tombstone, per `BinaryTrieDB`'s convention.
    pub nodes: BinaryTrieNodes,
    /// The same block's writes to the flat leaf mirror, derived from the
    /// [`LeafChangelog`] the trie's own commit emitted: one
    /// `BINARY_FLATKEYVALUE` row per leaf the block created, changed or removed.
    ///
    /// A removal (`None` in the changelog) becomes an **empty value**, which is
    /// the tombstone convention every layer downstream already speaks. It must
    /// not become 32 zero bytes: "zero means absent" is what put the removal in
    /// the changelog in the first place, and a zero row would be a mirror entry
    /// for a key the trie's root does not commit to.
    ///
    /// Taken from `commit` rather than reconstructed at the call sites, so
    /// `remove_prefix` — whose retired leaves only the trie knows — is covered
    /// by construction.
    pub flat: BinaryFlatWrites,
}

impl From<BinaryTrieAdvance> for BinaryLayerUpdate {
    fn from(advance: BinaryTrieAdvance) -> Self {
        BinaryLayerUpdate {
            root: advance.root,
            parent_root: advance.parent_root,
            nodes: advance.nodes,
            flat: advance.flat,
        }
    }
}

/// Turn the trie's [`LeafChangelog`] into `BINARY_FLATKEYVALUE` writes.
///
/// `Some(value)` is the 32 raw bytes, no tag and no length prefix. `None` is a
/// removal and becomes an empty value — the tombstone every consumer of these
/// pairs already understands, and deliberately *not* 32 zero bytes.
fn flat_writes_from_changelog(leaves: LeafChangelog) -> BinaryFlatWrites {
    leaves
        .into_iter()
        .map(|(key, value)| match value {
            Some(value) => (key, value.to_vec()),
            None => (key, Vec::new()),
        })
        .collect()
}

/// Storage trie updates grouped by account address hash.
pub type StorageUpdates = Vec<(H256, Vec<(Nibbles, Vec<u8>)>)>;

/// Collection of account state changes from block execution.
///
/// Contains all the data needed to update the state trie after
/// executing a block: account updates, storage updates, and code deployments.
pub struct AccountUpdatesList {
    /// Root hash of the state trie after applying these updates.
    pub state_trie_hash: H256,
    /// State trie node updates (path -> RLP-encoded node).
    pub state_updates: Vec<(Nibbles, Vec<u8>)>,
    /// Storage trie updates per account.
    pub storage_updates: StorageUpdates,
    /// New contract bytecode deployments.
    pub code_updates: Vec<(H256, Code)>,
}

/// Encodes a tx-location entry as the operand passed to `merge_cf`.
///
/// The operand uses the **same encoding as the stored value** — a
/// `Vec<(BlockNumber, BlockHash, Index)>` with a single element. This is
/// required for an *associative* merge operator: RocksDB folds operands
/// together with PartialMerge (during compaction, without a base value), and
/// the result becomes an operand for a later merge. If the operand format
/// differed from the merge output (e.g. operand = bare tuple, output = Vec),
/// the re-fed result would fail to decode and entries would be silently
/// dropped. Keeping both as `Vec` makes the merge truly associative.
pub(crate) fn encode_tx_location_operand(
    block_number: BlockNumber,
    block_hash: BlockHash,
    index: Index,
) -> Vec<u8> {
    vec![(block_number, block_hash, index)].encode_to_vec()
}

/// Merge function for the `TRANSACTION_LOCATIONS` column family.
///
/// The CF stores `Vec<(BlockNumber, BlockHash, Index)>` keyed by tx hash.
/// Both stored values and operands use this same `Vec` encoding — this
/// associativity requirement is mandatory: RocksDB folds operands together
/// during compaction without a base value (PartialMerge), then feeds that
/// result back into a later merge. A differing format would silently drop
/// entries. See `encode_tx_location_operand`.
///
/// Within the fold, a later entry with the same `block_hash` replaces an
/// earlier one (reorg dedupe). On decode failure the merge returns `None`
/// so RocksDB surfaces a corruption error rather than silently dropping
/// locations.
///
/// Merge instead of read-modify-write avoids the ~5–20 ms/block per-tx point
/// lookup on the write path; consolidation is deferred to compaction or the
/// next read.
pub fn tx_locations_merge(
    existing: Option<&[u8]>,
    operands: impl IntoIterator<Item = impl AsRef<[u8]>>,
) -> Option<Vec<u8>> {
    // Fold one RLP-encoded `Vec` chunk into `list`, deduping by block_hash
    // (later entry wins). Returns false on decode failure so the caller can
    // abort the whole merge.
    fn fold_chunk(
        list: &mut Vec<(BlockNumber, BlockHash, Index)>,
        bytes: &[u8],
        what: &str,
    ) -> bool {
        match <Vec<(BlockNumber, BlockHash, Index)>>::decode(bytes) {
            Ok(entries) => {
                for (bn, bh, idx) in entries {
                    list.retain(|(_, existing_bh, _)| *existing_bh != bh);
                    list.push((bn, bh, idx));
                }
                true
            }
            Err(e) => {
                error!(
                    "tx_locations_merge: failed to decode {what} ({} bytes): {e}; \
                     aborting merge to avoid silent data loss",
                    bytes.len()
                );
                false
            }
        }
    }

    let mut list: Vec<(BlockNumber, BlockHash, Index)> = Vec::new();

    // Order matters: RocksDB delivers operands oldest-first.
    if let Some(bytes) = existing
        && !fold_chunk(&mut list, bytes, "existing value")
    {
        return None;
    }
    for op in operands {
        if !fold_chunk(&mut list, op.as_ref(), "operand") {
            return None;
        }
    }
    Some(list.encode_to_vec())
}

impl Store {
    /// Block until the persist worker has fully processed all previously-sent
    /// `Block` messages (staged, trie-layer built, flushed, evicted).
    ///
    /// Uses an ack-based `Ping` rather than a bare send because the channel is
    /// buffered — a bare send proves nothing about prior message completion. The
    /// worker is FIFO, so it handles the `Ping` only after every earlier `Block`
    /// is done.
    ///
    /// Concurrent-producer caveat: if another thread sends a `Block` after the
    /// `Ping` is enqueued, that block may not be flushed by the time this returns.
    pub async fn wait_for_persistence_idle(&self) -> Result<(), StoreError> {
        let tx = self.persist_tx.clone();
        tokio::task::spawn_blocking(move || {
            let (ack_tx, ack_rx) = sync_channel::<Result<(), StoreError>>(1);
            tx.send(PersistMessage::Ping(ack_tx))
                .map_err(|e| StoreError::Custom(format!("wait_for_persistence_idle send: {e}")))?;
            ack_rx
                .recv()
                .map_err(|e| StoreError::Custom(format!("wait_for_persistence_idle ack: {e}")))?
        })
        .await
        .map_err(|e| StoreError::Custom(format!("wait_for_persistence_idle join: {e}")))?
    }

    /// Flushes all in-memory state to disk for a clean shutdown.
    ///
    /// Sends a `Shutdown` handshake to the persist worker, which (being FIFO)
    /// first drains every queued `Block`, then force-flushes the block-data
    /// buffer to disk. Once the worker acks, this syncs the backend (memtables +
    /// WAL) so the next process start needs no WAL recovery.
    ///
    /// The in-memory trie diff-layers are intentionally *not* force-committed.
    /// The on-disk trie is a single-version, path-based store, so folding the
    /// non-finalized tail into it would leave a post-restart reorg unable to
    /// reconstruct the overwritten ancestor state (the node would wedge). The
    /// recent (< `DB_COMMIT_THRESHOLD`) layers are dropped and re-executed on the
    /// next start from the deep, reorg-safe on-disk base — exactly as after any
    /// restart today.
    ///
    /// After this returns the persist worker has exited; the store must not be
    /// used for further writes. Idempotent only in the sense that a second call
    /// errors on the closed channel — call it exactly once, on shutdown.
    pub async fn shutdown(&self) -> Result<(), StoreError> {
        let tx = self.persist_tx.clone();
        let backend = self.backend.clone();
        tokio::task::spawn_blocking(move || {
            let (ack_tx, ack_rx) = sync_channel::<Result<(), StoreError>>(1);
            tx.send(PersistMessage::Shutdown { ack: ack_tx })
                .map_err(|e| StoreError::Custom(format!("shutdown send: {e}")))?;
            ack_rx
                .recv()
                .map_err(|e| StoreError::Custom(format!("shutdown ack: {e}")))??;
            // Worker has flushed block data to the WAL/memtables; make it durable
            // and recovery-free.
            backend.flush()
        })
        .await
        .map_err(|e| StoreError::Custom(format!("shutdown join: {e}")))?
    }

    /// Add a block in a single transaction.
    /// This will store -> BlockHeader, BlockBody, BlockTransactions, BlockNumber.
    pub async fn add_block(&self, block: Block) -> Result<(), StoreError> {
        self.add_blocks(vec![block]).await
    }

    /// Add a batch of blocks in a single transaction.
    /// This will store -> BlockHeader, BlockBody, BlockTransactions, BlockNumber.
    pub async fn add_blocks(&self, blocks: Vec<Block>) -> Result<(), StoreError> {
        let db = self.backend.clone();
        tokio::task::spawn_blocking(move || {
            let mut tx = db.begin_write()?;

            for block in blocks {
                write_block_data(
                    tx.as_mut(),
                    block.header.number,
                    block.hash(),
                    &block.header,
                    &block.body,
                )?;
            }

            tx.commit()
        })
        .await
        .map_err(|e| StoreError::Custom(format!("Task panicked: {}", e)))?
    }

    /// Add block header
    pub async fn add_block_header(
        &self,
        block_hash: BlockHash,
        block_header: BlockHeader,
    ) -> Result<(), StoreError> {
        let hash_key = block_hash.encode_to_vec();
        let header_value = BlockHeaderRLP::from(block_header).into_vec();
        self.write_async(HEADERS, hash_key, header_value).await
    }

    /// Add a batch of block headers
    pub async fn add_block_headers(
        &self,
        block_headers: Vec<BlockHeader>,
    ) -> Result<(), StoreError> {
        let mut txn = self.backend.begin_write()?;

        for header in block_headers {
            let block_hash = header.hash();
            let block_number = header.number;
            let hash_key = block_hash.encode_to_vec();
            let header_value = BlockHeaderRLP::from(header).into_vec();

            txn.put(HEADERS, &hash_key, &header_value)?;

            let number_key = block_number.to_le_bytes().to_vec();
            txn.put(BLOCK_NUMBERS, &hash_key, &number_key)?;
        }
        txn.commit()?;
        Ok(())
    }

    /// Obtain canonical block header
    pub fn get_block_header(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<BlockHeader>, StoreError> {
        let latest = self.latest_block_header.get();
        if block_number == latest.number {
            return Ok(Some((*latest).clone()));
        }
        // Resolve the canonical hash, then read through the buffer-aware by-hash
        // path so a canonical-but-still-buffered block is visible (mirrors
        // `get_block_body`). `load_block_header` is disk-only and would return
        // `None` for a block whose header has not been flushed yet.
        let Some(block_hash) = self.get_canonical_block_hash_sync(block_number)? else {
            return Ok(None);
        };
        self.get_block_header_by_hash(block_hash)
    }

    /// Add block body
    pub async fn add_block_body(
        &self,
        block_hash: BlockHash,
        block_body: BlockBody,
    ) -> Result<(), StoreError> {
        let hash_key = block_hash.encode_to_vec();
        let body_value = BlockBodyRLP::from(block_body).into_vec();
        self.write_async(BODIES, hash_key, body_value).await
    }

    /// Obtain canonical block body
    pub async fn get_block_body(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<BlockBody>, StoreError> {
        let Some(block_hash) = self.get_canonical_block_hash_sync(block_number)? else {
            return Ok(None);
        };

        self.get_block_body_by_hash(block_hash).await
    }

    /// Remove canonical block
    pub async fn remove_block(&self, block_number: BlockNumber) -> Result<(), StoreError> {
        let Some(hash) = self.get_canonical_block_hash_sync(block_number)? else {
            return Ok(());
        };

        let backend = self.backend.clone();
        tokio::task::spawn_blocking(move || {
            let hash_key = hash.encode_to_vec();

            let mut txn = backend.begin_write()?;
            txn.delete(
                CANONICAL_BLOCK_HASHES,
                block_number.to_le_bytes().as_slice(),
            )?;
            txn.delete(BODIES, &hash_key)?;
            txn.delete(HEADERS, &hash_key)?;
            txn.delete(BLOCK_NUMBERS, &hash_key)?;
            txn.commit()
        })
        .await
        .map_err(|e| StoreError::Custom(format!("Task panicked: {}", e)))?
    }

    /// Obtain canonical block bodies in from..=to
    pub async fn get_block_bodies(
        &self,
        from: BlockNumber,
        to: BlockNumber,
    ) -> Result<Vec<Option<BlockBody>>, StoreError> {
        // TODO: Implement read bulk
        let buffer = self.buffer()?;
        let backend = self.backend.clone();
        tokio::task::spawn_blocking(move || {
            let numbers: Vec<BlockNumber> = (from..=to).collect();
            let mut block_bodies = Vec::new();

            let txn = backend.begin_read()?;
            for number in numbers {
                let Some(hash) = txn
                    .get(CANONICAL_BLOCK_HASHES, number.to_le_bytes().as_slice())?
                    .map(|bytes| H256::decode(bytes.as_slice()))
                    .transpose()?
                else {
                    block_bodies.push(None);
                    continue;
                };
                // Consult the in-memory buffer first so a not-yet-flushed body
                // is not reported as missing (mirrors get_block_bodies_by_hash).
                if let Some(body) = buffer.get_body(&hash) {
                    block_bodies.push(Some(body));
                    continue;
                }
                let hash_key = hash.encode_to_vec();
                let block_body_opt = txn
                    .get(BODIES, &hash_key)?
                    .map(|bytes| BlockBodyRLP::from_bytes(bytes).to())
                    .transpose()
                    .map_err(StoreError::from)?;

                block_bodies.push(block_body_opt);
            }

            Ok(block_bodies)
        })
        .await
        .map_err(|e| StoreError::Custom(format!("Task panicked: {}", e)))?
    }

    /// Obtain block bodies from a list of hashes
    pub async fn get_block_bodies_by_hash(
        &self,
        hashes: Vec<BlockHash>,
    ) -> Result<Vec<BlockBody>, StoreError> {
        let buffer = self.buffer()?;
        let backend = self.backend.clone();
        // TODO: Implement read bulk
        tokio::task::spawn_blocking(move || {
            let txn = backend.begin_read()?;
            let mut block_bodies = Vec::new();
            for hash in hashes {
                // Consult the in-memory buffer first, like the single-hash reader,
                // so a not-yet-flushed body is not reported as missing.
                if let Some(body) = buffer.get_body(&hash) {
                    block_bodies.push(body);
                    continue;
                }
                let hash_key = hash.encode_to_vec();

                let Some(block_body) = txn
                    .get(BODIES, &hash_key)?
                    .map(|bytes| BlockBodyRLP::from_bytes(bytes).to())
                    .transpose()
                    .map_err(StoreError::from)?
                else {
                    return Err(StoreError::Custom(format!(
                        "Block body not found for hash: {hash}"
                    )));
                };
                block_bodies.push(block_body);
            }
            Ok(block_bodies)
        })
        .await
        .map_err(|e| StoreError::Custom(format!("Task panicked: {}", e)))?
    }

    /// Obtain any block body using the hash
    pub async fn get_block_body_by_hash(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockBody>, StoreError> {
        if let Some(b) = self.buffer()?.get_body(&block_hash) {
            return Ok(Some(b));
        }
        self.read_async(BODIES, block_hash.encode_to_vec())
            .await?
            .map(|bytes| BlockBodyRLP::from_bytes(bytes).to())
            .transpose()
            .map_err(StoreError::from)
    }

    pub fn get_block_header_by_hash(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockHeader>, StoreError> {
        let latest = self.latest_block_header.get();
        if block_hash == latest.hash() {
            return Ok(Some((*latest).clone()));
        }
        if let Some(h) = self.buffer()?.get_header(&block_hash) {
            return Ok(Some(h));
        }
        self.load_block_header_by_hash(block_hash)
    }

    pub fn add_pending_block(&self, block: Block) -> Result<(), StoreError> {
        let block_hash = block.hash();
        let block_value = BlockRLP::from(block).into_vec();
        self.write(PENDING_BLOCKS, block_hash.as_bytes().to_vec(), block_value)
    }

    pub async fn get_pending_block(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<Block>, StoreError> {
        self.read_async(PENDING_BLOCKS, block_hash.as_bytes().to_vec())
            .await?
            .map(|bytes| BlockRLP::from_bytes(bytes).to())
            .transpose()
            .map_err(StoreError::from)
    }

    /// Add block number for a given hash
    pub async fn add_block_number(
        &self,
        block_hash: BlockHash,
        block_number: BlockNumber,
    ) -> Result<(), StoreError> {
        let number_value = block_number.to_le_bytes().to_vec();
        self.write_async(BLOCK_NUMBERS, block_hash.encode_to_vec(), number_value)
            .await
    }

    /// Obtain block number for a given hash
    pub async fn get_block_number(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockNumber>, StoreError> {
        if let Some(n) = self.buffer()?.get_number(&block_hash) {
            return Ok(Some(n));
        }
        self.read_async(BLOCK_NUMBERS, block_hash.encode_to_vec())
            .await?
            .map(|bytes| -> Result<BlockNumber, StoreError> {
                let array: [u8; 8] = bytes
                    .try_into()
                    .map_err(|_| StoreError::Custom("Invalid BlockNumber bytes".to_string()))?;
                Ok(BlockNumber::from_le_bytes(array))
            })
            .transpose()
    }

    /// Store transaction location (block number and index of the transaction within the block)
    pub async fn add_transaction_location(
        &self,
        transaction_hash: H256,
        block_number: BlockNumber,
        block_hash: BlockHash,
        index: Index,
    ) -> Result<(), StoreError> {
        self.add_transaction_locations(vec![(transaction_hash, block_number, block_hash, index)])
            .await
    }

    /// Store transaction locations in batch (one db transaction for all)
    pub async fn add_transaction_locations(
        &self,
        locations: Vec<(H256, BlockNumber, BlockHash, Index)>,
    ) -> Result<(), StoreError> {
        let db = self.backend.clone();
        tokio::task::spawn_blocking(move || {
            let mut tx = db.begin_write()?;
            for (tx_hash, block_number, block_hash, index) in locations {
                tx.merge(
                    TRANSACTION_LOCATIONS,
                    tx_hash.as_bytes(),
                    &encode_tx_location_operand(block_number, block_hash, index),
                )?;
            }
            tx.commit()
        })
        .await
        .map_err(|e| StoreError::Custom(format!("Task panicked: {}", e)))?
    }

    /// Obtain transaction location (block hash and index)
    pub async fn get_transaction_location(
        &self,
        transaction_hash: H256,
    ) -> Result<Option<(BlockNumber, BlockHash, Index)>, StoreError> {
        let buffered = self.buffer()?.get_tx_locations(&transaction_hash);
        let db = self.backend.clone();
        tokio::task::spawn_blocking(move || {
            let tx = db.begin_read()?;
            let mut locations = buffered;
            if let Some(bytes) = tx.get(TRANSACTION_LOCATIONS, transaction_hash.as_bytes())? {
                locations.extend(<Vec<(BlockNumber, BlockHash, Index)>>::decode(&bytes)?);
            }
            for (block_number, block_hash, index) in locations {
                let canonical_hash = tx
                    .get(
                        CANONICAL_BLOCK_HASHES,
                        block_number.to_le_bytes().as_slice(),
                    )?
                    .map(|bytes| H256::decode(bytes.as_slice()))
                    .transpose()?;
                if canonical_hash == Some(block_hash) {
                    return Ok(Some((block_number, block_hash, index)));
                }
            }
            Ok(None)
        })
        .await
        .map_err(|e| StoreError::Custom(format!("Task panicked: {}", e)))?
    }

    /// Add receipt
    pub async fn add_receipt(
        &self,
        block_hash: BlockHash,
        index: Index,
        receipt: Receipt,
    ) -> Result<(), StoreError> {
        let key = receipt_key(&block_hash, index);
        // Storage codec (NOT wire/consensus): preserves frame-receipt
        // `succeeded` + aggregated logs; identical to encode_to_vec for
        // non-frame receipts.
        let value = receipt.encode_storage();
        self.write_async(RECEIPTS_V2, key, value).await
    }

    /// Add receipts
    pub async fn add_receipts(
        &self,
        block_hash: BlockHash,
        receipts: Vec<Receipt>,
    ) -> Result<(), StoreError> {
        let batch_items: Vec<_> = receipts
            .into_iter()
            .enumerate()
            .map(|(index, receipt)| {
                let key = receipt_key(&block_hash, index as u64);
                let value = receipt.encode_storage();
                (key, value)
            })
            .collect();
        self.write_batch_async(RECEIPTS_V2, batch_items).await
    }

    /// Obtain receipt for a canonical block represented by the block number.
    pub async fn get_receipt(
        &self,
        block_number: BlockNumber,
        index: Index,
    ) -> Result<Option<Receipt>, StoreError> {
        // FIXME (#4353)
        let Some(block_hash) = self.get_canonical_block_hash(block_number).await? else {
            return Ok(None);
        };
        self.get_receipt_by_block_hash(block_hash, index).await
    }

    /// Obtain receipt by block hash and index
    async fn get_receipt_by_block_hash(
        &self,
        block_hash: BlockHash,
        index: Index,
    ) -> Result<Option<Receipt>, StoreError> {
        if let Some(r) = self.buffer()?.get_receipt(&block_hash, index) {
            return Ok(Some(r));
        }
        let key = receipt_key(&block_hash, index);
        self.read_async(RECEIPTS_V2, key)
            .await?
            .map(|bytes| Receipt::decode_storage(bytes.as_slice()))
            .transpose()
            .map_err(StoreError::from)
    }

    /// Get account code by its hash.
    ///
    /// Checks the in-memory block-data buffer first, then the LRU cache
    /// (`account_code_cache`), and finally the database.  Code that has been
    /// inserted via `engine_newPayload` but not yet flushed to disk is therefore
    /// visible to callers without an explicit flush.
    pub fn get_account_code(&self, code_hash: H256) -> Result<Option<Code>, StoreError> {
        if let Some(code) = self.buffer()?.get_code(&code_hash) {
            return Ok(Some(code));
        }
        // check cache first
        if let Some(code) = self
            .account_code_cache
            .lock()
            .map_err(|_| StoreError::LockError)?
            .get(&code_hash)?
        {
            return Ok(Some(code));
        }

        let Some(bytes) = self
            .backend
            .begin_read()?
            .get(ACCOUNT_CODES, code_hash.as_bytes())?
        else {
            return Ok(None);
        };
        let (bytecode_slice, targets) = decode_bytes(&bytes)?;
        let code = Code::from_parts_unchecked(
            code_hash,
            bytecode_slice,
            <Vec<u32>>::decode(targets)?.into(),
        );

        // insert into cache and evict if needed
        self.account_code_cache
            .lock()
            .map_err(|_| StoreError::LockError)?
            .insert(&code)?;

        Ok(Some(code))
    }

    /// Check if account code exists by its hash, without constructing the full `Code` struct.
    /// More efficient than `get_account_code` for existence checks since it skips
    /// RLP decoding and `Code` struct construction (no `jump_targets` deserialization).
    /// Note: The underlying `get()` still reads the value from RocksDB (including blob files).
    pub fn code_exists(&self, code_hash: H256) -> Result<bool, StoreError> {
        // Code introduced by a not-yet-flushed block lives only in the buffer; check
        // it first so a contract created in the current block is visible (matches
        // get_account_code / get_code_metadata).
        if self.buffer()?.get_code(&code_hash).is_some() {
            return Ok(true);
        }
        // Check cache first
        if self
            .account_code_cache
            .lock()
            .map_err(|_| StoreError::LockError)?
            .get(&code_hash)?
            .is_some()
        {
            return Ok(true);
        }
        // Check DB without reading the full value
        Ok(self
            .backend
            .begin_read()?
            .get(ACCOUNT_CODES, code_hash.as_bytes())?
            .is_some())
    }

    /// Get code metadata (length) by its hash.
    ///
    /// Checks cache first, falls back to database. If metadata is missing,
    /// falls back to loading full code and extracts length (auto-migration).
    pub fn get_code_metadata(&self, code_hash: H256) -> Result<Option<CodeMetadata>, StoreError> {
        use ethrex_common::constants::EMPTY_KECCAK_HASH;

        // Empty code special case
        if code_hash == *EMPTY_KECCAK_HASH {
            return Ok(Some(CodeMetadata { length: 0 }));
        }

        // Check cache first
        if let Some(metadata) = self
            .code_metadata_cache
            .lock()
            .map_err(|_| StoreError::LockError)?
            .get(&code_hash)
            .copied()
        {
            return Ok(Some(metadata));
        }

        // Try reading from metadata table
        let metadata = if let Some(bytes) = self
            .backend
            .begin_read()?
            .get(ACCOUNT_CODE_METADATA, code_hash.as_bytes())?
        {
            let length =
                u64::from_be_bytes(bytes.try_into().map_err(|_| {
                    StoreError::Custom("Invalid metadata length encoding".to_string())
                })?);
            CodeMetadata { length }
        } else {
            // Fallback: load full code and extract length (auto-migration)
            let Some(code) = self.get_account_code(code_hash)? else {
                return Ok(None);
            };
            let metadata = CodeMetadata {
                length: code.len() as u64,
            };

            // Write metadata for future use (async, fire and forget)
            let metadata_buf = metadata.length.to_be_bytes().to_vec();
            let hash_key = code_hash.0.to_vec();
            let backend = self.backend.clone();
            tokio::task::spawn(async move {
                if let Err(e) = async {
                    let mut tx = backend.begin_write()?;
                    tx.put(ACCOUNT_CODE_METADATA, &hash_key, &metadata_buf)?;
                    tx.commit()
                }
                .await
                {
                    tracing::warn!("Failed to write code metadata during auto-migration: {}", e);
                }
            });

            metadata
        };

        // Update cache
        self.code_metadata_cache
            .lock()
            .map_err(|_| StoreError::LockError)?
            .insert(code_hash, metadata);

        Ok(Some(metadata))
    }

    /// Add account code
    pub async fn add_account_code(&self, code: Code) -> Result<(), StoreError> {
        let hash_key = code.hash.0.to_vec();
        let buf = encode_code(&code);
        let metadata_buf = (code.len() as u64).to_be_bytes();

        // Write both code and metadata atomically
        let backend = self.backend.clone();
        tokio::task::spawn_blocking(move || {
            let mut tx = backend.begin_write()?;
            tx.put(ACCOUNT_CODES, &hash_key, &buf)?;
            tx.put(ACCOUNT_CODE_METADATA, &hash_key, &metadata_buf)?;
            tx.commit()
        })
        .await
        .map_err(|e| StoreError::Custom(format!("Task panicked: {}", e)))?
    }

    /// Clears all checkpoint data created during the last snap sync
    pub async fn clear_snap_state(&self) -> Result<(), StoreError> {
        let db = self.backend.clone();
        tokio::task::spawn_blocking(move || db.clear_table(SNAP_STATE))
            .await
            .map_err(|e| StoreError::Custom(format!("Task panicked: {}", e)))?
    }

    pub async fn get_transaction_by_hash(
        &self,
        transaction_hash: H256,
    ) -> Result<Option<Transaction>, StoreError> {
        let (_block_number, block_hash, index) =
            match self.get_transaction_location(transaction_hash).await? {
                Some(location) => location,
                None => return Ok(None),
            };
        self.get_transaction_by_location(block_hash, index).await
    }

    pub async fn get_transaction_by_location(
        &self,
        block_hash: H256,
        index: u64,
    ) -> Result<Option<Transaction>, StoreError> {
        let block_body = match self.get_block_body_by_hash(block_hash).await? {
            Some(body) => body,
            None => return Ok(None),
        };
        let index: usize = index.try_into()?;
        Ok(block_body.transactions.get(index).cloned())
    }

    pub async fn get_block_by_hash(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<Block>, StoreError> {
        let header = match self.get_block_header_by_hash(block_hash)? {
            Some(header) => header,
            None => return Ok(None),
        };
        let body = match self.get_block_body_by_hash(block_hash).await? {
            Some(body) => body,
            None => return Ok(None),
        };
        Ok(Some(Block::new(header, body)))
    }

    pub async fn get_block_by_number(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<Block>, StoreError> {
        let Some(block_hash) = self.get_canonical_block_hash(block_number).await? else {
            return Ok(None);
        };
        self.get_block_by_hash(block_hash).await
    }

    // Get the canonical block hash for a given block number.
    pub async fn get_canonical_block_hash(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<BlockHash>, StoreError> {
        let last = self.latest_block_header.get();
        if last.number == block_number {
            return Ok(Some(last.hash()));
        }
        let backend = self.backend.clone();
        tokio::task::spawn_blocking(move || {
            backend
                .begin_read()?
                .get(
                    CANONICAL_BLOCK_HASHES,
                    block_number.to_le_bytes().as_slice(),
                )?
                .map(|bytes| H256::decode(bytes.as_slice()))
                .transpose()
                .map_err(StoreError::from)
        })
        .await
        .map_err(|e| StoreError::Custom(format!("Task panicked: {}", e)))?
    }

    /// Stores the chain configuration values, should only be called once after reading the genesis file
    /// Ignores previously stored values if present
    pub async fn set_chain_config(&mut self, chain_config: &ChainConfig) -> Result<(), StoreError> {
        self.chain_config = *chain_config;
        let key = chain_data_key(ChainDataIndex::ChainConfig);
        let value = serde_json::to_string(chain_config)
            .map_err(|_| StoreError::Custom("Failed to serialize chain config".to_string()))?
            .into_bytes();
        self.write_async(CHAIN_DATA, key, value).await
    }

    /// Update earliest block number
    pub async fn update_earliest_block_number(
        &self,
        block_number: BlockNumber,
    ) -> Result<(), StoreError> {
        let key = chain_data_key(ChainDataIndex::EarliestBlockNumber);
        let value = block_number.to_le_bytes().to_vec();
        self.write_async(CHAIN_DATA, key, value).await
    }

    /// Obtain earliest block number
    pub async fn get_earliest_block_number(&self) -> Result<BlockNumber, StoreError> {
        let key = chain_data_key(ChainDataIndex::EarliestBlockNumber);
        self.read_async(CHAIN_DATA, key)
            .await?
            .map(|bytes| -> Result<BlockNumber, StoreError> {
                let array: [u8; 8] = bytes
                    .try_into()
                    .map_err(|_| StoreError::Custom("Invalid BlockNumber bytes".to_string()))?;
                Ok(BlockNumber::from_le_bytes(array))
            })
            .ok_or(StoreError::MissingEarliestBlockNumber)?
    }

    /// Obtain finalized block number
    pub async fn get_finalized_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        let key = chain_data_key(ChainDataIndex::FinalizedBlockNumber);
        self.read_async(CHAIN_DATA, key)
            .await?
            .map(|bytes| -> Result<BlockNumber, StoreError> {
                let array: [u8; 8] = bytes
                    .try_into()
                    .map_err(|_| StoreError::Custom("Invalid BlockNumber bytes".to_string()))?;
                Ok(BlockNumber::from_le_bytes(array))
            })
            .transpose()
    }

    /// Obtain safe block number
    pub async fn get_safe_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        let key = chain_data_key(ChainDataIndex::SafeBlockNumber);
        self.read_async(CHAIN_DATA, key)
            .await?
            .map(|bytes| -> Result<BlockNumber, StoreError> {
                let array: [u8; 8] = bytes
                    .try_into()
                    .map_err(|_| StoreError::Custom("Invalid BlockNumber bytes".to_string()))?;
                Ok(BlockNumber::from_le_bytes(array))
            })
            .transpose()
    }

    /// Obtain latest block number
    pub async fn get_latest_block_number(&self) -> Result<BlockNumber, StoreError> {
        Ok(self.latest_block_header.get().number)
    }

    /// Update pending block number
    pub async fn update_pending_block_number(
        &self,
        block_number: BlockNumber,
    ) -> Result<(), StoreError> {
        let key = chain_data_key(ChainDataIndex::PendingBlockNumber);
        let value = block_number.to_le_bytes().to_vec();
        self.write_async(CHAIN_DATA, key, value).await
    }

    /// Obtain pending block number
    pub async fn get_pending_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        let key = chain_data_key(ChainDataIndex::PendingBlockNumber);
        self.read_async(CHAIN_DATA, key)
            .await?
            .map(|bytes| -> Result<BlockNumber, StoreError> {
                let array: [u8; 8] = bytes
                    .try_into()
                    .map_err(|_| StoreError::Custom("Invalid BlockNumber bytes".to_string()))?;
                Ok(BlockNumber::from_le_bytes(array))
            })
            .transpose()
    }

    /// DB mutation step of `forkchoice_update`.
    ///
    /// Callers MUST hold `fcu_lock` (only `forkchoice_update` should invoke this).
    /// The read of `LatestBlockNumber` below happens outside the write
    /// transaction and would be a TOCTOU window without that serialization.
    /// Applies a fork-choice update atomically: writes canonical hashes, latest/safe/
    /// finalized numbers, and prunes the state-history journal up to the new finalized.
    ///
    /// **Concurrency:** This function must be called with exclusive access. In
    /// production it is gated by `fcu_lock` from [`forkchoice_update`]. The
    /// pre-finalized read used to decide journal pruning happens outside the write
    /// transaction; without serialization, two concurrent callers could race on
    /// `prev_finalized` and double-prune. Tests that call this directly must not
    /// run concurrently with another FCU.
    async fn forkchoice_update_inner(
        &self,
        new_canonical_blocks: Vec<(BlockNumber, BlockHash)>,
        head_number: BlockNumber,
        head_hash: BlockHash,
        safe: Option<BlockNumber>,
        finalized: Option<BlockNumber>,
    ) -> Result<(), StoreError> {
        let latest = self.load_latest_block_number().await?.unwrap_or(0);
        let db = self.backend.clone();
        let journal_pruning_paused = self.journal_pruning_paused.clone();
        tokio::task::spawn_blocking(move || {
            let mut txn = db.begin_write()?;

            for (block_number, block_hash) in new_canonical_blocks {
                let head_key = block_number.to_le_bytes();
                let head_value = block_hash.encode_to_vec();
                txn.put(CANONICAL_BLOCK_HASHES, &head_key, &head_value)?;
            }

            // Delete canonical entries above the new head by enumerating each key.
            // `delete_range` is not safe here: keys are `u64::to_le_bytes()`, and
            // RocksDB's lexicographic comparator does not match LE numeric order
            // (e.g. block 256 = [0x00, 0x01, ..] sorts before block 11 = [0x0B, ..]),
            // so a range-delete would silently miss blocks whose LE first byte is
            // smaller than `head+1`'s first byte.
            for number in (head_number + 1)..=(latest) {
                txn.delete(CANONICAL_BLOCK_HASHES, number.to_le_bytes().as_slice())?;
            }

            // Make head canonical
            let head_key = head_number.to_le_bytes();
            let head_value = head_hash.encode_to_vec();
            txn.put(CANONICAL_BLOCK_HASHES, &head_key, &head_value)?;

            // Update chain data
            let latest_key = chain_data_key(ChainDataIndex::LatestBlockNumber);
            txn.put(CHAIN_DATA, &latest_key, &head_number.to_le_bytes())?;

            if let Some(safe) = safe {
                let safe_key = chain_data_key(ChainDataIndex::SafeBlockNumber);
                txn.put(CHAIN_DATA, &safe_key, &safe.to_le_bytes())?;
            }

            if let Some(finalized) = finalized {
                let finalized_key = chain_data_key(ChainDataIndex::FinalizedBlockNumber);

                // Read the previous finalized number from the same backend before we
                // overwrite it. The journal can only be pruned when finality actually
                // advances; a no-op or backwards FCU must not touch STATE_HISTORY.
                // Pre-merge or fresh chains have no entry; treat as 0.
                //
                // A length mismatch is treated as a hard error rather than a silent
                // fallback to 0: if a future schema change stores this field with a
                // different width, the silent default would make `finalized > 0` true
                // for every FCU and prune the entire journal. Bail out instead.
                let prev_finalized = match db.begin_read()?.get(CHAIN_DATA, &finalized_key)? {
                    Some(bytes) => {
                        let arr: [u8; 8] = bytes.as_slice().try_into().map_err(|_| {
                            StoreError::Custom(format!(
                                "FinalizedBlockNumber has unexpected length {} (want 8)",
                                bytes.len()
                            ))
                        })?;
                        BlockNumber::from_le_bytes(arr)
                    }
                    None => 0,
                };

                txn.put(CHAIN_DATA, &finalized_key, &finalized.to_le_bytes())?;

                // Prune every STATE_HISTORY entry at or below the new finalized number
                // in the same atomic txn. `delete_range` is half-open `[start, end)`,
                // so `end = finalized + 1`. STATE_HISTORY uses big-endian keys, so
                // lexicographic byte order matches numeric order.
                //
                // Skipped while a deep-reorg apply pass is in flight
                // (`journal_pruning_paused`): `Overlay::from_journal` reads entries
                // with no snapshot isolation, so pruning mid-construction fails it
                // with a spurious `MissingEntry`. The finalized-number update above
                // still lands; pruning catches up on the next advance after the
                // pass ends because `delete_range` is cumulative from zero.
                if finalized > prev_finalized
                    && !journal_pruning_paused.load(std::sync::atomic::Ordering::Acquire)
                {
                    let start = 0u64.to_be_bytes();
                    let end = finalized.saturating_add(1).to_be_bytes();
                    txn.delete_range(STATE_HISTORY, &start, &end)?;
                }
            }

            txn.commit()
        })
        .await
        .map_err(|e| StoreError::Custom(format!("Task panicked: {}", e)))?
    }

    pub async fn get_receipts_for_block(
        &self,
        block_hash: &BlockHash,
    ) -> Result<Vec<Receipt>, StoreError> {
        self.get_receipts_for_block_from_index(block_hash, 0, None)
            .await
    }

    /// Retrieves receipts for a block starting from the given index,
    /// optionally limited to `max_count` receipts.
    ///
    /// Uses cursor-based prefix iteration over the 32-byte block hash prefix
    /// for efficient batch retrieval. Used by:
    /// - eth/70 partial receipt requests (EIP-7975) via p2p
    /// - `eth_getTransactionReceipt` RPC with a count limit to avoid
    ///   fetching the entire block's receipts
    pub async fn get_receipts_for_block_from_index(
        &self,
        block_hash: &BlockHash,
        start_index: u64,
        max_count: Option<usize>,
    ) -> Result<Vec<Receipt>, StoreError> {
        if let Some(all) = self.buffer()?.get_receipts(block_hash) {
            let start = start_index as usize;
            let slice = all.into_iter().skip(start);
            return Ok(match max_count {
                Some(max) => slice.take(max).collect(),
                None => slice.collect(),
            });
        }
        let backend = self.backend.clone();
        let block_hash = *block_hash;

        tokio::task::spawn_blocking(move || {
            let txn = backend.begin_read()?;
            let prefix = block_hash.as_bytes().to_vec();
            // Seek directly to block_hash || start_index to avoid O(start_index) scan.
            // Keys are big-endian u64, so lexicographic order matches numeric order.
            let mut seek_key = prefix.clone();
            seek_key.extend_from_slice(&start_index.to_be_bytes());
            let iter = txn.prefix_iterator(RECEIPTS_V2, &seek_key)?;
            let mut receipts = Vec::new();
            for result in iter {
                let (k, v) = result?;
                if !k.starts_with(&prefix) {
                    break;
                }
                if k.len() != 40 {
                    continue;
                }
                receipts.push(Receipt::decode_storage(v.as_ref())?);
                if let Some(max) = max_count
                    && receipts.len() >= max
                {
                    break;
                }
            }
            Ok(receipts)
        })
        .await
        .map_err(|e| StoreError::Custom(format!("Task panicked: {e}")))?
    }

    // Snap State methods

    /// Sets the hash of the last header downloaded during a snap sync
    pub async fn set_header_download_checkpoint(
        &self,
        block_hash: BlockHash,
    ) -> Result<(), StoreError> {
        let key = snap_state_key(SnapStateIndex::HeaderDownloadCheckpoint);
        let value = block_hash.encode_to_vec();
        self.write_async(SNAP_STATE, key, value).await
    }

    /// Gets the hash of the last header downloaded during a snap sync
    pub async fn get_header_download_checkpoint(&self) -> Result<Option<BlockHash>, StoreError> {
        let key = snap_state_key(SnapStateIndex::HeaderDownloadCheckpoint);
        self.backend
            .begin_read()?
            .get(SNAP_STATE, &key)?
            .map(|bytes| H256::decode(bytes.as_slice()))
            .transpose()
            .map_err(StoreError::from)
    }

    /// The `forkchoice_update` and `new_payload` methods require the `latest_valid_hash`
    /// when processing an invalid payload. To provide this, we must track invalid chains.
    ///
    /// We only store the last known valid head upon encountering a bad block,
    /// rather than tracking every subsequent invalid block.
    pub async fn set_latest_valid_ancestor(
        &self,
        bad_block: BlockHash,
        latest_valid: BlockHash,
    ) -> Result<(), StoreError> {
        let value = latest_valid.encode_to_vec();
        self.write_async(INVALID_CHAINS, bad_block.as_bytes().to_vec(), value)
            .await
    }

    /// Returns the latest valid ancestor hash for a given invalid block hash.
    /// Used to provide `latest_valid_hash` in the Engine API when processing invalid payloads.
    pub async fn get_latest_valid_ancestor(
        &self,
        block: BlockHash,
    ) -> Result<Option<BlockHash>, StoreError> {
        self.read_async(INVALID_CHAINS, block.as_bytes().to_vec())
            .await?
            .map(|bytes| H256::decode(bytes.as_slice()))
            .transpose()
            .map_err(StoreError::from)
    }

    /// Records a block that failed validation so it can be served by
    /// `debug_getBadBlocks`. The list is bounded to [`MAX_BAD_BLOCKS`] entries,
    /// kept sorted by descending block number, with the oldest dropped once the
    /// bound is exceeded. Duplicate `(number, hash)` entries are ignored.
    pub async fn add_bad_block(&self, block: Block) -> Result<(), StoreError> {
        let mut bad_blocks = self.get_bad_blocks().await?;
        let block_number = block.header.number;
        let block_hash = block.hash();
        if bad_blocks
            .iter()
            .any(|b| b.header.number == block_number && b.hash() == block_hash)
        {
            return Ok(());
        }
        bad_blocks.push(block);
        bad_blocks.sort_by(|a, b| b.header.number.cmp(&a.header.number));
        bad_blocks.truncate(MAX_BAD_BLOCKS);
        self.write_async(
            BAD_BLOCKS,
            BAD_BLOCKS_KEY.to_vec(),
            bad_blocks.encode_to_vec(),
        )
        .await
    }

    /// Returns the recent bad blocks seen by the client, sorted by descending
    /// block number. Used by `debug_getBadBlocks`.
    pub async fn get_bad_blocks(&self) -> Result<Vec<Block>, StoreError> {
        match self.read_async(BAD_BLOCKS, BAD_BLOCKS_KEY.to_vec()).await? {
            Some(bytes) => Vec::<Block>::decode(&bytes).map_err(StoreError::from),
            None => Ok(Vec::new()),
        }
    }

    /// Obtain block number for a given hash
    pub fn get_block_number_sync(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockNumber>, StoreError> {
        if let Some(n) = self.buffer()?.get_number(&block_hash) {
            return Ok(Some(n));
        }
        let txn = self.backend.begin_read()?;
        txn.get(BLOCK_NUMBERS, &block_hash.encode_to_vec())?
            .map(|bytes| -> Result<BlockNumber, StoreError> {
                let array: [u8; 8] = bytes
                    .try_into()
                    .map_err(|_| StoreError::Custom("Invalid BlockNumber bytes".to_string()))?;
                Ok(BlockNumber::from_le_bytes(array))
            })
            .transpose()
    }

    /// Get the canonical block hash for a given block number.
    pub fn get_canonical_block_hash_sync(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<BlockHash>, StoreError> {
        let last = self.latest_block_header.get();
        if last.number == block_number {
            return Ok(Some(last.hash()));
        }
        let txn = self.backend.begin_read()?;
        txn.get(
            CANONICAL_BLOCK_HASHES,
            block_number.to_le_bytes().as_slice(),
        )?
        .map(|bytes| H256::decode(bytes.as_slice()))
        .transpose()
        .map_err(StoreError::from)
    }

    /// CAUTION: This method writes directly to the underlying database, bypassing any caching layer.
    /// For updating the state after block execution, use [`Self::store_block_updates`].
    pub async fn write_storage_trie_nodes_batch(
        &self,
        storage_trie_nodes: StorageUpdates,
    ) -> Result<(), StoreError> {
        let mut txn = self.backend.begin_write()?;
        tokio::task::spawn_blocking(move || {
            for (address_hash, nodes) in storage_trie_nodes {
                for (node_path, node_data) in nodes {
                    let key = apply_prefix(Some(address_hash), node_path);
                    if node_data.is_empty() {
                        txn.delete(STORAGE_TRIE_NODES, key.as_ref())?;
                    } else {
                        txn.put(STORAGE_TRIE_NODES, key.as_ref(), &node_data)?;
                    }
                }
            }
            txn.commit()
        })
        .await
        .map_err(|e| StoreError::Custom(format!("Task panicked: {}", e)))?
    }

    /// CAUTION: This method writes directly to the underlying database, bypassing any caching layer.
    /// For updating the state after block execution, use [`Self::store_block_updates`].
    pub async fn write_account_code_batch(
        &self,
        account_codes: Vec<(H256, Code)>,
    ) -> Result<(), StoreError> {
        let mut code_batch_items = Vec::new();
        let mut metadata_batch_items = Vec::new();

        for (code_hash, code) in account_codes {
            let buf = encode_code(&code);
            let metadata_buf = (code.len() as u64).to_be_bytes().to_vec();
            code_batch_items.push((code_hash.as_bytes().to_vec(), buf));
            metadata_batch_items.push((code_hash.as_bytes().to_vec(), metadata_buf));
        }

        // Write both batches
        self.write_batch_async(ACCOUNT_CODES, code_batch_items)
            .await?;
        self.write_batch_async(ACCOUNT_CODE_METADATA, metadata_batch_items)
            .await
    }

    /// Returns a snapshot of the current block-data buffer.
    fn buffer(&self) -> Result<Arc<BlockDataBuffer>, StoreError> {
        Ok(self
            .block_data_buffer
            .read()
            .map_err(|_| StoreError::LockError)?
            .clone())
    }

    // Helper methods for async operations with spawn_blocking
    // These methods ensure RocksDB I/O doesn't block the tokio runtime

    /// Helper method for async writes
    /// Spawns blocking task to avoid blocking tokio runtime
    pub fn write(
        &self,
        table: &'static str,
        key: Vec<u8>,
        value: Vec<u8>,
    ) -> Result<(), StoreError> {
        let backend = self.backend.clone();
        let mut txn = backend.begin_write()?;
        txn.put(table, &key, &value)?;
        txn.commit()
    }

    /// Helper method for async writes
    /// Spawns blocking task to avoid blocking tokio runtime
    async fn write_async(
        &self,
        table: &'static str,
        key: Vec<u8>,
        value: Vec<u8>,
    ) -> Result<(), StoreError> {
        let backend = self.backend.clone();

        tokio::task::spawn_blocking(move || {
            let mut txn = backend.begin_write()?;
            txn.put(table, &key, &value)?;
            txn.commit()
        })
        .await
        .map_err(|e| StoreError::Custom(format!("Task panicked: {}", e)))?
    }

    /// Helper method for async reads
    /// Spawns blocking task to avoid blocking tokio runtime
    pub async fn read_async(
        &self,
        table: &'static str,
        key: Vec<u8>,
    ) -> Result<Option<Vec<u8>>, StoreError> {
        let backend = self.backend.clone();

        tokio::task::spawn_blocking(move || {
            let txn = backend.begin_read()?;
            txn.get(table, &key)
        })
        .await
        .map_err(|e| StoreError::Custom(format!("Task panicked: {}", e)))?
    }

    /// Helper method for sync reads
    /// Spawns blocking task to avoid blocking tokio runtime
    pub fn read(&self, table: &'static str, key: Vec<u8>) -> Result<Option<Vec<u8>>, StoreError> {
        let backend = self.backend.clone();
        let txn = backend.begin_read()?;
        txn.get(table, &key)
    }

    /// Helper method for batch writes
    /// Spawns blocking task to avoid blocking tokio runtime
    /// This is the most important optimization for healing performance
    pub async fn write_batch_async(
        &self,
        table: &'static str,
        batch_ops: Vec<(Vec<u8>, Vec<u8>)>,
    ) -> Result<(), StoreError> {
        let backend = self.backend.clone();

        tokio::task::spawn_blocking(move || {
            let mut txn = backend.begin_write()?;
            txn.put_batch(table, batch_ops)?;
            txn.commit()
        })
        .await
        .map_err(|e| StoreError::Custom(format!("Task panicked: {}", e)))?
    }

    /// Helper method for batch writes
    pub fn write_batch(
        &self,
        table: &'static str,
        batch_ops: Vec<(Vec<u8>, Vec<u8>)>,
    ) -> Result<(), StoreError> {
        let backend = self.backend.clone();
        let mut txn = backend.begin_write()?;
        txn.put_batch(table, batch_ops)?;
        txn.commit()
    }

    pub async fn add_fullsync_batch(&self, headers: Vec<BlockHeader>) -> Result<(), StoreError> {
        self.write_batch_async(
            FULLSYNC_HEADERS,
            headers
                .into_iter()
                .map(|header| (header.number.to_le_bytes().to_vec(), header.encode_to_vec()))
                .collect(),
        )
        .await
    }

    pub async fn read_fullsync_batch(
        &self,
        start: BlockNumber,
        limit: u64,
    ) -> Result<Vec<Option<BlockHeader>>, StoreError> {
        let mut res = vec![];
        let read_tx = self.backend.begin_read()?;
        // TODO: use read_bulk here
        for key in start..start + limit {
            let header_opt = read_tx
                .get(FULLSYNC_HEADERS, &key.to_le_bytes())?
                .map(|header| BlockHeader::decode(&header))
                .transpose()?;
            res.push(header_opt);
        }
        Ok(res)
    }

    pub async fn clear_fullsync_headers(&self) -> Result<(), StoreError> {
        self.backend.clear_table(FULLSYNC_HEADERS)
    }

    /// Delete a key from a table
    pub fn delete(&self, table: &'static str, key: Vec<u8>) -> Result<(), StoreError> {
        let mut txn = self.backend.begin_write()?;
        txn.delete(table, &key)?;
        txn.commit()
    }

    pub fn store_block_updates(&self, update_batch: UpdateBatch) -> Result<(), StoreError> {
        self.apply_updates(update_batch)
    }

    /// Compute `(parent_state_root, last_state_root, last_block_number,
    /// last_block_hash)` for a batch's trie update: the state root of the first
    /// block's parent, the last block's own state root, and the last block's
    /// number and hash (the identity the journal records for the committed
    /// layer). Used by `apply_updates` for both the live and full-sync paths
    /// (which share the single persist worker).
    fn batch_state_roots(
        &self,
        update_batch: &UpdateBatch,
    ) -> Result<(H256, H256, BlockNumber, H256), StoreError> {
        let parent_state_root = self
            .get_block_header_by_hash(
                update_batch
                    .blocks
                    .first()
                    .ok_or(StoreError::UpdateBatchNoBlocks)?
                    .header
                    .parent_hash,
            )?
            .map(|header| header.state_root)
            .unwrap_or_default();
        let last_block = update_batch
            .blocks
            .last()
            .ok_or(StoreError::UpdateBatchNoBlocks)?;
        let last_state_root = last_block.header.state_root;
        let last_block_number = last_block.header.number;
        let last_block_hash = last_block.hash();
        Ok((
            parent_state_root,
            last_state_root,
            last_block_number,
            last_block_hash,
        ))
    }

    /// Single path for all updates: hand the whole unit (block data + one aggregate trie
    /// diff) to the SINGLE persist worker and wait for its ack. `commit_depth` selects the
    /// commit gate; `wait_for_flush` selects when the worker acks (see [`UpdateBatch`]).
    fn apply_updates(&self, update_batch: UpdateBatch) -> Result<(), StoreError> {
        let (parent_state_root, last_state_root, last_block_number, last_block_hash) =
            self.batch_state_roots(&update_batch)?;

        let UpdateBatch {
            account_updates,
            storage_updates,
            blocks,
            receipts,
            code_updates,
            binary_update,
            commit_depth,
            wait_for_flush,
        } = update_batch;

        // Register before handing off to the worker and before this returns, so
        // any reader opening this root blocks in `gated_snapshot` until the
        // layer is installed rather than snapshotting a stale cache.
        self.pending_trie_roots.register(last_state_root)?;

        // Pair blocks with receipts. Single-block fast path avoids a HashMap
        // allocation; full-sync batch joins by hash.
        let blocks_with_receipts: Vec<(Block, Vec<Receipt>)> = if blocks.len() == 1 {
            let block = blocks.into_iter().next().expect("len == 1");
            let hash = block.hash();
            let r = receipts
                .into_iter()
                .find(|(h, _)| *h == hash)
                .map(|(_, r)| r)
                .unwrap_or_default();
            vec![(block, r)]
        } else {
            let mut receipts_by_hash: std::collections::HashMap<BlockHash, Vec<Receipt>> =
                receipts.into_iter().collect();
            blocks
                .into_iter()
                .map(|b| {
                    let r = receipts_by_hash.remove(&b.hash()).unwrap_or_default();
                    (b, r)
                })
                .collect()
        };

        // Send to the persist worker and wait for its ack.
        // wait_for_flush=false: worker acks after staging; the ack carries the PRIOR flush
        //   result so a disk error surfaces on the next call.
        // wait_for_flush=true: worker acks after flush, bounding in-flight work to ~1.
        let (ack_tx, ack_rx) = sync_channel(1);
        self.persist_tx
            .send(PersistMessage::Block(Box::new(BlockPersist {
                blocks: blocks_with_receipts,
                codes: code_updates,
                parent_state_root,
                child_state_root: last_state_root,
                account_updates,
                storage_updates,
                binary_update,
                commit_depth,
                wait_for_flush,
                block_number: last_block_number,
                block_hash: last_block_hash,
                ack: ack_tx,
            })))
            .map_err(|e| StoreError::Custom(format!("failed to send block persist: {e}")))?;
        ack_rx
            .recv()
            .map_err(|e| StoreError::Custom(format!("block persist ack failed: {e}")))??;

        Ok(())
    }

    /// Opens (or creates) a store at `path` with the default [`StoreConfig`].
    ///
    /// Production callers that need to override storage tunables (e.g. the RocksDB
    /// block cache size from a CLI option) should use [`Store::new_with_config`].
    pub fn new(path: impl AsRef<Path>, engine_type: EngineType) -> Result<Self, StoreError> {
        Self::new_with_config(path, engine_type, StoreConfig::default())
    }

    /// Opens (or creates) a store at `path`, applying the supplied [`StoreConfig`].
    pub fn new_with_config(
        path: impl AsRef<Path>,
        engine_type: EngineType,
        // `config` only feeds the RocksDB backend; without that feature it is unused.
        #[cfg_attr(not(feature = "rocksdb"), allow(unused_variables))] config: StoreConfig,
    ) -> Result<Self, StoreError> {
        let db_path = path.as_ref().to_path_buf();

        if engine_type != EngineType::InMemory {
            let version = read_store_schema_version(&db_path)?;

            match version {
                None if db_path.exists() && dir_contains_legacy_db(&db_path)? => {
                    // Pre-metadata DB — cannot migrate safely
                    return Err(StoreError::NotFoundDBVersion);
                }
                None => {
                    // No metadata and no recognizable database files. The directory
                    // may still hold unrelated files (e.g. a JWT secret placed in the
                    // datadir by tooling such as EthDocker, see issue #5680), so treat
                    // this as a fresh datadir and write the initial metadata instead
                    // of erroring out.
                    init_metadata_file(&db_path)?;
                }
                Some(v) if v < 1 => {
                    return Err(StoreError::MigrationFailed {
                        from: v,
                        to: STORE_SCHEMA_VERSION,
                        reason: format!("DB version v{v} is invalid (predates migrations)"),
                    });
                }
                Some(v) if v > STORE_SCHEMA_VERSION => {
                    return Err(StoreError::MigrationFailed {
                        from: v,
                        to: STORE_SCHEMA_VERSION,
                        reason: format!(
                            "DB version v{v} is more recent than the client expects (v{STORE_SCHEMA_VERSION}). Rolling back is not supported"
                        ),
                    });
                }
                #[cfg(feature = "rocksdb")]
                Some(v) if v < STORE_SCHEMA_VERSION => {
                    // Open backend, run migrations, then drop obsolete CFs.
                    // Cleanup must happen AFTER migrations so legacy CFs (e.g.
                    // `receipts`) are still readable during the migration.
                    let rocksdb = Arc::new(RocksDBBackend::open(
                        &path,
                        config.rocksdb_block_cache_size,
                    )?);
                    crate::migrations::run_pending_migrations(rocksdb.as_ref(), &db_path, v)?;
                    rocksdb.drop_obsolete_cfs(&path);
                    let backend: Arc<dyn crate::api::StorageBackend> = rocksdb;
                    return Self::from_backend(
                        backend,
                        db_path,
                        DB_COMMIT_THRESHOLD,
                        config.persist_channel_capacity,
                    );
                }
                Some(_) => {
                    // version == STORE_SCHEMA_VERSION, proceed normally.
                    // Without the `rocksdb` feature this also covers v < target,
                    // but that path is unreachable since InMemory is the only
                    // engine type and the outer guard excludes it.
                }
            }
        }

        match engine_type {
            #[cfg(feature = "rocksdb")]
            EngineType::RocksDB => {
                let rocksdb = RocksDBBackend::open(&path, config.rocksdb_block_cache_size)?;
                rocksdb.drop_obsolete_cfs(&path);
                let backend: Arc<dyn StorageBackend> = Arc::new(rocksdb);
                Self::from_backend(
                    backend,
                    db_path,
                    DB_COMMIT_THRESHOLD,
                    config.persist_channel_capacity,
                )
            }
            EngineType::InMemory => {
                let backend = Arc::new(InMemoryBackend::open()?);
                Self::from_backend(
                    backend,
                    db_path,
                    IN_MEMORY_COMMIT_THRESHOLD,
                    config.persist_channel_capacity,
                )
            }
        }
    }

    fn from_backend(
        backend: Arc<dyn StorageBackend>,
        db_path: PathBuf,
        commit_threshold: usize,
        persist_channel_capacity: usize,
    ) -> Result<Self, StoreError> {
        debug!("Initializing Store with {commit_threshold} in-memory diff-layers");

        // Drain journal entries left by a previous codec version before the
        // `Store` exists, which is the only point at which nothing can yet be
        // holding a fork-choice update. `compute_reorg_ceiling` reads the journal
        // floor via `lowest_state_history_block_number`; leaving stale entries in
        // place would advertise reach the decoder refuses to serve, so the reorg
        // would be accepted and then fail mid-flight with `StateNotReachable`.
        // Draining first makes the floor honest and turns that halt into a clean
        // `-38006 TooDeepReorg`. See `journal::drain_stale_journal_entries` for the
        // exact semantics (contiguous bottom run only) and the cost.
        //
        // Every construction path funnels through here — fresh open, migration,
        // and in-memory — so there is no second place to remember. On a journal
        // with no stale bottom entry this is one `first_key` plus one `get`.
        crate::journal::drain_stale_journal_entries(backend.as_ref())?;

        let (fkv_tx, fkv_rx) = std::sync::mpsc::sync_channel(0);
        let persist_cap = persist_channel_capacity.max(1); // clamp: 0 would be a rendezvous channel
        let (persist_tx, persist_rx) = std::sync::mpsc::sync_channel(persist_cap);

        let (last_written, binary_last_written, initial_flushed_upto) = {
            let tx = backend.begin_read()?;
            let last_written = tx
                .get(MISC_VALUES, "last_written".as_bytes())?
                .unwrap_or_else(|| vec![0u8; 64]);
            let last_written = if last_written == [0xff] {
                vec![0xff; 64]
            } else {
                last_written
            };
            // Kept verbatim, unlike the MPT frontier above: the binary marker's
            // three states are already unambiguous, so there is nothing to
            // expand and no length to guess. See the `binary_last_computed`
            // field docs.
            let binary_last_written = tx
                .get(MISC_VALUES, BINARY_LAST_WRITTEN_KEY)?
                .unwrap_or_default();
            let initial_flushed_upto = match tx.get(MISC_VALUES, FLUSHED_UPTO_KEY)? {
                Some(bytes) => decode_flushed_upto(&bytes)?,
                None => 0,
            };
            (last_written, binary_last_written, initial_flushed_upto)
        };

        // The binary mirror's cold-start wipe, taken here rather than inside
        // the backfill generator where the MPT takes it.
        //
        // An absent marker means no sweep and no genesis seed has ever claimed
        // this table, so whatever is in it is of unknown provenance — rows an
        // abandoned sweep left behind for leaves the trie has since dropped
        // would make the mirror a *superset*, and a range served from it would
        // carry leaves the root does not commit to.
        //
        // It happens at open because Task 6's write-ownership rule reads an
        // absent marker as "the commit path owns the whole keyspace": blocks
        // imported before the generator's first `Continue` write mirror rows,
        // and a lazy wipe would truncate the table underneath them. Doing it
        // before the store exists makes that rule true instead of merely
        // usually-true. A partial frontier is left alone — that is a resume,
        // and its rows are exactly what the frontier claims.
        if binary_last_written.is_empty() {
            backend.clear_table(BINARY_FLATKEYVALUE)?;
        }
        let mut initial_buffer = BlockDataBuffer::new();
        initial_buffer.set_flushed_upto(initial_flushed_upto);

        let mut background_threads = Vec::new();
        let safe_commit_root = Arc::new(RwLock::new(H256::zero()));
        let mut store = Self {
            db_path,
            backend,
            chain_config: Default::default(),
            latest_block_header: Default::default(),
            trie_cache: Arc::new(RwLock::new(Arc::new(TrieLayerCache::new_with_safe_commit(
                commit_threshold,
                safe_commit_root.clone(),
            )))),
            flatkeyvalue_control_tx: fkv_tx,
            block_data_buffer: Arc::new(RwLock::new(Arc::new(initial_buffer))),
            persist_tx,
            pending_trie_roots: Arc::new(PendingTrieRoots::default()),
            last_computed_flatkeyvalue: Arc::new(RwLock::new(last_written)),
            binary_last_computed: Arc::new(RwLock::new(binary_last_written)),
            account_code_cache: Arc::new(Mutex::new(CodeCache::default())),
            code_metadata_cache: Arc::new(Mutex::new(rustc_hash::FxHashMap::default())),
            fcu_lock: Arc::new(tokio::sync::Mutex::new(())),
            safe_commit_root,
            journal_pruning_paused: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            background_threads: Default::default(),
        };
        let backend_clone = store.backend.clone();
        let last_computed_fkv = store.last_computed_flatkeyvalue.clone();
        let binary_last_computed = store.binary_last_computed.clone();
        background_threads.push(std::thread::spawn(move || {
            let rx = fkv_rx;
            // Wait for the first Continue to start generation
            loop {
                match rx.recv() {
                    Ok(FKVGeneratorControlMessage::Continue) => break,
                    Ok(FKVGeneratorControlMessage::Stop) => {}
                    Err(std::sync::mpsc::RecvError) => {
                        debug!("Closing FlatKeyValue generator.");
                        return;
                    }
                }
            }

            let _ = flatkeyvalue_generator(&backend_clone, &last_computed_fkv, &rx)
                .inspect_err(|err| error!("Error while generating FlatKeyValue: {err}"));
            // The binary mirror's sweep runs here, on this thread and over this
            // same channel, rather than on a second one. `commit_to_disk`'s
            // `Stop` is a rendezvous send that blocks the persist worker until
            // a generator receives it; a second independent rendezvous channel
            // would make the worker wait on two generators in series. Sharing
            // means one `Stop` pauses whichever sweep is running, and adds no
            // message to that critical path.
            //
            // Sequenced rather than chained on the MPT sweep's result: the two
            // are independent backfills, and an MPT failure is no reason to
            // leave the binary mirror unbuilt.
            let _ = binary_flat_generator(&backend_clone, &binary_last_computed, &rx)
                .inspect_err(|err| error!("Error while generating the binary flat mirror: {err}"));
        }));
        // The single persist worker: sole swapper of `block_data_buffer`, sole
        // builder of trie diff-layers. One DB transaction per `Block` message.
        let persist_backend = store.backend.clone();
        let persist_buffer = store.block_data_buffer.clone();
        let persist_trie_cache = store.trie_cache.clone();
        let persist_pending_roots = store.pending_trie_roots.clone();
        let persist_fkv_ctl = store.flatkeyvalue_control_tx.clone();
        background_threads.push(std::thread::spawn(move || {
            let rx = persist_rx;
            // Carries the prior flush result: the live path acks after staging,
            // so a disk failure surfaces on the next message's ack.
            let mut last_flush_result: Result<(), StoreError> = Ok(());
            loop {
                match rx.recv() {
                    Ok(PersistMessage::Block(bp)) => {
                        let bp = *bp;
                        // Stage block data (sole swapper of the buffer; codes
                        // are batch-level and attributed to the first block).
                        let staged = mutate_block_buffer(&persist_buffer, move |b| {
                            let mut codes = Some(bp.codes);
                            for (block, receipts) in bp.blocks {
                                b.insert(block, receipts, codes.take().unwrap_or_default());
                            }
                        });
                        if let Err(e) = staged {
                            // Stage failure is terminal for this message.
                            // Clear the pending root so gated readers are not
                            // blocked forever (apply_trie_phase1, which normally
                            // does this, is skipped when we continue here).
                            persist_pending_roots.clear(bp.child_state_root);
                            let _ = bp.ack.send(Err(e));
                            continue;
                        }
                        // ACK-AFTER-STAGING: ack now, carrying the prior flush result.
                        // NOTE: this acks block validity BEFORE apply_trie_phase1
                        // installs the trie layer below. A phase-1 failure (only
                        // reachable via lock poisoning, which is already fatal) is
                        // therefore deferred to the next block's ack via
                        // last_flush_result rather than attributed to this block;
                        // the pending root is still cleared unconditionally, so
                        // gated readers error rather than hang.
                        if !bp.wait_for_flush {
                            let _ = bp
                                .ack
                                .send(std::mem::replace(&mut last_flush_result, Ok(())));
                        }
                        // Build + install the trie layer; clear the read gate.
                        if let Err(err) = apply_trie_phase1(
                            &persist_trie_cache,
                            &persist_pending_roots,
                            bp.parent_state_root,
                            bp.child_state_root,
                            bp.block_number,
                            bp.block_hash,
                            bp.account_updates,
                            bp.storage_updates,
                            bp.binary_update,
                        ) {
                            error!("persist worker trie phase-1 failed: {err}");
                            if bp.wait_for_flush {
                                let _ = bp.ack.send(Err(err));
                            } else {
                                last_flush_result = Err(err);
                            }
                            continue;
                        }
                        // Flush block data + commit bottom trie layer when due.
                        let flushed = flush_block_data(persist_backend.as_ref(), &persist_buffer)
                            .inspect_err(|err| error!("flush_block_data failed: {err}"))
                            .and_then(|_| {
                                commit_trie_if_due(
                                    persist_backend.as_ref(),
                                    &persist_trie_cache,
                                    &persist_fkv_ctl,
                                    bp.parent_state_root,
                                    bp.commit_depth,
                                    bp.wait_for_flush,
                                )
                            });
                        // ACK-AFTER-FLUSH: ack now (bounds in-flight work to ~1), folding in
                        // any prior deferred error. ACK-AFTER-STAGING: stash for the next ack.
                        if bp.wait_for_flush {
                            let prior = std::mem::replace(&mut last_flush_result, Ok(()));
                            let _ = bp.ack.send(prior.and(flushed));
                        } else {
                            last_flush_result = flushed;
                        }
                    }
                    Ok(PersistMessage::Commit(root)) => match persist_trie_cache.read() {
                        Ok(guard) => {
                            let trie = guard.clone();
                            drop(guard);
                            // Forkchoice-driven flush is the live (non-batch) path, so
                            // journaling is enabled: pass `is_batch = false`.
                            let _ = commit_to_disk(
                                persist_backend.as_ref(),
                                &persist_fkv_ctl,
                                &persist_trie_cache,
                                &trie,
                                root,
                                false,
                            )
                            .inspect_err(|err| error!("commit_to_disk failed: {err}"));
                        }
                        Err(_) => error!("trie cache lock poisoned during commit"),
                    },
                    Ok(PersistMessage::Ping(ack)) => {
                        // Idle handshake: reached only after all earlier Block
                        // messages are fully processed. Carry the pending flush
                        // result so a live-path failure is not silently dropped.
                        let _ = ack.send(std::mem::replace(&mut last_flush_result, Ok(())));
                    }
                    Ok(PersistMessage::Shutdown { ack }) => {
                        // Graceful shutdown: drain (already guaranteed by FIFO) and
                        // force-flush the not-yet-flushed block-data tail. The trie
                        // diff-layers stay in memory and are dropped on exit: the
                        // on-disk trie is a single-version path store, so committing
                        // the non-finalized tail would make a post-restart reorg
                        // unrecoverable. Those layers re-execute on the next start.
                        let result = flush_block_data(persist_backend.as_ref(), &persist_buffer);
                        let prior = std::mem::replace(&mut last_flush_result, Ok(()));
                        let _ = ack.send(prior.and(result));
                        // No more work will follow a shutdown request.
                        return;
                    }
                    Err(_) => return,
                }
            }
        }));
        store.background_threads = Arc::new(ThreadList {
            list: background_threads,
        });
        Ok(store)
    }

    /// Opens (or creates) a store at `store_path` and seeds it from the
    /// given genesis file, using the default [`StoreConfig`].
    pub async fn new_from_genesis(
        store_path: &Path,
        engine_type: EngineType,
        genesis_path: &str,
    ) -> Result<Self, StoreError> {
        Self::new_from_genesis_with_config(
            store_path,
            engine_type,
            genesis_path,
            StoreConfig::default(),
        )
        .await
    }

    /// Opens (or creates) a store at `store_path` from genesis, applying the
    /// supplied [`StoreConfig`].
    pub async fn new_from_genesis_with_config(
        store_path: &Path,
        engine_type: EngineType,
        genesis_path: &str,
        config: StoreConfig,
    ) -> Result<Self, StoreError> {
        let file = std::fs::File::open(genesis_path)
            .map_err(|error| StoreError::Custom(format!("Failed to open genesis file: {error}")))?;
        let reader = std::io::BufReader::new(file);
        let genesis: Genesis = serde_json::from_reader(reader)
            .map_err(|e| StoreError::Custom(format!("Failed to deserialize genesis file: {e}")))?;
        let mut store = Self::new_with_config(store_path, engine_type, config)?;
        store.add_initial_state(genesis).await?;
        Ok(store)
    }

    pub async fn get_account_info(
        &self,
        block_number: BlockNumber,
        address: Address,
    ) -> Result<Option<AccountInfo>, StoreError> {
        match self.get_canonical_block_hash(block_number).await? {
            Some(block_hash) => self.get_account_info_by_hash(block_hash, address),
            None => Ok(None),
        }
    }

    /// Serves `eth_getBalance` (and the mempool's sender lookups), and through
    /// [`Self::get_code_by_account_address`] and
    /// [`Self::get_nonce_by_account_address`] also `eth_getCode` and
    /// `eth_getTransactionCount`. Errors with [`StoreError::MissingStateRoot`]
    /// for a known block whose state this node no longer holds, rather than
    /// answering from the on-disk trie's current state; `Ok(None)` keeps meaning
    /// "no such account".
    ///
    /// Resolves against whichever trie *this block's header* addresses — see
    /// [`Self::header_addresses_binary_trie`] — so a pre-activation block keeps
    /// answering out of the MPT after the chain has flipped, and both sides
    /// carry their own availability guard.
    ///
    /// [`AccountInfo`] rather than an `AccountState` is what makes one shared
    /// path possible across the two tries: it has no `storage_root`, which the
    /// binary trie has no value for (see `pbt_state::get_account_info`). All
    /// three account RPCs only ever wanted the balance, the nonce and the code
    /// hash, so none of them pays for the storage question
    /// [`Self::get_binary_account`] asks on execution's behalf.
    pub fn get_account_info_by_hash(
        &self,
        block_hash: BlockHash,
        address: Address,
    ) -> Result<Option<AccountInfo>, StoreError> {
        let Some(header) = self.get_block_header_by_hash(block_hash)? else {
            return Ok(None);
        };
        if self.header_addresses_binary_trie(&header) {
            self.ensure_binary_trie_state(block_hash, &header)?;
            return self.get_binary_account_info(header.state_root, address);
        }
        let state_trie = self.state_trie_checked_for_header(&header)?;
        let hashed_address = hash_address_fixed(&address);

        let Some(encoded_state) = state_trie.get(hashed_address.as_bytes())? else {
            return Ok(None);
        };

        let account_state = AccountState::decode(&encoded_state)?;
        Ok(Some(AccountInfo {
            code_hash: account_state.code_hash,
            balance: account_state.balance,
            nonce: account_state.nonce,
        }))
    }

    pub fn get_account_state_by_acc_hash(
        &self,
        block_hash: BlockHash,
        account_hash: H256,
    ) -> Result<Option<AccountState>, StoreError> {
        let Some(state_trie) = self.state_trie(block_hash)? else {
            return Ok(None);
        };
        let Some(encoded_state) = state_trie.get(account_hash.as_bytes())? else {
            return Ok(None);
        };
        let account_state = AccountState::decode(&encoded_state)?;
        Ok(Some(account_state))
    }

    pub async fn get_fork_id(&self) -> Result<ForkId, StoreError> {
        let chain_config = self.get_chain_config();
        let genesis_header = self
            .load_block_header(0)?
            .ok_or(StoreError::MissingEarliestBlockNumber)?;
        let block_header = self.latest_block_header.get();

        Ok(ForkId::new(
            chain_config,
            genesis_header,
            block_header.timestamp,
            block_header.number,
        ))
    }

    /// Serves `eth_getCode`. See [`Self::get_account_info_by_hash`] for which
    /// trie the block's header resolves against, and for the behaviour at a
    /// block whose state this node no longer holds.
    ///
    /// The bytecode itself is unaffected by the activation: it is fetched from
    /// the code table by hash, which is one table for both tries. Only reaching
    /// the hash goes through state.
    pub async fn get_code_by_account_address(
        &self,
        block_number: BlockNumber,
        address: Address,
    ) -> Result<Option<Code>, StoreError> {
        let Some(block_hash) = self.get_canonical_block_hash(block_number).await? else {
            return Ok(None);
        };
        let Some(account_info) = self.get_account_info_by_hash(block_hash, address)? else {
            return Ok(None);
        };
        self.get_account_code(account_info.code_hash)
    }

    /// Serves `eth_getTransactionCount`. See [`Self::get_account_info_by_hash`]
    /// for which trie the block's header resolves against, and for the behaviour
    /// at a block whose state this node no longer holds.
    pub async fn get_nonce_by_account_address(
        &self,
        block_number: BlockNumber,
        address: Address,
    ) -> Result<Option<u64>, StoreError> {
        let Some(block_hash) = self.get_canonical_block_hash(block_number).await? else {
            return Ok(None);
        };
        Ok(self
            .get_account_info_by_hash(block_hash, address)?
            .map(|account_info| account_info.nonce))
    }

    /// Applies account updates based on the block's latest storage state
    /// and returns the new state root after the updates have been applied.
    pub fn apply_account_updates_batch(
        &self,
        block_hash: BlockHash,
        account_updates: &[AccountUpdate],
    ) -> Result<Option<AccountUpdatesList>, StoreError> {
        let Some(mut state_trie) = self.state_trie(block_hash)? else {
            return Ok(None);
        };

        Ok(Some(self.apply_account_updates_from_trie_batch(
            &mut state_trie,
            account_updates,
        )?))
    }

    pub fn apply_account_updates_from_trie_batch<'a>(
        &self,
        state_trie: &mut Trie,
        account_updates: impl IntoIterator<Item = &'a AccountUpdate>,
    ) -> Result<AccountUpdatesList, StoreError> {
        let mut ret_storage_updates = Vec::new();
        let mut code_updates = Vec::new();
        let state_root = state_trie.hash_no_commit(&NativeCrypto);
        for update in account_updates {
            let hashed_address = hash_address_fixed(&update.address);
            if update.removed {
                // Remove account from trie
                state_trie.remove(hashed_address.as_bytes())?;
                continue;
            }
            // Add or update AccountState in the trie
            // Fetch current state or create a new state to be inserted
            let mut account_state = match state_trie.get(hashed_address.as_bytes())? {
                Some(encoded_state) => AccountState::decode(&encoded_state)?,
                None => AccountState::default(),
            };
            if update.removed_storage {
                account_state.storage_root = EMPTY_TRIE_HASH;
            }
            if let Some(info) = &update.info {
                account_state.nonce = info.nonce;
                account_state.balance = info.balance;
                account_state.code_hash = info.code_hash;
                // Store updated code in DB
                if let Some(code) = &update.code {
                    code_updates.push((info.code_hash, code.clone()));
                }
            }
            // Store the added storage in the account's storage trie and compute its new root
            if !update.added_storage.is_empty() {
                let mut storage_trie =
                    self.open_storage_trie(hashed_address, state_root, account_state.storage_root)?;
                for (storage_key, storage_value) in &update.added_storage {
                    let hashed_key = hash_key(storage_key);
                    if storage_value.is_zero() {
                        storage_trie.remove(&hashed_key)?;
                    } else {
                        storage_trie.insert(hashed_key, storage_value.encode_to_vec())?;
                    }
                }
                let (storage_hash, storage_updates) =
                    storage_trie.collect_changes_since_last_hash(&NativeCrypto);
                account_state.storage_root = storage_hash;
                ret_storage_updates.push((hashed_address, storage_updates));
            }
            state_trie.insert(
                hashed_address.as_bytes().to_vec(),
                account_state.encode_to_vec(),
            )?;
        }
        let (state_trie_hash, state_updates) =
            state_trie.collect_changes_since_last_hash(&NativeCrypto);

        Ok(AccountUpdatesList {
            state_trie_hash,
            state_updates,
            storage_updates: ret_storage_updates,
            code_updates,
        })
    }

    /// Performs the same actions as apply_account_updates_from_trie
    ///  but also returns the used storage tries with witness recorded
    pub fn apply_account_updates_from_trie_with_witness(
        &self,
        mut state_trie: Trie,
        account_updates: &[AccountUpdate],
        mut storage_tries: StorageTries,
    ) -> Result<(StorageTries, AccountUpdatesList), StoreError> {
        let mut ret_storage_updates = Vec::new();

        let mut code_updates = Vec::new();

        let state_root = state_trie.hash_no_commit(&NativeCrypto);

        for update in account_updates.iter() {
            let hashed_address = hash_address(&update.address);

            if update.removed {
                // Remove account from trie
                state_trie.remove(&hashed_address)?;

                continue;
            }

            // Add or update AccountState in the trie
            // Fetch current state or create a new state to be inserted
            let mut account_state = match state_trie.get(&hashed_address)? {
                Some(encoded_state) => AccountState::decode(&encoded_state)?,
                None => AccountState::default(),
            };

            if update.removed_storage {
                account_state.storage_root = EMPTY_TRIE_HASH;
            }

            if let Some(info) = &update.info {
                account_state.nonce = info.nonce;

                account_state.balance = info.balance;

                account_state.code_hash = info.code_hash;

                // Store updated code in DB
                if let Some(code) = &update.code {
                    code_updates.push((info.code_hash, code.clone()));
                }
            }

            // Store the added storage in the account's storage trie and compute its new root
            if !update.added_storage.is_empty() {
                let (_witness, storage_trie) = match storage_tries.entry(update.address) {
                    Entry::Occupied(value) => value.into_mut(),
                    Entry::Vacant(vacant) => {
                        let trie = self.open_storage_trie(
                            H256::from_slice(&hashed_address),
                            state_root,
                            account_state.storage_root,
                        )?;
                        vacant.insert(TrieLogger::open_trie(trie))
                    }
                };

                for (storage_key, storage_value) in &update.added_storage {
                    let hashed_key = hash_key(storage_key);

                    if storage_value.is_zero() {
                        storage_trie.remove(&hashed_key)?;
                    } else {
                        storage_trie.insert(hashed_key, storage_value.encode_to_vec())?;
                    }
                }

                let (storage_hash, storage_updates) =
                    storage_trie.collect_changes_since_last_hash(&NativeCrypto);

                account_state.storage_root = storage_hash;

                ret_storage_updates.push((H256::from_slice(&hashed_address), storage_updates));
            }

            state_trie.insert(hashed_address, account_state.encode_to_vec())?;
        }

        let (state_trie_hash, state_updates) =
            state_trie.collect_changes_since_last_hash(&NativeCrypto);

        let account_updates_list = AccountUpdatesList {
            state_trie_hash,
            state_updates,
            storage_updates: ret_storage_updates,
            code_updates,
        };

        Ok((storage_tries, account_updates_list))
    }

    /// Adds all genesis accounts and returns the genesis block's state_root
    pub async fn setup_genesis_state_trie(
        &self,
        genesis_accounts: BTreeMap<Address, GenesisAccount>,
    ) -> Result<H256, StoreError> {
        let mut storage_trie_nodes = vec![];
        let mut genesis_state_trie = self.open_direct_state_trie(EMPTY_TRIE_HASH)?;
        for (address, account) in genesis_accounts {
            let hashed_address = hash_address(&address);
            let h256_hashed_address = H256::from_slice(&hashed_address);

            // Store account code (as this won't be stored in the trie)
            let code = Code::from_bytecode(account.code, &NativeCrypto);
            let code_hash = code.hash;
            self.add_account_code(code).await?;

            // Store the account's storage in a clean storage trie and compute its root
            let mut storage_trie =
                self.open_direct_storage_trie(h256_hashed_address, EMPTY_TRIE_HASH)?;
            for (storage_key, storage_value) in account.storage {
                if !storage_value.is_zero() {
                    let hashed_key = hash_key(&H256(storage_key.to_big_endian()));
                    storage_trie.insert(hashed_key, storage_value.encode_to_vec())?;
                }
            }

            let (storage_root, storage_nodes) =
                storage_trie.collect_changes_since_last_hash(&NativeCrypto);

            storage_trie_nodes.extend(
                storage_nodes
                    .into_iter()
                    .map(|(path, n)| (apply_prefix(Some(h256_hashed_address), path).into_vec(), n)),
            );

            // Add account to trie
            let account_state = AccountState {
                nonce: account.nonce,
                balance: account.balance,
                storage_root,
                code_hash,
            };
            genesis_state_trie.insert(hashed_address, account_state.encode_to_vec())?;
        }

        let (state_root, account_trie_nodes) =
            genesis_state_trie.collect_changes_since_last_hash(&NativeCrypto);
        let account_trie_nodes = account_trie_nodes
            .into_iter()
            .map(|(path, n)| (apply_prefix(None, path).into_vec(), n))
            .collect::<Vec<_>>();

        let mut tx = self.backend.begin_write()?;
        tx.put_batch(ACCOUNT_TRIE_NODES, account_trie_nodes)?;
        tx.put_batch(STORAGE_TRIE_NODES, storage_trie_nodes)?;
        tx.commit()?;

        Ok(state_root)
    }

    /// Adds all genesis accounts to the EIP-8297 binary trie, persists
    /// it, and returns its root: the counterpart of
    /// [`Store::setup_genesis_state_trie`].
    ///
    /// The two differ in more than the trie. The MPT keeps an account's
    /// state in two places — one leaf in the state trie, one storage
    /// subtrie per account — and keeps code out of the trie entirely.
    /// The binary trie is a single tree in which every part of an
    /// account, code chunks included, is a leaf; there are no subtries
    /// to build and no root to thread into an account leaf.
    ///
    /// Code is nevertheless *also* written to the code table, exactly
    /// as the MPT path writes it. The two are not alternatives: the
    /// trie commits to code as chunks, while the EVM fetches whole
    /// bytecode by hash, and only the table answers that.
    ///
    /// **Per-account updates, not the bulk load.** [`BinaryTrie::from_sorted_leaves`]
    /// exists for exactly this shape of input, but it takes *leaves*,
    /// and reaching them from an alloc means deriving each account's
    /// keys — a second copy of the embedding, which could then disagree
    /// with the one block import uses. Routing through
    /// [`apply_account_updates`] instead makes the genesis path and the
    /// import path the same code by construction, which matters more
    /// here than anywhere else: genesis is the anchor every later block
    /// builds on. The cost is bounded — mainnet's alloc is under ten
    /// thousand accounts, and this runs once.
    pub async fn setup_genesis_binary_trie(
        &self,
        genesis_accounts: BTreeMap<Address, GenesisAccount>,
    ) -> Result<H256, StoreError> {
        let mut updates = Vec::with_capacity(genesis_accounts.len());
        for (address, account) in genesis_accounts {
            let code = Code::from_bytecode(account.code, &NativeCrypto);
            self.add_account_code(code.clone()).await?;

            updates.push(AccountUpdate {
                address,
                removed: false,
                info: Some(AccountInfo {
                    code_hash: code.hash,
                    balance: account.balance,
                    nonce: account.nonce,
                }),
                code: Some(code),
                // Zero-valued slots are not filtered out here, unlike
                // in the MPT path: `apply_account_updates` resolves a
                // zero value to an absent leaf itself, for every leaf
                // kind and not just storage.
                added_storage: account
                    .storage
                    .into_iter()
                    .map(|(slot, value)| (H256(slot.to_big_endian()), value))
                    .collect(),
                removed_storage: false,
            });
        }

        let mut trie = BinaryTrie::new(Box::new(BackendBinaryTrieDB::new(self.backend.clone())?));
        apply_account_updates(&mut trie, &updates)?;
        let committed = trie.commit()?;

        // Seed the flat mirror from the same changelog the commit reported,
        // and declare it complete. A chain that starts from genesis therefore
        // never runs the backfill generator: the mirror has covered the whole
        // keyspace since block 0, and every later block maintains it through
        // `commit_to_disk`.
        //
        // **Rows and marker in one batch, and after the nodes.** The nodes went
        // to disk in `commit`'s own transaction, so a crash can land between
        // the two — which is why the marker rides with the rows rather than
        // ahead of them, and why it is written last in program order. Nodes
        // without a marker is the recoverable state: coverage reads as
        // `Nothing`, no reader trusts the mirror, and Task 9's generator
        // rebuilds it. A marker without rows would be the unrecoverable one.
        let mut tx = self.backend.begin_write()?;
        stage_binary_flat_leaves(&mut tx, &committed.leaves)?;
        tx.put(
            MISC_VALUES,
            BINARY_LAST_WRITTEN_KEY,
            BINARY_FLAT_FRONTIER_COMPLETE,
        )?;
        tx.commit()?;
        // Only now: the in-memory frontier is what readers gate on, so
        // publishing it before the batch landed would open the gate on a
        // mirror a failed commit left empty.
        self.publish_binary_flat_frontier(BINARY_FLAT_FRONTIER_COMPLETE)?;

        Ok(committed.root)
    }

    /// Replace the in-memory copy of the mirror's backfill frontier.
    ///
    /// Separate from the durable write on purpose: the durable marker is what
    /// survives a restart and the in-memory one is what readers gate on, so
    /// the in-memory copy must be published *after* the batch carrying the
    /// rows has committed. Publishing first would claim coverage a failed
    /// write never delivered.
    fn publish_binary_flat_frontier(&self, frontier: &[u8]) -> Result<(), StoreError> {
        *self
            .binary_last_computed
            .write()
            .map_err(|_| StoreError::LockError)? = frontier.to_vec();
        Ok(())
    }

    /// Advances the EIP-8297 binary trie by one block: opens it at
    /// `parent_root`, applies `account_updates`, persists the nodes
    /// that changed, and returns the new root. The binary-trie
    /// counterpart of [`Store::apply_account_updates_from_trie_batch`],
    /// and the step that follows [`Store::setup_genesis_binary_trie`]'s
    /// anchor.
    ///
    /// **Storage layer only, deliberately.** Nothing here reads or
    /// writes a block header. Block import reaches it through
    /// [`Store::advance_binary_trie_for_block`], which is what supplies
    /// the parent root and records the result; no header commits to a
    /// binary-trie root yet, so there is still nothing to validate
    /// against.
    ///
    /// **Why this is cheap.** Opening at `parent_root` loads no nodes —
    /// the root is a bare [`NodeRef::Stored`] hash until something
    /// touches it. Applying the updates loads only the root-to-leaf
    /// paths they reach, and [`BinaryTrie::commit`] writes only the
    /// nodes those paths dirtied. A block therefore costs its own
    /// footprint, not the trie's size.
    ///
    /// [`NodeRef::Stored`]: ethrex_binary_trie::trie::NodeRef::Stored
    ///
    /// **Code goes to the code table too**, exactly as the genesis path
    /// and the MPT path write it. The trie commits to code as chunks,
    /// but the EVM fetches whole bytecode by hash, and only
    /// `ACCOUNT_CODES` answers that.
    ///
    /// **No diff layer is involved: nodes land on disk immediately.** This is
    /// the direct-write primitive, kept for the paths that genuinely have no
    /// block and therefore no layer to stage into — the genesis anchor and the
    /// storage-layer tests that exercise the trie in isolation. Block import
    /// does *not* use it; it goes through
    /// [`Store::advance_binary_trie_for_block`], which stages.
    ///
    /// # Errors
    ///
    /// [`StoreError::PbtState`] if the updates cannot be embedded — see
    /// [`Store::apply_account_updates_to_binary_trie_blocking`]. Nothing
    /// is committed in that case, so `parent_root` remains the state on
    /// disk.
    pub async fn apply_account_updates_to_binary_trie(
        &self,
        parent_root: H256,
        account_updates: &[AccountUpdate],
    ) -> Result<H256, StoreError> {
        self.apply_account_updates_to_binary_trie_blocking(parent_root, account_updates)
    }

    /// [`Store::apply_account_updates_to_binary_trie`] without the `async`.
    ///
    /// The async form is the ergonomic one for callers already in a
    /// runtime; this one exists because block import is not. Every entry
    /// point that advances the chain — [`crate::Store::add_block`]'s
    /// callers in `crates/blockchain` — is a synchronous function called
    /// from inside a tokio worker, so `block_on` is not available to it
    /// and making the whole import path async for one storage call is
    /// not a trade worth making.
    ///
    /// The two are the same work: the trie itself is CPU-bound and was
    /// never awaited, and the code writes go through one write batch
    /// instead of one `spawn_blocking` per account.
    pub fn apply_account_updates_to_binary_trie_blocking(
        &self,
        parent_root: H256,
        account_updates: &[AccountUpdate],
    ) -> Result<H256, StoreError> {
        // Codes first and in a single batch: the EVM fetches whole
        // bytecode by hash and only `ACCOUNT_CODES` answers that, while
        // the trie only ever commits to code as chunks.
        self.write_account_codes(account_updates)?;

        let mut trie = BinaryTrie::open(
            Box::new(BackendBinaryTrieDB::new(self.backend.clone())?),
            parent_root,
        );
        apply_account_updates(&mut trie, account_updates)?;
        let committed = trie.commit()?;

        // The mirror follows the nodes here too. This path bypasses the diff
        // layers, so nothing downstream would ever fold its leaf changes into
        // `BINARY_FLATKEYVALUE` — and after genesis marks the frontier
        // complete, a mirror that skipped a commit is not merely stale but
        // *authoritative and wrong*, since a read below the frontier does not
        // fall back to a descent. Cheap insurance: one row per changed leaf,
        // which the changelog already has in hand.
        let mut tx = self.backend.begin_write()?;
        stage_binary_flat_leaves(&mut tx, &committed.leaves)?;
        tx.commit()?;

        Ok(committed.root)
    }

    /// The binary-trie root `account_updates` produce on top of
    /// `parent_root`, **without persisting or staging anything**.
    ///
    /// [`BinaryTrie::root`] merkleizes the in-memory overlay; only the
    /// root-to-leaf paths the updates reach are loaded, and no node,
    /// code entry or root mapping is written. That is what payload
    /// building needs: a proposed block's root has to be known before
    /// the block exists, and most proposals are never imported.
    ///
    /// The reads still go through the diff layers, gated on `parent_hash`, or
    /// a payload built on a parent whose own nodes are still in memory would be
    /// merkleized over stale on-disk state and commit a root no importer can
    /// reproduce.
    ///
    /// The block that *is* imported recomputes the same root through
    /// [`Store::advance_binary_trie_for_block`], which stages it.
    ///
    /// # Errors
    ///
    /// [`StoreError::PbtState`] if the updates cannot be embedded — see
    /// [`Store::apply_account_updates_to_binary_trie_blocking`].
    pub fn compute_binary_trie_root(
        &self,
        parent_hash: BlockHash,
        parent_root: H256,
        account_updates: &[AccountUpdate],
    ) -> Result<H256, StoreError> {
        let mut trie = BinaryTrie::open(
            Box::new(self.layered_binary_trie_db(
                parent_root,
                self.binary_layer_gate(parent_hash, parent_root)?,
                LayeredBinaryTrieDB::staging_buffer(),
            )?),
            parent_root,
        );
        apply_account_updates(&mut trie, account_updates)?;
        Ok(trie.root())
    }

    /// A [`LayeredBinaryTrieDB`] reading at `binary_root` through the layer
    /// cache snapshot taken at `gate_root`, staging its writes into `staged`.
    ///
    /// `gate_root` is the *layer* key to wait on before snapshotting (see
    /// [`Self::binary_layer_gate`]); `binary_root` is what the read walk starts
    /// from. They differ for every pre-activation block.
    fn layered_binary_trie_db(
        &self,
        binary_root: H256,
        gate_root: H256,
        staged: StagedBinaryNodes,
    ) -> Result<LayeredBinaryTrieDB, StoreError> {
        // One read view for both handles, so a node read and a mirror read
        // taken in one traversal see one consistent snapshot: the mirror is
        // written in the nodes' own `write_tx`, and reading the two through
        // different views could land either side of that batch.
        let read_view = self.backend.begin_read()?;
        Ok(LayeredBinaryTrieDB::new(
            binary_root,
            self.gated_snapshot(gate_root)?,
            BackendBinaryTrieDB::with_view(self.backend.clone(), read_view.clone()),
            BackendBinaryFlatDB::with_view(self.backend.clone(), read_view),
            self.binary_flat_coverage()?,
            staged,
        ))
    }

    /// How much of the keyspace a read may trust the binary flat mirror for,
    /// from the in-memory frontier.
    ///
    /// The **read** gate. Its write-side sibling `binary_flat_frontier_covers`
    /// answers the opposite question for an absent marker; see
    /// [`BinaryFlatCoverage`] for why the two must not be unified.
    fn binary_flat_coverage(&self) -> Result<BinaryFlatCoverage, StoreError> {
        let frontier = self
            .binary_last_computed
            .read()
            .map_err(|_| StoreError::LockError)?;
        Ok(BinaryFlatCoverage::from_marker(Some(frontier.as_slice())))
    }

    /// Whether the binary flat mirror's backfill has finished, read from the
    /// **durable** marker rather than the in-memory frontier — the same choice
    /// [`Self::flatkeyvalue_fully_generated`] makes, and for the same reason:
    /// the in-memory copy lags by design.
    ///
    /// Gates journal-backed deep reorgs, which the mirror cannot survive
    /// mid-sweep: entries journaled while the generator runs omit pre-images
    /// for keys past the frontier.
    pub fn binary_flat_fully_generated(&self) -> Result<bool, StoreError> {
        let tx = self.backend.begin_read()?;
        let marker = tx.get(MISC_VALUES, BINARY_LAST_WRITTEN_KEY)?;
        Ok(BinaryFlatCoverage::from_marker(marker.as_deref()).is_complete())
    }

    /// Whether the binary mirror's backfill is still outstanding on a chain
    /// that has one — the gate a deep reorg must not cross.
    ///
    /// **Conditional on the schedule, and that is load-bearing.** An
    /// unscheduled chain has no binary trie and never writes the frontier
    /// marker, so an unconditional completeness check would read as "still
    /// generating" for ever and defer every deep reorg on every existing MPT
    /// chain. The binary hazard only exists where the binary mirror does.
    pub fn binary_flat_generation_pending(&self) -> Result<bool, StoreError> {
        if !self.get_chain_config().binary_tree_scheduled() {
            return Ok(false);
        }
        Ok(!self.binary_flat_fully_generated()?)
    }

    /// The root to gate a binary read at `parent_hash` on: the parent's
    /// *header* state root, because that is what its diff layer is keyed by and
    /// what [`PendingTrieRoots`] tracks.
    ///
    /// This is the one place the two roots genuinely diverge. Before activation
    /// a block's header carries an MPT root while its binary root lives only in
    /// `BINARY_TRIE_ROOTS`, so gating on the binary root would wait for nothing
    /// and could snapshot a cache that does not yet hold the parent's layer —
    /// the reader would then fall through to disk, where the parent's binary
    /// nodes are not. After activation the two coincide and this is a no-op.
    ///
    /// Falls back to `binary_root` when the parent header is unknown (genesis
    /// on a fresh store, tests driving the storage layer directly): there is no
    /// layer to wait for in that case.
    fn binary_layer_gate(
        &self,
        parent_hash: BlockHash,
        binary_root: H256,
    ) -> Result<H256, StoreError> {
        Ok(self
            .get_block_header_by_hash(parent_hash)?
            .map(|header| header.state_root)
            .unwrap_or(binary_root))
    }

    /// Open the persistent binary trie at `root` for reading.
    ///
    /// Opening loads nothing: the root is a bare stored reference until a
    /// traversal touches it, so this costs a cache snapshot and a backend
    /// handle. Reads cascade diff layers -> disk, so a block whose nodes are
    /// still staged is readable at its own root.
    ///
    /// The gate is `root` itself, which is correct because every caller is a
    /// read at an *active* header, where the header state root and the binary
    /// root are the same value.
    fn open_binary_trie(&self, root: H256) -> Result<BinaryTrie, StoreError> {
        Ok(BinaryTrie::open(
            Box::new(self.layered_binary_trie_db(
                root,
                root,
                LayeredBinaryTrieDB::staging_buffer(),
            )?),
            root,
        ))
    }

    /// Root-addressed account read against the binary trie: the counterpart of
    /// [`Self::get_account_state_by_root`] for a header past the binary-tree
    /// activation, whose `state_root` addresses the binary trie and no MPT.
    ///
    /// Returns an [`AccountInfo`] rather than an `AccountState` because the
    /// binary trie has no storage root to put in one — storage is not a
    /// per-account subtrie there. The caller that needs an `AccountState` wants
    /// [`Self::get_binary_account`], which also asks whether the account holds
    /// any storage, and decides what to put in the field; see `StoreVmDatabase`
    /// in `crates/blockchain/vm.rs`.
    ///
    /// Deliberately unchecked, exactly as [`Self::get_account_state_by_root`]
    /// is: `StoreVmDatabase::new` gates on [`Self::has_binary_trie_state`] once
    /// up front.
    pub fn get_binary_account_info(
        &self,
        state_root: H256,
        address: Address,
    ) -> Result<Option<AccountInfo>, StoreError> {
        let mut trie = self.open_binary_trie(state_root)?;
        Ok(pbt_state::get_account_info(&mut trie, address)?)
    }

    /// [`Self::get_binary_account_info`] plus the "does this account hold any
    /// storage" answer, for the caller that has to fill an MPT-shaped
    /// `AccountState`.
    ///
    /// A separate method rather than more work inside
    /// [`Self::get_binary_account_info`] because the two callers want different
    /// things. The account RPCs want the balance, the nonce and the code hash
    /// and nothing else, and would pay two extra trie walks per call for a
    /// field they never read; execution needs the storage answer for EIP-7610.
    ///
    /// Both come out of one open trie, so the header-storage half of the
    /// storage question re-walks nodes the account read has already loaded —
    /// see `pbt_state::get_account`.
    pub fn get_binary_account(
        &self,
        state_root: H256,
        address: Address,
    ) -> Result<Option<BinaryAccount>, StoreError> {
        let mut trie = self.open_binary_trie(state_root)?;
        Ok(pbt_state::get_account(&mut trie, address)?)
    }

    /// Root-addressed storage read against the binary trie, the counterpart of
    /// [`Self::get_storage_at_root`].
    ///
    /// No account lookup precedes it: the slot's key is derived from the
    /// address and the slot alone, so unlike the MPT there is no storage root
    /// to fetch first and no second trie to open.
    pub fn get_binary_storage_slot(
        &self,
        state_root: H256,
        address: Address,
        storage_key: H256,
    ) -> Result<Option<U256>, StoreError> {
        let mut trie = self.open_binary_trie(state_root)?;
        Ok(pbt_state::get_storage_slot(
            &mut trie,
            address,
            &storage_key,
        )?)
    }

    /// Whether this node holds the binary-trie state `state_root` names for
    /// `block_hash` — the binary-trie counterpart of [`Self::has_state_root`],
    /// and the gate `StoreVmDatabase::new` runs before executing on an active
    /// header.
    ///
    /// **Why it takes a block hash and not just a root.** [`Self::has_state_root`]
    /// can verify an MPT root against the nodes on disk: it reads the root node
    /// by path and hashes it. The same trick is not available here —
    /// [`BinaryTrie::open`] records the root it was given and caches it as the
    /// hash of whatever node it later resolves at that path, so a wrong root is
    /// indistinguishable from a right one through the public API, and this crate
    /// does not reach inside `ethrex_binary_trie` to hash nodes itself. What the
    /// node does know is which binary root it computed for which block, which is
    /// exactly [`BINARY_TRIE_ROOTS`]. An active header whose own recorded root
    /// matches the one it commits to is a header this node has binary state for.
    ///
    /// **What used to sit behind this, and no longer does.** The write path
    /// once committed straight to disk, so after a reorg the path-keyed,
    /// single-version node table held the abandoned branch's state while both
    /// branches' roots stayed recorded, and this check could not tell them
    /// apart. Diff layers removed the cause: a reorg within the layer window
    /// discards the abandoned branch's nodes before they are ever written, so
    /// the recorded root and the nodes agree again
    /// (see [`Self::advance_binary_trie_for_block`]).
    ///
    /// A reorg *deeper* than the layer cache — where the abandoned branch's
    /// nodes were already flushed — is now covered too: `STATE_HISTORY` carries
    /// a binary reverse-diff, so the deep-reorg overlay puts the on-disk trie
    /// back at the pivot before any new-chain block executes. The presence check
    /// below reads through that overlay like any other reader, so it answers for
    /// the unwound trie rather than the abandoned one.
    ///
    /// **What remains**: the journal does not cover full sync, which writes one
    /// layer per ~1024 blocks and skips journaling entirely, so a reorg into a
    /// batch-imported range is out of reach on both tries alike.
    ///
    /// [`BINARY_TRIE_ROOTS`]: crate::api::tables::BINARY_TRIE_ROOTS
    /// **Two questions, both required.** The mapping answers "is `state_root`
    /// the root this block committed", and [`Self::binary_trie_holds_root`]
    /// answers "does this node still hold that trie". The mapping alone is not
    /// enough, and believing it caused a devnet node to wedge permanently:
    /// `advance_binary_trie_for_block` writes the mapping row durably at import
    /// while the nodes behind it are only staged into the in-memory diff layer,
    /// and `Store::shutdown` deliberately leaves those layers in memory. After
    /// a restart the row survives for every block the node ever executed and
    /// the nodes do not, so a bookkeeping-only check claims state that cannot
    /// be read. The node then resumes on absent state, serves genesis values
    /// for every post-activation block and never recovers — strictly worse than
    /// re-executing, which is what it did when the check was merely pessimistic.
    pub fn has_binary_trie_state(
        &self,
        block_hash: BlockHash,
        state_root: H256,
    ) -> Result<bool, StoreError> {
        if self.get_binary_trie_root(block_hash)? != Some(state_root) {
            return Ok(false);
        }
        self.binary_trie_holds_root(state_root)
    }

    /// Whether the binary trie this node can read really resolves to `root`.
    ///
    /// The binary counterpart of [`Self::trie_holds_state_root`], and it exists
    /// for the same reason: binary-trie nodes are keyed by path, not by hash,
    /// so opening a trie at a root records the request without validating it.
    /// Reading the node at the root path and re-hashing it is what turns that
    /// into a real answer.
    ///
    /// The read goes through the layered DB, so it cascades cache-then-disk
    /// exactly as the MPT check does: state staged in a diff layer counts as
    /// held for a running node, and only what reached disk counts for one that
    /// has just restarted.
    fn binary_trie_holds_root(&self, root: H256) -> Result<bool, StoreError> {
        if root == BINARY_EMPTY_TRIE_ROOT {
            return Ok(true);
        }
        let db = self.layered_binary_trie_db(root, root, LayeredBinaryTrieDB::staging_buffer())?;
        let Some(encoded) = db
            .get(&BitPath::new())
            .map_err(|e| StoreError::Custom(format!("binary root node read failed: {e}")))?
        else {
            return Ok(false);
        };
        Ok(hash_stored_node(&encoded) == root)
    }

    /// The binary-trie root recorded for `block_hash`, if any.
    ///
    /// See [`BINARY_TRIE_ROOTS`] for why this mapping exists and how
    /// narrow its scope is: block import consults it to find the root a
    /// block must extend from, and nothing else ever reads it.
    ///
    /// [`BINARY_TRIE_ROOTS`]: crate::api::tables::BINARY_TRIE_ROOTS
    pub fn get_binary_trie_root(&self, block_hash: BlockHash) -> Result<Option<H256>, StoreError> {
        let Some(raw) = self.read(BINARY_TRIE_ROOTS, block_hash.as_bytes().to_vec())? else {
            return Ok(None);
        };
        if raw.len() != 32 {
            return Err(StoreError::Custom(format!(
                "malformed binary-trie root recorded for block {block_hash:#x}: {} bytes",
                raw.len()
            )));
        }
        Ok(Some(H256::from_slice(&raw)))
    }

    /// Record `root` as `block_hash`'s binary-trie root.
    pub fn set_binary_trie_root(
        &self,
        block_hash: BlockHash,
        root: H256,
    ) -> Result<(), StoreError> {
        self.write(
            BINARY_TRIE_ROOTS,
            block_hash.as_bytes().to_vec(),
            root.as_bytes().to_vec(),
        )
    }

    /// Forget the binary-trie root recorded for `block_hash`.
    ///
    /// Used when a block is rejected *after* its binary root was
    /// computed. From activation onward the root has to exist before the
    /// header can be validated against it, so a header carrying the
    /// wrong root would otherwise leave a recorded root behind for a
    /// block that never entered the chain — and a later block naming it
    /// as parent would then extend a branch nobody accepted.
    ///
    /// Only the mapping needs undoing. The nodes the rejected block produced
    /// were staged into a [`BinaryTrieAdvance`] the caller is about to drop,
    /// never written, so there is nothing on disk to undo — which is exactly
    /// what the diff layers bought. Removing the mapping is still required:
    /// it is the only thing a later block consults.
    pub fn remove_binary_trie_root(&self, block_hash: BlockHash) -> Result<(), StoreError> {
        self.delete(BINARY_TRIE_ROOTS, block_hash.as_bytes().to_vec())
    }

    /// Shadow-tracking step: advance the binary trie by one block and
    /// record where it landed.
    ///
    /// Looks up `parent_hash`'s recorded binary root, applies
    /// `account_updates` on top of it, and records the resulting root
    /// under `block_hash`. Callers gate this on
    /// [`ChainConfig::binary_tree_scheduled`]; on a chain that never
    /// schedules the commitment it is not called at all, so an
    /// unscheduled node pays nothing — no lookup, no write, no column
    /// family touched.
    ///
    /// **Cost.** On a scheduled chain this roughly doubles the per-block
    /// state-write work for the whole pre-activation period: every
    /// account and slot a block touches is written into two tries rather
    /// than one. That is the price of carry-over — the first active
    /// block commits the *full* state, so the binary trie has to have
    /// been kept current all along. The alternative is a stop-the-world
    /// conversion at the flip, which does not scale and which EIP-8297
    /// does not specify.
    ///
    /// **Not a validation step.** The returned root is recorded and
    /// nothing more: no header carries it and nothing is compared
    /// against it until activation.
    ///
    /// **Staged, not written.** The nodes this produces go into the returned
    /// [`BinaryTrieAdvance`], which the caller carries in the block's
    /// [`UpdateBatch`]; from there they join the same diff layer as the block's
    /// MPT nodes and reach disk only when that layer clears the safe-commit
    /// gate, in the same write batch. Three things follow, and they are the
    /// point of Phase E:
    ///
    /// - the two tries are persisted atomically, so disk never holds the MPT at
    ///   block N alongside the binary trie at block N-5;
    /// - a reorg discards the abandoned branch's binary nodes with its layer,
    ///   instead of stranding them on a path-keyed store with no other version
    ///   to fall back to;
    /// - competing blocks at one height cannot overwrite each other at shared
    ///   paths, because neither has written anything.
    ///
    /// Reads during the advance cascade through the layer chain, so a parent
    /// whose own nodes are still staged resolves correctly.
    ///
    /// The recorded root *is* written immediately, because it is what a later
    /// block consults to find the root it must extend, and because a rejected
    /// block un-records it ([`Self::remove_binary_trie_root`]).
    ///
    /// A reorg *deeper* than the layer window is covered by the other half of
    /// the same machinery: when the layer does flush, `commit_to_disk` records a
    /// reverse diff of these nodes in `STATE_HISTORY` alongside the MPT's, and
    /// the deep-reorg overlay replays it to put the on-disk trie back at the
    /// pivot. See the module docs in `layering.rs`.
    ///
    /// # Errors
    ///
    /// [`StoreError::MissingBinaryTrieRoot`] when the parent has no
    /// recorded root. That is a hard error by design: it means the
    /// shadow trie has a gap, and quietly restarting from an empty trie
    /// would defer the failure to the flip block, where it is
    /// unrecoverable.
    ///
    /// [`ChainConfig::binary_tree_scheduled`]: ethrex_common::types::ChainConfig::binary_tree_scheduled
    pub fn advance_binary_trie_for_block(
        &self,
        block_hash: BlockHash,
        parent_hash: BlockHash,
        account_updates: &[AccountUpdate],
    ) -> Result<BinaryTrieAdvance, StoreError> {
        let parent_root = self
            .get_binary_trie_root(parent_hash)?
            .ok_or(StoreError::MissingBinaryTrieRoot { parent_hash })?;

        // Codes first and in a single batch, exactly as the direct-write path
        // does: the EVM fetches whole bytecode by hash and only `ACCOUNT_CODES`
        // answers that, while the trie only ever commits to code as chunks.
        // Content-addressed, so writing them ahead of the layer flush is safe —
        // an abandoned branch leaves unreferenced code, never wrong code.
        self.write_account_codes(account_updates)?;

        let staged = LayeredBinaryTrieDB::staging_buffer();
        let gate_root = self.binary_layer_gate(parent_hash, parent_root)?;
        let db = self.layered_binary_trie_db(parent_root, gate_root, staged.clone())?;

        // The mapping row says which root this block must extend. It does not
        // say the trie still *holds* it — the row is durable and the nodes are
        // not, so a trie advanced past that root (or parked elsewhere by a
        // snapshot install) still answers the lookup above. `BinaryTrie::open`
        // records a root without validating it, and a path-keyed walk then
        // resolves whatever is on disk, so without this the commit would be a
        // root computed over the wrong base with nothing downstream able to
        // tell.
        //
        // Checked through `db` rather than via `binary_trie_holds_root`: that
        // helper gates its layer read on the root itself, but for a
        // pre-activation block the layer is keyed by the parent's *header*
        // root, which is an MPT root. `binary_layer_gate` above is what
        // resolves that, so the check has to run against this db to avoid
        // spuriously refusing a live import.
        if parent_root != BINARY_EMPTY_TRIE_ROOT {
            let holds = db
                .get(&BitPath::new())
                .map_err(|e| StoreError::Custom(format!("binary root node read failed: {e}")))?
                .is_some_and(|encoded| hash_stored_node(&encoded) == parent_root);
            if !holds {
                return Err(StoreError::BinaryTrieRootNotHeld {
                    parent_hash,
                    parent_root,
                });
            }
        }

        let mut trie = BinaryTrie::open(Box::new(db), parent_root);
        apply_account_updates(&mut trie, account_updates)?;
        let committed = trie.commit()?;
        let root = committed.root;
        // The leaf changelog is the mirror's only input: every path that can
        // retire a leaf (`remove_prefix` above all) reports through it, so a
        // future caller cannot add a mutation path that bypasses the mirror.
        let flat = flat_writes_from_changelog(committed.leaves);
        drop(trie);

        let nodes = std::mem::take(&mut *staged.lock().map_err(|_| StoreError::LockError)?);
        self.set_binary_trie_root(block_hash, root)?;
        Ok(BinaryTrieAdvance {
            root,
            parent_root,
            nodes,
            flat,
        })
    }

    /// Write every code carried by `account_updates` into `ACCOUNT_CODES` and
    /// `ACCOUNT_CODE_METADATA`, in one batch. Shared by the direct-write and
    /// staged binary-trie paths.
    fn write_account_codes(&self, account_updates: &[AccountUpdate]) -> Result<(), StoreError> {
        let codes: Vec<_> = account_updates
            .iter()
            .filter_map(|update| update.code.as_ref())
            .collect();
        if codes.is_empty() {
            return Ok(());
        }
        let mut tx = self.backend.begin_write()?;
        for code in codes {
            let hash_key = code.hash.0.to_vec();
            tx.put(ACCOUNT_CODES, &hash_key, &encode_code(code))?;
            tx.put(
                ACCOUNT_CODE_METADATA,
                &hash_key,
                &(code.len() as u64).to_be_bytes(),
            )?;
        }
        tx.commit()
    }

    // Key format: block_number (8 bytes, big-endian) + block_hash (32 bytes)
    fn make_witness_key(block_number: u64, block_hash: &BlockHash) -> Vec<u8> {
        let mut composite_key = Vec::with_capacity(8 + 32);
        composite_key.extend_from_slice(&block_number.to_be_bytes());
        composite_key.extend_from_slice(block_hash.as_bytes());
        composite_key
    }

    /// Stores a pre-serialized execution witness for a block.
    ///
    /// The witness is converted to RPC format (RpcExecutionWitness) before storage
    /// to avoid expensive `encode_subtrie` traversal on every read. This pre-computes
    /// the serialization at write time instead of read time.
    pub fn store_witness(
        &self,
        block_hash: BlockHash,
        block_number: u64,
        witness: ExecutionWitness,
    ) -> Result<(), StoreError> {
        // Convert to RPC format once at storage time
        let rpc_witness = RpcExecutionWitness::try_from(witness)?;
        let key = Self::make_witness_key(block_number, &block_hash);
        let value = serde_json::to_vec(&rpc_witness)?;
        self.write(EXECUTION_WITNESSES, key, value)?;
        // Clean up old witnesses (keep only last 128)
        self.cleanup_old_witnesses(block_number)
    }

    fn cleanup_old_witnesses(&self, latest_block_number: u64) -> Result<(), StoreError> {
        // If we have less than 128 blocks, no cleanup needed
        if latest_block_number <= MAX_WITNESSES {
            return Ok(());
        }

        let threshold = latest_block_number - MAX_WITNESSES;

        if let Some(oldest_block_number) = self.get_oldest_witness_number()? {
            let prefix = oldest_block_number.to_be_bytes();
            let mut to_delete = Vec::new();

            {
                let read_txn = self.backend.begin_read()?;
                let iter = read_txn.prefix_iterator(EXECUTION_WITNESSES, &prefix)?;

                // We may have multiple witnesses for the same block number (forks)
                for item in iter {
                    let (key, _value) = item?;
                    let mut block_number_bytes = [0u8; 8];
                    block_number_bytes.copy_from_slice(&key[0..8]);
                    let block_number = u64::from_be_bytes(block_number_bytes);
                    if block_number > threshold {
                        break;
                    }
                    to_delete.push(key.to_vec());
                }
            }

            for key in to_delete {
                self.delete(EXECUTION_WITNESSES, key)?;
            }
        };

        self.update_oldest_witness_number(threshold + 1)?;

        Ok(())
    }

    fn update_oldest_witness_number(&self, oldest_block_number: u64) -> Result<(), StoreError> {
        self.write(
            MISC_VALUES,
            b"oldest_witness_block_number".to_vec(),
            oldest_block_number.to_le_bytes().to_vec(),
        )?;
        Ok(())
    }

    fn get_oldest_witness_number(&self) -> Result<Option<u64>, StoreError> {
        let Some(value) = self.read(MISC_VALUES, b"oldest_witness_block_number".to_vec())? else {
            return Ok(None);
        };

        let array: [u8; 8] = value.as_slice().try_into().map_err(|_| {
            StoreError::Custom("Invalid oldest witness block number bytes".to_string())
        })?;
        Ok(Some(u64::from_le_bytes(array)))
    }

    /// Returns the raw JSON bytes of a cached witness for a block.
    ///
    /// This is the most efficient method for the RPC handler since it avoids
    /// deserialization and re-serialization. The bytes can be parsed directly
    /// as a JSON Value for the RPC response.
    pub fn get_witness_json_bytes(
        &self,
        block_number: u64,
        block_hash: BlockHash,
    ) -> Result<Option<Vec<u8>>, StoreError> {
        let key = Self::make_witness_key(block_number, &block_hash);
        self.read(EXECUTION_WITNESSES, key)
    }

    /// Returns the deserialized RpcExecutionWitness for a block.
    ///
    /// Prefer `get_witness_json_bytes` when you need to return the witness
    /// as JSON (e.g., for RPC responses) to avoid re-serialization.
    pub fn get_witness_by_number_and_hash(
        &self,
        block_number: u64,
        block_hash: BlockHash,
    ) -> Result<Option<RpcExecutionWitness>, StoreError> {
        let key = Self::make_witness_key(block_number, &block_hash);
        match self.read(EXECUTION_WITNESSES, key)? {
            Some(value) => {
                let witness: RpcExecutionWitness = serde_json::from_slice(&value)?;
                Ok(Some(witness))
            }
            None => Ok(None),
        }
    }

    /// Stores a block access list for a given block hash.
    pub fn store_block_access_list(
        &self,
        block_hash: BlockHash,
        bal: &BlockAccessList,
    ) -> Result<(), StoreError> {
        let key = block_hash.as_bytes().to_vec();
        let mut value = vec![];
        bal.encode(&mut value);
        self.write(BLOCK_ACCESS_LISTS, key, value)
    }

    /// Returns the block access list for a given block hash, if stored.
    pub fn get_block_access_list(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockAccessList>, StoreError> {
        let key = block_hash.as_bytes().to_vec();
        match self.read(BLOCK_ACCESS_LISTS, key)? {
            Some(value) => {
                let bal = BlockAccessList::decode(&value)
                    .map_err(|e| StoreError::Custom(format!("Failed to decode BAL: {e}")))?;
                Ok(Some(bal))
            }
            None => Ok(None),
        }
    }

    pub async fn add_initial_state(&mut self, genesis: Genesis) -> Result<(), StoreError> {
        self.add_initial_state_inner(genesis, false).await
    }

    /// Like [`Store::add_initial_state`], but trusts a pre-existing datadir's
    /// state instead of validating it against the provided genesis. If a genesis
    /// header is already stored, it is kept as-is rather than recomputing the
    /// genesis state root from `genesis.alloc` and rejecting on mismatch. The
    /// chain config from the genesis file is still applied either way.
    ///
    /// Intended for booting a datadir produced out-of-band (e.g. by a state
    /// generator that writes the state trie directly and emits a genesis file
    /// with an empty `alloc`), where the operator vouches for the stored state
    /// root. Has no effect on a fresh datadir: the genesis is built normally.
    pub async fn add_initial_state_skip_validation(
        &mut self,
        genesis: Genesis,
    ) -> Result<(), StoreError> {
        self.add_initial_state_inner(genesis, true).await
    }

    async fn add_initial_state_inner(
        &mut self,
        genesis: Genesis,
        skip_genesis_validation: bool,
    ) -> Result<(), StoreError> {
        debug!("Storing initial state from genesis");

        // Obtain genesis block
        let genesis_block = genesis.get_block();
        let genesis_block_number = genesis_block.header.number;

        let genesis_hash = genesis_block.hash();

        let stored_genesis_header = self.load_block_header(genesis_block_number)?;

        // Always set the chain config from the genesis file. The in-memory
        // `chain_config` starts at `Default::default()` on every boot and is
        // not reloaded from the datadir, so skipping this would leave the store
        // with the wrong chainId and an empty fork schedule. Skip-validation
        // only waives the genesis state-root/header check; the `config` section
        // of the genesis file is still authoritative and must be applied.
        self.set_chain_config(&genesis.config).await?;

        // The cache can't be empty. Clamp the head to the durable block: after a
        // crash, `LatestBlockNumber` can be ahead of `flushed_upto` (FCU writes the
        // head synchronously while block bodies are buffered), so loading the raw
        // latest header would brick boot when its body was never flushed.
        if let Some(latest) = self.load_latest_block_number().await? {
            self.anchor_to_durable_head(latest).await?;
        }

        match stored_genesis_header {
            Some(header) if skip_genesis_validation => {
                info!(
                    stored_genesis = %header.hash(),
                    "Skipping genesis state validation; trusting the genesis header and state already stored in the datadir"
                );
                return Ok(());
            }
            Some(header) if header.hash() == genesis_hash => {
                info!("Received genesis file matching a previously stored one, nothing to do");
                return Ok(());
            }
            Some(_) => {
                error!(
                    "The chain configuration stored in the database is incompatible with the provided configuration. If you intended to switch networks, choose another datadir or clear the database (e.g., run `ethrex removedb`) and try again."
                );
                return Err(StoreError::IncompatibleChainConfig);
            }
            None => {
                self.add_block_header(genesis_hash, genesis_block.header.clone())
                    .await?
            }
        }
        // Store genesis accounts
        // TODO: Should we use this root instead of computing it before the block hash check?
        //
        // On a chain with `binaryTreeTime` scheduled the same alloc also seeds
        // the EIP-8297 binary trie, so shadow tracking has an anchor to extend
        // from: block 1 looks up genesis' recorded binary root and builds on
        // the alloc, exactly as it builds on the alloc in the MPT. Genesis
        // itself normally still commits the MPT root in its header — activation
        // at genesis would change the genesis hash and hence the chain's
        // identity, which is why the schedule is always in the future on any
        // real network.
        //
        // The alloc is cloned only when scheduled; unscheduled chains keep the
        // move into `setup_genesis_state_trie` and do no binary-trie work.
        let binary_tree_alloc = genesis
            .config
            .binary_tree_scheduled()
            .then(|| genesis.alloc.clone());
        // Per-header rule, at genesis: the header commits whichever root is
        // active at *genesis' own timestamp*. Under the degenerate, test-only
        // genesis activation that is the binary root, and the MPT root computed
        // here is then a shadow root that no header names — so it is only
        // compared against the header when the header is actually MPT-committed.
        let binary_tree_active_at_genesis = genesis.config.is_binary_tree_active(genesis.timestamp);
        let genesis_state_root = self.setup_genesis_state_trie(genesis.alloc).await?;
        debug_assert!(
            binary_tree_active_at_genesis || genesis_state_root == genesis_block.header.state_root
        );

        // Reached only on a fresh datadir: the branches above return early when
        // a genesis header is already stored. That is the right behaviour for a
        // reopen — the binary trie is persistent, so a restarted node resumes
        // from what is on disk with no replay. It also means enabling the
        // schedule on a datadir that was synced without it does not silently
        // half-seed: genesis is skipped, and the next block fails loudly with
        // `MissingBinaryTrieRoot` naming its parent, which is accurate — the
        // shadow trie cannot be built without reprocessing from genesis.
        if let Some(alloc) = binary_tree_alloc {
            let genesis_binary_root = self.setup_genesis_binary_trie(alloc).await?;
            self.set_binary_trie_root(genesis_hash, genesis_binary_root)?;
            info!(
                genesis_binary_root = %genesis_binary_root,
                "Seeded the EIP-8297 binary trie from the genesis alloc"
            );
        }

        // Store genesis block
        info!(hash = %genesis_hash, "Storing genesis block");

        self.add_block(genesis_block).await?;
        self.update_earliest_block_number(genesis_block_number)
            .await?;
        self.forkchoice_update(vec![], genesis_block_number, genesis_hash, None, None)
            .await?;
        Ok(())
    }

    pub async fn load_initial_state(&self) -> Result<(), StoreError> {
        info!("Loading initial state from DB");
        let Some(latest) = self.load_latest_block_number().await? else {
            return Err(StoreError::MissingLatestBlockNumber);
        };
        // Use the same durable-head clamp as the node boot path so export and the
        // running node agree on the head. The persisted head is only rewritten when
        // it actually moved, so a plain export run does not mutate `CHAIN_DATA`.
        self.anchor_to_durable_head(latest).await?;
        Ok(())
    }

    /// Serves `eth_getStorageAt`. Guarded: a known block whose state this node no
    /// longer holds is [`StoreError::MissingStateRoot`], not a successful read of
    /// the on-disk trie's current slot value.
    ///
    /// Resolves against whichever trie *this block's header* addresses — see
    /// [`Self::header_addresses_binary_trie`].
    ///
    /// The binary side does no account lookup first, and does not need one: the
    /// slot's leaf key is derived from the address and the slot alone, so there
    /// is no storage root to fetch and no second trie to open (see
    /// [`Self::get_binary_storage_slot`]). The two paths still agree on the
    /// answer for a nonexistent account, because an account with no state has no
    /// slot leaves either, so both report `None`.
    pub fn get_storage_at(
        &self,
        block_number: BlockNumber,
        address: Address,
        storage_key: H256,
    ) -> Result<Option<U256>, StoreError> {
        let Some(header) = self.get_block_header(block_number)? else {
            return Ok(None);
        };
        if self.header_addresses_binary_trie(&header) {
            self.ensure_binary_trie_state(header.hash(), &header)?;
            return self.get_binary_storage_slot(header.state_root, address, storage_key);
        }
        self.get_storage_at_root_inner(
            header.state_root,
            address,
            storage_key,
            RootCheck::Verify(Some(header.number)),
        )
    }

    /// Root-addressed storage read.
    ///
    /// Deliberately unchecked, for store-internal callers that address state by
    /// root directly and expect a root with no diff layer to fall through to disk
    /// (the deep-reorg overlay tests read at roots the overlay does not serve, and
    /// assert exactly that). [`Self::get_storage_at`], the block-addressed entry
    /// point the RPC layer uses, is the guarded one.
    pub fn get_storage_at_root(
        &self,
        state_root: H256,
        address: Address,
        storage_key: H256,
    ) -> Result<Option<U256>, StoreError> {
        self.get_storage_at_root_inner(state_root, address, storage_key, RootCheck::Skip)
    }

    fn get_storage_at_root_inner(
        &self,
        state_root: H256,
        address: Address,
        storage_key: H256,
        check: RootCheck,
    ) -> Result<Option<U256>, StoreError> {
        let account_hash = hash_address_fixed(&address);

        // Pre-acquire shared resources once for both trie opens
        let read_view = self.backend.begin_read()?;
        let cache = self.gated_snapshot(state_root)?;
        let last_written = self.last_written()?;
        // While a deep-reorg overlay serves this root, flat-KV reads must
        // go through the trie: journal entries written while the FKV generator was
        // running lack pre-images for keys past the generator frontier, so disk
        // flat-KV may hold the generator's stale values. `TrieWrapper` already
        // forces the trie-node read path in this window; mirror the gate here so
        // the EMPTY_TRIE_HASH shortcut doesn't bypass it.
        let use_fkv = Self::flatkeyvalue_computed_with_last_written(account_hash, &last_written)
            && !cache.overlay_serves(state_root);

        let storage_root = if use_fkv {
            // The flat-KV fast path reads the account by path and never touches
            // the state root, so a guarded read has to check it explicitly.
            // Opening a state trie here is wrapper construction only — the read
            // view, layer snapshot and FKV cursor it needs are already in hand —
            // so the guard costs one root-node read plus one keccak, not a second
            // trie setup.
            if let RootCheck::Verify(block) = check {
                let state_trie = self.open_state_trie_shared(
                    state_root,
                    read_view.clone(),
                    cache.clone(),
                    last_written.clone(),
                )?;
                Self::ensure_trie_holds_state_root(&state_trie, state_root, block)?;
            }
            // We will use FKVs, we don't need the root
            EMPTY_TRIE_HASH
        } else {
            let state_trie = self.open_state_trie_shared(
                state_root,
                read_view.clone(),
                cache.clone(),
                last_written.clone(),
            )?;
            // Runs against the trie already opened for the account lookup, so the
            // guard adds no second open here either.
            if let RootCheck::Verify(block) = check {
                Self::ensure_trie_holds_state_root(&state_trie, state_root, block)?;
            }
            let Some(encoded_account) = state_trie.get(account_hash.as_bytes())? else {
                return Ok(None);
            };
            let account = AccountState::decode(&encoded_account)?;
            account.storage_root
        };
        let storage_trie = self.open_storage_trie_shared(
            account_hash,
            state_root,
            storage_root,
            read_view,
            cache,
            last_written,
        )?;

        let hashed_key = hash_key_fixed(&storage_key);
        storage_trie
            .get(&hashed_key)?
            .map(|rlp| U256::decode(&rlp).map_err(StoreError::RLPDecode))
            .transpose()
    }

    /// Gets storage value when the account hash and storage root are already known.
    ///
    /// This skips the state-trie account lookup and account RLP decode done by
    /// [`Self::get_storage_at_root`], and directly opens the account storage trie.
    pub fn get_storage_at_root_with_known_storage_root(
        &self,
        state_root: H256,
        account_hash: H256,
        storage_root: H256,
        storage_key: H256,
    ) -> Result<Option<U256>, StoreError> {
        let read_view = self.backend.begin_read()?;
        let cache = self.gated_snapshot(state_root)?;
        let last_written = self.last_written()?;
        // When FKV is active the real storage root is in the flatkeyvalue store,
        // not in the account's RLP-encoded storage_root field. Use EMPTY_TRIE_HASH
        // so open_storage_trie_shared falls through to the FKV path. While a
        // deep-reorg overlay serves this root, keep the real root instead:
        // disk flat-KV may hold stale generator values, so reads must go through
        // the trie (see `TrieWrapper::flatkeyvalue_computed`).
        let storage_root =
            if Self::flatkeyvalue_computed_with_last_written(account_hash, &last_written)
                && !cache.overlay_serves(state_root)
            {
                EMPTY_TRIE_HASH
            } else {
                storage_root
            };
        let storage_trie = self.open_storage_trie_shared(
            account_hash,
            state_root,
            storage_root,
            read_view,
            cache,
            last_written,
        )?;

        let hashed_key = hash_key_fixed(&storage_key);
        storage_trie
            .get(&hashed_key)?
            .map(|rlp| U256::decode(&rlp).map_err(StoreError::RLPDecode))
            .transpose()
    }

    pub fn get_chain_config(&self) -> ChainConfig {
        self.chain_config
    }

    pub async fn get_latest_canonical_block_hash(&self) -> Result<Option<BlockHash>, StoreError> {
        Ok(Some(self.latest_block_header.get().hash()))
    }

    /// Updates the canonical chain.
    /// Inserts new canonical blocks, removes blocks beyond the new head,
    /// and updates the head, safe, and finalized block pointers.
    /// All operations are performed in a single database transaction.
    pub async fn forkchoice_update(
        &self,
        new_canonical_blocks: Vec<(BlockNumber, BlockHash)>,
        head_number: BlockNumber,
        head_hash: BlockHash,
        safe: Option<BlockNumber>,
        finalized: Option<BlockNumber>,
    ) -> Result<(), StoreError> {
        // Serialize concurrent forkchoice updates. Without this, two callers
        // could interleave their `latest_block_header` cache updates with each
        // other's DB writes, leaving the cache inconsistent with the DB or
        // letting a later caller's write reorder relative to the cache update
        // order (see the TOCTOU discussion around canonical/latest drift).
        let _guard = self.fcu_lock.lock().await;

        // Updates first the latest_block_header to avoid nonce inconsistencies #3927.
        // Snapshot the previous header so we can roll the cache back if the DB
        // write fails — otherwise the cache would point at a block the DB does
        // not consider canonical.
        let previous_head = self.latest_block_header.get();
        let new_head = self
            .get_block_header_by_hash(head_hash)?
            .ok_or_else(|| StoreError::MissingLatestBlockNumber)?;
        self.latest_block_header.update(new_head);
        if let Err(err) = self
            .forkchoice_update_inner(
                new_canonical_blocks,
                head_number,
                head_hash,
                safe,
                finalized,
            )
            .await
        {
            self.latest_block_header.update((*previous_head).clone());
            return Err(err);
        }

        // Refresh the canonical safe-commit root now that the canonical tables reflect the new
        // head. `None` (chain shorter than the threshold, e.g. genesis init at head 0) leaves the
        // cell unchanged so genesis-on-disk is never gated away.
        // No `latest_block_header` rollback on error here (unlike the `inner` failure above): the
        // only error is a poisoned safe-commit `RwLock`, which is an unrecoverable process state.
        if let Some(root) = self.compute_safe_commit_root(head_number)? {
            // Advancing the cell alone does not flush: the commit step (Phase 2) only runs
            // while blocks execute, so an execute-all-then-one-forkchoice flow (e.g. block
            // import) would accumulate every layer and never persist. When the safe-commit
            // root advances, poke the worker to flush the now-committable backlog up to it.
            if self.set_safe_commit_root(root)? {
                let tx = self.persist_tx.clone();
                let _ = tokio::task::spawn_blocking(move || tx.send(PersistMessage::Commit(root)))
                    .await;
            }
        }

        Ok(())
    }

    /// Updates the dedicated safe-commit-root cell with the given state root,
    /// returning `true` if the cell changed (so callers can skip a redundant flush).
    ///
    /// This is a plain synchronous function; it touches only the dedicated cell
    /// and is disjoint from the trie-cache Arc (no cache clone or replacement).
    /// Crate-private: only `forkchoice_update` (post-canonicalization) may set it,
    /// preserving the invariant that the cell only ever holds a canonical state root.
    pub(crate) fn set_safe_commit_root(&self, root: H256) -> Result<bool, StoreError> {
        let mut guard = self
            .safe_commit_root
            .write()
            .map_err(|_| StoreError::LockError)?;
        let changed = *guard != root;
        *guard = root;
        Ok(changed)
    }

    /// Computes the canonical safe-commit state root: the state root of the canonical block
    /// `commit_threshold` layers below `head`.
    ///
    /// Returns `Ok(None)` when the chain is shorter than the threshold (underflow), or when the
    /// target block is not yet canonical / its header is absent. Synchronous getters only; no
    /// await and no lock guard held across one. The threshold is read from the trie cache to
    /// avoid duplicating the IN_MEMORY/DB selection that `from_backend` already made.
    /// Crate-private: only `forkchoice_update` consumes it.
    pub(crate) fn compute_safe_commit_root(
        &self,
        head: BlockNumber,
    ) -> Result<Option<H256>, StoreError> {
        let commit_threshold = self
            .trie_cache
            .read()
            .map_err(|_| StoreError::LockError)?
            .commit_threshold;
        let Some(target) = head.checked_sub(commit_threshold as u64) else {
            return Ok(None);
        };
        let Some(hash) = self.get_canonical_block_hash_sync(target)? else {
            return Ok(None);
        };
        let Some(header) = self.get_block_header_by_hash(hash)? else {
            return Ok(None);
        };
        Ok(Some(header.state_root))
    }

    /// Obtain the storage trie for the given block.
    ///
    /// Unchecked: the returned trie will happily read through to the on-disk
    /// trie even when this node no longer holds the block's state (see
    /// [`Self::trie_holds_state_root`]). Kept unchecked for the block-execution
    /// and L2 callers that open a parent's trie to build state on top of it, and
    /// for snap sync, where the pivot's state is incomplete by construction.
    /// Read paths that answer user queries must use [`Self::state_trie_checked`].
    pub fn state_trie(&self, block_hash: BlockHash) -> Result<Option<Trie>, StoreError> {
        let Some(header) = self.get_block_header_by_hash(block_hash)? else {
            return Ok(None);
        };
        Ok(Some(self.open_state_trie(header.state_root)?))
    }

    /// [`Self::state_trie`], but refusing to serve a block whose state this node
    /// no longer holds.
    ///
    /// `Ok(None)` still means "no such block". A known block whose state has
    /// fallen out of the retention window (or belongs to an abandoned fork) is
    /// now [`StoreError::MissingStateRoot`] instead of a successful read of some
    /// other block's state.
    ///
    /// Costs one root-node read and one keccak over the happy path — no extra
    /// header lookup and no extra trie open, since the check runs against the
    /// trie this function had to open anyway.
    fn state_trie_checked(&self, block_hash: BlockHash) -> Result<Option<Trie>, StoreError> {
        let Some(header) = self.get_block_header_by_hash(block_hash)? else {
            return Ok(None);
        };
        Ok(Some(self.state_trie_checked_for_header(&header)?))
    }

    /// [`Self::state_trie_checked`] for a header the caller already has, so the
    /// state-reading RPCs can decide which trie a header addresses (see
    /// [`Self::header_addresses_binary_trie`]) without reading the header twice.
    fn state_trie_checked_for_header(&self, header: &BlockHeader) -> Result<Trie, StoreError> {
        let trie = self.open_state_trie(header.state_root)?;
        Self::ensure_trie_holds_state_root(&trie, header.state_root, Some(header.number))?;
        Ok(trie)
    }

    /// The per-header trie question: does *this header's* `state_root` address
    /// the EIP-8297 binary trie, or the MPT?
    ///
    /// **Per header, never per chain**, and this is the whole rule the state
    /// RPCs turn on. It mirrors `StoreVmDatabase::open` (`crates/blockchain/vm.rs`),
    /// which decides the same thing the same way for block execution; the two
    /// must not drift, or a block would execute against one trie and be queried
    /// against the other.
    ///
    /// A header from before the activation genuinely carries an MPT root and has
    /// to keep resolving against the MPT forever — after the flip, across
    /// restarts, and on either side of a reorg. Asking a chain-level question
    /// instead (`binary_tree_scheduled()`, "have we passed it") makes the whole
    /// pre-flip history unqueryable, which is what
    /// `state_rpc_reads_at_pre_flip_blocks_keep_using_the_mpt_after_the_flip`
    /// (in `test/tests/blockchain/binary_tree_shadow_tests.rs`) falsifies.
    ///
    /// Nothing maps a block hash to an MPT root here or anywhere else: each
    /// header names the trie that answers for it through `header.state_root`
    /// alone.
    fn header_addresses_binary_trie(&self, header: &BlockHeader) -> bool {
        self.chain_config.is_binary_tree_active(header.timestamp)
    }

    /// The binary-trie counterpart of [`Self::ensure_trie_holds_state_root`]:
    /// refuse a header whose binary state this node does not hold.
    ///
    /// Without it the binary reads below would answer from whatever the
    /// single-version binary trie currently holds — the same silently-wrong
    /// answer the MPT guard was added to stop, just on the other trie. What
    /// "holding it" means, and the reorg case it cannot yet see, are documented
    /// on [`Self::has_binary_trie_state`].
    ///
    /// The error is [`StoreError::MissingStateRoot`], identical in shape to the
    /// MPT side, so a caller cannot tell (and does not need to tell) which trie
    /// was missing.
    fn ensure_binary_trie_state(
        &self,
        block_hash: BlockHash,
        header: &BlockHeader,
    ) -> Result<(), StoreError> {
        if self.has_binary_trie_state(block_hash, header.state_root)? {
            Ok(())
        } else {
            Err(StoreError::MissingStateRoot {
                block: Some(header.number),
                state_root: header.state_root,
            })
        }
    }

    /// Obtain the storage trie for the given account on the given block
    pub fn storage_trie(
        &self,
        block_hash: BlockHash,
        address: Address,
    ) -> Result<Option<Trie>, StoreError> {
        let Some(header) = self.get_block_header_by_hash(block_hash)? else {
            return Ok(None);
        };
        // Fetch Account from state_trie
        let Some(state_trie) = self.state_trie(block_hash)? else {
            return Ok(None);
        };
        let hashed_address = hash_address_fixed(&address);
        let Some(encoded_account) = state_trie.get(hashed_address.as_bytes())? else {
            return Ok(None);
        };
        let account = AccountState::decode(&encoded_account)?;
        // Open storage_trie
        let storage_root = account.storage_root;
        Ok(Some(self.open_storage_trie(
            hashed_address,
            header.state_root,
            storage_root,
        )?))
    }

    /// Block-addressed account read. See [`Self::state_trie_checked`] for the
    /// behaviour at a block whose state this node no longer holds.
    ///
    /// **MPT only, deliberately.** It returns an `AccountState`, whose
    /// `storage_root` the binary trie cannot report at all (see
    /// `StoreVmDatabase::account_state_from_binary_trie` in
    /// `crates/blockchain/vm.rs`), so past the activation this fails loudly with
    /// [`StoreError::MissingStateRoot`] rather than answering with a field that
    /// says nothing — execution can accept the empty root there because it asks
    /// the storage question separately through `VmDatabase::has_storage`, and a
    /// caller of this method has no such second channel. Nothing on the RPC path
    /// uses it:
    /// the account RPCs go through [`Self::get_account_info_by_hash`], which is
    /// `AccountInfo`-shaped and therefore has no such field to invent.
    pub async fn get_account_state(
        &self,
        block_number: BlockNumber,
        address: Address,
    ) -> Result<Option<AccountState>, StoreError> {
        let Some(block_hash) = self.get_canonical_block_hash(block_number).await? else {
            return Ok(None);
        };
        let Some(state_trie) = self.state_trie_checked(block_hash)? else {
            return Ok(None);
        };
        self.get_account_state_from_trie(&state_trie, address)
    }

    /// Root-addressed account read.
    ///
    /// Deliberately unchecked. This is the per-account read `StoreVmDatabase`
    /// issues for every account an execution touches, and that caller has already
    /// gated on [`Self::has_state_root`] once at `StoreVmDatabase::new`
    /// (`crates/blockchain/vm.rs`). Verifying here would add a root-node read and
    /// a keccak to every account access during block execution to re-prove
    /// something established before the first one. Callers that address state by
    /// block should use [`Self::get_account_state`], which is guarded.
    pub fn get_account_state_by_root(
        &self,
        state_root: H256,
        address: Address,
    ) -> Result<Option<AccountState>, StoreError> {
        let state_trie = self.open_state_trie(state_root)?;
        self.get_account_state_from_trie(&state_trie, address)
    }

    pub fn get_account_state_from_trie(
        &self,
        state_trie: &Trie,
        address: Address,
    ) -> Result<Option<AccountState>, StoreError> {
        let hashed_address = hash_address_fixed(&address);
        let Some(encoded_state) = state_trie.get(hashed_address.as_bytes())? else {
            return Ok(None);
        };
        Ok(Some(AccountState::decode(&encoded_state)?))
    }

    /// Batch lookup of account states by address against a given state root.
    ///
    /// Fast path: for addresses whose hashed path falls within the FKV cursor
    /// (and which are not present in the in-memory diff-layer cache), values
    /// are fetched in a single `multi_get` on `ACCOUNT_FLATKEYVALUE`. Other
    /// addresses fall back to per-address trie walks.
    ///
    /// Results are returned in the same order as the input addresses.
    pub fn get_account_states_batch_by_root(
        &self,
        state_root: H256,
        addresses: &[Address],
    ) -> Result<Vec<Option<AccountState>>, StoreError> {
        if addresses.is_empty() {
            return Ok(Vec::new());
        }

        let last_written = self.last_written()?;
        let trie_cache = self
            .trie_cache
            .read()
            .map_err(|_| StoreError::LockError)?
            .clone();

        let mut results: Vec<Option<AccountState>> = vec![None; addresses.len()];
        // Per-address leaf paths (nibbles + leaf flag). Length 65.
        let leaf_paths: Vec<Vec<u8>> = addresses
            .iter()
            .map(|addr| {
                let hashed = hash_address_fixed(addr);
                Nibbles::from_bytes(hashed.as_bytes()).into_vec()
            })
            .collect();

        let mut fkv_indices: Vec<usize> = Vec::new();
        let mut trie_indices: Vec<usize> = Vec::new();

        // Match `BackendTrieDB::flatkeyvalue_computed` semantics: a path is
        // covered by FKV iff `last_written >= path` as raw nibble bytes. This
        // is the same check `Trie::get` uses; the related helper
        // `Store::flatkeyvalue_computed_with_last_written` slices `[0..64]`
        // and is intentionally more conservative — using that here would
        // unnecessarily fall back to the trie when the cursor sits inside an
        // account's storage sweep (the account leaf is already in FKV at that
        // point; see `flatkeyvalue_generator`).
        //
        // While a deep-reorg overlay serves this root, skip the FKV fast
        // path entirely: this function never consults the overlay, and disk
        // flat-KV may hold values the generator computed against the chain being
        // reorged away (journal entries written during generation lack
        // past-frontier flat-KV pre-images). The per-address trie fallback goes
        // through `TrieWrapper`, which routes reads through the overlay.
        let overlay_active = trie_cache.overlay_serves(state_root);
        let fkv_cursor: &[u8] = last_written.as_slice();
        for (i, path) in leaf_paths.iter().enumerate() {
            if let Some(value) = trie_cache.get(state_root, path.as_slice()) {
                if !value.is_empty() {
                    results[i] = Some(AccountState::decode(&value)?);
                }
                continue;
            }
            if !overlay_active && fkv_cursor >= path.as_slice() {
                fkv_indices.push(i);
            } else {
                trie_indices.push(i);
            }
        }

        if !fkv_indices.is_empty() {
            let read_view = self.backend.begin_read()?;
            let keys: Vec<&[u8]> = fkv_indices
                .iter()
                .map(|&i| leaf_paths[i].as_slice())
                .collect();
            let raw = read_view.multi_get(ACCOUNT_FLATKEYVALUE, &keys);
            for (slot, res) in fkv_indices.iter().zip(raw.into_iter()) {
                let Some(encoded) = res? else { continue };
                if encoded.is_empty() {
                    continue;
                }
                results[*slot] = Some(AccountState::decode(&encoded)?);
            }
        }

        if !trie_indices.is_empty() {
            // Fall back to the regular trie path for any addresses whose path
            // hasn't been swept by the FKV generator yet. Parallelized to
            // recover the per-address fan-out the pre-batch `par_iter` path
            // had, which matters during initial sync when most addresses
            // miss FKV.
            let state_trie = self.open_state_trie(state_root)?;
            let fetched: Result<Vec<(usize, Option<AccountState>)>, StoreError> = trie_indices
                .par_iter()
                .map(|&i| {
                    self.get_account_state_from_trie(&state_trie, addresses[i])
                        .map(|s| (i, s))
                })
                .collect();
            for (i, s) in fetched? {
                results[i] = s;
            }
        }

        Ok(results)
    }

    /// Constructs a merkle proof for the given account address against a given state.
    /// If storage_keys are provided, also constructs the storage proofs for those keys.
    ///
    /// Returns `None` if the state trie is missing, otherwise returns the proof.
    pub async fn get_account_proof(
        &self,
        state_root: H256,
        address: Address,
        storage_keys: &[H256],
    ) -> Result<Option<AccountProof>, StoreError> {
        let state_trie = self.open_state_trie(state_root)?;
        // Without this the response contradicts itself at a root this node does
        // not hold: `Trie::get_proof` uses the *checked* node accessor and bails
        // to an empty proof, while the account lookup beside it goes through the
        // unchecked `Trie::get` and returns the on-disk trie's current account.
        // `eth_getProof` would then answer with a live-looking account and no
        // proof at all. Blocked at the source instead.
        //
        // The block number is not reported here because this method is addressed
        // by root; the state root in the message identifies it.
        Self::ensure_trie_holds_state_root(&state_trie, state_root, None)?;
        let address_path = hash_address_fixed(&address);
        let proof = state_trie.get_proof(address_path.as_bytes())?;
        let account_opt = state_trie
            .get(address_path.as_bytes())?
            .map(|encoded_state| AccountState::decode(&encoded_state))
            .transpose()?;

        let mut storage_proof = Vec::with_capacity(storage_keys.len());

        if let Some(account) = &account_opt {
            let storage_trie =
                self.open_storage_trie(address_path, state_root, account.storage_root)?;

            for key in storage_keys {
                let hashed_key = hash_key(key);
                let proof = storage_trie.get_proof(&hashed_key)?;
                let value = storage_trie
                    .get(&hashed_key)?
                    .map(|rlp| U256::decode(&rlp).map_err(StoreError::RLPDecode))
                    .transpose()?
                    .unwrap_or_default();

                let slot_proof = StorageSlotProof {
                    proof,
                    key: *key,
                    value,
                };
                storage_proof.push(slot_proof);
            }
        } else {
            storage_proof.extend(storage_keys.iter().map(|key| StorageSlotProof {
                proof: Vec::new(),
                key: *key,
                value: U256::zero(),
            }));
        }
        let account = account_opt.unwrap_or_default();
        let account_proof = AccountProof {
            proof,
            account,
            storage_proof,
        };
        Ok(Some(account_proof))
    }

    // Returns an iterator across all accounts in the state trie given by the state_root
    // Does not check that the state_root is valid
    pub fn iter_accounts_from(
        &self,
        state_root: H256,
        starting_address: H256,
    ) -> Result<impl Iterator<Item = (H256, AccountState)>, StoreError> {
        let mut iter = self.open_locked_state_trie(state_root)?.into_iter();
        iter.advance(starting_address.0.to_vec())?;
        Ok(iter.content().map_while(|(path, value)| {
            Some((H256::from_slice(&path), AccountState::decode(&value).ok()?))
        }))
    }

    // Returns an iterator across all accounts in the state trie given by the state_root
    // Does not check that the state_root is valid
    pub fn iter_accounts(
        &self,
        state_root: H256,
    ) -> Result<impl Iterator<Item = (H256, AccountState)>, StoreError> {
        self.iter_accounts_from(state_root, H256::zero())
    }

    // Returns an iterator across all accounts in the state trie given by the state_root
    // Does not check that the state_root is valid
    pub fn iter_storage_from(
        &self,
        state_root: H256,
        hashed_address: H256,
        starting_slot: H256,
    ) -> Result<Option<impl Iterator<Item = (H256, U256)>>, StoreError> {
        let state_trie = self.open_locked_state_trie(state_root)?;
        let Some(account_rlp) = state_trie.get(hashed_address.as_bytes())? else {
            return Ok(None);
        };
        let storage_root = AccountState::decode(&account_rlp)?.storage_root;
        let mut iter = self
            .open_locked_storage_trie(hashed_address, state_root, storage_root)?
            .into_iter();
        iter.advance(starting_slot.0.to_vec())?;
        Ok(Some(iter.content().map_while(|(path, value)| {
            Some((H256::from_slice(&path), U256::decode(&value).ok()?))
        })))
    }

    // Returns an iterator across all accounts in the state trie given by the state_root
    // Does not check that the state_root is valid
    pub fn iter_storage(
        &self,
        state_root: H256,
        hashed_address: H256,
    ) -> Result<Option<impl Iterator<Item = (H256, U256)>>, StoreError> {
        self.iter_storage_from(state_root, hashed_address, H256::zero())
    }

    pub fn get_account_range_proof(
        &self,
        state_root: H256,
        starting_hash: H256,
        last_hash: Option<H256>,
    ) -> Result<Vec<Vec<u8>>, StoreError> {
        let state_trie = self.open_state_trie(state_root)?;
        let mut proof = state_trie.get_proof(starting_hash.as_bytes())?;
        if let Some(last_hash) = last_hash {
            proof.extend_from_slice(&state_trie.get_proof(last_hash.as_bytes())?);
        }
        Ok(proof)
    }

    pub fn get_storage_range_proof(
        &self,
        state_root: H256,
        hashed_address: H256,
        starting_hash: H256,
        last_hash: Option<H256>,
    ) -> Result<Option<Vec<Vec<u8>>>, StoreError> {
        let state_trie = self.open_state_trie(state_root)?;
        let Some(account_rlp) = state_trie.get(hashed_address.as_bytes())? else {
            return Ok(None);
        };
        let storage_root = AccountState::decode(&account_rlp)?.storage_root;
        let storage_trie = self.open_storage_trie(hashed_address, state_root, storage_root)?;
        let mut proof = storage_trie.get_proof(starting_hash.as_bytes())?;
        if let Some(last_hash) = last_hash {
            proof.extend_from_slice(&storage_trie.get_proof(last_hash.as_bytes())?);
        }
        Ok(Some(proof))
    }

    /// Receives the root of the state trie and a list of paths where the first path will correspond to a path in the state trie
    /// (aka a hashed account address) and the following paths will be paths in the account's storage trie (aka hashed storage keys)
    /// If only one hash (account) is received, then the state trie node containing the account will be returned.
    /// If more than one hash is received, then the storage trie nodes where each storage key is stored will be returned
    /// For more information check out snap capability message [`GetTrieNodes`](https://github.com/ethereum/devp2p/blob/master/caps/snap.md#gettrienodes-0x06)
    /// The paths can be either full paths (hash) or partial paths (compact-encoded nibbles), if a partial path is given for the account this method will not return storage nodes for it
    pub fn get_trie_nodes(
        &self,
        state_root: H256,
        paths: Vec<Vec<u8>>,
        byte_limit: u64,
    ) -> Result<Vec<Vec<u8>>, StoreError> {
        let Some(account_path) = paths.first() else {
            return Ok(vec![]);
        };
        let state_trie = self.open_state_trie(state_root)?;
        // State Trie Nodes Request
        if paths.len() == 1 {
            // Fetch state trie node
            let node = state_trie.get_node(account_path)?;
            return Ok(vec![node]);
        }
        // Storage Trie Nodes Request
        let Some(account_state) = state_trie
            .get(account_path)?
            .map(|ref rlp| AccountState::decode(rlp))
            .transpose()?
        else {
            return Ok(vec![]);
        };
        // We can't access the storage trie without the account's address hash
        let Ok(hashed_address) = account_path.clone().try_into().map(H256) else {
            return Ok(vec![]);
        };
        let storage_trie =
            self.open_storage_trie(hashed_address, state_root, account_state.storage_root)?;
        // Fetch storage trie nodes
        let mut nodes = vec![];
        let mut bytes_used = 0;
        // The number of sub-paths is bounded upstream by the snap server's per-request lookup
        // cap (`MAX_SERVE_LOOKUPS`), which truncates the pathset before this call; here the
        // byte budget only bounds response size.
        for path in paths.iter().skip(1) {
            if bytes_used >= byte_limit {
                break;
            }
            let node = storage_trie.get_node(path)?;
            bytes_used += node.len() as u64;
            nodes.push(node);
        }
        Ok(nodes)
    }

    // Methods exclusive for trie management during snap-syncing

    /// Snapshot the trie layer cache for reading at `state_root`, blocking until
    /// that root's diff layer has been installed if it is still in-flight (see
    /// [`PendingTrieRoots`]). This is the read barrier for deferred layer builds:
    /// taking it at trie-open time guarantees the snapshot contains the layer, so
    /// a just-added block's state is never read as stale. Roots that are not
    /// pending (already installed, historical/committed, genesis) never block.
    fn gated_snapshot(&self, state_root: H256) -> Result<Arc<TrieLayerCache>, StoreError> {
        self.pending_trie_roots.wait_until_ready(state_root)?;
        Ok(self
            .trie_cache
            .read()
            .map_err(|_| StoreError::LockError)?
            .clone())
    }

    /// Obtain a state trie from the given state root
    /// Doesn't check if the state root is valid
    /// Used for internal store operations
    pub fn open_state_trie(&self, state_root: H256) -> Result<Trie, StoreError> {
        let trie_db = TrieWrapper::new(
            state_root,
            self.gated_snapshot(state_root)?,
            Box::new(BackendTrieDB::new_for_accounts(
                self.backend.clone(),
                self.last_written()?,
            )?),
            None,
        );
        Ok(Trie::open(Box::new(trie_db), state_root))
    }

    /// Obtain a state trie from the given state root
    /// Doesn't check if the state root is valid
    /// Used for internal store operations
    pub fn open_direct_state_trie(&self, state_root: H256) -> Result<Trie, StoreError> {
        Ok(Trie::open(
            Box::new(BackendTrieDB::new_for_accounts(
                self.backend.clone(),
                self.last_written()?,
            )?),
            state_root,
        ))
    }

    /// Obtain a state trie locked for reads from the given state root
    /// Doesn't check if the state root is valid
    /// Used for internal store operations
    pub fn open_locked_state_trie(&self, state_root: H256) -> Result<Trie, StoreError> {
        let trie_db = TrieWrapper::new(
            state_root,
            self.gated_snapshot(state_root)?,
            Box::new(state_trie_locked_backend(
                self.backend.as_ref(),
                self.last_written()?,
            )?),
            None,
        );
        Ok(Trie::open(Box::new(trie_db), state_root))
    }

    /// Obtain a storage trie from the given address and storage_root.
    /// Doesn't check if the account is stored
    pub fn open_storage_trie(
        &self,
        account_hash: H256,
        state_root: H256,
        storage_root: H256,
    ) -> Result<Trie, StoreError> {
        let trie_db = TrieWrapper::new(
            state_root,
            self.gated_snapshot(state_root)?,
            Box::new(BackendTrieDB::new_for_storages(
                self.backend.clone(),
                self.last_written()?,
            )?),
            Some(account_hash),
        );
        Ok(Trie::open(Box::new(trie_db), storage_root))
    }

    /// Open a state trie using pre-acquired shared resources.
    /// Avoids redundant RwLock acquisitions when multiple tries are opened
    /// in the same operation (e.g., state trie + storage trie in get_storage_at_root).
    fn open_state_trie_shared(
        &self,
        state_root: H256,
        read_view: Arc<dyn StorageReadView>,
        cache: Arc<TrieLayerCache>,
        last_written: Vec<u8>,
    ) -> Result<Trie, StoreError> {
        let trie_db = TrieWrapper::new(
            state_root,
            cache,
            Box::new(BackendTrieDB::new_for_accounts_with_view(
                self.backend.clone(),
                read_view,
                last_written,
            )?),
            None,
        );
        Ok(Trie::open(Box::new(trie_db), state_root))
    }

    /// Open a storage trie using pre-acquired shared resources.
    fn open_storage_trie_shared(
        &self,
        account_hash: H256,
        state_root: H256,
        storage_root: H256,
        read_view: Arc<dyn StorageReadView>,
        cache: Arc<TrieLayerCache>,
        last_written: Vec<u8>,
    ) -> Result<Trie, StoreError> {
        let trie_db = TrieWrapper::new(
            state_root,
            cache,
            Box::new(BackendTrieDB::new_for_storages_with_view(
                self.backend.clone(),
                read_view,
                last_written,
            )?),
            Some(account_hash),
        );
        Ok(Trie::open(Box::new(trie_db), storage_root))
    }

    /// Obtain a storage trie from the given address and storage_root.
    /// Doesn't check if the account is stored
    pub fn open_direct_storage_trie(
        &self,
        account_hash: H256,
        storage_root: H256,
    ) -> Result<Trie, StoreError> {
        Ok(Trie::open(
            Box::new(BackendTrieDB::new_for_account_storage(
                self.backend.clone(),
                account_hash,
                self.last_written()?,
            )?),
            storage_root,
        ))
    }

    /// Obtain a read-locked storage trie from the given address and storage_root.
    /// Doesn't check if the account is stored
    pub fn open_locked_storage_trie(
        &self,
        account_hash: H256,
        state_root: H256,
        storage_root: H256,
    ) -> Result<Trie, StoreError> {
        let trie_db = TrieWrapper::new(
            state_root,
            self.gated_snapshot(state_root)?,
            Box::new(state_trie_locked_backend(
                self.backend.as_ref(),
                self.last_written()?,
            )?),
            Some(account_hash),
        );
        Ok(Trie::open(Box::new(trie_db), storage_root))
    }

    pub fn has_state_root(&self, state_root: H256) -> Result<bool, StoreError> {
        // Empty state trie is always available
        if state_root == EMPTY_TRIE_HASH {
            return Ok(true);
        }
        let trie = self.open_state_trie(state_root)?;
        Self::trie_holds_state_root(&trie, state_root)
    }

    /// Does this node hold the state `header` commits to?
    ///
    /// The header-addressed form of [`Self::has_state_root`], and the one any
    /// caller holding a header should reach for. A bare `state_root` cannot
    /// answer this question on a scheduled chain: from the activation timestamp
    /// onwards a header's `state_root` is a binary-trie root that resolves
    /// against no MPT node, so [`Self::has_state_root`] reports `false` for
    /// state the node holds perfectly well. Only the header carries the
    /// timestamp that says which trie is being named — the same per-header rule
    /// block execution and the state RPCs turn on (see
    /// [`Self::header_addresses_binary_trie`]).
    ///
    /// `block_hash` must be `header`'s own hash; the binary side is keyed by it
    /// because the binary trie is single-version and its recorded root is what
    /// distinguishes "this block's state" from whatever the trie holds now.
    ///
    /// Forkchoice is why this exists: its reachability gate ran on the bare root
    /// and so refused every post-activation head, halting a devnet at the flip
    /// block with the block itself executing and validating cleanly
    /// (`forkchoice_accepts_every_block_across_the_flip` in
    /// `test/tests/blockchain/binary_tree_shadow_tests.rs`).
    pub fn has_state_for_header(
        &self,
        block_hash: BlockHash,
        header: &BlockHeader,
    ) -> Result<bool, StoreError> {
        if self.header_addresses_binary_trie(header) {
            return self.has_binary_trie_state(block_hash, header.state_root);
        }
        self.has_state_root(header.state_root)
    }

    /// Walking back from `head_block_number`, the highest block whose post-state
    /// this node holds — the point a restart can resume execution from.
    ///
    /// `Ok(None)` means the walk reached genesis without finding held state,
    /// which callers report as an unrecoverable database. In practice genesis
    /// state is always present, so a walk that reaches block 0 and stops there
    /// returns `Ok(Some(0))` and the caller replays the whole chain.
    ///
    /// Lives here because there were two hand-maintained copies of this walk —
    /// `ethrex::initializers::regenerate_head_state` and the L2 committer's
    /// `find_last_known_state_root` — with the same structure and the same
    /// error text. Both asked [`Self::has_state_root`] about a header, and both
    /// had to be fixed by hand when that turned out to be wrong past
    /// `binaryTreeTime`. One copy means one place to get it right.
    pub fn last_block_with_state(
        &self,
        head_block_number: BlockNumber,
    ) -> Result<Option<BlockNumber>, StoreError> {
        let Some(mut header) = self.get_block_header(head_block_number)? else {
            return Ok(None);
        };

        while !self.has_state_for_header(header.hash(), &header)? {
            if header.number == 0 {
                return Ok(None);
            }
            let parent_number = header.number - 1;
            // One line per step, so a walk that descends further than expected
            // is diagnosable from a log rather than only from its aftermath.
            debug!("State for block {} not held; walking back", header.number);
            let Some(parent) = self.get_block_header(parent_number)? else {
                return Err(StoreError::Custom(format!(
                    "parent header for block {parent_number} not found"
                )));
            };
            header = parent;
        }

        Ok(Some(header.number))
    }

    /// Whether `trie`, already opened at `state_root`, really resolves to that
    /// root on this node.
    ///
    /// This is the body of [`Self::has_state_root`] with the trie open factored
    /// out, so read paths that have *already* opened the state trie can run the
    /// check without paying for a second open — a second layer-cache snapshot, a
    /// second `last_written` read and a second backend read view. What is left is
    /// one read of the root node plus one keccak over it.
    ///
    /// The check is necessary because trie nodes are keyed by path, not by hash:
    /// `Trie::open` records the requested root without validating it, and
    /// `Trie::get` re-reads the root by path and discards that hash. An unheld
    /// root therefore reads whatever the on-disk trie's current root is, and
    /// answers from the wrong version of state instead of failing.
    fn trie_holds_state_root(trie: &Trie, state_root: H256) -> Result<bool, StoreError> {
        // Empty state trie is always available
        if state_root == EMPTY_TRIE_HASH {
            return Ok(true);
        }
        // NOTE: here we hash the root because the trie doesn't check the state root is correct
        let Some(root) = trie.db().get(Nibbles::default())? else {
            return Ok(false);
        };
        let root_hash = ethrex_trie::Node::decode(&root)?
            .compute_hash(&NativeCrypto)
            .finalize(&NativeCrypto);
        Ok(state_root == root_hash)
    }

    /// [`Self::trie_holds_state_root`] as a guard.
    ///
    /// Fails with [`StoreError::MissingStateRoot`] when this node does not hold
    /// the state behind `state_root`, mirroring what `StoreVmDatabase::new`
    /// already reports for `eth_call` at the same block, so a single node cannot
    /// answer "what was the balance at block N" and "run this call at block N"
    /// two different ways.
    fn ensure_trie_holds_state_root(
        trie: &Trie,
        state_root: H256,
        block: Option<BlockNumber>,
    ) -> Result<(), StoreError> {
        if Self::trie_holds_state_root(trie, state_root)? {
            Ok(())
        } else {
            Err(StoreError::MissingStateRoot { block, state_root })
        }
    }

    // ===========================================================================
    // Deep-reorg primitives (storage side).
    // ===========================================================================

    /// Returns `true` if the in-memory layer cache currently has a layer with
    /// the given `state_root`. Used by the deep-reorg orchestrator to decide
    /// whether the head's state can be reached through forward execution (cache
    /// hit) or whether a deep-reorg path with overlay construction is required.
    pub fn is_state_in_layer_cache(&self, state_root: H256) -> Result<bool, StoreError> {
        let trie = self
            .trie_cache
            .read()
            .map_err(|_| StoreError::LockError)?
            .clone();
        Ok(trie.contains(state_root))
    }

    /// Returns the highest block number with a `STATE_HISTORY` entry; the cache
    /// edge `D` (the deepest block whose post-state is on disk). Returns `None`
    /// if the journal is empty (no commits since boot, or fully pruned by finality).
    ///
    /// O(1) via reverse seek on the column family's last key.
    pub fn highest_state_history_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        let read = self.backend.begin_read()?;
        let Some(key) = read.last_key(STATE_HISTORY)? else {
            return Ok(None);
        };
        let arr = <[u8; 8]>::try_from(key.as_slice()).map_err(|_| {
            StoreError::Custom(format!(
                "STATE_HISTORY key has unexpected length: {}",
                key.len()
            ))
        })?;
        Ok(Some(BlockNumber::from_be_bytes(arr)))
    }

    /// Returns the lowest block number with a `STATE_HISTORY` entry. Returns `None`
    /// if the journal is empty (no commits since boot, or fully pruned by finality).
    ///
    /// O(1) via forward seek on the column family's first key.
    pub fn lowest_state_history_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        let read = self.backend.begin_read()?;
        let Some(key) = read.first_key(STATE_HISTORY)? else {
            return Ok(None);
        };
        let arr = <[u8; 8]>::try_from(key.as_slice()).map_err(|_| {
            StoreError::Custom(format!(
                "STATE_HISTORY key has unexpected length: {}",
                key.len()
            ))
        })?;
        Ok(Some(BlockNumber::from_be_bytes(arr)))
    }

    /// The in-memory diff-layer retention (the layer cache's commit threshold):
    /// the deepest reorg the node can serve straight from the layer cache, with no
    /// journal/overlay reconstruction. RocksDB default 128, in-memory 10000. Used by
    /// `compute_reorg_ceiling` as the physical floor when there is no finality signal.
    pub fn reorg_retention(&self) -> Result<u64, StoreError> {
        let cache = self.trie_cache.read().map_err(|_| StoreError::LockError)?;
        Ok(cache.commit_threshold() as u64)
    }

    /// Test-only: inserts a pre-encoded `STATE_HISTORY` entry at the given block
    /// number. Lets integration tests seed the journal without running enough
    /// commits to trip the in-memory cache's flush threshold.
    #[doc(hidden)]
    pub fn put_state_history_entry_for_test(
        &self,
        block_number: BlockNumber,
        encoded: &[u8],
    ) -> Result<(), StoreError> {
        let mut tx = self.backend.begin_write()?;
        tx.put(STATE_HISTORY, &block_number.to_be_bytes(), encoded)?;
        tx.commit()?;
        Ok(())
    }

    /// Atomically prepares the store for a deep-reorg apply pass.
    ///
    /// Builds an [`Overlay`] from journal entries in `[to_block, from_block]`
    /// (inclusive both ends), verifies each entry's `block_hash` against
    /// `expected_hash`, then swaps the in-memory layer cache for a fresh one
    /// with the overlay installed. After this call:
    ///
    /// - The layer cache contains zero forward layers.
    /// - The overlay is in place; subsequent `TrieWrapper::get` calls cascade
    ///   layer cache -> overlay -> disk.
    /// - The on-disk trie/flat-KV state is unchanged (still at the OLD chain's
    ///   edge `D`).
    ///
    /// Side-chain blocks `[pivot+1 .. new_head]` should now be executed in
    /// chain order through the normal `Blockchain::add_block` path; each
    /// block's reads cascade through the overlay, and each block's commit
    /// produces a new forward layer.
    ///
    /// Errors abort the swap: if overlay construction fails (missing entry,
    /// hash mismatch, decode error), the existing layer cache is left intact.
    pub fn install_overlay_for_reorg(
        &self,
        from_block: BlockNumber,
        to_block: BlockNumber,
        expected_hash: impl Fn(BlockNumber) -> Option<H256>,
    ) -> Result<(), StoreError> {
        // Build the overlay first so any failure aborts before mutating the cache.
        let overlay =
            Overlay::from_journal(self.backend.as_ref(), from_block, to_block, expected_hash)
                .map_err(|e| StoreError::Custom(format!("overlay construction failed: {e}")))?;

        let threshold = {
            let current = self.trie_cache.read().map_err(|_| StoreError::LockError)?;
            current.commit_threshold()
        };
        // Share the Store's safe-commit-root cell so the canonical commit gate keeps
        // working after the cache swap (the cell only advances via forkchoice).
        let mut fresh =
            TrieLayerCache::new_with_safe_commit(threshold, self.safe_commit_root.clone());
        fresh.set_overlay(Arc::new(overlay));

        // The overlay bridges `BINARY_TRIE_NODES` as well as the four MPT/flat-KV
        // CFs, so a scheduled chain's on-disk binary trie is unwound to the pivot
        // along with its MPT and both are folded into the same reconciliation
        // write. A zero `serves_binary_root` on a scheduled chain means the
        // journal entry at the pivot's height carried no binary root, which
        // should be impossible — shadow tracking runs from genesis on a
        // scheduled chain — so it is reported rather than papered over: the
        // replay would fall through to disk and read the abandoned branch.
        if self.chain_config.binary_tree_scheduled()
            && let Some(overlay) = fresh.overlay()
            && overlay.serves_binary_root().is_zero()
        {
            warn!(
                from_block,
                to_block,
                "deep reorg on a binary-tree-scheduled chain: the journal entry at the pivot \
                 records no binary root, so the on-disk binary trie will NOT be unwound and may \
                 hold the abandoned chain's state"
            );
        }

        // Wait for the persist worker to be idle before swapping the cache (see
        // [`Self::rendezvous_persist_worker`]).
        self.rendezvous_persist_worker("install_overlay_for_reorg")?;

        // The fresh cache starts with no layers, so its binary-root index and
        // binary bloom start empty too: the abandoned chain's staged binary
        // nodes go away with its MPT layers, in one step, by construction.
        // Anything the abandoned chain already *flushed* is handled by the
        // overlay's binary reverse-diff, not here.
        let mut guard = self.trie_cache.write().map_err(|_| StoreError::LockError)?;
        *guard = Arc::new(fresh);
        Ok(())
    }

    /// Waits for the persist worker to be idle before a layer-cache swap. That
    /// worker owns the trie-layer install (`apply_trie_phase1`, run from the
    /// `PersistMessage::Block` handler): it reads `trie_cache`, mutates a local
    /// clone, and RCU-writes it back; if it is mid-flight when we swap, its
    /// write-back can clobber the freshly swapped cache (e.g. drop a just-installed
    /// overlay, or install a side-chain layer over a base that no longer has the
    /// overlay underneath). `PersistMessage::Ping` carries an ack channel and the
    /// worker is FIFO, so its ack proves every earlier `Block` (and thus every
    /// earlier trie install) is fully processed and the worker is back at
    /// `rx.recv()`. This is the synchronous core of [`wait_for_persistence_idle`];
    /// we inline it here because callers are not async. The caller's subsequent
    /// `trie_cache.write()` serialising any future RCU makes the swap safe.
    fn rendezvous_persist_worker(&self, caller: &str) -> Result<(), StoreError> {
        let (ack_tx, ack_rx) = sync_channel::<Result<(), StoreError>>(1);
        self.persist_tx
            .send(PersistMessage::Ping(ack_tx))
            .map_err(|e| {
                StoreError::Custom(format!("{caller}: failed to ping persist worker: {e}"))
            })?;
        ack_rx.recv().map_err(|e| {
            StoreError::Custom(format!("{caller}: persist worker ping ack failed: {e}"))
        })??;
        Ok(())
    }

    /// Returns `(entry_count, byte_size)` of the currently installed overlay, or
    /// `(0, 0)` when no overlay is installed. Used by the observability layer in
    /// `fork_choice.rs` immediately after `install_overlay_for_reorg` to emit
    /// `ethrex_reorg_overlay_entries` / `ethrex_reorg_overlay_bytes`.
    pub fn reorg_overlay_size_hint(&self) -> Result<(usize, usize), StoreError> {
        let guard = self.trie_cache.read().map_err(|_| StoreError::LockError)?;
        match guard.overlay() {
            Some(ov) => Ok((ov.len(), ov.byte_size())),
            None => Ok((0, 0)),
        }
    }

    /// Pauses or resumes STATE_HISTORY pruning at finality advance (see the
    /// `journal_pruning_paused` field). Called by `Blockchain::enter_reorg` /
    /// `ReorgGuard::drop` to bracket a deep-reorg apply pass.
    pub fn set_journal_pruning_paused(&self, paused: bool) {
        self.journal_pruning_paused
            .store(paused, std::sync::atomic::Ordering::Release);
    }

    /// Removes any installed overlay from the layer cache. Called by the
    /// reconciliation path after the first new-chain commit folds
    /// the overlay into disk. Idempotent.
    pub fn clear_reorg_overlay(&self) -> Result<(), StoreError> {
        let mut guard = self.trie_cache.write().map_err(|_| StoreError::LockError)?;
        let mut updated = (**guard).clone();
        updated.clear_overlay();
        *guard = Arc::new(updated);
        Ok(())
    }

    /// Aborts an in-progress deep reorg and resets the layer cache to a fresh
    /// empty state with the same commit threshold. Both the overlay and any
    /// partially-built new-chain layers are discarded. On-disk state is
    /// untouched (still at the OLD chain's `D`), so subsequent FCU evaluations
    /// start from a clean foundation.
    ///
    /// That holds for the binary trie too, and for the same reason: the unwind
    /// lives entirely in the overlay until the reconciliation commit folds it
    /// into one write batch, so an abort before that point leaves
    /// `BINARY_TRIE_NODES` exactly as the old chain left it. There is no partial
    /// unwind to repair — the two tries abort together because they were only
    /// ever going to be persisted together.
    pub fn abort_reorg(&self) -> Result<(), StoreError> {
        // Rendezvous with the persist worker before swapping, exactly like
        // `install_overlay_for_reorg`: the live-path ack fires BEFORE
        // `apply_trie_phase1` installs the layer, so the worker can still be
        // mid-flight with a pre-abort RCU snapshot. Without the rendezvous its
        // write-back could install a side-chain layer into the fresh cache with
        // no overlay underneath — reads at that root would then cascade into
        // old-chain disk state.
        self.rendezvous_persist_worker("abort_reorg")?;
        let mut guard = self.trie_cache.write().map_err(|_| StoreError::LockError)?;
        let threshold = guard.commit_threshold();
        *guard = Arc::new(TrieLayerCache::new_with_safe_commit(
            threshold,
            self.safe_commit_root.clone(),
        ));
        Ok(())
    }

    /// Takes a block hash and returns an iterator to its ancestors. Block headers are returned
    /// in reverse order, starting from the given block and going up to the genesis block.
    pub fn ancestors(&self, block_hash: BlockHash) -> AncestorIterator {
        AncestorIterator {
            store: self.clone(),
            next_hash: block_hash,
        }
    }

    /// Checks if a given block belongs to the current canonical chain. Returns false if the block is not known
    pub fn is_canonical_sync(&self, block_hash: BlockHash) -> Result<bool, StoreError> {
        let Some(block_number) = self.get_block_number_sync(block_hash)? else {
            return Ok(false);
        };
        Ok(self
            .get_canonical_block_hash_sync(block_number)?
            .is_some_and(|h| h == block_hash))
    }

    pub fn generate_flatkeyvalue(&self) -> Result<(), StoreError> {
        self.flatkeyvalue_control_tx
            .send(FKVGeneratorControlMessage::Continue)
            .map_err(|_| StoreError::Custom("FlatKeyValue thread disconnected.".to_string()))
    }

    pub fn create_checkpoint(&self, path: impl AsRef<Path>) -> Result<(), StoreError> {
        self.backend.create_checkpoint(path.as_ref())?;
        init_metadata_file(path.as_ref())?;
        Ok(())
    }

    pub fn get_store_directory(&self) -> Result<PathBuf, StoreError> {
        Ok(self.db_path.clone())
    }

    /// Loads the latest block number stored in the database, bypassing the latest block number cache
    async fn load_latest_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        let key = chain_data_key(ChainDataIndex::LatestBlockNumber);
        self.read_async(CHAIN_DATA, key)
            .await?
            .map(|bytes| -> Result<BlockNumber, StoreError> {
                let array: [u8; 8] = bytes
                    .try_into()
                    .map_err(|_| StoreError::Custom("Invalid BlockNumber bytes".to_string()))?;
                Ok(BlockNumber::from_le_bytes(array))
            })
            .transpose()
    }

    fn load_canonical_block_hash(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<BlockHash>, StoreError> {
        let txn = self.backend.begin_read()?;
        txn.get(
            CANONICAL_BLOCK_HASHES,
            block_number.to_le_bytes().as_slice(),
        )?
        .map(|bytes| H256::decode(bytes.as_slice()))
        .transpose()
        .map_err(StoreError::from)
    }

    fn load_block_header(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<BlockHeader>, StoreError> {
        let Some(block_hash) = self.load_canonical_block_hash(block_number)? else {
            return Ok(None);
        };
        self.load_block_header_by_hash(block_hash)
    }

    /// Load a block header, bypassing the latest header cache
    fn load_block_header_by_hash(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockHeader>, StoreError> {
        let txn = self.backend.begin_read()?;
        let hash_key = block_hash.encode_to_vec();
        let header_value = txn.get(HEADERS, hash_key.as_slice())?;
        let mut header = header_value
            .map(|bytes| BlockHeaderRLP::from_bytes(bytes).to())
            .transpose()
            .map_err(StoreError::from)?;
        header.as_mut().inspect(|h| {
            // Set the hash so we avoid recomputing it later
            let _ = h.hash.set(block_hash);
        });
        Ok(header)
    }

    pub fn last_written(&self) -> Result<Vec<u8>, StoreError> {
        let last_computed_flatkeyvalue = self
            .last_computed_flatkeyvalue
            .read()
            .map_err(|_| StoreError::LockError)?;
        Ok(last_computed_flatkeyvalue.clone())
    }

    /// Returns `true` once the flat-key-value generator has finished its full pass.
    ///
    /// Completion is recorded by the 1-byte `[0xff]` sentinel the generator writes to
    /// `MISC_VALUES["last_written"]` on the final iteration (see `flatkeyvalue_generator`);
    /// every non-final frontier value is a real nibble path (bytes `0x00..=0x0f`), so the
    /// sentinel is unambiguous. Reads the durable marker rather than the in-memory frontier,
    /// which is expanded to `[0xff; 64]`/`[0xff; 131]` and would need length-aware handling.
    ///
    /// Used to gate journal-backed deep reorgs: entries journaled while generation is still
    /// in progress omit past-frontier flat-KV pre-images.
    pub fn flatkeyvalue_fully_generated(&self) -> Result<bool, StoreError> {
        let tx = self.backend.begin_read()?;
        let marker = tx.get(MISC_VALUES, "last_written".as_bytes())?;
        Ok(Self::flatkeyvalue_generation_complete(marker.as_deref()))
    }

    /// Pure completeness test for the durable `last_written` marker: complete iff it is the
    /// exact 1-byte `[0xff]` sentinel. Any in-progress frontier is a nibble path (bytes
    /// `0x00..=0x0f`) and an unset marker is absent, so neither matches.
    fn flatkeyvalue_generation_complete(marker: Option<&[u8]>) -> bool {
        marker == Some([0xff].as_slice())
    }

    fn flatkeyvalue_computed_with_last_written(account: H256, last_written: &[u8]) -> bool {
        let account_nibbles = Nibbles::from_bytes(account.as_bytes());
        &last_written[0..64] > account_nibbles.as_ref()
    }

    /// Returns the highest block number durably flushed to disk, or `0` when
    /// the marker is absent. Use [`Self::read_flushed_upto_opt`] when you need
    /// to distinguish "absent marker" (legacy DB, everything is durable) from
    /// "marker present and equal to 0".
    pub fn read_flushed_upto(&self) -> Result<BlockNumber, StoreError> {
        Ok(self.read_flushed_upto_opt()?.unwrap_or(0))
    }

    /// Returns `None` when the marker has never been written — a legacy or fresh
    /// DB where everything is durable and the head must not be clamped to 0.
    fn read_flushed_upto_opt(&self) -> Result<Option<BlockNumber>, StoreError> {
        let tx = self.backend.begin_read()?;
        match tx.get(MISC_VALUES, FLUSHED_UPTO_KEY)? {
            Some(bytes) => Ok(Some(decode_flushed_upto(&bytes)?)),
            None => Ok(None),
        }
    }

    /// Insert a block into the in-memory buffer without writing to disk.
    /// For testing only — gates production code off.
    #[cfg(any(test, feature = "testing"))]
    pub fn buffer_block_for_test(&self, block: &Block) {
        mutate_block_buffer(&self.block_data_buffer, |b| {
            b.insert(block.clone(), vec![], vec![])
        })
        .expect("block_data_buffer lock poisoned");
    }

    /// Synchronously flush the block data buffer to disk.
    /// For testing only — gates production code off.
    #[cfg(any(test, feature = "testing"))]
    pub fn flush_block_data_for_test(&self) -> Result<(), StoreError> {
        flush_block_data(self.backend.as_ref(), &self.block_data_buffer)
    }

    /// Read a raw trie node straight from the on-disk account/storage trie-node
    /// table by its committed key. For testing only — lets a reopen assert which
    /// trie diff-layers a shutdown flush did (or did not) commit to disk.
    #[cfg(any(test, feature = "testing"))]
    pub fn get_trie_node_for_test(
        &self,
        is_account: bool,
        key: &[u8],
    ) -> Result<Option<Vec<u8>>, StoreError> {
        let table = if is_account {
            ACCOUNT_TRIE_NODES
        } else {
            STORAGE_TRIE_NODES
        };
        self.backend.begin_read()?.get(table, key)
    }

    /// Number of entries in the binary-trie node table. For testing only —
    /// lets a test assert that an *unscheduled* chain does literally no
    /// binary-trie work, which is the property that makes shadow tracking safe
    /// to land.
    #[cfg(any(test, feature = "testing"))]
    pub fn binary_trie_node_count_for_test(&self) -> Result<usize, StoreError> {
        use crate::api::tables::BINARY_TRIE_NODES;
        let read = self.backend.begin_read()?;
        let count = read.prefix_iterator(BINARY_TRIE_NODES, &[])?.count();
        Ok(count)
    }

    /// Every `(tree key, leaf value)` row in the binary flat mirror. For
    /// testing only — the mirror side of the agreement invariant, which needs
    /// *which* rows exist and not just how many, because a mirror that is a
    /// superset of the trie is exactly as wrong as one that is a subset.
    #[cfg(any(test, feature = "testing"))]
    pub fn binary_flat_rows_for_test(&self) -> Result<BinaryFlatWrites, StoreError> {
        use crate::api::tables::BINARY_FLATKEYVALUE;
        let read = self.backend.begin_read()?;
        read.prefix_iterator(BINARY_FLATKEYVALUE, &[])?
            .map(|entry| entry.map(|(k, v)| (k.into_vec(), v.into_vec())))
            .collect()
    }

    /// The mirror's durable backfill frontier marker, verbatim. For testing
    /// only: `None` is absent, `Some([0xff])` is complete, anything else is a
    /// tree key the sweep has reached.
    #[cfg(any(test, feature = "testing"))]
    pub fn binary_flat_frontier_for_test(&self) -> Result<Option<Vec<u8>>, StoreError> {
        self.backend
            .begin_read()?
            .get(MISC_VALUES, BINARY_LAST_WRITTEN_KEY)
    }

    /// Number of recorded block-hash -> binary-root entries. For testing only;
    /// companion to [`Store::binary_trie_node_count_for_test`].
    #[cfg(any(test, feature = "testing"))]
    pub fn binary_trie_root_count_for_test(&self) -> Result<usize, StoreError> {
        let read = self.backend.begin_read()?;
        let count = read.prefix_iterator(BINARY_TRIE_ROOTS, &[])?.count();
        Ok(count)
    }

    /// Every `(key, value)` pair in the binary-trie node table. For testing
    /// only — the counterpart of [`Store::binary_trie_node_count_for_test`] for
    /// tests that must compare *which* nodes reached disk, not just how many
    /// (the reorg-discard assertion).
    #[cfg(any(test, feature = "testing"))]
    pub fn binary_trie_nodes_for_test(&self) -> Result<BinaryTrieNodes, StoreError> {
        use crate::api::tables::BINARY_TRIE_NODES;
        let read = self.backend.begin_read()?;
        read.prefix_iterator(BINARY_TRIE_NODES, &[])?
            .map(|entry| entry.map(|(k, v)| (k.into_vec(), v.into_vec())))
            .collect()
    }

    /// Number of entries in the MPT's account-trie node table. For testing
    /// only — the MPT baseline the binary-trie flush is compared against, so a
    /// test can state that the two land at the same commit point rather than
    /// independently.
    #[cfg(any(test, feature = "testing"))]
    pub fn account_trie_node_count_for_test(&self) -> Result<usize, StoreError> {
        let read = self.backend.begin_read()?;
        let count = read.prefix_iterator(ACCOUNT_TRIE_NODES, &[])?.count();
        Ok(count)
    }

    /// Force the trie-layer commit gate at `root` and wait for the flush.
    ///
    /// For testing only. Live nodes reach this through `forkchoice_update`,
    /// which advances the safe-commit cell to the canonical `head - 128` root
    /// and pokes the persist worker; a test that wants to observe a flush
    /// without building 128 blocks needs the same two steps without the depth
    /// requirement. `root` must be a canonical state root, exactly as the real
    /// gate's would be.
    #[cfg(any(test, feature = "testing"))]
    pub async fn commit_trie_layers_for_test(&self, root: H256) -> Result<(), StoreError> {
        self.set_safe_commit_root(root)?;
        let tx = self.persist_tx.clone();
        tokio::task::spawn_blocking(move || tx.send(PersistMessage::Commit(root)))
            .await
            .map_err(|e| StoreError::Custom(format!("commit poke join failed: {e}")))?
            .map_err(|e| StoreError::Custom(format!("commit poke send failed: {e}")))?;
        self.wait_for_persistence_idle().await
    }

    /// Drop every in-memory trie diff-layer, keeping whatever has reached disk.
    ///
    /// This is what a process restart does to a node: `Store::shutdown`
    /// force-flushes the block-data buffer but deliberately leaves the trie
    /// diff-layers in memory, so they are simply gone on the next boot. Tests
    /// that assert what a restarted node can still *see* need to reproduce that
    /// loss without spawning a process.
    #[cfg(any(test, feature = "testing"))]
    pub fn drop_trie_layers_for_test(&self) -> Result<(), StoreError> {
        let mut guard = self.trie_cache.write().map_err(|_| StoreError::LockError)?;
        let threshold = guard.commit_threshold();
        *guard = Arc::new(TrieLayerCache::new_with_safe_commit(
            threshold,
            self.safe_commit_root.clone(),
        ));
        Ok(())
    }

    /// Insert a block plus associated codes into the in-memory buffer without
    /// writing to disk.  For testing only — proves the buffer overlay resolves
    /// code that has not been persisted yet.
    #[cfg(any(test, feature = "testing"))]
    pub fn buffer_block_with_codes_for_test(&self, block: &Block, codes: Vec<(H256, Code)>) {
        mutate_block_buffer(&self.block_data_buffer, |b| {
            b.insert(block.clone(), vec![], codes)
        })
        .expect("block_data_buffer lock poisoned");
    }

    /// Mark a state root as in-flight (build pending) without doing a build.
    /// For testing only — simulates the window where the persist worker has not
    /// yet installed the layer, so reads at this root must block in
    /// `gated_snapshot`.
    #[cfg(any(test, feature = "testing"))]
    pub fn register_pending_root_for_test(&self, root: H256) -> Result<(), StoreError> {
        self.pending_trie_roots.register(root)
    }

    /// Clear an in-flight state root (simulates the worker having installed the
    /// layer), unblocking readers waiting in `gated_snapshot`. For testing only.
    #[cfg(any(test, feature = "testing"))]
    pub fn clear_pending_root_for_test(&self, root: H256) {
        self.pending_trie_roots.clear(root)
    }

    /// Boot-time recovery: clamp `latest_block_header` to the durable head.
    ///
    /// Durable head = `min(flushed_upto, latest)` when the marker is present
    /// (buffered blocks past `flushed_upto` may be lost after a crash; the CL
    /// re-sends them via `newPayload`). When the marker is absent the DB
    /// predates deferred persistence and everything is on disk — use `latest`
    /// as-is, never rewind to 0. On first boot the marker is seeded so a later
    /// crash clamps against it rather than an absent (→ 0) marker.
    ///
    /// The marker tracks the max flushed *block number*, not which hash is
    /// canonical at that height. A tip reorg inside the flush window — `Na` at
    /// height N is flushed (marker = N), then `newPayload(Nb)` buffers a sibling
    /// and FCU durably repoints `canonical[N]` to the still-unflushed `Nb` — can
    /// leave `canonical[head]` resolving to a header that never reached disk if
    /// we crash before `Nb` flushes. So we walk `head` down to the highest height
    /// whose canonical hash actually resolves on disk rather than bricking with
    /// `MissingLatestBlockNumber`. A legacy DB (no marker) is exempt: everything
    /// there is durable, so a missing header is real corruption and must surface.
    async fn anchor_to_durable_head(&self, latest: BlockNumber) -> Result<(), StoreError> {
        let marker = self.read_flushed_upto_opt()?;
        let start = match marker {
            Some(flushed) => flushed.min(latest),
            None => latest,
        };

        let mut head = start;
        let latest_block_header = loop {
            match self.load_block_header(head)? {
                Some(header) => break header,
                // Legacy/fresh DB: everything is supposed to be durable, so a
                // missing header is real corruption — surface it, don't rewind.
                None if marker.is_none() => return Err(StoreError::MissingLatestBlockNumber),
                None if head == 0 => return Err(StoreError::MissingLatestBlockNumber),
                None => {
                    warn!(
                        "durable head {head}: canonical hash has no on-disk header \
                         (reorg inside flush window); rewinding"
                    );
                    head -= 1;
                }
            }
        };
        self.latest_block_header.update(latest_block_header);

        // Re-anchor the persisted head when we moved below `latest`, and (re)write
        // the marker to the resolved head: an absent marker is seeded to the
        // durable baseline, and a walked-down head lowers the marker so a later
        // crash clamps against a hash known to resolve.
        let reanchor = head != latest;
        let rewrite_marker = marker != Some(head);
        if reanchor || rewrite_marker {
            let mut tx = self.backend.begin_write()?;
            if reanchor {
                // Re-anchor the persisted head so `get_latest_block_number` and
                // every downstream consumer agree with the clamped head.
                let latest_key = chain_data_key(ChainDataIndex::LatestBlockNumber);
                tx.put(CHAIN_DATA, &latest_key, &head.to_le_bytes())?;
            }
            if rewrite_marker {
                write_flushed_upto(tx.as_mut(), head)?;
            }
            tx.commit()?;
        }
        Ok(())
    }
}

/// Writes the `flushed_upto` block number into an open write batch.
///
/// The caller is responsible for committing `tx` afterward.
pub fn write_flushed_upto(
    tx: &mut dyn StorageWriteBatch,
    n: BlockNumber,
) -> Result<(), StoreError> {
    tx.put(MISC_VALUES, FLUSHED_UPTO_KEY, &n.to_le_bytes())
}

/// Decode an 8-byte little-endian `flushed_upto` marker value.
///
/// Returns an error for a present-but-malformed value so on-disk corruption is
/// surfaced loudly rather than silently resetting the durable marker. Single
/// source of truth for both `from_backend` and [`Store::read_flushed_upto`].
fn decode_flushed_upto(bytes: &[u8]) -> Result<BlockNumber, StoreError> {
    let arr: [u8; 8] = bytes
        .try_into()
        .map_err(|_| StoreError::Custom("Invalid flushed_upto bytes".to_string()))?;
    Ok(BlockNumber::from_le_bytes(arr))
}

/// RCU-swap the block-data buffer. The persist worker is the sole caller in
/// production (no lost-update race); test helpers also call this on one thread.
fn mutate_block_buffer(
    buffer: &Arc<RwLock<Arc<BlockDataBuffer>>>,
    f: impl FnOnce(&mut BlockDataBuffer),
) -> Result<(), StoreError> {
    let mut new_buf = (*buffer.read().map_err(|_| StoreError::LockError)?.clone()).clone();
    f(&mut new_buf);
    *buffer.write().map_err(|_| StoreError::LockError)? = Arc::new(new_buf);
    Ok(())
}

/// Default for [`StoreConfig::persist_channel_capacity`].
const DEFAULT_PERSIST_CHANNEL_CAPACITY: usize = 2;

/// One unit of work for the persist worker: stage block(s), build the trie diff-layer,
/// flush to disk. `commit_depth` selects the commit gate (`None` = canonical safe-commit
/// root, `Some(depth)` = commit layers deeper than `depth`). `wait_for_flush` selects the
/// ack point independently: `false` acks after staging (carrying the prior flush result),
/// `true` acks after flush.
struct BlockPersist {
    blocks: Vec<(Block, Vec<Receipt>)>,
    codes: Vec<(H256, Code)>,
    parent_state_root: H256,
    child_state_root: H256,
    account_updates: TrieNodesUpdate,
    storage_updates: Vec<(H256, TrieNodesUpdate)>,
    /// The same unit's staged EIP-8297 binary-trie advance, if the chain
    /// schedules the commitment. Installed into the same diff layer as
    /// `account_updates` / `storage_updates`.
    binary_update: Option<BinaryTrieAdvance>,
    commit_depth: Option<usize>,
    wait_for_flush: bool,
    /// Number of the block whose layer this update represents (the last block in
    /// the batch, matching `child_state_root`). Threaded into the trie layer so
    /// the committed-layer identity is available for the journal write path;
    /// harmless for batch updates, since journal writes are skipped when
    /// `wait_for_flush` (batch mode) is set.
    block_number: BlockNumber,
    /// Hash of the block whose layer this update represents (see `block_number`).
    block_hash: H256,
    ack: std::sync::mpsc::SyncSender<Result<(), StoreError>>,
}

/// Messages for the persist worker. `Ping(ack)` is the idle handshake for
/// [`Store::wait_for_persistence_idle`]: the FIFO worker handles it only after
/// all earlier `Block` messages are fully processed.
enum PersistMessage {
    Block(Box<BlockPersist>),
    /// Flush the committable layer backlog up to and including this state root, then
    /// prune the flushed layers. Sent by `forkchoice_update` when the safe-commit root
    /// advances: the commit step otherwise only runs while blocks execute, so an
    /// execute-all-then-one-forkchoice flow (e.g. block import) would accumulate every
    /// layer and never persist anything to disk.
    Commit(H256),
    Ping(std::sync::mpsc::SyncSender<Result<(), StoreError>>),
    /// Graceful-shutdown handshake. Handled only after every earlier `Block`
    /// (FIFO), so it both drains in-flight work and force-flushes the block-data
    /// buffer to disk. The trie diff-layers are deliberately left in memory (see
    /// [`Store::shutdown`]). The worker acks and exits.
    Shutdown {
        ack: std::sync::mpsc::SyncSender<Result<(), StoreError>>,
    },
}

/// Write one block's header, body, number, and tx locations into an open batch.
/// Shared by [`Store::add_blocks`] (sync import) and [`flush_block_data`]
/// (deferred flush) so the on-disk encoding stays in lockstep. Receipts and codes
/// are written by callers that need them (only `flush_block_data` does).
fn write_block_data(
    tx: &mut dyn StorageWriteBatch,
    number: BlockNumber,
    hash: BlockHash,
    header: &BlockHeader,
    body: &BlockBody,
) -> Result<(), StoreError> {
    let hash_key = hash.encode_to_vec();
    tx.put(
        HEADERS,
        &hash_key,
        BlockHeaderRLP::from(header.clone()).bytes(),
    )?;
    tx.put(
        BODIES,
        &hash_key,
        BlockBodyRLP::from_bytes(body.encode_to_vec()).bytes(),
    )?;
    tx.put(BLOCK_NUMBERS, &hash_key, &number.to_le_bytes())?;
    for (index, transaction) in body.transactions.iter().enumerate() {
        tx.merge(
            TRANSACTION_LOCATIONS,
            transaction.hash(&NativeCrypto).as_bytes(),
            &encode_tx_location_operand(number, hash, index as u64),
        )?;
    }
    Ok(())
}

/// Write all unflushed blocks to disk in one tx, advance `flushed_upto`, then
/// evict. Eviction is gap-safe: blocks stay buffered until the commit succeeds.
fn flush_block_data(
    backend: &dyn StorageBackend,
    buffer: &Arc<RwLock<Arc<BlockDataBuffer>>>,
) -> Result<(), StoreError> {
    let snapshot = buffer.read().map_err(|_| StoreError::LockError)?.clone();
    let to_flush = snapshot.flushable();
    if to_flush.is_empty() {
        return Ok(());
    }
    let hashes: Vec<_> = to_flush.iter().map(|b| b.header.hash()).collect();
    let codes = snapshot.codes_for(&hashes);
    let mut max_number = snapshot.flushed_upto();

    let mut tx = backend.begin_write()?;
    for b in &to_flush {
        let hash = b.header.hash();
        write_block_data(tx.as_mut(), b.number, hash, &b.header, &b.body)?;
        for (index, receipt) in b.receipts.iter().enumerate() {
            tx.put(
                RECEIPTS_V2,
                &receipt_key(&hash, index as u64),
                &receipt.encode_to_vec(),
            )?;
        }
        max_number = max_number.max(b.number);
    }
    for (code_hash, code) in codes {
        let buf = encode_code(&code);
        tx.put(ACCOUNT_CODES, code_hash.as_ref(), &buf)?;
        tx.put(
            ACCOUNT_CODE_METADATA,
            code_hash.as_ref(),
            &(code.len() as u64).to_be_bytes(),
        )?;
    }
    write_flushed_upto(tx.as_mut(), max_number)?;
    tx.commit()?;

    // Phase 3: evict only after the commit succeeded (gap safety).
    mutate_block_buffer(buffer, |b| b.evict_flushed(max_number))
}

type TrieNodesUpdate = Vec<(Nibbles, Vec<u8>)>;

/// Tracks state roots whose trie diff-layer is in-flight (building but not yet
/// installed in `trie_cache`). `apply_updates` registers a root *before*
/// returning; the worker clears it *after* swapping the layer in. This ordering
/// is mandatory: a reader opening a trie at a pending root blocks until the
/// layer is installed, preventing stale on-disk reads.
#[derive(Debug, Default)]
struct PendingTrieRoots {
    /// Fast-path: when zero, nothing is in flight and readers skip the lock.
    count: AtomicUsize,
    roots: Mutex<HashSet<H256>>,
    ready: Condvar,
}

impl PendingTrieRoots {
    /// Mark `root` as in-flight. MUST be called before the build is handed to
    /// the worker (so the worker's `clear` always finds it) and before the head
    /// can advance to `root` (so any reader that can reference it sees it pending).
    fn register(&self, root: H256) -> Result<(), StoreError> {
        let mut roots = self.roots.lock().map_err(|_| StoreError::LockError)?;
        if roots.insert(root) {
            self.count.fetch_add(1, Ordering::Release);
        }
        Ok(())
    }

    /// Mark `root` as installed and wake any waiting readers. MUST be called only
    /// after the layer is swapped into `trie_cache`, so a woken reader sees it.
    /// Best-effort: a poisoned lock means a reader's `wait_until_ready` also errors,
    /// so no reader deadlocks.
    fn clear(&self, root: H256) {
        let Ok(mut roots) = self.roots.lock() else {
            return;
        };
        if roots.remove(&root) {
            self.count.fetch_sub(1, Ordering::Release);
            self.ready.notify_all();
        }
    }

    /// Block until `root` is no longer in-flight (its layer is installed). Returns
    /// immediately on the fast path when nothing is pending.
    fn wait_until_ready(&self, root: H256) -> Result<(), StoreError> {
        if self.count.load(Ordering::Acquire) == 0 {
            return Ok(());
        }
        let mut roots = self.roots.lock().map_err(|_| StoreError::LockError)?;
        while roots.contains(&root) {
            roots = self.ready.wait(roots).map_err(|_| StoreError::LockError)?;
        }
        Ok(())
    }
}

/// Build the trie diff-layer, RCU-swap it into `trie_cache`, then clear the
/// pending root. Swap MUST precede the clear so a woken reader sees the layer.
/// On swap failure the root is still cleared so gated readers error, not deadlock.
#[allow(clippy::too_many_arguments)]
fn apply_trie_phase1(
    trie_cache: &Arc<RwLock<Arc<TrieLayerCache>>>,
    pending_roots: &PendingTrieRoots,
    parent_state_root: H256,
    child_state_root: H256,
    block_number: BlockNumber,
    block_hash: H256,
    account_updates: TrieNodesUpdate,
    storage_updates: Vec<(H256, TrieNodesUpdate)>,
    binary_update: Option<BinaryTrieAdvance>,
) -> Result<(), StoreError> {
    let build: Result<(), StoreError> = (|| {
        let new_layer = storage_updates
            .into_iter()
            .flat_map(|(account_hash, nodes)| {
                nodes
                    .into_iter()
                    .map(move |(path, node)| (apply_prefix(Some(account_hash), path), node))
            })
            .chain(account_updates)
            .collect();
        let trie = trie_cache
            .read()
            .map_err(|_| StoreError::LockError)?
            .clone();
        let mut trie_mut = (*trie).clone();
        // One layer, both node sets: they must be staged, flushed and dropped
        // as a unit, so there is exactly one insertion here and never two.
        trie_mut.put_batch_with_binary(
            parent_state_root,
            child_state_root,
            block_number,
            block_hash,
            new_layer,
            binary_update
                .map(BinaryLayerUpdate::from)
                .unwrap_or_default(),
        );
        *trie_cache.write().map_err(|_| StoreError::LockError)? = Arc::new(trie_mut);
        Ok(())
    })();
    // Always clear the pending root, whether or not the swap succeeded: on success
    // readers see the installed layer; on failure (poisoning) the lock is poisoned
    // so gated readers error rather than read stale, and we must not leave them
    // blocked forever.
    pending_roots.clear(child_state_root);
    build
}

/// Flush and prune the committable trie-layer backlog. No-ops when nothing is committable.
///
/// `commit_depth` selects the gate. `Some(depth)`: single-canonical-chain execution (batch
/// import, full sync, startup regeneration) commits by depth, because the canonical `head - 128`
/// safe-commit root never lands on a batch layer boundary; sound because these paths only ever
/// extend a single canonical chain (no competing forks to mis-commit). `None`: live block-by-block
/// execution uses the canonical safe-commit gate (`TrieLayerCache::get_commitable`) so non-canonical
/// `newPayload` state is never persisted.
///
/// `is_batch` is independent of the gate and only selects journaling (see
/// [`commit_to_disk`]). It tracks `wait_for_flush`, not `commit_depth.is_some()`, so every
/// per-block path journals and only the bespoke batch path skips it. That keeps the
/// full-sync tail and the import tail journaling exactly as they did when `commit_depth`
/// and `wait_for_flush` were a single flag.
///
/// Startup state regeneration is the one path where this is new behavior rather than
/// preserved behavior: it runs before any forkchoice update, so the safe-commit cell is
/// still zero and the canonical gate committed nothing there, which is the unbounded-layer
/// problem the depth gate fixes. Now that it does commit, it also journals. Deriving
/// `is_batch` from `commit_depth` instead would suppress that, but it would also suppress
/// the full-sync and import tails, and it would leave the journal discontiguous with the
/// on-disk root: surviving entries below a non-journaled commit describe pre-images
/// relative to a root the disk has already moved past.
fn commit_trie_if_due(
    backend: &dyn StorageBackend,
    trie_cache: &Arc<RwLock<Arc<TrieLayerCache>>>,
    fkv_ctl: &SyncSender<FKVGeneratorControlMessage>,
    parent_state_root: H256,
    commit_depth: Option<usize>,
    is_batch: bool,
) -> Result<(), StoreError> {
    let trie = trie_cache
        .read()
        .map_err(|_| StoreError::LockError)?
        .clone();
    // Phase 2 + 3: flush and prune the committable backlog.
    let commitable = match commit_depth {
        Some(depth) => trie.get_commitable_by_depth(parent_state_root, depth),
        None => trie.get_commitable(parent_state_root),
    };
    let Some(root) = commitable else {
        // Nothing to commit to disk, move on.
        return Ok(());
    };
    commit_to_disk(backend, fkv_ctl, trie_cache, &trie, root, is_batch)
}

/// Flush the layer at `root` and all older ancestors to disk, then prune them from the
/// in-memory cache (Phases 2 and 3 of the persistence pipeline).
///
/// `trie` must be the current cache snapshot and `root` a committable layer (as returned
/// by [`TrieLayerCache::get_commitable`]). A `root` that is not a layer commits nothing.
///
/// Reused by both the per-block path ([`commit_trie_if_due`]) and the forkchoice-driven
/// flush ([`PersistMessage::Commit`]): without the latter, an execute-all-then-one-forkchoice
/// flow (block import) would never persist, because the commit step only runs while blocks execute.
fn commit_to_disk(
    backend: &dyn StorageBackend,
    fkv_ctl: &SyncSender<FKVGeneratorControlMessage>,
    trie_cache: &Arc<RwLock<Arc<TrieLayerCache>>>,
    trie: &Arc<TrieLayerCache>,
    root: H256,
    is_batch: bool,
) -> Result<(), StoreError> {
    // `root` need not have a layer: the forkchoice `PersistMessage::Commit(root)` path
    // forwards the safe-commit root without consulting the cache, and `put_batch` skips
    // blocks whose state root equals their parent's. Bail before the side effects below.
    if !trie.has_layer(root) {
        debug!(
            root = ?root,
            layers = trie.layer_count(),
            is_batch,
            "Skipping trie commit: state root has no in-memory layer. Expected when the block \
             did not change the state root (empty L2 blocks) or the root was already flushed."
        );
        return Ok(());
    }

    // Stop the flat-key-value generator thread, as the underlying trie is about to change.
    // Ignore the error, if the channel is closed it means there is no worker to notify.
    let _ = fkv_ctl.send(FKVGeneratorControlMessage::Stop);

    // RCU to remove the bottom layer: update step needs to happen after disk layer is updated.
    let mut trie_mut = (**trie).clone();

    // Open the read view BEFORE the write batch so each `.get()` sees disk as it was
    // before our writes. The journal records `(key, prev_value)` so a future rollback
    // can apply diffs directly without reading state.
    //
    // NOTE: `StorageReadView` does not currently promise true snapshot isolation
    // (see the trait docs in `api/mod.rs`). What makes the pre-image read safe here
    // is that `commit_to_disk` only ever runs on the single persist worker thread,
    // `write_tx` is a buffered batch that does not become visible until
    // `write_tx.commit()` at the end of the function, and the other writers to
    // these CFs touch disjoint key space: the flat-KV generator only writes keys
    // strictly past the committed `last_written` frontier, and snap-sync healing
    // completes before block execution commits layers. So every `.get()` below
    // sees on-disk state as of the begin_read call.
    let read_view = backend.begin_read()?;
    let last_written = read_view
        .get(MISC_VALUES, "last_written".as_bytes())?
        .unwrap_or_default();
    // The binary mirror's frontier, read from the same pre-write view for the
    // same reason. Absent until Task 9's generator exists, which
    // [`binary_flat_frontier_covers`] reads as "this write path owns the whole
    // keyspace" — a write-ownership answer, not a coverage claim. See that
    // function's docs.
    let binary_last_written = read_view
        .get(MISC_VALUES, BINARY_LAST_WRITTEN_KEY)?
        .unwrap_or_default();

    let mut write_tx = backend.begin_write()?;

    // Before encoding, accounts have only the account address as their path, while storage keys have
    // the account address (32 bytes) + storage path (up to 32 bytes).

    // Snapshot the overlay (if any) BEFORE commit so reconciliation can fold its entries
    // into this write batch. After a deep reorg, the first
    // new-chain commit advances disk from the OLD chain's edge `D` directly to the new
    // chain's tip `T` in a single atomic write; the overlay supplies the bridge for keys
    // layer_T does not touch. Only meaningful when `!is_batch` (full sync does not journal).
    let overlay_for_reconciliation = if !is_batch {
        trie.overlay().cloned()
    } else {
        None
    };

    // While an overlay is installed, commit only the bottom layer per pass. The
    // The reconciliation below is defined for a single layer at the pivot
    // tip `T`: bridge entries are folded into that layer's writes, [T, D] is
    // delete_ranged, and T's journal entry records pre-images against the
    // old-chain disk state. A multi-layer sweep would journal upper layers'
    // pre-images against old-chain disk instead of the new-chain/bridge state
    // (the intra-batch pre-image map is not seeded with bridge values), silently
    // corrupting a future unwind. The backlog above `T` drains in later passes,
    // after this commit clears the overlay (see `trie_mut.clear_overlay` below).
    let root = if overlay_for_reconciliation.is_some() {
        trie.bottom_layer_root(root)
    } else {
        root
    };

    // `commit` removes the committed layer(s) and returns one `CommittedLayer` per block
    // in oldest-first order. In normal block-by-block operation this is a single layer,
    // one commit-cadence behind the just-added block. A forkchoice-driven flush of an
    // accumulated backlog (e.g. block import) can return several layers at once, so we
    // write one journal entry per block below rather than merging diffs across blocks.
    //
    // `has_layer` above established `root` is a layer and `trie_mut` is a `Clone` that
    // preserves the map, so this must be `Some`. Reaching the error means the clone lost the
    // map, which is corruption, not the ordinary no-op handled above.
    let committed_layers = trie_mut.commit(root).ok_or_else(|| {
        StoreError::Custom(format!(
            "trie layer for state root {root:?} disappeared between the has_layer check and \
             commit (layers={}, is_batch={is_batch}): the TrieLayerCache clone lost the layer \
             map, so this block's state was not written",
            trie.layer_count(),
        ))
    })?;

    // Deep-reorg reconciliation is a single new-chain commit advancing disk to the pivot
    // tip `T`; it must map to exactly one committed layer. A multi-layer commit with an
    // overlay installed would mean a backlog accumulated between overlay install and
    // flush, corrupting the [T, D] delete_range below.
    debug_assert!(
        overlay_for_reconciliation.is_none() || committed_layers.len() == 1,
        "overlay-backed reconciliation must commit exactly one layer (T), got {}",
        committed_layers.len()
    );

    // Reconciliation: overlay entries the new chain has NOT rewritten must be
    // bridged onto disk so disk fully reflects the pivot->T transition. Keys any committed
    // layer touches are skipped (the layer wins, its value is the post-T state). Overlay-only
    // entries with `None` become an empty-value write -> deleted on disk, matching the
    // "absent at pivot" semantics.
    let extra_writes: Vec<(Vec<u8>, Vec<u8>)> = match &overlay_for_reconciliation {
        Some(overlay) => {
            let layer_keys: rustc_hash::FxHashSet<&Vec<u8>> = committed_layers
                .iter()
                .flat_map(|l| l.nodes.iter().map(|(k, _)| k))
                .collect();
            overlay
                .iter_all_entries()
                .filter(|(_, key, _)| !layer_keys.contains(key))
                .map(|(_, key, value)| (key.clone(), value.clone().unwrap_or_default()))
                .collect()
        }
        None => Vec::new(),
    };
    // The same bridge for `BINARY_TRIE_NODES`, kept as a separate list because
    // these keys go to one fixed column family and, crucially, cannot be routed
    // by length: a `BitPath` key overlaps every range `classify_trie_key`
    // dispatches on, so folding them into `extra_writes` would write binary nodes
    // into `ACCOUNT_TRIE_NODES` and corrupt the MPT.
    let binary_extra_writes: Vec<(Vec<u8>, Vec<u8>)> = match &overlay_for_reconciliation {
        Some(overlay) => {
            let layer_keys: rustc_hash::FxHashSet<&Vec<u8>> = committed_layers
                .iter()
                .flat_map(|l| l.binary_nodes.iter().map(|(k, _)| k))
                .collect();
            overlay
                .iter_binary_entries()
                .filter(|(key, _)| !layer_keys.contains(key))
                .map(|(key, value)| (key.clone(), value.clone().unwrap_or_default()))
                .collect()
        }
        None => Vec::new(),
    };
    // And the same bridge again for `BINARY_FLATKEYVALUE`. A third list, not a
    // share of `binary_extra_writes`: the two binary key spaces have identical
    // length ranges, so a merged list would write mirror rows into
    // `BINARY_TRIE_NODES` and node encodings into the mirror, with nothing in
    // the bytes to tell them apart. An overlay `None` becomes an empty value,
    // which the write loop below turns into a delete — the "absent at the pivot"
    // semantics, and the reason a reorg cannot leave a stale row behind for a
    // leaf the new chain does not have.
    let binary_flat_extra_writes: Vec<(Vec<u8>, Vec<u8>)> = match &overlay_for_reconciliation {
        Some(overlay) => {
            let layer_keys: rustc_hash::FxHashSet<&Vec<u8>> = committed_layers
                .iter()
                .flat_map(|l| l.binary_flat.iter().map(|(k, _)| k))
                .collect();
            overlay
                .iter_binary_flat_entries()
                .filter(|(key, _)| !layer_keys.contains(key))
                .map(|(key, value)| (key.clone(), value.clone().unwrap_or_default()))
                .collect()
        }
        None => Vec::new(),
    };
    // Pivot heights (T, D) for the reconciliation commit; `None` in steady state.
    let reorg_heights = overlay_for_reconciliation
        .as_ref()
        .map(|ov| (ov.to_block(), ov.from_block()));

    // Intra-batch overlay of values already staged in THIS write batch, so each block's
    // reverse diff records the value as of the *previous* committed block's write, not
    // just the pre-batch on-disk value. `None` means an earlier block deleted the key.
    // For the common single-layer commit this stays empty and every pre-image comes
    // straight from `read_view`. Only consulted/maintained when journaling (`!is_batch`).
    //
    // PERF: the first touch of each key does one synchronous `read_view.get(table, &key)`.
    // For large state diffs this is O(N) extra reads on the per-block critical path.
    // A follow-up could batch these via `multi_get_cf` if profiling shows it's significant.
    let mut overlay: HashMap<Vec<u8>, Option<Vec<u8>>> = HashMap::new();
    // The same intra-batch pre-image map for `BINARY_TRIE_NODES`, and separate
    // for the same reason the overlay's binary map is: a `BitPath` key can be
    // byte-identical to an account-trie path, so one shared map would let a
    // binary write supply an MPT pre-image (or the reverse) and silently
    // corrupt whichever reverse diff read it second.
    //
    // PERF: same shape as above — one `read_view.get` per first-touched binary
    // key, so a scheduled chain pays roughly twice the pre-image reads an
    // unscheduled one does. That is the same doubling shadow tracking already
    // imposes on the write side, and it buys the only thing that makes a deep
    // reorg recoverable past activation. Untouched on unscheduled chains, where
    // `layer.binary_nodes` is empty and this map stays so.
    let mut binary_overlay: HashMap<Vec<u8>, Option<Vec<u8>>> = HashMap::new();
    // A third intra-batch pre-image map, for `BINARY_FLATKEYVALUE`, separate
    // from `binary_overlay` for the sharper form of the same reason that one is
    // separate from `overlay`: a `BitPath` DB key at bit-depth 240 is 34 bytes
    // and an account-zone tree key is 34 bytes, so the two spaces are not
    // distinguishable at all. One shared map would let a node write supply a
    // leaf's pre-image, and a multi-layer commit would then journal a reverse
    // diff that restores a node encoding into the mirror.
    let mut binary_flat_overlay: HashMap<Vec<u8>, Option<Vec<u8>>> = HashMap::new();

    let mut result = Ok(());
    'layers: for layer in &committed_layers {
        // Reverse-diff accumulators for this block's journal entry, one per CF. Each entry
        // stores the on-disk key as-is (storage CFs carry their nibble-encoded account-hash
        // prefix), so a future rollback applies diffs directly without interpretation. For
        // full sync (`is_batch == true`), no journal entry is written: reorgs aren't
        // supported during full sync, and journaling would slow it down by a read per write.
        let mut journal_account_trie: FlatDiff = Vec::new();
        let mut journal_storage_trie: FlatDiff = Vec::new();
        let mut journal_account_flat: FlatDiff = Vec::new();
        let mut journal_storage_flat: FlatDiff = Vec::new();
        let mut journal_binary_trie: FlatDiff = Vec::new();
        let mut journal_binary_flat: FlatDiff = Vec::new();

        // The reconciliation commit is single-layer at `T`; fold the overlay bridge entries
        // into that layer's writes so they land in T's journal entry. `extra` is empty in
        // steady state (no overlay) and for any non-T layer.
        // With the bottom-layer-only rule above, a commit made while the overlay
        // is installed always contains exactly this one (bottom) layer, so it is
        // the reconciliation layer by construction. Matching by block number
        // (`layer.block_number == to_block`) would be fragile: root-preserving
        // blocks create no layer (L2 empty blocks), so the bottom layer can sit
        // above `to_block` and the bridge entries would be neither written nor
        // journaled before the overlay is cleared.
        let is_reconciliation_layer = overlay_for_reconciliation.is_some();
        let extra: &[(Vec<u8>, Vec<u8>)] = if is_reconciliation_layer {
            &extra_writes
        } else {
            &[]
        };

        for (key, value) in layer.nodes.iter().chain(extra.iter()) {
            let (is_leaf, is_account) = classify_trie_key(key.len());

            // Keys past the flat-KV generator's frontier aren't written to disk yet, so
            // they must not be journaled either (a `Some(None)` entry recorded here
            // would cause a rollback to delete a key that was never put). The `continue`
            // jumps over both the write and the journal push below.
            if is_leaf && key.as_slice() > last_written.as_slice() {
                continue;
            }
            let table = if is_leaf {
                if is_account {
                    &ACCOUNT_FLATKEYVALUE
                } else {
                    &STORAGE_FLATKEYVALUE
                }
            } else if is_account {
                &ACCOUNT_TRIE_NODES
            } else {
                &STORAGE_TRIE_NODES
            };

            // Pre-image: the intra-batch overlay wins over disk so multi-layer commits
            // record each block's true pre-state. Skipped for batch (full-sync) commits.
            let prev_value = if !is_batch {
                match overlay.get(key) {
                    Some(v) => Some(v.clone()),
                    None => match read_view.get(table, key) {
                        Ok(v) => Some(v),
                        Err(e) => {
                            result = Err(e);
                            break 'layers;
                        }
                    },
                }
            } else {
                None
            };

            let new_value = if value.is_empty() {
                result = write_tx.delete(table, key);
                None
            } else {
                result = write_tx.put(table, key, value);
                Some(value.clone())
            };
            if result.is_err() {
                break 'layers;
            }

            // Record the reverse-diff entry after the put/delete is staged, so a write
            // error doesn't accumulate state we won't persist.
            if let Some(prev) = prev_value {
                let bucket = match (is_leaf, is_account) {
                    (false, true) => &mut journal_account_trie,
                    (false, false) => &mut journal_storage_trie,
                    (true, true) => &mut journal_account_flat,
                    (true, false) => &mut journal_storage_flat,
                };
                bucket.push((key.clone(), prev));
                // Advance the overlay so a later block in this same commit sees this
                // block's write as its pre-image.
                overlay.insert(key.clone(), new_value);
            }
        }

        // The same block's EIP-8297 binary-trie nodes, into the SAME write
        // batch. That is the whole flush-parity guarantee: `write_tx` commits
        // once at the end, so the two tries advance on disk together or not at
        // all, and a crash can never leave the MPT at block N with the binary
        // trie at some earlier block.
        //
        // Empty on unscheduled chains, so they do exactly the work they did.
        //
        // Journaled exactly as the MPT's are, into their own section of the
        // entry: the node table is path-keyed and single-version, so this write
        // destroys the previous version and only a recorded pre-image can put it
        // back. Reorgs within the layer window never reach here (the nodes stay
        // staged), so the reverse diff is only ever consumed by a reorg deeper
        // than the cache — which is precisely the case that was unrecoverable
        // before.
        //
        // `binary_extra` folds the overlay's binary bridge into the single
        // reconciliation layer, mirroring `extra` above.
        let binary_extra: &[(Vec<u8>, Vec<u8>)] = if is_reconciliation_layer {
            &binary_extra_writes
        } else {
            &[]
        };
        for (key, value) in layer.binary_nodes.iter().chain(binary_extra.iter()) {
            // Pre-image first, from the intra-batch map or disk, exactly as the
            // MPT loop above does — a multi-layer commit must record each
            // block's own pre-state, not the pre-batch one.
            let prev_value = if !is_batch {
                match binary_overlay.get(key) {
                    Some(v) => Some(v.clone()),
                    None => match read_view.get(BINARY_TRIE_NODES, key) {
                        Ok(v) => Some(v),
                        Err(e) => {
                            result = Err(e);
                            break 'layers;
                        }
                    },
                }
            } else {
                None
            };

            let new_value = if value.is_empty() {
                // Tombstone: the node left the tree. Storing zero bytes would
                // make the path read back as a node the trie never wrote.
                result = write_tx.delete(BINARY_TRIE_NODES, key);
                None
            } else {
                result = write_tx.put(BINARY_TRIE_NODES, key, value);
                Some(value.clone())
            };
            if result.is_err() {
                break 'layers;
            }

            if let Some(prev) = prev_value {
                journal_binary_trie.push((key.clone(), prev));
                binary_overlay.insert(key.clone(), new_value);
            }
        }

        // The same block's writes to the flat leaf mirror, into the SAME write
        // batch again. The mirror is *derived* from the binary trie, so this is
        // stronger than the flush parity above: a mirror persisted at a
        // different block than the trie does not merely lag, it disagrees with
        // the root it is a view of, and a range served from it would carry
        // leaves the root does not commit to.
        //
        // Empty on unscheduled chains.
        //
        // `binary_flat_extra` folds the overlay's mirror bridge into the single
        // reconciliation layer, mirroring `extra` and `binary_extra` above.
        let binary_flat_extra: &[(Vec<u8>, Vec<u8>)] = if is_reconciliation_layer {
            &binary_flat_extra_writes
        } else {
            &[]
        };
        for (key, value) in layer.binary_flat.iter().chain(binary_flat_extra.iter()) {
            // The frontier skip, the binary counterpart of the MPT one above:
            // a key this write path does not own is neither written nor
            // journaled, because a `Some(None)` pre-image for a row that was
            // never put would make a rollback delete something that never
            // existed. The frontier is absent until Task 9 builds the generator,
            // which reads as "we own everything"; see
            // `binary_flat_frontier_covers` for why that is a write-ownership
            // answer and not a coverage claim.
            if !binary_flat_frontier_covers(&binary_last_written, key) {
                continue;
            }

            // "Zero means absent", enforced at the writer because that is where
            // the invariant is. A leaf whose encoding is 32 zero bytes was
            // *removed* from the trie, so the mirror must delete the row, not
            // store zeros. Storing them would put a row in the mirror for a key
            // the trie's root does not commit to; a range served from the mirror
            // and proved against that root fails on it. The empty value is the
            // tombstone; 32 zero bytes is a corruption signal, and refusing the
            // whole commit is the right response to one.
            //
            // The same check `BackendBinaryFlatDB::put_batch` makes, repeated
            // here because this path writes through `write_tx` directly — it has
            // to, to stay in one transaction with the nodes — and so does not go
            // through that writer.
            if !value.is_empty() && value.iter().all(|byte| *byte == 0) {
                result = Err(StoreError::Custom(format!(
                    "refusing to commit a 32-zero-byte binary flat value for key {key:?} at \
                     block {}: zero means absent, so the trie removed this leaf and the row \
                     must be deleted with an empty value, not written as zeros",
                    layer.block_number
                )));
                break 'layers;
            }

            // Pre-image first, from the intra-batch map or disk, exactly as the
            // two loops above do.
            let prev_value = if !is_batch {
                match binary_flat_overlay.get(key) {
                    Some(v) => Some(v.clone()),
                    None => match read_view.get(BINARY_FLATKEYVALUE, key) {
                        Ok(v) => Some(v),
                        Err(e) => {
                            result = Err(e);
                            break 'layers;
                        }
                    },
                }
            } else {
                None
            };

            let new_value = if value.is_empty() {
                // Tombstone: the leaf left the tree. The row must go, not become
                // zeros — see above.
                result = write_tx.delete(BINARY_FLATKEYVALUE, key);
                None
            } else {
                result = write_tx.put(BINARY_FLATKEYVALUE, key, value);
                Some(value.clone())
            };
            if result.is_err() {
                break 'layers;
            }

            if let Some(prev) = prev_value {
                journal_binary_flat.push((key.clone(), prev));
                binary_flat_overlay.insert(key.clone(), new_value);
            }
        }

        // Reconciliation: BEFORE writing this block's journal entry, wipe the
        // obsolete OLD-chain entries in `[T, D]` so the new T entry (below) isn't clobbered
        // by the range delete. Fires only for the single reconciliation layer at height T.
        // T = `overlay.to_block()`, D = `overlay.from_block()`.
        if !is_batch && let Some((t, d)) = reorg_heights {
            debug_assert_eq!(
                layer.block_number, t,
                "first new-chain commit must be at the pivot's T height (overlay.to_block)"
            );
            // Only reconcile when this committed layer is exactly the pivot height `t`.
            // The `delete_range` wipes the OLD-chain journal entries `[t, d]`; if the layer
            // is not at `t` the range delete would drop new-chain entries that belong in
            // `[t, d]` and leave gaps that break later reorg recovery. The debug_assert
            // catches this loudly in tests; in release we skip (and log) rather than corrupt
            // STATE_HISTORY.
            if layer.block_number == t {
                let start = t.to_be_bytes();
                let end = d.saturating_add(1).to_be_bytes();
                result = write_tx.delete_range(STATE_HISTORY, &start, &end);
                if result.is_err() {
                    break 'layers;
                }
            } else {
                error!(
                    block_number = layer.block_number,
                    t,
                    d,
                    "deep-reorg reconciliation skipped: committed layer not at pivot height; skipping STATE_HISTORY delete_range to avoid history gaps"
                );
            }
        }

        // Stage this block's journal entry into the same write batch as the trie/flat-KV
        // overwrites. `put` is buffered until `commit`, so all CFs and every block's entry
        // land atomically (or none do on commit failure). Each entry is keyed and identified
        // by its own COMMITTED block, not the in-flight block whose insertion triggered this
        // commit (that block commits later, one cadence behind).
        if !is_batch {
            let entry = JournalEntry {
                block_hash: layer.block_hash,
                parent_state_root: layer.parent_state_root,
                parent_binary_root: layer.parent_binary_root,
                account_trie_diff: journal_account_trie,
                storage_trie_diff: journal_storage_trie,
                account_flat_diff: journal_account_flat,
                storage_flat_diff: journal_storage_flat,
                binary_trie_diff: journal_binary_trie,
                binary_flat_diff: journal_binary_flat,
            };
            result = write_tx.put(
                STATE_HISTORY,
                &layer.block_number.to_be_bytes(),
                &entry.encode(),
            );
            if result.is_err() {
                break 'layers;
            }
        }
    }

    if result.is_ok() {
        result = write_tx.commit();
    }
    // We want to send this message even if there was an error during the batch write
    let _ = fkv_ctl.send(FKVGeneratorControlMessage::Continue);
    result?;

    // Reconciliation succeeded: drop the overlay from the cache. Subsequent
    // commits revert to the normal one-block path.
    if overlay_for_reconciliation.is_some() {
        trie_mut.clear_overlay();
    }

    // Phase 3: update diff layers with the removal of bottom layer.
    //
    // SAFETY: `install_overlay_for_reorg` rendezvous-pings this worker before
    // swapping `trie_cache`, so no concurrent RCU can race the write-back here.
    // See the Ping comment in `install_overlay_for_reorg`.
    *trie_cache.write().map_err(|_| StoreError::LockError)? = Arc::new(trie_mut);
    Ok(())
}

// NOTE: we don't receive `Store` here to avoid cyclic dependencies
// with the other end of `control_rx`
fn flatkeyvalue_generator(
    backend: &Arc<dyn StorageBackend>,
    last_computed_fkv: &RwLock<Vec<u8>>,
    control_rx: &std::sync::mpsc::Receiver<FKVGeneratorControlMessage>,
) -> Result<(), StoreError> {
    info!("Generation of FlatKeyValue started.");
    let initial_last_written = backend
        .begin_read()?
        .get(MISC_VALUES, "last_written".as_bytes())?
        .unwrap_or_default();

    if initial_last_written.is_empty() {
        // First time generating the FKV. Remove all FKV entries just in case
        backend.clear_table(ACCOUNT_FLATKEYVALUE)?;
        backend.clear_table(STORAGE_FLATKEYVALUE)?;
    } else if initial_last_written == [0xff] {
        // FKV was already generated
        info!("FlatKeyValue already generated. Skipping.");
        return Ok(());
    }

    loop {
        // Acquire a fresh read view per iteration so updates performed while the
        // generator is paused are visible after a Continue signal.
        let read_tx = backend.begin_read()?;
        let root = read_tx
            .get(ACCOUNT_TRIE_NODES, &[])?
            .ok_or(StoreError::MissingLatestBlockNumber)?;
        let root: Node = ethrex_trie::Node::decode(&root)?;
        let state_root = root.compute_hash(&NativeCrypto).finalize(&NativeCrypto);

        let last_written = read_tx
            .get(MISC_VALUES, "last_written".as_bytes())?
            .unwrap_or_default();
        let last_written_account = last_written
            .get(0..64)
            .map(|v| Nibbles::from_hex(v.to_vec()))
            .unwrap_or_default();
        let mut last_written_storage = last_written
            .get(66..130)
            .map(|v| Nibbles::from_hex(v.to_vec()))
            .unwrap_or_default();

        debug!("Starting FlatKeyValue loop pivot={last_written:?} SR={state_root:x}");

        let mut ctr = 0;
        let mut write_txn = backend.begin_write()?;
        let mut iter = Trie::open(
            Box::new(BackendTrieDB::new_for_accounts_with_view(
                backend.clone(),
                read_tx.clone(),
                last_written.clone(),
            )?),
            state_root,
        )
        .into_iter();
        if last_written_account > Nibbles::default() {
            iter.advance(last_written_account.to_bytes())?;
        }
        let res = iter.try_for_each(|(path, node)| -> Result<(), StoreError> {
            let Node::Leaf(node) = node else {
                return Ok(());
            };
            let account_state = AccountState::decode(&node.value)?;
            let account_hash = H256::from_slice(&path.to_bytes());
            write_txn.put(MISC_VALUES, "last_written".as_bytes(), path.as_ref())?;
            write_txn.put(ACCOUNT_FLATKEYVALUE, path.as_ref(), &node.value)?;
            ctr += 1;
            if ctr > 10_000 {
                write_txn.commit()?;
                write_txn = backend.begin_write()?;
                *last_computed_fkv
                    .write()
                    .map_err(|_| StoreError::LockError)? = path.as_ref().to_vec();
                ctr = 0;
            }

            let mut iter_inner = Trie::open(
                Box::new(BackendTrieDB::new_for_account_storage_with_view(
                    backend.clone(),
                    read_tx.clone(),
                    account_hash,
                    path.as_ref().to_vec(),
                )?),
                account_state.storage_root,
            )
            .into_iter();
            if last_written_storage > Nibbles::default() {
                iter_inner.advance(last_written_storage.to_bytes())?;
                last_written_storage = Nibbles::default();
            }
            iter_inner.try_for_each(|(path, node)| -> Result<(), StoreError> {
                let Node::Leaf(node) = node else {
                    return Ok(());
                };
                let key = apply_prefix(Some(account_hash), path);
                write_txn.put(MISC_VALUES, "last_written".as_bytes(), key.as_ref())?;
                write_txn.put(STORAGE_FLATKEYVALUE, key.as_ref(), &node.value)?;
                ctr += 1;
                if ctr > 10_000 {
                    write_txn.commit()?;
                    write_txn = backend.begin_write()?;
                    *last_computed_fkv
                        .write()
                        .map_err(|_| StoreError::LockError)? = key.into_vec();
                    ctr = 0;
                }
                fkv_check_for_stop_msg(control_rx)?;
                Ok(())
            })?;
            fkv_check_for_stop_msg(control_rx)?;
            Ok(())
        });
        match res {
            Err(StoreError::PivotChanged) => {
                match control_rx.recv() {
                    Ok(FKVGeneratorControlMessage::Continue) => {}
                    Ok(FKVGeneratorControlMessage::Stop) => {
                        return Err(StoreError::Custom("Unexpected Stop message".to_string()));
                    }
                    // If the channel was closed, we stop generation prematurely
                    Err(std::sync::mpsc::RecvError) => {
                        info!("Store closed, stopping FlatKeyValue generation.");
                        return Ok(());
                    }
                }
            }
            Err(err) => return Err(err),
            Ok(()) => {
                write_txn.put(MISC_VALUES, "last_written".as_bytes(), &[0xff])?;
                write_txn.commit()?;
                *last_computed_fkv
                    .write()
                    .map_err(|_| StoreError::LockError)? = vec![0xff; 131];
                info!("FlatKeyValue generation finished.");
                return Ok(());
            }
        };
    }
}

/// Leaves per write batch in the binary mirror's backfill sweep. The MPT
/// generator's figure, unchanged: large enough that the per-batch overhead
/// disappears, small enough that a stop is honoured promptly and a dropped
/// batch is cheap to redo.
const BINARY_FLAT_BATCH: usize = 10_000;

/// Backfill [`BINARY_FLATKEYVALUE`] from the on-disk binary trie, advancing the
/// durable frontier as it goes.
///
/// The MPT's `flatkeyvalue_generator`, transposed, with one structural
/// simplification: that sweep is *nested* — accounts outer, each account's
/// storage trie inner — which is the only reason the MPT needs a second,
/// stricter coverage predicate that slices `[0..64]` off its frontier. This
/// trie is one tree over one keyspace, so a single linear pass and a single
/// `key <= frontier` compare are sufficient (plan Decision 2).
///
/// **It runs on the MPT generator's thread and shares its control channel**,
/// which is the one thing the plan says not to copy about the MPT. That
/// channel is `sync_channel(0)` — a rendezvous — so `commit_to_disk`'s `Stop`
/// *blocks the persist worker* until the generator reaches its next
/// `try_recv`. A second independent rendezvous channel on that path would make
/// the worker wait on two generators in series, each between checks that can
/// sit behind a trie open. Sharing means one `Stop` pauses whichever sweep is
/// running and one `Continue` resumes it, with no new message on the critical
/// path at all.
///
/// Four behaviours are copied deliberately:
///
/// - **`clear_table` on a cold start.** An absent marker means no sweep has
///   ever run, so whatever is in the table is of unknown provenance and is
///   wiped rather than trusted.
/// - **The cursor rides in the same batch as the data.** One extra small write
///   per leaf buys crash safety by construction: the durable marker can never
///   run ahead of the durable rows it claims.
/// - **The in-memory frontier advances only at batch commit**, so it lags the
///   durable one and readers under-claim coverage — the conservative direction.
/// - **Start-once.** A `Continue` arriving mid-sweep is an error, exactly as it
///   is for the MPT: it would mean two drivers think they own this sweep.
fn binary_flat_generator(
    backend: &Arc<dyn StorageBackend>,
    binary_last_computed: &RwLock<Vec<u8>>,
    control_rx: &std::sync::mpsc::Receiver<FKVGeneratorControlMessage>,
) -> Result<(), StoreError> {
    binary_flat_generator_with_batch(backend, binary_last_computed, control_rx, BINARY_FLAT_BATCH)
}

/// [`binary_flat_generator`] with the batch size exposed, so a test can drive
/// several batches over a handful of leaves rather than needing ten thousand.
fn binary_flat_generator_with_batch(
    backend: &Arc<dyn StorageBackend>,
    binary_last_computed: &RwLock<Vec<u8>>,
    control_rx: &std::sync::mpsc::Receiver<FKVGeneratorControlMessage>,
    batch: usize,
) -> Result<(), StoreError> {
    let initial = backend
        .begin_read()?
        .get(MISC_VALUES, BINARY_LAST_WRITTEN_KEY)?
        .unwrap_or_default();
    if initial == BINARY_FLAT_FRONTIER_COMPLETE {
        info!("Binary flat mirror already generated. Skipping.");
        return Ok(());
    }
    // The cold-start wipe is *not* here, unlike the MPT's, and the difference
    // is forced by Task 6's write-ownership rule. The MPT reads an absent
    // marker as an all-zero frontier, so its commit path writes almost nothing
    // and a lazy `clear_table` discards almost nothing. The binary rule is the
    // opposite — absent means "the commit path owns the whole keyspace" — so a
    // wipe here would delete rows blocks imported since startup had already
    // written. Those rows are uncovered and re-derived by this sweep, so it is
    // not a correctness bug, but it is a race, and a background thread
    // truncating a table under the write path is not a thing to leave in.
    // `Store::from_backend` does the wipe once, before any block can commit.
    info!("Generation of the binary flat mirror started.");

    loop {
        // A fresh read view per pass, so state written while the sweep was
        // paused is visible after the Continue.
        let read_tx = backend.begin_read()?;
        let Some(encoded_root) = read_tx.get(BINARY_TRIE_NODES, &BitPath::new().to_db_key())?
        else {
            // No binary trie on disk: an unscheduled chain, or a scheduled one
            // whose genesis has not landed yet. Nothing to mirror, and marking
            // an empty keyspace complete here would be a claim about a trie
            // that does not exist.
            info!("No binary trie on disk; binary flat generation has nothing to do.");
            return Ok(());
        };
        let root = hash_stored_node(&encoded_root);

        let mut frontier = read_tx
            .get(MISC_VALUES, BINARY_LAST_WRITTEN_KEY)?
            .unwrap_or_default();
        debug!("Starting binary flat loop pivot={frontier:?} root={root:x}");

        let outcome = (|| -> Result<(), StoreError> {
            loop {
                // Reopened per batch, on purpose: the walk caches every node it
                // resolves, so one instance held across a whole sweep would
                // accumulate the entire trie in memory. A batch costs one
                // root-to-leaf descent to get back to the frontier.
                let mut trie = BinaryTrie::open(
                    Box::new(BackendBinaryTrieDB::with_view(
                        backend.clone(),
                        read_tx.clone(),
                    )),
                    root,
                );
                // `+ 1` when resuming: `leaves_from`'s origin is inclusive, so
                // the frontier leaf comes back and is dropped below.
                let resuming = !frontier.is_empty();
                let mut leaves = trie
                    .leaves_from(&frontier, batch + usize::from(resuming))
                    .map_err(|e| StoreError::Custom(format!("binary flat sweep failed: {e}")))?;
                if resuming && leaves.first().is_some_and(|(key, _)| key == &frontier) {
                    leaves.remove(0);
                }
                if leaves.is_empty() {
                    return Ok(());
                }

                let mut write_txn = backend.begin_write()?;
                for (key, value) in &leaves {
                    if value.iter().all(|byte| *byte == 0) {
                        return Err(StoreError::Custom(format!(
                            "binary trie holds a 32-zero-byte leaf at {key:?}: zero means absent, \
                             so this leaf should not exist"
                        )));
                    }
                    // Cursor first and in the same batch, so a crash between
                    // batches leaves the marker naming a row that is on disk.
                    write_txn.put(MISC_VALUES, BINARY_LAST_WRITTEN_KEY, key)?;
                    write_txn.put(BINARY_FLATKEYVALUE, key, value)?;
                    fkv_check_for_stop_msg(control_rx)?;
                }
                write_txn.commit()?;

                frontier = leaves.last().expect("the batch is non-empty").0.clone();
                // Only after the batch is durable: an in-memory frontier ahead
                // of disk would have readers trusting rows a crash would take
                // back.
                *binary_last_computed
                    .write()
                    .map_err(|_| StoreError::LockError)? = frontier.clone();
            }
        })();

        match outcome {
            Err(StoreError::PivotChanged) => match control_rx.recv() {
                Ok(FKVGeneratorControlMessage::Continue) => {}
                Ok(FKVGeneratorControlMessage::Stop) => {
                    return Err(StoreError::Custom("Unexpected Stop message".to_string()));
                }
                Err(std::sync::mpsc::RecvError) => {
                    info!("Store closed, stopping binary flat generation.");
                    return Ok(());
                }
            },
            Err(err) => return Err(err),
            Ok(()) => {
                let mut write_txn = backend.begin_write()?;
                write_txn.put(
                    MISC_VALUES,
                    BINARY_LAST_WRITTEN_KEY,
                    BINARY_FLAT_FRONTIER_COMPLETE,
                )?;
                write_txn.commit()?;
                *binary_last_computed
                    .write()
                    .map_err(|_| StoreError::LockError)? = BINARY_FLAT_FRONTIER_COMPLETE.to_vec();
                info!("Binary flat mirror generation finished.");
                return Ok(());
            }
        }
    }
}

fn fkv_check_for_stop_msg(
    control_rx: &std::sync::mpsc::Receiver<FKVGeneratorControlMessage>,
) -> Result<(), StoreError> {
    match control_rx.try_recv() {
        Ok(FKVGeneratorControlMessage::Stop) | Err(TryRecvError::Disconnected) => {
            return Err(StoreError::PivotChanged);
        }
        Ok(FKVGeneratorControlMessage::Continue) => {
            return Err(StoreError::Custom(
                "Unexpected Continue message".to_string(),
            ));
        }
        Err(TryRecvError::Empty) => {}
    }
    Ok(())
}

fn state_trie_locked_backend(
    backend: &dyn StorageBackend,
    last_written: Vec<u8>,
) -> Result<BackendTrieDBLocked, StoreError> {
    // No address prefix for state trie
    BackendTrieDBLocked::new(backend, last_written)
}

#[derive(Debug)]
pub struct AccountProof {
    pub proof: Vec<NodeRLP>,
    pub account: AccountState,
    pub storage_proof: Vec<StorageSlotProof>,
}

#[derive(Debug)]
pub struct StorageSlotProof {
    pub proof: Vec<NodeRLP>,
    pub key: H256,
    pub value: U256,
}

pub struct AncestorIterator {
    store: Store,
    next_hash: BlockHash,
}

impl Iterator for AncestorIterator {
    type Item = Result<(BlockHash, BlockHeader), StoreError>;

    fn next(&mut self) -> Option<Self::Item> {
        let next_hash = self.next_hash;
        // Buffer-aware: a not-yet-flushed ancestor (e.g. on a side branch during
        // a reorg) must be visible here, or a BLOCKHASH opcode resolving through
        // this walk would wrongly reject a valid block.
        match self.store.get_block_header_by_hash(next_hash) {
            Ok(Some(header)) => {
                let ret_hash = self.next_hash;
                self.next_hash = header.parent_hash;
                Some(Ok((ret_hash, header)))
            }
            Ok(None) => None,
            Err(e) => Some(Err(e)),
        }
    }
}

pub fn hash_address(address: &Address) -> Vec<u8> {
    keccak_hash(address.to_fixed_bytes()).to_vec()
}

fn hash_address_fixed(address: &Address) -> H256 {
    keccak(address.to_fixed_bytes())
}

pub fn hash_key(key: &H256) -> Vec<u8> {
    keccak_hash(key.to_fixed_bytes()).to_vec()
}

pub fn hash_key_fixed(key: &H256) -> [u8; 32] {
    keccak_hash(key.to_fixed_bytes())
}

fn chain_data_key(index: ChainDataIndex) -> Vec<u8> {
    (index as u8).encode_to_vec()
}

fn snap_state_key(index: SnapStateIndex) -> Vec<u8> {
    (index as u8).encode_to_vec()
}

/// Builds a fixed-width RECEIPTS key: block_hash (32B) || index (8B BE).
pub fn receipt_key(block_hash: &BlockHash, index: u64) -> Vec<u8> {
    let mut key = Vec::with_capacity(40);
    key.extend_from_slice(block_hash.as_bytes());
    key.extend_from_slice(&index.to_be_bytes());
    key
}

fn encode_code(code: &Code) -> Vec<u8> {
    let mut buf =
        Vec::with_capacity(6 + code.len() + std::mem::size_of_val::<[u32]>(&code.jump_targets));
    code.code().encode(&mut buf);
    // `Arc<[u32]>` (the in-memory share) has no `RLPEncode` impl; encode through an
    // owned `Vec` on this cold DB-write path (code is persisted once per hash).
    code.jump_targets.to_vec().encode(&mut buf);
    buf
}

#[derive(Debug, Default, Clone)]
struct LatestBlockHeaderCache {
    current: Arc<Mutex<Arc<BlockHeader>>>,
}

impl LatestBlockHeaderCache {
    pub fn get(&self) -> Arc<BlockHeader> {
        self.current.lock().expect("poisoned mutex").clone()
    }

    pub fn update(&self, header: BlockHeader) {
        let new = Arc::new(header);
        *self.current.lock().expect("poisoned mutex") = new;
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StoreMetadata {
    pub schema_version: u64,
}

impl StoreMetadata {
    pub fn new(schema_version: u64) -> Self {
        Self { schema_version }
    }
}

/// Reads the schema version from the metadata file, if it exists.
///
/// Returns `Some(version)` when metadata.json is present and valid,
/// or `None` when the file does not exist.
fn read_store_schema_version(path: &Path) -> Result<Option<u64>, StoreError> {
    let metadata_path = path.join(STORE_METADATA_FILENAME);
    if !metadata_path.exists() {
        return Ok(None);
    }
    if !metadata_path.is_file() {
        return Err(StoreError::Custom(
            "store schema path exists but is not a file".to_string(),
        ));
    }
    let file_contents = std::fs::read_to_string(metadata_path)?;
    let metadata: StoreMetadata = serde_json::from_str(&file_contents)?;
    Ok(Some(metadata.schema_version))
}

fn init_metadata_file(parent_path: &Path) -> Result<(), StoreError> {
    std::fs::create_dir_all(parent_path)?;

    let metadata_path = parent_path.join(STORE_METADATA_FILENAME);
    let metadata = StoreMetadata::new(STORE_SCHEMA_VERSION);
    let serialized_metadata = serde_json::to_string_pretty(&metadata)?;
    let mut new_file = std::fs::File::create_new(metadata_path)?;
    new_file.write_all(serialized_metadata.as_bytes())?;
    Ok(())
}

/// Returns `true` if `path` contains a *legacy* database — one written before
/// the metadata file existed, so it has no `metadata.json` to identify it.
/// Detected by RocksDB's own marker files, as opposed to unrelated files that
/// merely share the datadir. Only meaningful once metadata has been confirmed
/// absent; otherwise prefer `has_valid_db`, which keys off the metadata file.
///
/// Previously the caller treated *any* non-empty directory as such a legacy
/// database, which made startup fail when unrelated files lived alongside the DB
/// — e.g. EthDocker writes the JWT secret into the datadir (issue #5680). We
/// instead look for RocksDB's marker files, so a datadir that only contains such
/// unrelated files is correctly treated as fresh.
fn dir_contains_legacy_db(path: &Path) -> Result<bool, StoreError> {
    // `CURRENT` has a fixed name and is written by every RocksDB instance, so
    // check for it directly instead of scanning a datadir that may hold many
    // unrelated files.
    if path.join("CURRENT").is_file() {
        return Ok(true);
    }
    // The manifest has a numeric suffix (`MANIFEST-<n>`), so it can only be
    // found by scanning. Restrict to plain files: a directory that happens to
    // share the name is not a database marker.
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        if entry.file_name().to_string_lossy().starts_with("MANIFEST-") {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Checks whether a valid (or migratable) database exists at the given path
/// by looking for a metadata.json file with a schema version between 1 and
/// `STORE_SCHEMA_VERSION` (inclusive).
pub fn has_valid_db(path: &Path) -> bool {
    let metadata_path = path.join(STORE_METADATA_FILENAME);
    if !metadata_path.is_file() {
        return false;
    }
    let Ok(contents) = std::fs::read_to_string(&metadata_path) else {
        return false;
    };
    let Ok(metadata) = serde_json::from_str::<StoreMetadata>(&contents) else {
        return false;
    };
    metadata.schema_version >= 1 && metadata.schema_version <= STORE_SCHEMA_VERSION
}

/// Reads the chain ID from an existing database without performing a full
/// store initialization. Returns `None` if the database doesn't exist or
/// the chain config can't be read. Always returns `None` when compiled
/// without the `rocksdb` feature.
///
/// Each failure mode logs a warning so callers (and operators) can diagnose
/// why an existing database was not usable — previously every error was
/// silently swallowed by `.ok()?`.
pub fn read_chain_id_from_db(path: &Path) -> Option<u64> {
    if !has_valid_db(path) {
        return None;
    }
    #[cfg(feature = "rocksdb")]
    {
        // The cache size is irrelevant for this one-shot chain-id read (the LRU
        // is sized as a ceiling, not pre-allocated), so we use the default.
        let backend = match RocksDBBackend::open(path, DEFAULT_ROCKSDB_BLOCK_CACHE_SIZE_BYTES) {
            Ok(backend) => backend,
            Err(e) => {
                warn!("Failed to open RocksDB at {path:?} to read chain ID: {e}");
                return None;
            }
        };
        let read = match backend.begin_read() {
            Ok(read) => read,
            Err(e) => {
                warn!("Failed to begin read transaction at {path:?}: {e}");
                return None;
            }
        };
        let key = chain_data_key(ChainDataIndex::ChainConfig);
        let bytes = match read.get(CHAIN_DATA, &key) {
            Ok(Some(bytes)) => bytes,
            Ok(None) => {
                warn!("Chain config entry not found in database at {path:?}");
                return None;
            }
            Err(e) => {
                warn!("Failed to read chain config from database at {path:?}: {e}");
                return None;
            }
        };
        // Only extract chain_id here: the stored `ChainConfig` JSON may include
        // fields whose serialization changed across releases (e.g. pre-v10 wrote
        // `terminal_total_difficulty` as a plain number, v10 expects hex string).
        // Deserializing the full struct would reject otherwise-migratable v9 data.
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct ChainIdOnly {
            chain_id: u64,
        }
        match serde_json::from_slice::<ChainIdOnly>(&bytes) {
            Ok(partial) => Some(partial.chain_id),
            Err(e) => {
                warn!("Failed to deserialize chain ID from database at {path:?}: {e}");
                None
            }
        }
    }
    #[cfg(not(feature = "rocksdb"))]
    {
        let _ = path;
        None
    }
}

#[cfg(test)]
mod state_history_tests {
    use super::*;
    use crate::api::tables::STATE_HISTORY;
    use crate::backend::in_memory::InMemoryBackend;
    use crate::journal::JournalEntry;
    use ethrex_common::types::{BlockBody, BlockHeader};
    use ethrex_trie::Nibbles;
    use std::time::{Duration, Instant};

    fn make_block(number: BlockNumber, parent_hash: H256, state_root: H256) -> Block {
        let header = BlockHeader {
            number,
            parent_hash,
            state_root,
            ..Default::default()
        };
        Block::new(header, BlockBody::default())
    }

    /// Polls `STATE_HISTORY` for an entry at the given block number, up to `timeout`.
    /// The trie worker commits to disk asynchronously after `store_block_updates`
    /// returns, so a small wait window is required.
    fn await_journal_entry(
        backend: &Arc<dyn StorageBackend>,
        block_number: BlockNumber,
        timeout: Duration,
    ) -> Option<Vec<u8>> {
        let key = block_number.to_be_bytes();
        let deadline = Instant::now() + timeout;
        loop {
            let read = backend.begin_read().ok()?;
            if let Ok(Some(v)) = read.get(STATE_HISTORY, &key) {
                return Some(v);
            }
            if Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Asserts no STATE_HISTORY entry materializes for `block_number` within the
    /// given window. Polls repeatedly; if an entry appears at any poll, fails
    /// loudly. Absence at every poll over the full window counts as verified.
    /// More robust than a single fixed sleep under CI load.
    fn assert_no_journal_entry(backend: &Arc<dyn StorageBackend>, block_number: BlockNumber) {
        let window = Duration::from_millis(500);
        let key = block_number.to_be_bytes();
        let deadline = Instant::now() + window;
        loop {
            let read = backend.begin_read().expect("read view");
            let v = read.get(STATE_HISTORY, &key).expect("get");
            assert!(
                v.is_none(),
                "expected no STATE_HISTORY entry for block {block_number}, got {v:?}"
            );
            if Instant::now() >= deadline {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Live commits gate on the canonical safe-commit root cell (see `get_commitable`),
    /// which only advances via forkchoice. We simulate that by calling
    /// `set_safe_commit_root` to the parent block's state root before storing the next
    /// block: storing block N+1 then commits block N's layer to disk, producing one
    /// journal entry per committed block. We verify entries for block 1 and block 2.
    #[test]
    fn journal_entry_written_per_block_in_regular_mode() {
        let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::open().unwrap());
        let dir = tempfile::tempdir().unwrap();
        let store = Store::from_backend(
            backend.clone(),
            dir.path().to_path_buf(),
            1,
            DEFAULT_PERSIST_CHANNEL_CAPACITY,
        )
        .unwrap();

        let state_root_1 = H256::repeat_byte(0x11);
        let block1 = make_block(1, H256::zero(), state_root_1);
        let block1_hash = block1.hash();
        store
            .store_block_updates(UpdateBatch {
                account_updates: vec![(Nibbles::from_raw(&[0x00, 0x01], false), vec![0xab, 0xcd])],
                storage_updates: vec![],
                blocks: vec![block1],
                receipts: vec![],
                code_updates: vec![],
                binary_update: None,
                commit_depth: None,
                wait_for_flush: false,
            })
            .unwrap();

        // Advance the safe-commit root to block 1's state root, then store block 2:
        // the canonical gate now finds block 1's layer committable and flushes it.
        store.set_safe_commit_root(state_root_1).unwrap();
        let state_root_2 = H256::repeat_byte(0x22);
        let block2 = make_block(2, block1_hash, state_root_2);
        let block2_hash = block2.hash();
        store
            .store_block_updates(UpdateBatch {
                account_updates: vec![(Nibbles::from_raw(&[0x00, 0x02], false), vec![0xef, 0x11])],
                storage_updates: vec![],
                blocks: vec![block2],
                receipts: vec![],
                code_updates: vec![],
                binary_update: None,
                commit_depth: None,
                wait_for_flush: false,
            })
            .unwrap();

        let bytes = await_journal_entry(&backend, 1, Duration::from_secs(2))
            .expect("STATE_HISTORY entry for block 1 should appear after block 2 commits it");
        let entry = JournalEntry::decode(&bytes).unwrap();
        assert_eq!(entry.block_hash, block1_hash);
        assert_eq!(entry.parent_state_root, H256::zero());
        assert!(!entry.account_trie_diff.is_empty());
        let (path, prev) = &entry.account_trie_diff[0];
        assert_eq!(prev, &None, "first-time write means previous value is None");
        assert!(path.len() < 65);

        // Advance the safe-commit root to block 2's state root, then store block 3:
        // block 2's layer is now committable.
        store.set_safe_commit_root(state_root_2).unwrap();
        let state_root_3 = H256::repeat_byte(0x33);
        let block3 = make_block(3, block2_hash, state_root_3);
        store
            .store_block_updates(UpdateBatch {
                account_updates: vec![(Nibbles::from_raw(&[0x00, 0x03], false), vec![0x77])],
                storage_updates: vec![],
                blocks: vec![block3],
                receipts: vec![],
                code_updates: vec![],
                binary_update: None,
                commit_depth: None,
                wait_for_flush: false,
            })
            .unwrap();

        let bytes = await_journal_entry(&backend, 2, Duration::from_secs(2))
            .expect("STATE_HISTORY entry for block 2 should appear after block 3 commits it");
        let entry = JournalEntry::decode(&bytes).unwrap();
        assert_eq!(entry.block_hash, block2_hash);
        assert_eq!(entry.parent_state_root, state_root_1);
        assert!(!entry.account_trie_diff.is_empty());
    }

    /// A depth-gated commit that is NOT in batch mode SHALL journal, and its entries
    /// SHALL carry the committed layer's own identity and pre-image.
    ///
    /// `commit_depth: Some(d)` together with `wait_for_flush: false` — the startup
    /// regeneration and sync/import tail paths — is a combination that did not exist while
    /// the commit gate and the ack timing were a single flag, so no other test covers it.
    /// Journaling follows `wait_for_flush`, so deriving it from `commit_depth.is_some()`
    /// instead would silently stop journaling on exactly these paths; this test fails in
    /// that case.
    #[test]
    fn depth_gated_per_block_commit_journals_with_correct_pre_image() {
        let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::open().unwrap());
        let dir = tempfile::tempdir().unwrap();
        let store = Store::from_backend(
            backend.clone(),
            dir.path().to_path_buf(),
            1,
            DEFAULT_PERSIST_CHANNEL_CAPACITY,
        )
        .unwrap();

        // One shared key rewritten every block, so pre-images are checkable.
        let shared = Nibbles::from_raw(&[0x0a, 0x0b], false);
        let mut prev_hash = H256::zero();
        let mut hashes = vec![H256::zero()];
        let mut roots = vec![H256::zero()];
        for n in 1..=6u64 {
            let state_root = H256::repeat_byte(0xc0 | (n as u8));
            let block = make_block(n, prev_hash, state_root);
            prev_hash = block.hash();
            hashes.push(prev_hash);
            roots.push(state_root);
            store
                .store_block_updates(UpdateBatch {
                    account_updates: vec![(shared.clone(), vec![0xb0 | (n as u8)])],
                    storage_updates: vec![],
                    blocks: vec![block],
                    receipts: vec![],
                    code_updates: vec![],
                    binary_update: None,
                    commit_depth: Some(3),
                    wait_for_flush: false,
                })
                .unwrap();
        }

        let bytes = await_journal_entry(&backend, 1, Duration::from_secs(2))
            .expect("a depth-gated per-block commit must journal");
        let entry = JournalEntry::decode(&bytes).unwrap();
        assert_eq!(
            entry.block_hash, hashes[1],
            "entry 1 carries block 1's identity"
        );
        assert_eq!(entry.parent_state_root, H256::zero());
        assert_eq!(
            entry.account_trie_diff[0].1, None,
            "block 1 first-writes the key, so its pre-image is absence"
        );

        let bytes =
            await_journal_entry(&backend, 2, Duration::from_secs(2)).expect("entry for block 2");
        let entry = JournalEntry::decode(&bytes).unwrap();
        assert_eq!(
            entry.block_hash, hashes[2],
            "entry 2 carries block 2's identity"
        );
        assert_eq!(
            entry.parent_state_root, roots[1],
            "entry 2's parent_state_root is block 1's state root"
        );
        assert_eq!(
            entry.account_trie_diff[0].1,
            Some(vec![0xb0 | 1u8]),
            "block 2's pre-image is the value block 1 wrote"
        );
    }

    /// The bespoke batch path SHALL skip the journal entirely. To actually exercise the
    /// gating we push enough batches to trigger a commit under
    /// `BATCH_COMMIT_THRESHOLD = 4`, then verify no STATE_HISTORY entry materializes
    /// despite the commit happening.
    ///
    /// That path is `wait_for_flush: true` with `commit_depth: Some(BATCH_COMMIT_THRESHOLD)`
    /// — the combination the single `batch_mode` flag used to stand for. Journaling follows
    /// `wait_for_flush`, so the depth-gated per-block paths are not covered here.
    #[test]
    fn journal_skipped_in_batch_mode() {
        let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::open().unwrap());
        let dir = tempfile::tempdir().unwrap();
        let store = Store::from_backend(
            backend.clone(),
            dir.path().to_path_buf(),
            1,
            DEFAULT_PERSIST_CHANNEL_CAPACITY,
        )
        .unwrap();

        let mut prev_hash = H256::zero();
        for n in 1..=5u64 {
            let state_root = H256::repeat_byte(0xa0 | (n as u8));
            let block = make_block(n, prev_hash, state_root);
            prev_hash = block.hash();
            store
                .store_block_updates(UpdateBatch {
                    account_updates: vec![(
                        Nibbles::from_raw(&[n as u8], false),
                        vec![0xde, 0xad, n as u8],
                    )],
                    storage_updates: vec![],
                    blocks: vec![block],
                    receipts: vec![],
                    code_updates: vec![],
                    binary_update: None,
                    commit_depth: Some(BATCH_COMMIT_THRESHOLD),
                    wait_for_flush: true,
                })
                .unwrap();
        }

        for n in 1..=5u64 {
            assert_no_journal_entry(&backend, n);
        }
    }

    /// Storage trie updates SHALL appear in `storage_trie_diff` (not
    /// `account_trie_diff`), with their on-disk keys as written.
    #[test]
    fn journal_storage_updates_appear_in_storage_diff() {
        let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::open().unwrap());
        let dir = tempfile::tempdir().unwrap();
        let store = Store::from_backend(
            backend.clone(),
            dir.path().to_path_buf(),
            1,
            DEFAULT_PERSIST_CHANNEL_CAPACITY,
        )
        .unwrap();

        let account_hash_a = H256::repeat_byte(0xa0);
        let account_hash_b = H256::repeat_byte(0xb0);

        let state_root_1 = H256::repeat_byte(0x33);
        let block1 = make_block(1, H256::zero(), state_root_1);
        let block1_hash = block1.hash();
        store
            .store_block_updates(UpdateBatch {
                account_updates: vec![],
                storage_updates: vec![
                    (
                        account_hash_a,
                        vec![(Nibbles::from_raw(&[0x05], false), vec![0x01])],
                    ),
                    (
                        account_hash_b,
                        vec![(Nibbles::from_raw(&[0x06], false), vec![0x02])],
                    ),
                ],
                blocks: vec![block1],
                receipts: vec![],
                code_updates: vec![],
                binary_update: None,
                commit_depth: None,
                wait_for_flush: false,
            })
            .unwrap();

        // Advance the safe-commit root to block 1's state root, then store block 2:
        // block 1's layer is now committable and flushes to disk.
        store.set_safe_commit_root(state_root_1).unwrap();
        let state_root_2 = H256::repeat_byte(0x44);
        let block2 = make_block(2, block1_hash, state_root_2);
        store
            .store_block_updates(UpdateBatch {
                account_updates: vec![(Nibbles::from_raw(&[0xee], false), vec![0xff])],
                storage_updates: vec![],
                blocks: vec![block2],
                receipts: vec![],
                code_updates: vec![],
                binary_update: None,
                commit_depth: None,
                wait_for_flush: false,
            })
            .unwrap();

        let bytes = await_journal_entry(&backend, 1, Duration::from_secs(2))
            .expect("STATE_HISTORY entry for block 1");
        let entry = JournalEntry::decode(&bytes).unwrap();
        assert_eq!(entry.block_hash, block1_hash);
        assert_eq!(
            entry.storage_trie_diff.len(),
            2,
            "two distinct account hashes must produce two storage_trie entries"
        );
        for (_key, prev) in &entry.storage_trie_diff {
            assert_eq!(prev, &None, "first-time storage write has None pre-image");
        }
        assert!(entry.account_trie_diff.is_empty());
    }

    fn seed_journal_entries(backend: &Arc<dyn StorageBackend>, block_numbers: &[BlockNumber]) {
        let mut tx = backend.begin_write().unwrap();
        for n in block_numbers {
            let entry = JournalEntry {
                block_hash: H256::repeat_byte(*n as u8),
                parent_state_root: H256::zero(),
                parent_binary_root: H256::zero(),
                account_trie_diff: vec![(vec![*n as u8], None)],
                storage_trie_diff: vec![],
                account_flat_diff: vec![],
                storage_flat_diff: vec![],
                binary_trie_diff: vec![],
                binary_flat_diff: vec![],
            };
            tx.put(STATE_HISTORY, &n.to_be_bytes(), &entry.encode())
                .unwrap();
        }
        tx.commit().unwrap();
    }

    fn journal_entry_exists(backend: &Arc<dyn StorageBackend>, block_number: BlockNumber) -> bool {
        backend
            .begin_read()
            .unwrap()
            .get(STATE_HISTORY, &block_number.to_be_bytes())
            .unwrap()
            .is_some()
    }

    /// Finality advance SHALL prune every STATE_HISTORY entry at or below the
    /// new finalized number, in the same atomic txn as the finalized-number update.
    #[tokio::test]
    async fn finality_advance_prunes_journal_below_boundary() {
        let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::open().unwrap());
        let dir = tempfile::tempdir().unwrap();
        let store = Store::from_backend(
            backend.clone(),
            dir.path().to_path_buf(),
            1,
            DEFAULT_PERSIST_CHANNEL_CAPACITY,
        )
        .unwrap();

        seed_journal_entries(&backend, &(1..=10).collect::<Vec<_>>());
        for n in 1..=10 {
            assert!(journal_entry_exists(&backend, n), "seed entry {n} present");
        }

        store
            .forkchoice_update_inner(vec![], 100, H256::zero(), None, Some(5))
            .await
            .unwrap();

        for n in 1..=5 {
            assert!(
                !journal_entry_exists(&backend, n),
                "entry {n} should have been pruned (<= finalized)"
            );
        }
        for n in 6..=10 {
            assert!(
                journal_entry_exists(&backend, n),
                "entry {n} should remain (> finalized)"
            );
        }
    }

    /// Forkchoice updates that don't advance finalized SHALL NOT prune the journal.
    #[tokio::test]
    async fn finality_no_op_does_not_prune() {
        let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::open().unwrap());
        let dir = tempfile::tempdir().unwrap();
        let store = Store::from_backend(
            backend.clone(),
            dir.path().to_path_buf(),
            1,
            DEFAULT_PERSIST_CHANNEL_CAPACITY,
        )
        .unwrap();

        store
            .forkchoice_update_inner(vec![], 100, H256::zero(), None, Some(5))
            .await
            .unwrap();

        seed_journal_entries(&backend, &(6..=10).collect::<Vec<_>>());

        // FCU re-asserting finalized = 5: must not prune anything.
        store
            .forkchoice_update_inner(vec![], 100, H256::zero(), None, Some(5))
            .await
            .unwrap();

        for n in 6..=10 {
            assert!(
                journal_entry_exists(&backend, n),
                "entry {n} should still exist after no-op finality update"
            );
        }

        // FCU with finalized = None: also a no-op for pruning.
        store
            .forkchoice_update_inner(vec![], 100, H256::zero(), None, None)
            .await
            .unwrap();

        for n in 6..=10 {
            assert!(
                journal_entry_exists(&backend, n),
                "entry {n} should still exist when finalized is None"
            );
        }
    }

    /// A malformed (wrong-length) `FinalizedBlockNumber` value SHALL surface as a
    /// hard error rather than a silent fallback to 0, which would over-prune the
    /// journal on the next FCU.
    #[tokio::test]
    async fn malformed_finalized_returns_error_not_silent_zero() {
        let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::open().unwrap());
        let dir = tempfile::tempdir().unwrap();
        let store = Store::from_backend(
            backend.clone(),
            dir.path().to_path_buf(),
            1,
            DEFAULT_PERSIST_CHANNEL_CAPACITY,
        )
        .unwrap();

        // Plant a 4-byte (instead of 8-byte) FinalizedBlockNumber value.
        let mut tx = backend.begin_write().unwrap();
        let finalized_key = chain_data_key(ChainDataIndex::FinalizedBlockNumber);
        tx.put(CHAIN_DATA, &finalized_key, &[0u8, 0, 0, 0]).unwrap();
        tx.commit().unwrap();

        seed_journal_entries(&backend, &(1..=5).collect::<Vec<_>>());

        let err = store
            .forkchoice_update_inner(vec![], 100, H256::zero(), None, Some(3))
            .await
            .expect_err("malformed finalized must surface as an error");
        let msg = format!("{err}");
        assert!(
            msg.contains("FinalizedBlockNumber has unexpected length"),
            "expected length-mismatch error, got: {msg}"
        );

        // Journal must not have been pruned.
        for n in 1..=5 {
            assert!(
                journal_entry_exists(&backend, n),
                "entry {n} must NOT be pruned when finalized read failed"
            );
        }
    }

    // -----------------------------------------------------------------------
    // Deep-reorg primitive tests.
    // -----------------------------------------------------------------------

    /// `highest_state_history_block_number` SHALL return the max key present in
    /// `STATE_HISTORY`, or `None` when the table is empty.
    #[tokio::test]
    async fn highest_state_history_block_number_finds_max() {
        let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::open().unwrap());
        let dir = tempfile::tempdir().unwrap();
        let store = Store::from_backend(
            backend.clone(),
            dir.path().to_path_buf(),
            1,
            DEFAULT_PERSIST_CHANNEL_CAPACITY,
        )
        .unwrap();

        assert_eq!(store.highest_state_history_block_number().unwrap(), None);

        seed_journal_entries(&backend, &[3, 7, 5, 11, 8]);
        assert_eq!(
            store.highest_state_history_block_number().unwrap(),
            Some(11),
            "max over present entries"
        );
    }

    /// `lowest_state_history_block_number` SHALL return the min key present in
    /// `STATE_HISTORY`, or `None` when the table is empty. Phase 2's cap fallback
    /// depends on this when no finalized hash is known.
    #[tokio::test]
    async fn lowest_state_history_block_number_finds_min() {
        let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::open().unwrap());
        let dir = tempfile::tempdir().unwrap();
        let store = Store::from_backend(
            backend.clone(),
            dir.path().to_path_buf(),
            1,
            DEFAULT_PERSIST_CHANNEL_CAPACITY,
        )
        .unwrap();

        assert_eq!(store.lowest_state_history_block_number().unwrap(), None);

        seed_journal_entries(&backend, &[7, 3, 11, 5, 8]);
        assert_eq!(
            store.lowest_state_history_block_number().unwrap(),
            Some(3),
            "min over present entries"
        );
    }

    /// A stale-version (v1) entry as the previous binary wrote it: version byte
    /// 1, `block_hash`, `parent_state_root`, then four diff sections — no
    /// `parent_binary_root` and no binary section. The v1 encoder is gone, so
    /// the shape is written out by hand.
    fn encode_stale_journal_entry(block_number: BlockNumber) -> Vec<u8> {
        let mut bytes = vec![1u8];
        bytes.extend_from_slice(H256::repeat_byte(block_number as u8).as_bytes());
        bytes.extend_from_slice(H256::zero().as_bytes());
        // Four sections, all with a zero count.
        bytes.extend_from_slice(&[0u8; 4]);
        bytes
    }

    /// Seeds `STATE_HISTORY` on a raw backend, before any `Store` is built over it.
    fn seed_raw_journal_entries(
        backend: &Arc<dyn StorageBackend>,
        entries: &[(BlockNumber, Vec<u8>)],
    ) {
        let mut tx = backend.begin_write().unwrap();
        for (n, encoded) in entries {
            tx.put(STATE_HISTORY, &n.to_be_bytes(), encoded).unwrap();
        }
        tx.commit().unwrap();
    }

    /// Opening a store over a journal written entirely by a previous codec
    /// version SHALL drain it, so the floor `compute_reorg_ceiling` reads reports
    /// no journal reach at all.
    ///
    /// Without the drain the floor would be 10 and the ceiling would advertise
    /// reach back to block 9 — reach the decoder refuses to serve. The forkchoice
    /// update would be accepted and then fail mid-flight with `StateNotReachable`
    /// instead of being refused up front with `-38006 TooDeepReorg`.
    #[tokio::test]
    async fn startup_drains_an_all_stale_journal_and_clears_the_floor() {
        let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::open().unwrap());
        let dir = tempfile::tempdir().unwrap();
        seed_raw_journal_entries(
            &backend,
            &[
                (10, encode_stale_journal_entry(10)),
                (11, encode_stale_journal_entry(11)),
                (12, encode_stale_journal_entry(12)),
            ],
        );

        let store = Store::from_backend(
            backend.clone(),
            dir.path().to_path_buf(),
            1,
            DEFAULT_PERSIST_CHANNEL_CAPACITY,
        )
        .unwrap();

        assert_eq!(
            store.lowest_state_history_block_number().unwrap(),
            None,
            "an all-stale journal must be gone, leaving no journal reach to advertise"
        );
        assert_eq!(store.highest_state_history_block_number().unwrap(), None);
    }

    /// A node restarting a second time inside the upgrade window has already
    /// written current-version entries above the stale ones. Startup SHALL drain
    /// only the stale bottom and report the lowest survivor as the new floor —
    /// reach the node can actually deliver.
    #[tokio::test]
    async fn startup_drain_reports_the_lowest_surviving_entry_as_the_floor() {
        let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::open().unwrap());
        let dir = tempfile::tempdir().unwrap();
        seed_raw_journal_entries(
            &backend,
            &[
                (10, encode_stale_journal_entry(10)),
                (11, encode_stale_journal_entry(11)),
            ],
        );
        // Two entries the restarted binary wrote itself, in the current format.
        seed_journal_entries(&backend, &[12, 13]);

        let store = Store::from_backend(
            backend.clone(),
            dir.path().to_path_buf(),
            1,
            DEFAULT_PERSIST_CHANNEL_CAPACITY,
        )
        .unwrap();

        assert_eq!(
            store.lowest_state_history_block_number().unwrap(),
            Some(12),
            "the floor must be the lowest entry this binary can decode"
        );
        assert_eq!(
            store.highest_state_history_block_number().unwrap(),
            Some(13)
        );
        assert!(!journal_entry_exists(&backend, 10));
        assert!(!journal_entry_exists(&backend, 11));
    }

    /// A journal written entirely by this binary SHALL survive startup intact;
    /// the drain must not cost a node its reorg depth on an ordinary restart.
    #[tokio::test]
    async fn startup_leaves_a_current_version_journal_untouched() {
        let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::open().unwrap());
        let dir = tempfile::tempdir().unwrap();
        seed_journal_entries(&backend, &[10, 11, 12]);

        let store = Store::from_backend(
            backend.clone(),
            dir.path().to_path_buf(),
            1,
            DEFAULT_PERSIST_CHANNEL_CAPACITY,
        )
        .unwrap();

        assert_eq!(store.lowest_state_history_block_number().unwrap(), Some(10));
        assert_eq!(
            store.highest_state_history_block_number().unwrap(),
            Some(12)
        );
    }

    /// `install_overlay_for_reorg` SHALL atomically swap the layer cache for a
    /// fresh empty one with the overlay installed; layer-cache hits in the OLD
    /// cache MUST NOT survive the swap.
    #[tokio::test]
    async fn install_overlay_replaces_cache_with_fresh_one() {
        let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::open().unwrap());
        let dir = tempfile::tempdir().unwrap();
        let store = Store::from_backend(
            backend.clone(),
            dir.path().to_path_buf(),
            4,
            DEFAULT_PERSIST_CHANNEL_CAPACITY,
        )
        .unwrap();

        seed_journal_entries(&backend, &[3, 4]);

        // Pre-populate the cache with a sentinel layer to verify the swap discards it.
        {
            let mut guard = store.trie_cache.write().unwrap();
            let mut updated = (**guard).clone();
            updated.put_batch(
                H256::zero(),
                H256::repeat_byte(0xff),
                42,
                H256::repeat_byte(0x42),
                vec![],
            );
            *guard = Arc::new(updated);
        }
        assert!(
            store
                .is_state_in_layer_cache(H256::repeat_byte(0xff))
                .unwrap()
        );

        store
            .install_overlay_for_reorg(4, 3, |_| None)
            .expect("install must succeed");

        assert!(
            !store
                .is_state_in_layer_cache(H256::repeat_byte(0xff))
                .unwrap(),
            "old cache layer must be gone after swap"
        );
        let cache = store.trie_cache.read().unwrap().clone();
        assert!(cache.overlay().is_some(), "overlay must be installed");
        assert_eq!(
            cache.commit_threshold(),
            4,
            "fresh cache must inherit threshold"
        );
    }

    /// `install_overlay_for_reorg` SHALL leave the existing cache intact when
    /// overlay construction fails (missing journal entry).
    #[tokio::test]
    async fn install_overlay_failure_preserves_cache() {
        let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::open().unwrap());
        let dir = tempfile::tempdir().unwrap();
        let store = Store::from_backend(
            backend.clone(),
            dir.path().to_path_buf(),
            4,
            DEFAULT_PERSIST_CHANNEL_CAPACITY,
        )
        .unwrap();

        // Only seed block 5; constructor will walk down through 4 and 3 and fail.
        seed_journal_entries(&backend, &[5]);
        {
            let mut guard = store.trie_cache.write().unwrap();
            let mut updated = (**guard).clone();
            updated.put_batch(
                H256::zero(),
                H256::repeat_byte(0xaa),
                7,
                H256::repeat_byte(0x77),
                vec![],
            );
            *guard = Arc::new(updated);
        }
        assert!(
            store
                .is_state_in_layer_cache(H256::repeat_byte(0xaa))
                .unwrap()
        );

        let err = store
            .install_overlay_for_reorg(5, 3, |_| None)
            .expect_err("missing entries 3 and 4 must abort");
        let msg = format!("{err}");
        assert!(
            msg.contains("overlay construction failed"),
            "error should explain construction failure: {msg}"
        );

        // Cache must be intact: the sentinel layer must still be there.
        assert!(
            store
                .is_state_in_layer_cache(H256::repeat_byte(0xaa))
                .unwrap(),
            "cache must survive a failed overlay install"
        );
        let cache = store.trie_cache.read().unwrap().clone();
        assert!(cache.overlay().is_none(), "no overlay should be installed");
    }

    /// `abort_reorg` SHALL discard both the overlay and any partial new-chain
    /// layers, restoring the cache to a fresh empty state with the same
    /// commit threshold.
    #[tokio::test]
    async fn abort_reorg_resets_cache_to_fresh() {
        let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::open().unwrap());
        let dir = tempfile::tempdir().unwrap();
        let store = Store::from_backend(
            backend.clone(),
            dir.path().to_path_buf(),
            4,
            DEFAULT_PERSIST_CHANNEL_CAPACITY,
        )
        .unwrap();

        seed_journal_entries(&backend, &[3, 4]);
        store.install_overlay_for_reorg(4, 3, |_| None).unwrap();
        // Simulate partial new-chain layer.
        {
            let mut guard = store.trie_cache.write().unwrap();
            let mut updated = (**guard).clone();
            updated.put_batch(
                H256::zero(),
                H256::repeat_byte(0xbb),
                4,
                H256::repeat_byte(0xb4),
                vec![],
            );
            *guard = Arc::new(updated);
        }
        assert!(
            store
                .is_state_in_layer_cache(H256::repeat_byte(0xbb))
                .unwrap()
        );

        store.abort_reorg().unwrap();

        assert!(
            !store
                .is_state_in_layer_cache(H256::repeat_byte(0xbb))
                .unwrap(),
            "partial new-chain layer must be discarded"
        );
        let cache = store.trie_cache.read().unwrap().clone();
        assert!(cache.overlay().is_none(), "overlay must be cleared");
        assert_eq!(
            cache.commit_threshold(),
            4,
            "fresh cache must inherit threshold"
        );
    }

    /// `clear_reorg_overlay` SHALL remove an installed overlay and SHALL be
    /// idempotent (safe to call when no overlay is installed).
    #[tokio::test]
    async fn clear_reorg_overlay_removes_overlay_and_is_idempotent() {
        let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::open().unwrap());
        let dir = tempfile::tempdir().unwrap();
        let store = Store::from_backend(
            backend.clone(),
            dir.path().to_path_buf(),
            4,
            DEFAULT_PERSIST_CHANNEL_CAPACITY,
        )
        .unwrap();

        // Idempotent when no overlay is installed.
        store.clear_reorg_overlay().unwrap();
        assert!(store.trie_cache.read().unwrap().overlay().is_none());

        seed_journal_entries(&backend, &[7]);
        store.install_overlay_for_reorg(7, 7, |_| None).unwrap();
        assert!(store.trie_cache.read().unwrap().overlay().is_some());

        store.clear_reorg_overlay().unwrap();
        assert!(store.trie_cache.read().unwrap().overlay().is_none());

        // Calling again is still a no-op.
        store.clear_reorg_overlay().unwrap();
        assert!(store.trie_cache.read().unwrap().overlay().is_none());
    }

    /// Regression test: journal entries written while the FKV generator
    /// was running lack flat-KV pre-images for keys past the generator frontier,
    /// and the generator's own flat-KV writes are never journaled. While an
    /// overlay serves the read's state root, every flat-KV fast path must be
    /// disabled so reads fall back to the (always journaled) trie nodes;
    /// otherwise they serve the generator's stale values for the chain being
    /// reorged away. Roots the overlay does not serve must keep the fast path.
    #[tokio::test]
    async fn deep_reorg_overlay_disables_flat_kv_fast_paths() {
        let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::open().unwrap());

        // Simulate a finished FKV generation BEFORE the Store boots, so its
        // in-memory frontier starts at [0xff; 64] and every path counts as
        // "computed" (the Store expands the durable [0xff] sentinel at open).
        {
            let mut tx = backend.begin_write().unwrap();
            tx.put(MISC_VALUES, "last_written".as_bytes(), &[0xff])
                .unwrap();
            tx.commit().unwrap();
        }

        let dir = tempfile::tempdir().unwrap();
        let store = Store::from_backend(
            backend.clone(),
            dir.path().to_path_buf(),
            1,
            DEFAULT_PERSIST_CHANNEL_CAPACITY,
        )
        .unwrap();

        // The pivot state is empty, so the correct value for every read at the
        // pivot root is None. The old chain created the account/slot after the
        // pivot; the generator then wrote their flat-KV leaves against the old
        // chain — writes that are never journaled.
        let address = Address::from_low_u64_be(0xbeef);
        let account_hash = hash_address_fixed(&address);
        let slot = H256::from_low_u64_be(1);
        let slot_hash = hash_key_fixed(&slot);
        let generator_account = AccountState {
            nonce: 7,
            balance: U256::from(999u64),
            storage_root: H256::zero(),
            code_hash: H256::zero(),
        };
        let generator_slot_value = U256::from(42u64);
        {
            let mut tx = backend.begin_write().unwrap();
            tx.put(
                ACCOUNT_FLATKEYVALUE,
                &Nibbles::from_bytes(account_hash.as_bytes()).into_vec(),
                &generator_account.encode_to_vec(),
            )
            .unwrap();
            tx.put(
                STORAGE_FLATKEYVALUE,
                &apply_prefix(Some(account_hash), Nibbles::from_bytes(&slot_hash)).into_vec(),
                &generator_slot_value.encode_to_vec(),
            )
            .unwrap();
            tx.commit().unwrap();
        }

        // Journal entries for blocks 10 and 11 above the pivot (block 9), as
        // written DURING generation: flat diffs are empty because the keys were
        // past the frontier at commit time (the incomplete-journal hole). The pivot's state
        // root is the empty trie.
        let pivot_root = EMPTY_TRIE_HASH;
        {
            let mut tx = backend.begin_write().unwrap();
            for (n, parent_root) in [(10u64, pivot_root), (11, H256::repeat_byte(0x10))] {
                let entry = JournalEntry {
                    block_hash: H256::repeat_byte(n as u8),
                    parent_state_root: parent_root,
                    parent_binary_root: H256::zero(),
                    account_trie_diff: vec![],
                    storage_trie_diff: vec![],
                    account_flat_diff: vec![],
                    storage_flat_diff: vec![],
                    binary_trie_diff: vec![],
                    binary_flat_diff: vec![],
                };
                tx.put(STATE_HISTORY, &n.to_be_bytes(), &entry.encode())
                    .unwrap();
            }
            tx.commit().unwrap();
        }

        store
            .install_overlay_for_reorg(11, 10, |_| None)
            .expect("overlay install must succeed");

        // Reads at the pivot root must return the pivot state (account and slot
        // absent), NOT the generator's stale flat-KV values.
        let accounts = store
            .get_account_states_batch_by_root(pivot_root, &[address])
            .unwrap();
        assert_eq!(
            accounts,
            vec![None],
            "batch account read must not serve the generator's stale flat-KV value"
        );

        let slot_value = store
            .get_storage_at_root(pivot_root, address, slot)
            .unwrap();
        assert_eq!(
            slot_value, None,
            "storage read must not serve the generator's stale flat-KV value"
        );

        let state_trie = store.open_state_trie(pivot_root).unwrap();
        assert_eq!(
            state_trie.get(account_hash.as_bytes()).unwrap(),
            None,
            "trie read must not serve the generator's stale flat-KV value"
        );

        // Outside the overlay window the fast paths are untouched: a root the
        // overlay does not serve still reads flat-KV straight from disk.
        let unserved_root = H256::repeat_byte(0x99);
        let accounts = store
            .get_account_states_batch_by_root(unserved_root, &[address])
            .unwrap();
        assert_eq!(
            accounts,
            vec![Some(generator_account)],
            "unserved roots must keep the flat-KV fast path"
        );
        let slot_value = store
            .get_storage_at_root(unserved_root, address, slot)
            .unwrap();
        assert_eq!(
            slot_value,
            Some(generator_slot_value),
            "unserved roots must keep the flat-KV fast path"
        );
    }

    /// Adversarial regression for the multi-layer reconciliation finding: with an
    /// overlay installed, a commit targeting a root SEVERAL layers above the pivot
    /// must commit ONLY the bottom layer (the pivot tip `T`) per pass. Before the
    /// fix this swept every layer at once: the `debug_assert!(len == 1)` panicked in
    /// debug builds, and in release the upper layers' journal entries recorded
    /// pre-images against the OLD-chain disk state instead of the new-chain/bridge
    /// state — silent corruption for a future unwind.
    #[tokio::test]
    async fn overlay_backed_commit_only_commits_bottom_layer_per_pass() {
        let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::open().unwrap());
        let dir = tempfile::tempdir().unwrap();
        let store = Store::from_backend(
            backend.clone(),
            dir.path().to_path_buf(),
            4,
            DEFAULT_PERSIST_CHANNEL_CAPACITY,
        )
        .unwrap();

        // Pivot at block 9's root; journal entries for blocks 10 and 11 (the old
        // chain above the pivot). The overlay serves the pivot root.
        let pivot_root = H256::repeat_byte(0x09);
        {
            let mut tx = backend.begin_write().unwrap();
            for (n, parent_root) in [(10u64, pivot_root), (11, H256::repeat_byte(0x0a))] {
                let entry = JournalEntry {
                    block_hash: H256::repeat_byte(n as u8),
                    parent_state_root: parent_root,
                    parent_binary_root: H256::zero(),
                    account_trie_diff: vec![(vec![0x00, n as u8], None)],
                    storage_trie_diff: vec![],
                    account_flat_diff: vec![],
                    storage_flat_diff: vec![],
                    binary_trie_diff: vec![],
                    binary_flat_diff: vec![],
                };
                tx.put(STATE_HISTORY, &n.to_be_bytes(), &entry.encode())
                    .unwrap();
            }
            tx.commit().unwrap();
        }
        store.install_overlay_for_reorg(11, 10, |_| None).unwrap();

        // Three new-chain layers above the pivot: blocks 10, 11, 12.
        let roots: Vec<H256> = (10..=12u8).map(H256::repeat_byte).collect();
        {
            let mut guard = store.trie_cache.write().unwrap();
            let mut updated = (**guard).clone();
            let mut parent = pivot_root;
            for (i, root) in roots.iter().enumerate() {
                let n = 10 + i as u64;
                updated.put_batch(
                    parent,
                    *root,
                    n,
                    H256::repeat_byte(n as u8),
                    vec![(Nibbles::from_raw(&[0x01, n as u8], false), vec![n as u8])],
                );
                parent = *root;
            }
            *guard = Arc::new(updated);
        }

        // Commit targeting the TOP layer (3 deep). With the overlay installed this
        // MUST reduce to the bottom layer only.
        let trie = store.trie_cache.read().unwrap().clone();
        commit_to_disk(
            store.backend.as_ref(),
            &store.flatkeyvalue_control_tx,
            &store.trie_cache,
            &trie,
            roots[2],
            false,
        )
        .expect("bottom-layer commit must succeed (no debug_assert panic)");

        // Only the bottom layer (block 10) is committed and pruned; 11 and 12 stay resident.
        assert!(
            !store.is_state_in_layer_cache(roots[0]).unwrap(),
            "bottom layer must be committed and pruned"
        );
        assert!(
            store.is_state_in_layer_cache(roots[1]).unwrap()
                && store.is_state_in_layer_cache(roots[2]).unwrap(),
            "upper layers must remain resident after the bottom-layer-only commit"
        );
        // The overlay was consumed by the reconciliation commit.
        assert!(
            store.trie_cache.read().unwrap().overlay().is_none(),
            "reconciliation must clear the overlay"
        );
        // Journal: an entry exists for block 10, none yet for 11/12.
        assert!(
            journal_entry_exists(&backend, 10),
            "bottom layer T must be journaled"
        );
        assert!(!journal_entry_exists(&backend, 11));
        assert!(!journal_entry_exists(&backend, 12));

        // The reconciliation fold MUST land in the bottom layer's journal entry:
        // the overlay's bridge keys (untouched by the layer) appear with their
        // old-chain pre-images. This is what block-number-keyed reconciliation
        // matching loses when the bottom layer sits above `to_block`.
        let entry_bytes = backend
            .begin_read()
            .unwrap()
            .get(STATE_HISTORY, &10u64.to_be_bytes())
            .unwrap()
            .expect("journal entry for block 10");
        let entry = JournalEntry::decode(&entry_bytes).unwrap();
        let diff_keys: Vec<&Vec<u8>> = entry.account_trie_diff.iter().map(|(k, _)| k).collect();
        assert!(
            diff_keys.contains(&&vec![0x00, 10]) && diff_keys.contains(&&vec![0x00, 11]),
            "bridge keys from the overlay must be folded into T's journal entry, got {diff_keys:?}"
        );

        // Second pass: with the overlay gone the remaining backlog commits normally.
        let trie = store.trie_cache.read().unwrap().clone();
        commit_to_disk(
            store.backend.as_ref(),
            &store.flatkeyvalue_control_tx,
            &store.trie_cache,
            &trie,
            roots[2],
            false,
        )
        .unwrap();
        assert!(!store.is_state_in_layer_cache(roots[1]).unwrap());
        assert!(!store.is_state_in_layer_cache(roots[2]).unwrap());
        assert!(journal_entry_exists(&backend, 11));
        assert!(journal_entry_exists(&backend, 12));
    }

    // -----------------------------------------------------------------------
    // The flat leaf mirror in `commit_to_disk`.
    // -----------------------------------------------------------------------

    /// A 34-byte account-zone tree key: zone byte `0x00`, digest, sub-index.
    /// The same length a `BitPath` DB key has at bit-depth 240, which is the
    /// collision these tests keep honest.
    fn tree_key(b: u8) -> Vec<u8> {
        let mut key = vec![0u8; 34];
        key[1] = b;
        key
    }

    /// A 66-byte overflow-storage tree key, beginning with the `0xff` zone byte.
    /// It is `> [0xff]` lexicographically, which is why the frontier's `[0xff]`
    /// completion sentinel needs an explicit arm rather than a comparison.
    fn storage_tree_key(b: u8) -> Vec<u8> {
        let mut key = vec![0u8; 66];
        key[0] = 0xff;
        key[1] = b;
        key
    }

    /// A 32-byte leaf value.
    fn leaf_value(b: u8) -> Vec<u8> {
        vec![b; 32]
    }

    fn flat_row(backend: &Arc<dyn StorageBackend>, key: &[u8]) -> Option<Vec<u8>> {
        backend
            .begin_read()
            .unwrap()
            .get(BINARY_FLATKEYVALUE, key)
            .unwrap()
    }

    fn decode_entry(backend: &Arc<dyn StorageBackend>, block: BlockNumber) -> JournalEntry {
        let bytes = backend
            .begin_read()
            .unwrap()
            .get(STATE_HISTORY, &block.to_be_bytes())
            .unwrap()
            .unwrap_or_else(|| panic!("journal entry for block {block}"));
        JournalEntry::decode(&bytes).unwrap()
    }

    /// Stages one block's layer carrying MPT nodes, binary nodes and mirror
    /// writes, then flushes it through `commit_to_disk`.
    ///
    /// Eight parameters because a layer genuinely has that many independent
    /// dimensions here: two roots and a parent root, the block number, and the
    /// two staged key sets. Grouping them into a struct would only move the
    /// same list one level down for a fixture builder used in one module.
    #[allow(clippy::too_many_arguments)]
    fn stage_and_commit(
        store: &Store,
        parent: H256,
        root: H256,
        block: BlockNumber,
        binary_root: H256,
        parent_binary_root: H256,
        binary_nodes: Vec<(Vec<u8>, Vec<u8>)>,
        binary_flat: Vec<(Vec<u8>, Vec<u8>)>,
    ) {
        {
            let mut guard = store.trie_cache.write().unwrap();
            let mut updated = (**guard).clone();
            updated.put_batch_with_binary(
                parent,
                root,
                block,
                H256::repeat_byte(block as u8),
                vec![(Nibbles::from_raw(&[0x01, block as u8], false), vec![1])],
                BinaryLayerUpdate {
                    root: binary_root,
                    parent_root: parent_binary_root,
                    nodes: binary_nodes,
                    flat: binary_flat,
                },
            );
            *guard = Arc::new(updated);
        }
        let trie = store.trie_cache.read().unwrap().clone();
        commit_to_disk(
            store.backend.as_ref(),
            &store.flatkeyvalue_control_tx,
            &store.trie_cache,
            &trie,
            root,
            false,
        )
        .expect("commit");
    }

    fn test_store(backend: &Arc<dyn StorageBackend>) -> (Store, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::from_backend(
            backend.clone(),
            dir.path().to_path_buf(),
            1,
            DEFAULT_PERSIST_CHANNEL_CAPACITY,
        )
        .unwrap();
        (store, dir)
    }

    /// The mirror lands on disk in the same batch as the nodes, and its
    /// pre-images land in the journal's sixth section — `None` for a row the
    /// block created, the old value for one it overwrote.
    #[tokio::test]
    async fn a_blocks_mirror_writes_reach_disk_and_the_sixth_journal_section() {
        let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::open().unwrap());
        let (store, _dir) = test_store(&backend);

        stage_and_commit(
            &store,
            H256::zero(),
            H256::repeat_byte(1),
            1,
            H256::repeat_byte(0x81),
            H256::zero(),
            vec![(tree_key(0xa), vec![0xaa])],
            vec![
                (tree_key(0x01), leaf_value(0x11)),
                (storage_tree_key(0x02), leaf_value(0x22)),
            ],
        );

        assert_eq!(flat_row(&backend, &tree_key(0x01)), Some(leaf_value(0x11)));
        assert_eq!(
            flat_row(&backend, &storage_tree_key(0x02)),
            Some(leaf_value(0x22)),
            "an overflow-storage key must be written; it is the one the `[0xff]` \
             frontier sentinel would wrongly exclude under a naive comparison"
        );

        let entry = decode_entry(&backend, 1);
        let mut diff = entry.binary_flat_diff.clone();
        diff.sort();
        assert_eq!(
            diff,
            vec![(tree_key(0x01), None), (storage_tree_key(0x02), None),],
            "both rows were created by this block, so both pre-images are absences"
        );
        assert!(
            entry
                .binary_trie_diff
                .iter()
                .any(|(k, _)| *k == tree_key(0xa)),
            "the node section must still carry the node write"
        );

        // Second block overwrites one row and removes the other.
        stage_and_commit(
            &store,
            H256::repeat_byte(1),
            H256::repeat_byte(2),
            2,
            H256::repeat_byte(0x82),
            H256::repeat_byte(0x81),
            vec![],
            vec![
                (tree_key(0x01), leaf_value(0x99)),
                (storage_tree_key(0x02), vec![]),
            ],
        );

        assert_eq!(flat_row(&backend, &tree_key(0x01)), Some(leaf_value(0x99)));
        assert_eq!(
            flat_row(&backend, &storage_tree_key(0x02)),
            None,
            "an empty value is a tombstone: the row must be deleted, not zeroed"
        );

        let entry = decode_entry(&backend, 2);
        let mut diff = entry.binary_flat_diff;
        diff.sort();
        assert_eq!(
            diff,
            vec![
                (tree_key(0x01), Some(leaf_value(0x11))),
                (storage_tree_key(0x02), Some(leaf_value(0x22))),
            ],
            "pre-images must be the values block 1 left on disk"
        );
    }

    /// The rollback direction, which is what the sixth section exists for:
    /// applying the reverse diff must restore the parent's mirror exactly,
    /// **including deletions** — a row the block created is removed, not left
    /// behind with a stale value.
    #[tokio::test]
    async fn applying_the_sixth_section_restores_the_parents_mirror_including_deletions() {
        let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::open().unwrap());
        let (store, _dir) = test_store(&backend);

        // Block 1 establishes two rows.
        stage_and_commit(
            &store,
            H256::zero(),
            H256::repeat_byte(1),
            1,
            H256::repeat_byte(0x81),
            H256::zero(),
            vec![],
            vec![
                (tree_key(0x01), leaf_value(0x11)),
                (tree_key(0x02), leaf_value(0x22)),
            ],
        );
        // Block 2 overwrites one, removes one, and creates a third.
        stage_and_commit(
            &store,
            H256::repeat_byte(1),
            H256::repeat_byte(2),
            2,
            H256::repeat_byte(0x82),
            H256::repeat_byte(0x81),
            vec![],
            vec![
                (tree_key(0x01), leaf_value(0x99)),
                (tree_key(0x02), vec![]),
                (tree_key(0x03), leaf_value(0x33)),
            ],
        );

        // Apply block 2's reverse diff by hand, the way a rollback would:
        // `Some(prev)` is a put, `None` is a delete.
        let entry = decode_entry(&backend, 2);
        {
            let mut tx = backend.begin_write().unwrap();
            for (key, prev) in &entry.binary_flat_diff {
                match prev {
                    Some(v) => tx.put(BINARY_FLATKEYVALUE, key, v).unwrap(),
                    None => tx.delete(BINARY_FLATKEYVALUE, key).unwrap(),
                }
            }
            tx.commit().unwrap();
        }

        assert_eq!(
            flat_row(&backend, &tree_key(0x01)),
            Some(leaf_value(0x11)),
            "the overwritten row must be restored to block 1's value"
        );
        assert_eq!(
            flat_row(&backend, &tree_key(0x02)),
            Some(leaf_value(0x22)),
            "the row block 2 removed must come back"
        );
        assert_eq!(
            flat_row(&backend, &tree_key(0x03)),
            None,
            "the row block 2 created must be DELETED, not left with a stale value; \
             this is the direction a missing `None` pre-image would silently break"
        );
    }

    /// A deep-reorg reconciliation must bridge the overlay's mirror entries the
    /// new chain did not rewrite, into the same batch and the same journal
    /// entry, with an overlay `None` becoming a delete.
    #[tokio::test]
    async fn deep_reorg_reconciliation_bridges_untouched_mirror_keys() {
        let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::open().unwrap());
        let (store, _dir) = test_store(&backend);

        // Old-chain disk state: three mirror rows.
        {
            let mut tx = backend.begin_write().unwrap();
            for (k, v) in [
                (tree_key(0xb1), leaf_value(0xb1)),
                (tree_key(0xb2), leaf_value(0xb2)),
                (tree_key(0xb3), leaf_value(0xb3)),
            ] {
                tx.put(BINARY_FLATKEYVALUE, &k, &v).unwrap();
            }
            tx.commit().unwrap();
        }

        // Journal entries for the old chain above the pivot, carrying mirror
        // pre-images: `b1` had a different value at the pivot, `b2` did not
        // exist at all, `b3` is not mentioned (so disk already holds the pivot
        // value and no bridge entry is needed).
        let pivot_root = H256::repeat_byte(0x09);
        {
            let mut tx = backend.begin_write().unwrap();
            let entry = JournalEntry {
                block_hash: H256::repeat_byte(10),
                parent_state_root: pivot_root,
                parent_binary_root: H256::repeat_byte(0x99),
                account_trie_diff: vec![],
                storage_trie_diff: vec![],
                account_flat_diff: vec![],
                storage_flat_diff: vec![],
                binary_trie_diff: vec![],
                binary_flat_diff: vec![
                    (tree_key(0xb1), Some(leaf_value(0xa1))),
                    (tree_key(0xb2), None),
                ],
            };
            tx.put(STATE_HISTORY, &10u64.to_be_bytes(), &entry.encode())
                .unwrap();
            tx.commit().unwrap();
        }
        store.install_overlay_for_reorg(10, 10, |_| None).unwrap();

        // The new chain's block at T rewrites only `b1`.
        stage_and_commit(
            &store,
            pivot_root,
            H256::repeat_byte(10),
            10,
            H256::repeat_byte(0x8a),
            H256::repeat_byte(0x99),
            vec![],
            vec![(tree_key(0xb1), leaf_value(0xc1))],
        );

        assert_eq!(
            flat_row(&backend, &tree_key(0xb1)),
            Some(leaf_value(0xc1)),
            "the layer wins over the bridge for a key it rewrote"
        );
        assert_eq!(
            flat_row(&backend, &tree_key(0xb2)),
            None,
            "an overlay `None` must become a DELETE: the leaf did not exist at \
             the pivot, and leaving the old chain's row would make the mirror a \
             superset of the trie"
        );
        assert_eq!(
            flat_row(&backend, &tree_key(0xb3)),
            Some(leaf_value(0xb3)),
            "a key no entry mentions is already at its pivot value"
        );

        let entry = decode_entry(&backend, 10);
        let keys: Vec<&Vec<u8>> = entry.binary_flat_diff.iter().map(|(k, _)| k).collect();
        assert!(
            keys.contains(&&tree_key(0xb1)) && keys.contains(&&tree_key(0xb2)),
            "both the layer's key and the bridged key must be journaled in T's \
             entry, got {keys:?}"
        );
    }

    /// Batch mode (full sync) writes the mirror and journals nothing, matching
    /// the rule the other five sections already follow.
    #[tokio::test]
    async fn a_batch_commit_writes_the_mirror_and_journals_nothing() {
        let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::open().unwrap());
        let (store, _dir) = test_store(&backend);

        {
            let mut guard = store.trie_cache.write().unwrap();
            let mut updated = (**guard).clone();
            updated.put_batch_with_binary(
                H256::zero(),
                H256::repeat_byte(1),
                1,
                H256::repeat_byte(1),
                vec![(Nibbles::from_raw(&[0x01], false), vec![1])],
                BinaryLayerUpdate {
                    root: H256::repeat_byte(0x81),
                    parent_root: H256::zero(),
                    nodes: vec![],
                    flat: vec![(tree_key(0x01), leaf_value(0x11))],
                },
            );
            *guard = Arc::new(updated);
        }
        let trie = store.trie_cache.read().unwrap().clone();
        commit_to_disk(
            store.backend.as_ref(),
            &store.flatkeyvalue_control_tx,
            &store.trie_cache,
            &trie,
            H256::repeat_byte(1),
            true,
        )
        .unwrap();

        assert_eq!(
            flat_row(&backend, &tree_key(0x01)),
            Some(leaf_value(0x11)),
            "batch mode still writes the mirror; only journaling is skipped"
        );
        assert!(
            !journal_entry_exists(&backend, 1),
            "batch mode must journal nothing"
        );
    }

    /// The frontier skip. Task 9 builds the generator that makes this marker
    /// non-absent; the skip is wired now so it fires the moment it does, and
    /// this pins the behaviour of all three durable states.
    #[tokio::test]
    async fn the_frontier_skips_both_the_write_and_the_journal_push() {
        let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::open().unwrap());
        let (store, _dir) = test_store(&backend);

        // A partial frontier: the generator has swept up to `tree_key(0x05)`.
        {
            let mut tx = backend.begin_write().unwrap();
            tx.put(MISC_VALUES, BINARY_LAST_WRITTEN_KEY, &tree_key(0x05))
                .unwrap();
            tx.commit().unwrap();
        }

        stage_and_commit(
            &store,
            H256::zero(),
            H256::repeat_byte(1),
            1,
            H256::repeat_byte(0x81),
            H256::zero(),
            vec![],
            vec![
                (tree_key(0x01), leaf_value(0x11)), // below the frontier: ours
                (tree_key(0x05), leaf_value(0x55)), // at it: ours
                (tree_key(0x09), leaf_value(0x99)), // past it: the generator's
                (storage_tree_key(0x01), leaf_value(0xaa)), // far past it
            ],
        );

        assert_eq!(flat_row(&backend, &tree_key(0x01)), Some(leaf_value(0x11)));
        assert_eq!(
            flat_row(&backend, &tree_key(0x05)),
            Some(leaf_value(0x55)),
            "the frontier is inclusive, matching the MPT's `key > last_written` skip"
        );
        assert_eq!(
            flat_row(&backend, &tree_key(0x09)),
            None,
            "a key past the frontier is the generator's to write, not ours"
        );
        assert_eq!(flat_row(&backend, &storage_tree_key(0x01)), None);

        let entry = decode_entry(&backend, 1);
        let mut journaled: Vec<Vec<u8>> = entry
            .binary_flat_diff
            .iter()
            .map(|(k, _)| k.clone())
            .collect();
        journaled.sort();
        let mut expected = vec![tree_key(0x01), tree_key(0x05)];
        expected.sort();
        assert_eq!(
            journaled, expected,
            "the skip must jump over the journal push as well as the write: a \
             `Some(None)` pre-image for an unwritten row would make a rollback \
             delete a key that was never put"
        );
    }

    /// The completion sentinel is one byte and an overflow-storage tree key
    /// begins with `0xff`, so `[0xff, ..] > [0xff]` lexicographically. A naive
    /// `key <= frontier` would silently drop every storage-zone leaf on a
    /// *complete* mirror — the exact opposite of what "complete" means.
    #[tokio::test]
    async fn the_complete_sentinel_covers_the_storage_zone_it_sorts_below() {
        let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::open().unwrap());
        let (store, _dir) = test_store(&backend);
        {
            let mut tx = backend.begin_write().unwrap();
            tx.put(
                MISC_VALUES,
                BINARY_LAST_WRITTEN_KEY,
                BINARY_FLAT_FRONTIER_COMPLETE,
            )
            .unwrap();
            tx.commit().unwrap();
        }

        stage_and_commit(
            &store,
            H256::zero(),
            H256::repeat_byte(1),
            1,
            H256::repeat_byte(0x81),
            H256::zero(),
            vec![],
            vec![
                (tree_key(0x01), leaf_value(0x11)),
                (storage_tree_key(0x01), leaf_value(0xaa)),
            ],
        );

        assert_eq!(flat_row(&backend, &tree_key(0x01)), Some(leaf_value(0x11)));
        assert_eq!(
            flat_row(&backend, &storage_tree_key(0x01)),
            Some(leaf_value(0xaa)),
            "a complete frontier must cover the storage zone, which sorts ABOVE \
             the one-byte `[0xff]` sentinel"
        );
        assert_eq!(decode_entry(&backend, 1).binary_flat_diff.len(), 2);
    }

    /// "Zero means absent" at the production writer. A 32-zero-byte value is not
    /// a leaf the trie holds — the embedding resolved it to a removal — so a row
    /// carrying one would put the mirror ahead of the trie's root, and a range
    /// proved against that root would fail on it. Refuse the commit.
    #[tokio::test]
    async fn a_32_zero_byte_mirror_value_is_refused_rather_than_written() {
        let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::open().unwrap());
        let (store, _dir) = test_store(&backend);

        {
            let mut guard = store.trie_cache.write().unwrap();
            let mut updated = (**guard).clone();
            updated.put_batch_with_binary(
                H256::zero(),
                H256::repeat_byte(1),
                1,
                H256::repeat_byte(1),
                vec![],
                BinaryLayerUpdate {
                    root: H256::repeat_byte(0x81),
                    parent_root: H256::zero(),
                    nodes: vec![],
                    flat: vec![
                        (tree_key(0x01), leaf_value(0x11)),
                        (tree_key(0x02), vec![0u8; 32]),
                    ],
                },
            );
            *guard = Arc::new(updated);
        }
        let trie = store.trie_cache.read().unwrap().clone();
        let err = commit_to_disk(
            store.backend.as_ref(),
            &store.flatkeyvalue_control_tx,
            &store.trie_cache,
            &trie,
            H256::repeat_byte(1),
            false,
        )
        .expect_err("a zero-valued leaf must not be committed");
        let message = err.to_string();
        assert!(
            message.contains("zero means absent"),
            "the error must name the invariant, got: {message}"
        );

        // Nothing was written: the whole `write_tx` is abandoned, so even the
        // valid row beside it does not land.
        assert_eq!(flat_row(&backend, &tree_key(0x01)), None);
        assert_eq!(flat_row(&backend, &tree_key(0x02)), None);
        assert!(!journal_entry_exists(&backend, 1));
    }

    /// The Decision 6 collision at the journal level, end to end through
    /// `commit_to_disk`: one 34-byte key written as a node and as a mirror row,
    /// with different values, in one block. Each must reach its own section with
    /// its own pre-image and land in its own column family.
    #[tokio::test]
    async fn a_shared_34_byte_key_is_committed_to_both_tables_independently() {
        let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::open().unwrap());
        let (store, _dir) = test_store(&backend);
        let shared = tree_key(0x07);

        stage_and_commit(
            &store,
            H256::zero(),
            H256::repeat_byte(1),
            1,
            H256::repeat_byte(0x81),
            H256::zero(),
            vec![(shared.clone(), vec![0xa7])],
            vec![(shared.clone(), leaf_value(0xf7))],
        );

        let read = backend.begin_read().unwrap();
        assert_eq!(
            read.get(BINARY_TRIE_NODES, &shared).unwrap(),
            Some(vec![0xa7]),
            "the node encoding must be in the node table"
        );
        assert_eq!(
            read.get(BINARY_FLATKEYVALUE, &shared).unwrap(),
            Some(leaf_value(0xf7)),
            "the leaf value must be in the mirror, not the node's bytes"
        );

        let entry = decode_entry(&backend, 1);
        assert_eq!(entry.binary_trie_diff, vec![(shared.clone(), None)]);
        assert_eq!(entry.binary_flat_diff, vec![(shared.clone(), None)]);

        // Now the direction that separates the pre-image maps: a second block
        // rewrites both, and each reverse diff must record its OWN previous
        // value. A shared intra-batch map would cross them.
        stage_and_commit(
            &store,
            H256::repeat_byte(1),
            H256::repeat_byte(2),
            2,
            H256::repeat_byte(0x82),
            H256::repeat_byte(0x81),
            vec![(shared.clone(), vec![0xb8])],
            vec![(shared.clone(), leaf_value(0xf8))],
        );

        let entry = decode_entry(&backend, 2);
        assert_eq!(
            entry.binary_trie_diff,
            vec![(shared.clone(), Some(vec![0xa7]))],
            "the node's pre-image is the node's previous encoding"
        );
        assert_eq!(
            entry.binary_flat_diff,
            vec![(shared, Some(leaf_value(0xf7)))],
            "the leaf's pre-image is the leaf's previous value, not the node's"
        );
    }

    /// Adversarial regression for the journal-pruning race: while a deep-reorg
    /// apply pass holds the pause flag, a finality advance must NOT prune
    /// STATE_HISTORY (a concurrent `Overlay::from_journal` reads entries with no
    /// snapshot isolation). Pruning must catch up once the pause is released.
    #[tokio::test]
    async fn journal_pruning_pauses_during_reorg_and_catches_up_after() {
        let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::open().unwrap());
        let dir = tempfile::tempdir().unwrap();
        let store = Store::from_backend(
            backend.clone(),
            dir.path().to_path_buf(),
            1,
            DEFAULT_PERSIST_CHANNEL_CAPACITY,
        )
        .unwrap();

        seed_journal_entries(&backend, &(1..=5).collect::<Vec<_>>());

        // Paused: finality advance to 3 must not prune anything.
        store.set_journal_pruning_paused(true);
        store
            .forkchoice_update_inner(vec![], 100, H256::zero(), None, Some(3))
            .await
            .unwrap();
        for n in 1..=5 {
            assert!(
                journal_entry_exists(&backend, n),
                "entry {n} must survive pruning while the reorg pause is held"
            );
        }

        // Released: the next advance prunes cumulatively from zero.
        store.set_journal_pruning_paused(false);
        store
            .forkchoice_update_inner(vec![], 100, H256::zero(), None, Some(4))
            .await
            .unwrap();
        for n in 1..=4 {
            assert!(
                !journal_entry_exists(&backend, n),
                "entry {n} must be pruned once the pause is released"
            );
        }
        assert!(journal_entry_exists(&backend, 5));
    }
}

#[cfg(test)]
mod merge_tests {
    use super::*;

    fn h256(b: u8) -> H256 {
        H256::from_low_u64_be(b as u64)
    }

    fn op(bn: BlockNumber, bh: H256, idx: Index) -> Vec<u8> {
        encode_tx_location_operand(bn, bh, idx)
    }

    fn decode(v: &[u8]) -> Vec<(BlockNumber, BlockHash, Index)> {
        <Vec<(BlockNumber, BlockHash, Index)>>::decode(v).unwrap()
    }

    #[test]
    fn single_operand_on_empty_base() {
        let out = tx_locations_merge(None, vec![op(100, h256(0x10), 0)]).unwrap();
        assert_eq!(decode(&out), vec![(100, h256(0x10), 0)]);
    }

    #[test]
    fn operand_appended_to_existing_base() {
        let base = vec![(100u64, h256(0x10), 0u64)].encode_to_vec();
        let out = tx_locations_merge(Some(&base), vec![op(101, h256(0x11), 5)]).unwrap();
        let mut got = decode(&out);
        got.sort();
        let mut want = vec![(100, h256(0x10), 0), (101, h256(0x11), 5)];
        want.sort();
        assert_eq!(got, want);
    }

    #[test]
    fn multiple_operands_combined() {
        let out = tx_locations_merge(
            None,
            vec![
                op(100, h256(0x10), 0),
                op(100, h256(0x11), 1),
                op(101, h256(0x12), 2),
            ],
        )
        .unwrap();
        assert_eq!(decode(&out).len(), 3);
    }

    #[test]
    fn same_block_hash_is_deduped() {
        // Two operands with the same block_hash: the later one replaces the earlier.
        let out =
            tx_locations_merge(None, vec![op(100, h256(0x10), 0), op(100, h256(0x10), 7)]).unwrap();
        assert_eq!(decode(&out), vec![(100, h256(0x10), 7)]);
    }

    #[test]
    fn malformed_operand_aborts_merge() {
        // Fail loud: a malformed operand must abort the merge (return None), not
        // silently drop it and commit a partial result.
        let out = tx_locations_merge(None, vec![vec![0xff, 0xff], op(100, h256(0x10), 0)]);
        assert!(out.is_none(), "merge must abort on a malformed operand");
    }

    #[test]
    fn malformed_base_value_aborts_merge() {
        let out = tx_locations_merge(Some(&[0xff, 0xff]), vec![op(100, h256(0x10), 0)]);
        assert!(out.is_none(), "merge must abort on a corrupt base value");
    }

    /// Regression for the associative-merge format bug: a PartialMerge result
    /// must be re-mergeable as an operand. RocksDB folds operands together
    /// without a base value during compaction, then feeds that result back into
    /// a later merge. If the operand format differed from the output format,
    /// the re-fed result would fail to decode and entries would be dropped
    /// (observed as 1664 silent drops during a compaction pass on mainnet).
    #[test]
    fn partial_merge_result_is_a_valid_operand() {
        // Step 1: PartialMerge — combine operands with NO base value.
        let partial =
            tx_locations_merge(None, vec![op(100, h256(0x10), 0), op(101, h256(0x11), 1)]).unwrap();

        // Step 2: the partial result is now itself an operand in a later merge,
        // on top of an existing base value. This is the path that used to drop
        // entries.
        let base = vec![(99u64, h256(0x09), 9u64)].encode_to_vec();
        let out = tx_locations_merge(Some(&base), vec![partial]).unwrap();

        let mut got = decode(&out);
        got.sort();
        let mut want = vec![
            (99, h256(0x09), 9),
            (100, h256(0x10), 0),
            (101, h256(0x11), 1),
        ];
        want.sort();
        assert_eq!(
            got, want,
            "no entries may be lost when re-merging a partial result"
        );
    }

    /// Operand and stored-value encodings must be byte-identical types, so a
    /// freshly-encoded operand round-trips through the value decoder.
    #[test]
    fn operand_encoding_matches_value_encoding() {
        let operand = op(100, h256(0x10), 3);
        // Decoding the operand as the stored Vec type must succeed.
        assert_eq!(decode(&operand), vec![(100, h256(0x10), 3)]);
    }

    /// Chained PartialMerges (operand-only folds applied repeatedly) stay valid.
    #[test]
    fn chained_partial_merges() {
        let p1 = tx_locations_merge(None, vec![op(1, h256(0x01), 0)]).unwrap();
        let p2 = tx_locations_merge(None, vec![p1, op(2, h256(0x02), 0)]).unwrap();
        let p3 = tx_locations_merge(None, vec![p2, op(3, h256(0x03), 0)]).unwrap();
        let out = tx_locations_merge(None, vec![p3]).unwrap();
        assert_eq!(decode(&out).len(), 3);
    }
}

#[cfg(test)]
mod datadir_tests {
    use super::*;
    use std::fs;

    #[test]
    fn empty_dir_has_no_existing_db() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!dir_contains_legacy_db(dir.path()).unwrap());
    }

    #[test]
    fn dir_with_only_unrelated_files_has_no_existing_db() {
        // Regression for #5680: a JWT secret (or any unrelated file) in the
        // datadir must not be mistaken for an existing database.
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("jwt.hex"), "0xdeadbeef").unwrap();
        fs::write(dir.path().join("LOG"), "noise").unwrap();
        assert!(!dir_contains_legacy_db(dir.path()).unwrap());
    }

    #[test]
    fn dir_with_rocksdb_markers_has_existing_db() {
        // A `CURRENT` file (and, separately, a `MANIFEST-*` file) marks a real DB.
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("CURRENT"), "MANIFEST-000001\n").unwrap();
        assert!(dir_contains_legacy_db(dir.path()).unwrap());

        let dir2 = tempfile::tempdir().unwrap();
        fs::write(dir2.path().join("MANIFEST-000007"), "x").unwrap();
        assert!(dir_contains_legacy_db(dir2.path()).unwrap());
    }

    #[test]
    fn dir_with_marker_named_subdirectories_has_no_existing_db() {
        // A *directory* named like a marker file must not be mistaken for a DB;
        // RocksDB only ever visits these as plain files.
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("CURRENT")).unwrap();
        fs::create_dir(dir.path().join("MANIFEST-000001")).unwrap();
        assert!(!dir_contains_legacy_db(dir.path()).unwrap());
    }
}

/// The binary mirror's backfill sweep: the path an *existing* datadir takes,
/// where the trie is on disk and the mirror is not. A chain that started from
/// genesis never reaches any of this — genesis marks the frontier complete —
/// so these drive the generator directly rather than through a store.
#[cfg(test)]
mod binary_flat_backfill_tests {
    use super::*;
    use crate::backend::in_memory::InMemoryBackend;
    use ethrex_binary_trie::trie::BinaryTrie;

    /// `n` leaves spanning both key lengths and all three zones, sorted — the
    /// order the sweep must produce them in.
    fn leaves(n: u8) -> Vec<(Vec<u8>, [u8; 32])> {
        let mut out = Vec::new();
        for i in 0..n {
            for zone in [0x00u8, 0x01, 0xff] {
                let mut key = vec![0x00; if zone == 0xff { 66 } else { 34 }];
                key[0] = zone;
                key[1] = i;
                key[2] = i.wrapping_mul(31).wrapping_add(1);
                out.push((key, [i.wrapping_add(zone) | 1; 32]));
            }
        }
        out.sort();
        out
    }

    /// A backend holding `leaves` as a committed binary trie and nothing in the
    /// mirror.
    fn backend_with_trie(leaves: &[(Vec<u8>, [u8; 32])]) -> Arc<dyn StorageBackend> {
        let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::open().unwrap());
        let mut trie = BinaryTrie::new(Box::new(
            BackendBinaryTrieDB::new(Arc::clone(&backend)).unwrap(),
        ));
        for (key, value) in leaves {
            trie.insert(key.clone(), *value).unwrap();
        }
        trie.commit().unwrap();
        backend
    }

    fn marker(backend: &Arc<dyn StorageBackend>) -> Option<Vec<u8>> {
        backend
            .begin_read()
            .unwrap()
            .get(MISC_VALUES, BINARY_LAST_WRITTEN_KEY)
            .unwrap()
    }

    fn rows(backend: &Arc<dyn StorageBackend>) -> Vec<(Vec<u8>, Vec<u8>)> {
        backend
            .begin_read()
            .unwrap()
            .prefix_iterator(BINARY_FLATKEYVALUE, &[])
            .unwrap()
            .map(|entry| {
                let (k, v) = entry.unwrap();
                (k.into_vec(), v.into_vec())
            })
            .collect()
    }

    /// Run the sweep to completion with no control traffic. The sender is held
    /// for the call so a `try_recv` sees `Empty`, not `Disconnected` — a
    /// disconnected channel is a stop signal.
    fn sweep(backend: &Arc<dyn StorageBackend>, batch: usize) -> (Vec<u8>, Result<(), StoreError>) {
        let (tx, rx) = std::sync::mpsc::sync_channel(0);
        let frontier = RwLock::new(
            backend
                .begin_read()
                .unwrap()
                .get(MISC_VALUES, BINARY_LAST_WRITTEN_KEY)
                .unwrap()
                .unwrap_or_default(),
        );
        let result = binary_flat_generator_with_batch(backend, &frontier, &rx, batch);
        drop(tx);
        (frontier.into_inner().unwrap(), result)
    }

    #[test]
    fn a_sweep_mirrors_every_leaf_and_marks_the_frontier_complete() {
        let expected = leaves(9);
        let backend = backend_with_trie(&expected);
        assert!(rows(&backend).is_empty(), "the mirror starts empty");

        let (in_memory, result) = sweep(&backend, 4);
        result.unwrap();

        assert_eq!(marker(&backend).as_deref(), Some([0xff].as_slice()));
        assert_eq!(in_memory, vec![0xff]);
        // Both directions with a count check, so neither can pass vacuously.
        let rows = rows(&backend);
        assert_eq!(rows.len(), expected.len());
        for ((key, value), (expected_key, expected_value)) in rows.iter().zip(&expected) {
            assert_eq!(key, expected_key, "and in key order");
            assert_eq!(value.as_slice(), expected_value.as_slice());
        }
    }

    #[test]
    fn a_sweep_resumes_from_a_durable_frontier_rather_than_restarting() {
        let expected = leaves(9);
        let backend = backend_with_trie(&expected);
        let split = expected.len() / 2;

        // What a crash mid-sweep leaves behind: rows up to the frontier and a
        // marker naming the last of them.
        let mut tx = backend.begin_write().unwrap();
        for (key, value) in &expected[..split] {
            tx.put(BINARY_FLATKEYVALUE, key, value).unwrap();
        }
        // A deliberately wrong value *below* the frontier. A sweep that
        // restarted would correct it; a sweep that resumed will not. That is
        // what makes this a resume test rather than a completeness test.
        tx.put(BINARY_FLATKEYVALUE, &expected[0].0, &[0xcd; 32])
            .unwrap();
        tx.put(MISC_VALUES, BINARY_LAST_WRITTEN_KEY, &expected[split - 1].0)
            .unwrap();
        tx.commit().unwrap();

        sweep(&backend, 3).1.unwrap();

        assert_eq!(marker(&backend).as_deref(), Some([0xff].as_slice()));
        let rows = rows(&backend);
        assert_eq!(rows.len(), expected.len(), "the sweep filled in the rest");
        assert_eq!(
            rows[0].1,
            vec![0xcd; 32],
            "a row below the frontier was not revisited"
        );
        for ((key, value), (expected_key, expected_value)) in rows.iter().zip(&expected).skip(1) {
            assert_eq!(key, expected_key);
            assert_eq!(value.as_slice(), expected_value.as_slice());
        }
    }

    #[test]
    fn a_completed_sweep_is_not_run_again() {
        let expected = leaves(4);
        let backend = backend_with_trie(&expected);
        let mut tx = backend.begin_write().unwrap();
        tx.put(MISC_VALUES, BINARY_LAST_WRITTEN_KEY, &[0xff])
            .unwrap();
        tx.commit().unwrap();

        sweep(&backend, 4).1.unwrap();
        assert!(
            rows(&backend).is_empty(),
            "a complete marker means the mirror is someone else's business"
        );
    }

    #[test]
    fn no_binary_trie_means_no_claim_of_completeness() {
        // An unscheduled chain, or a scheduled one before genesis lands. There
        // is nothing to mirror — and marking an empty keyspace complete would
        // be a coverage claim about a trie that does not exist, which is
        // exactly the "mirror is a subset" failure the read gate exists to
        // prevent.
        let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::open().unwrap());
        sweep(&backend, 4).1.unwrap();
        assert_eq!(marker(&backend), None);
        assert!(rows(&backend).is_empty());
    }

    #[test]
    fn a_stop_pauses_the_sweep_and_a_continue_finishes_it() {
        // The shared control channel, end to end. `Stop` is a rendezvous send,
        // so it does not return until the sweep receives it — which makes this
        // deterministic rather than timing-dependent.
        let expected = leaves(30);
        let backend = backend_with_trie(&expected);
        let (tx, rx) = std::sync::mpsc::sync_channel(0);
        let frontier = Arc::new(RwLock::new(Vec::new()));

        let sweeping = {
            let backend = Arc::clone(&backend);
            let frontier = Arc::clone(&frontier);
            std::thread::spawn(move || {
                binary_flat_generator_with_batch(&backend, &frontier, &rx, 2)
            })
        };

        // `Stop` fails only if the sweep already finished and dropped the
        // receiver, which is a legal interleaving on a trie this small and not
        // something to make the assertion depend on.
        if tx.send(FKVGeneratorControlMessage::Stop).is_ok() {
            // The send returned, so the sweep took the message and is now
            // blocked in `recv`. Whatever it had committed is durable and the
            // marker is not the completion sentinel: work stopped where it was
            // told to, mid-sweep.
            match marker(&backend) {
                Some(paused) => {
                    assert_ne!(paused, vec![0xff], "a stopped sweep has not completed");
                    assert!(
                        expected.iter().any(|(key, _)| key == &paused),
                        "the durable frontier names a real leaf, never a partial batch"
                    );
                }
                // Stopped inside the first batch, before anything committed.
                None => assert!(rows(&backend).is_empty()),
            }
            tx.send(FKVGeneratorControlMessage::Continue).unwrap();
        }
        sweeping.join().unwrap().unwrap();

        assert_eq!(marker(&backend).as_deref(), Some([0xff].as_slice()));
        assert_eq!(rows(&backend).len(), expected.len());
        assert_eq!(*frontier.read().unwrap(), vec![0xff]);
    }

    #[tokio::test]
    async fn opening_a_store_with_an_absent_marker_wipes_a_stale_mirror() {
        // Rows of unknown provenance are a *superset* hazard: a leaf the trie
        // has since dropped would still be served by a range scan, and proved
        // against the root it would fail. The wipe happens at open rather than
        // in the generator because the commit path owns the whole keyspace
        // while the marker is absent, so a lazy wipe would truncate the table
        // under blocks already importing.
        let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::open().unwrap());
        let mut tx = backend.begin_write().unwrap();
        tx.put(BINARY_FLATKEYVALUE, &[0x00; 34], &[0x77; 32])
            .unwrap();
        tx.commit().unwrap();

        let dir = tempfile::tempdir().unwrap();
        let store = Store::from_backend(
            Arc::clone(&backend),
            dir.path().to_path_buf(),
            1,
            DEFAULT_PERSIST_CHANNEL_CAPACITY,
        )
        .unwrap();
        assert!(store.binary_flat_rows_for_test().unwrap().is_empty());
    }

    #[tokio::test]
    async fn opening_a_store_mid_backfill_keeps_the_rows_the_frontier_claims() {
        // The other half: a partial frontier is a *resume*, and its rows are
        // exactly what it claims. Wiping them would restart the sweep on every
        // restart and never finish on a large trie.
        let expected = leaves(4);
        let backend = backend_with_trie(&expected);
        let mut tx = backend.begin_write().unwrap();
        tx.put(BINARY_FLATKEYVALUE, &expected[0].0, &expected[0].1)
            .unwrap();
        tx.put(MISC_VALUES, BINARY_LAST_WRITTEN_KEY, &expected[0].0)
            .unwrap();
        tx.commit().unwrap();

        let dir = tempfile::tempdir().unwrap();
        let store = Store::from_backend(
            Arc::clone(&backend),
            dir.path().to_path_buf(),
            1,
            DEFAULT_PERSIST_CHANNEL_CAPACITY,
        )
        .unwrap();
        assert_eq!(store.binary_flat_rows_for_test().unwrap().len(), 1);
        // And the in-memory frontier came back in its durable shape, so the
        // read gate covers exactly that one key.
        assert!(store.binary_flat_coverage().unwrap().covers(&expected[0].0));
        assert!(!store.binary_flat_coverage().unwrap().covers(&expected[1].0));
    }

    #[tokio::test]
    async fn a_deep_reorg_is_deferred_only_on_a_chain_that_has_a_mirror() {
        // An unscheduled chain never writes the marker. An unconditional
        // completeness check would therefore read as "still generating" for
        // ever and defer every deep reorg on every existing MPT chain.
        let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::open().unwrap());
        let dir = tempfile::tempdir().unwrap();
        let store = Store::from_backend(
            Arc::clone(&backend),
            dir.path().to_path_buf(),
            1,
            DEFAULT_PERSIST_CHANNEL_CAPACITY,
        )
        .unwrap();
        assert!(!store.get_chain_config().binary_tree_scheduled());
        assert!(!store.binary_flat_fully_generated().unwrap());
        assert!(
            !store.binary_flat_generation_pending().unwrap(),
            "no binary tree, no binary hazard"
        );
    }
}

#[cfg(test)]
mod flatkeyvalue_completeness_tests {
    use super::*;

    /// The `last_written` marker signals a finished FKV pass ONLY as the exact 1-byte
    /// `[0xff]` sentinel. This gates journal-backed deep reorgs, so the
    /// boundary must be exact: an unset marker, the initial all-zero frontier, and any
    /// mid-generation nibble path must all read as "not complete".
    #[test]
    fn only_the_one_byte_ff_sentinel_counts_as_complete() {
        // Complete: the durable completion sentinel the generator writes.
        assert!(Store::flatkeyvalue_generation_complete(Some(&[0xff])));

        // Not complete:
        assert!(!Store::flatkeyvalue_generation_complete(None)); // marker never written
        assert!(!Store::flatkeyvalue_generation_complete(Some(&[]))); // empty
        assert!(!Store::flatkeyvalue_generation_complete(Some(&[0u8; 64]))); // initial frontier
        assert!(!Store::flatkeyvalue_generation_complete(Some(&[0x0a; 64]))); // mid-gen nibble path
        // The in-memory frontier is expanded to all-0xff (64/131 bytes), but that is never
        // the durable marker; only the 1-byte form means complete, so these read false.
        assert!(!Store::flatkeyvalue_generation_complete(Some(&[0xff; 64])));
        assert!(!Store::flatkeyvalue_generation_complete(Some(&[
            0xff, 0xff
        ])));
    }
}

#[cfg(test)]
mod genesis_binary_trie_tests {
    use super::*;
    use crate::backend::in_memory::InMemoryBackend;
    use crate::binary_trie::BackendBinaryTrieDB;
    use ethrex_binary_trie::embedding::{
        address20_to_address32, chunkify_code, encode_basic_data, get_tree_key_for_basic_data,
        get_tree_key_for_code_chunk, get_tree_key_for_code_hash, get_tree_key_for_storage_slot,
    };
    use ethrex_binary_trie::trie::{BinaryTrie, EMPTY_TRIE_ROOT};
    use ethrex_common::Bytes;
    use ethrex_common::constants::EMPTY_KECCAK_HASH;
    use ethrex_common::types::pbt_state::apply_account_updates;
    use rustc_hash::FxHashMap;
    use serde::Deserialize;

    pub(super) const ADDR_A: Address = Address::repeat_byte(0xaa);
    pub(super) const ADDR_B: Address = Address::repeat_byte(0xbb);

    /// A store over a fresh in-memory backend, plus the backend itself
    /// so a test can open its own trie handle on the same bytes.
    pub(super) fn test_store() -> (Store, Arc<dyn StorageBackend>, tempfile::TempDir) {
        let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::open().unwrap());
        let dir = tempfile::tempdir().unwrap();
        let store = Store::from_backend(
            backend.clone(),
            dir.path().to_path_buf(),
            1,
            DEFAULT_PERSIST_CHANNEL_CAPACITY,
        )
        .unwrap();
        (store, backend, dir)
    }

    pub(super) fn genesis_account(
        nonce: u64,
        balance: u64,
        code: Vec<u8>,
        storage: &[(u64, u64)],
    ) -> GenesisAccount {
        GenesisAccount {
            code: Bytes::from(code),
            storage: storage
                .iter()
                .map(|(slot, value)| (U256::from(*slot), U256::from(*value)))
                .collect(),
            balance: U256::from(balance),
            nonce,
        }
    }

    pub(super) fn alloc(
        entries: Vec<(Address, GenesisAccount)>,
    ) -> BTreeMap<Address, GenesisAccount> {
        entries.into_iter().collect()
    }

    pub(super) fn code_of(bytes: Vec<u8>) -> Code {
        Code::from_bytecode(Bytes::from(bytes), &NativeCrypto)
    }

    pub(super) fn storage_map(slots: &[(u64, u64)]) -> FxHashMap<H256, U256> {
        slots
            .iter()
            .map(|(slot, value)| (H256(U256::from(*slot).to_big_endian()), U256::from(*value)))
            .collect()
    }

    /// The root the same accounts reach with no store in sight: the
    /// plain path, built by hand rather than through the conversion the
    /// store method performs, so the two are independent statements.
    pub(super) fn plain_root(updates: &[AccountUpdate]) -> H256 {
        let mut trie = BinaryTrie::new_temp();
        apply_account_updates(&mut trie, updates).expect("updates apply");
        trie.root()
    }

    /// A trie opened at `root` over a *fresh* handle on `backend`, so
    /// nothing is inherited but what actually reached the database.
    pub(super) fn reopen(backend: &Arc<dyn StorageBackend>, root: H256) -> BinaryTrie {
        let db = BackendBinaryTrieDB::new(Arc::clone(backend)).expect("read view opens");
        BinaryTrie::open(Box::new(db), root)
    }

    /// Every leaf the same updates produce on a store-less trie, as a
    /// key -> value map: the mirror's expected contents computed independently
    /// of the store, so an agreement assertion is two statements meeting
    /// rather than one restated.
    pub(super) fn plain_leaves(updates: &[AccountUpdate]) -> BTreeMap<Vec<u8>, [u8; 32]> {
        let mut trie = BinaryTrie::new_temp();
        apply_account_updates(&mut trie, updates).expect("updates apply");
        trie.commit()
            .expect("commit")
            .leaves
            .into_iter()
            .filter_map(|(key, value)| value.map(|value| (key, value)))
            .collect()
    }

    #[tokio::test]
    async fn genesis_seeds_the_mirror_and_marks_it_complete() {
        // A chain that starts from genesis never runs the backfill generator:
        // the alloc *is* the whole state, so the changelog `commit` hands back
        // is the whole mirror, and the frontier can be declared complete on the
        // spot.
        let (store, backend, _dir) = test_store();
        // Both key lengths, all three zones, and a storage slot on each side of
        // the header/overflow boundary.
        let bytecode = vec![0x01u8; 31 * 130];
        let storage = [(63u64, 0xaaaa), (64, 0xbbbb)];
        let root = store
            .setup_genesis_binary_trie(alloc(vec![
                (ADDR_A, genesis_account(1, 7, bytecode.clone(), &storage)),
                (ADDR_B, genesis_account(2, 9, vec![], &[])),
            ]))
            .await
            .unwrap();

        assert_eq!(
            store.binary_flat_frontier_for_test().unwrap().as_deref(),
            Some([0xff].as_slice()),
            "the mirror covers the whole keyspace from block 0"
        );
        assert!(store.binary_flat_fully_generated().unwrap());

        let rows = store.binary_flat_rows_for_test().unwrap();
        let expected = plain_leaves(&[
            AccountUpdate {
                address: ADDR_A,
                info: Some(AccountInfo {
                    code_hash: code_of(bytecode.clone()).hash,
                    balance: U256::from(7u64),
                    nonce: 1,
                }),
                code: Some(code_of(bytecode)),
                added_storage: storage_map(&storage),
                ..AccountUpdate::new(ADDR_A)
            },
            AccountUpdate {
                address: ADDR_B,
                info: Some(AccountInfo {
                    code_hash: *EMPTY_KECCAK_HASH,
                    balance: U256::from(9u64),
                    nonce: 2,
                }),
                code: Some(code_of(vec![])),
                ..AccountUpdate::new(ADDR_B)
            },
        ]);

        // Both directions, so neither can pass vacuously: the mirror holds
        // every leaf the trie does, and no leaf the trie does not.
        assert_eq!(
            rows.len(),
            expected.len(),
            "a superset is as wrong as a subset"
        );
        let mut trie = reopen(&backend, root);
        for (key, value) in &rows {
            assert_ne!(
                value.as_slice(),
                [0u8; 32].as_slice(),
                "zero means absent, so no row may hold 32 zero bytes: {key:?}"
            );
            let value: [u8; 32] = value.as_slice().try_into().expect("32-byte leaf value");
            assert_eq!(expected.get(key), Some(&value), "key {key:?}");
            assert_eq!(
                trie.get(key).unwrap(),
                Some(value),
                "the trie disagrees at {key:?}"
            );
        }

        // And the rows came out of the table in tree-key order, which is what
        // makes the mirror an ordered leaf index rather than just a cache.
        let mut sorted = rows.iter().map(|(key, _)| key.clone()).collect::<Vec<_>>();
        sorted.sort();
        assert_eq!(
            rows.into_iter().map(|(key, _)| key).collect::<Vec<_>>(),
            sorted
        );
    }

    #[tokio::test]
    async fn the_embedding_reads_the_same_answers_with_the_mirror_on_and_off() {
        // The composition that matters: not raw key reads, but the real
        // `pbt_state` accessors, over a state shaped to hit every leaf kind the
        // embedding produces. If the mirror and the descent ever disagreed
        // here, the disagreement would reach consensus.
        use crate::binary_trie::{BackendBinaryFlatDB, BinaryFlatCoverage};
        use crate::layering::TrieLayerCache;
        use ethrex_common::types::pbt_state::{get_account_info, get_storage_slot};

        let (store, backend, _dir) = test_store();
        // Identical bytecode from two senders, so their code-zone chunks are
        // the same shared leaves; storage on both sides of the header/overflow
        // boundary; and an account with zero balance and zero nonce, which
        // exists but whose basic-data leaf is at its emptiest.
        let shared_code = vec![0x60u8; 31 * 130];
        let root = store
            .setup_genesis_binary_trie(alloc(vec![
                (
                    ADDR_A,
                    genesis_account(1, 7, shared_code.clone(), &[(63, 0xaaaa), (64, 0xbbbb)]),
                ),
                (ADDR_B, genesis_account(0, 0, shared_code.clone(), &[])),
            ]))
            .await
            .unwrap();

        // A delegation, applied on top so `CODE_HASH_LEAF_KEY` gives way to
        // `DELEGATION_LEAF_KEY` for that account.
        let delegated = Address::repeat_byte(0xcc);
        let root = store
            .apply_account_updates_to_binary_trie(
                root,
                &[AccountUpdate {
                    address: delegated,
                    info: Some(AccountInfo {
                        code_hash: *EMPTY_KECCAK_HASH,
                        balance: U256::from(5u64),
                        nonce: 1,
                    }),
                    code: Some(code_of(
                        [&[0xef, 0x01, 0x00][..], ADDR_A.as_bytes()].concat(),
                    )),
                    ..AccountUpdate::new(delegated)
                }],
            )
            .await
            .unwrap();

        let read_at = |coverage: BinaryFlatCoverage| {
            let mut trie = BinaryTrie::open(
                Box::new(LayeredBinaryTrieDB::new(
                    root,
                    Arc::new(TrieLayerCache::default()),
                    BackendBinaryTrieDB::new(Arc::clone(&backend)).unwrap(),
                    BackendBinaryFlatDB::new(Arc::clone(&backend)).unwrap(),
                    coverage,
                    LayeredBinaryTrieDB::staging_buffer(),
                )),
                root,
            );
            let accounts: Vec<_> = [ADDR_A, ADDR_B, delegated, Address::repeat_byte(0xde)]
                .into_iter()
                .map(|address| get_account_info(&mut trie, address).unwrap())
                .collect();
            let slots: Vec<_> = [63u64, 64, 65]
                .into_iter()
                .map(|slot| {
                    get_storage_slot(&mut trie, ADDR_A, &H256::from_low_u64_be(slot)).unwrap()
                })
                .collect();
            (accounts, slots)
        };

        let covered = read_at(BinaryFlatCoverage::Everything);
        assert_eq!(covered, read_at(BinaryFlatCoverage::Nothing));
        // And the answers are not vacuously equal: the accounts are really
        // there, the absent one is really absent, and the storage is real.
        assert!(covered.0[0].is_some() && covered.0[1].is_some() && covered.0[2].is_some());
        assert_eq!(covered.0[3], None);
        assert_eq!(covered.1[0], Some(U256::from(0xaaaau64)));
        assert_eq!(covered.1[1], Some(U256::from(0xbbbbu64)));
        assert_eq!(covered.1[2], None);
    }

    #[tokio::test]
    async fn an_empty_alloc_still_marks_the_mirror_complete() {
        // An empty state is a *covered* state, not an uncovered one: every key
        // is legitimately absent, so a reader may trust a miss. Leaving the
        // marker off here would make a fresh chain run a backfill over nothing.
        let (store, _backend, _dir) = test_store();
        store
            .setup_genesis_binary_trie(BTreeMap::new())
            .await
            .unwrap();
        assert!(store.binary_flat_fully_generated().unwrap());
        assert!(store.binary_flat_rows_for_test().unwrap().is_empty());
    }

    #[tokio::test]
    async fn an_unscheduled_chain_writes_no_mirror_rows() {
        // The shadow-tracking property, extended to the mirror: a chain with no
        // binary tree scheduled does literally no binary work, so the column
        // family is untouched and the frontier marker is absent — which the
        // read gate reads as "trust nothing", the only safe answer for a table
        // nothing has populated.
        let mut genesis = Genesis::default();
        assert!(!genesis.config.binary_tree_scheduled());
        genesis.alloc.insert(
            ADDR_A,
            genesis_account(1, 1_000, vec![0x60, 0x00], &[(1, 2)]),
        );
        let mut store = Store::new("", EngineType::InMemory).unwrap();
        store.add_initial_state(genesis).await.unwrap();

        assert_eq!(store.binary_trie_node_count_for_test().unwrap(), 0);
        assert!(store.binary_flat_rows_for_test().unwrap().is_empty());
        assert_eq!(store.binary_flat_frontier_for_test().unwrap(), None);
        assert!(!store.binary_flat_fully_generated().unwrap());
    }

    #[tokio::test]
    async fn an_empty_alloc_seeds_the_empty_root() {
        let (store, _backend, _dir) = test_store();
        assert_eq!(
            store
                .setup_genesis_binary_trie(BTreeMap::new())
                .await
                .unwrap(),
            EMPTY_TRIE_ROOT
        );
    }

    #[tokio::test]
    async fn a_single_eoa_matches_the_plain_path() {
        let (store, _backend, _dir) = test_store();
        let root = store
            .setup_genesis_binary_trie(alloc(vec![(
                ADDR_A,
                genesis_account(3, 1_000, vec![], &[]),
            )]))
            .await
            .unwrap();

        assert_ne!(root, EMPTY_TRIE_ROOT);
        assert_eq!(
            root,
            plain_root(&[AccountUpdate {
                address: ADDR_A,
                info: Some(AccountInfo {
                    code_hash: *EMPTY_KECCAK_HASH,
                    balance: U256::from(1_000u64),
                    nonce: 3,
                }),
                code: Some(code_of(vec![])),
                ..AccountUpdate::new(ADDR_A)
            }])
        );
    }

    #[tokio::test]
    async fn a_contract_with_overflow_code_and_boundary_storage_matches_the_plain_path() {
        let (store, _backend, _dir) = test_store();
        // 130 chunks: 128 in the header stem, 2 in the code zone. And
        // storage on both sides of the slot-63/64 header boundary.
        let bytecode = vec![0x01u8; 31 * 130];
        let storage = [(63u64, 0xaaaa), (64, 0xbbbb)];
        let root = store
            .setup_genesis_binary_trie(alloc(vec![(
                ADDR_A,
                genesis_account(1, 7, bytecode.clone(), &storage),
            )]))
            .await
            .unwrap();

        let code = code_of(bytecode);
        assert_eq!(
            root,
            plain_root(&[AccountUpdate {
                address: ADDR_A,
                info: Some(AccountInfo {
                    code_hash: code.hash,
                    balance: U256::from(7u64),
                    nonce: 1,
                }),
                code: Some(code),
                added_storage: storage_map(&storage),
                ..AccountUpdate::new(ADDR_A)
            }])
        );
    }

    #[tokio::test]
    async fn the_seeded_trie_is_readable_from_the_database() {
        let (store, backend, _dir) = test_store();
        let bytecode = vec![0x01u8; 31 * 130];
        let code = code_of(bytecode.clone());
        let root = store
            .setup_genesis_binary_trie(alloc(vec![
                (
                    ADDR_A,
                    genesis_account(1, 7, bytecode.clone(), &[(63, 0xaaaa), (64, 0xbbbb)]),
                ),
                (ADDR_B, genesis_account(2, 200, vec![], &[])),
            ]))
            .await
            .unwrap();

        let mut trie = reopen(&backend, root);
        assert_eq!(trie.root(), root);

        let a32 = address20_to_address32(ADDR_A);
        assert_eq!(
            trie.get(&get_tree_key_for_basic_data(&a32)).unwrap(),
            Some(encode_basic_data(31 * 130, 1, U256::from(7u64)).unwrap())
        );
        assert_eq!(
            trie.get(&get_tree_key_for_code_hash(&a32)).unwrap(),
            Some(code.hash.0)
        );
        // A header-stem slot and an overflow-zone one.
        assert_eq!(
            trie.get(&get_tree_key_for_storage_slot(&a32, U256::from(63)))
                .unwrap(),
            Some(U256::from(0xaaaau64).to_big_endian())
        );
        assert_eq!(
            trie.get(&get_tree_key_for_storage_slot(&a32, U256::from(64)))
                .unwrap(),
            Some(U256::from(0xbbbbu64).to_big_endian())
        );
        // A code chunk, which lives outside the account stem: chunks
        // are content-addressed by code hash, never keyed by account.
        assert_eq!(
            trie.get(&get_tree_key_for_code_chunk(&code.hash.0, 129))
                .unwrap(),
            Some(chunkify_code(&bytecode)[129])
        );

        let b32 = address20_to_address32(ADDR_B);
        assert_eq!(
            trie.get(&get_tree_key_for_basic_data(&b32)).unwrap(),
            Some(encode_basic_data(0, 2, U256::from(200u64)).unwrap())
        );
    }

    #[tokio::test]
    async fn account_code_is_retrievable_by_hash() {
        let (store, _backend, _dir) = test_store();
        let bytecode = vec![0x60u8, 0x01, 0x60, 0x02, 0x01];
        let code = code_of(bytecode.clone());
        store
            .setup_genesis_binary_trie(alloc(vec![(
                ADDR_A,
                genesis_account(1, 7, bytecode.clone(), &[]),
            )]))
            .await
            .unwrap();

        // The trie commits code as chunks, but the EVM fetches bytecode
        // by hash, so it must also be in the code table.
        let stored = store
            .get_account_code(code.hash)
            .unwrap()
            .expect("genesis code is stored");
        assert_eq!(stored.code(), bytecode.as_slice());
        assert_eq!(stored.hash, code.hash);
    }

    #[tokio::test]
    async fn a_zero_valued_storage_slot_in_the_alloc_is_absent() {
        let (store, backend, _dir) = test_store();
        let with_zero = store
            .setup_genesis_binary_trie(alloc(vec![(
                ADDR_A,
                genesis_account(1, 7, vec![], &[(5, 0), (6, 42)]),
            )]))
            .await
            .unwrap();

        let (other, _other_backend, _other_dir) = test_store();
        let without_zero = other
            .setup_genesis_binary_trie(alloc(vec![(
                ADDR_A,
                genesis_account(1, 7, vec![], &[(6, 42)]),
            )]))
            .await
            .unwrap();

        assert_eq!(
            with_zero, without_zero,
            "a zero-valued slot must commit to nothing at all"
        );

        let a32 = address20_to_address32(ADDR_A);
        let mut trie = reopen(&backend, with_zero);
        assert_eq!(
            trie.get(&get_tree_key_for_storage_slot(&a32, U256::from(5)))
                .unwrap(),
            None
        );
        assert_eq!(
            trie.get(&get_tree_key_for_storage_slot(&a32, U256::from(6)))
                .unwrap(),
            Some(U256::from(42u64).to_big_endian())
        );
    }

    /// Genesis seeding against the spec's own whole-state roots.
    ///
    /// The `pbt_state` section of the vendored fixture is a set of
    /// complete states and the roots they commit to, produced by the
    /// spec's `src/ethereum/state_pbt.py`. `crates/common`'s
    /// `pbt_state_vectors.rs` runs them through
    /// [`apply_account_updates`] directly; running the very same cases
    /// through the store's genesis path pins the alloc conversion and
    /// the storage round trip to the spec too, not merely to each
    /// other.
    mod spec_vectors {
        use super::*;

        #[derive(Deserialize)]
        struct Fixture {
            pbt_state: Vec<StateCase>,
        }

        #[derive(Deserialize)]
        struct StateCase {
            name: String,
            /// Keyed by 20-byte address hex; order is not significant.
            accounts: BTreeMap<String, AccountSpec>,
            root: String,
        }

        #[derive(Deserialize)]
        struct AccountSpec {
            nonce: u64,
            /// Hex: balances can exceed a JSON-safe integer.
            balance: String,
            code: String,
            /// `keccak256(code)`, restated because overflow code chunk
            /// keys are content-addressed by it.
            code_hash: String,
            /// Keyed by decimal slot number, values 32-byte hex.
            storage: BTreeMap<String, String>,
        }

        fn unhex(s: &str) -> Vec<u8> {
            hex::decode(s.strip_prefix("0x").unwrap_or(s)).expect("fixture hex string")
        }

        fn hex_u256(s: &str) -> U256 {
            U256::from_str_radix(s.trim_start_matches("0x"), 16).expect("fixture hex integer")
        }

        fn genesis_alloc(case: &StateCase) -> BTreeMap<Address, GenesisAccount> {
            case.accounts
                .iter()
                .map(|(address, account)| {
                    let code = Bytes::from(unhex(&account.code));
                    assert_eq!(
                        Code::from_bytecode(code.clone(), &NativeCrypto)
                            .hash
                            .as_bytes(),
                        unhex(&account.code_hash).as_slice(),
                        "case {}: fixture code_hash is keccak256(code)",
                        case.name
                    );
                    (
                        Address::from_slice(&unhex(address)),
                        GenesisAccount {
                            code,
                            storage: account
                                .storage
                                .iter()
                                .map(|(slot, value)| {
                                    (
                                        U256::from_dec_str(slot).expect("fixture decimal slot"),
                                        U256::from_big_endian(&unhex(value)),
                                    )
                                })
                                .collect(),
                            balance: hex_u256(&account.balance),
                            nonce: account.nonce,
                        },
                    )
                })
                .collect()
        }

        #[tokio::test]
        async fn seeded_genesis_roots_match_spec() {
            let fixture: Fixture = serde_json::from_str(include_str!(
                "../common/binary-trie/tests/vectors/binary_trie_vectors.json"
            ))
            .expect("fixture parses");
            // The fixture is vendored and refreshed upstream, so its
            // case count is expected to grow; assert only that the
            // section did not arrive empty.
            assert!(!fixture.pbt_state.is_empty(), "no pbt_state cases");

            for case in &fixture.pbt_state {
                let (store, _backend, _dir) = test_store();
                let root = store
                    .setup_genesis_binary_trie(genesis_alloc(case))
                    .await
                    .unwrap_or_else(|err| panic!("case {}: seeding: {err}", case.name));
                assert_eq!(
                    root.as_bytes(),
                    unhex(&case.root).as_slice(),
                    "genesis state root, case {}",
                    case.name
                );
            }
        }
    }
}

/// The store's per-block binary-trie advance,
/// [`Store::apply_account_updates_to_binary_trie`].
///
/// Every test seeds through [`Store::setup_genesis_binary_trie`] and
/// then advances, so the two halves of the storage path — the anchor
/// and the step — are exercised as one, and the helpers are shared with
/// [`genesis_binary_trie_tests`] rather than restated.
#[cfg(test)]
mod binary_trie_block_tests {
    use super::genesis_binary_trie_tests::{
        ADDR_A, ADDR_B, alloc, code_of, genesis_account, plain_root, reopen, storage_map,
        test_store,
    };
    use super::*;
    use crate::api::tables::BINARY_TRIE_NODES;
    use crate::api::{StorageLockedView, StorageReadView, StorageWriteBatch};
    use crate::backend::in_memory::InMemoryBackend;
    use ethrex_binary_trie::embedding::{
        address20_to_address32, encode_basic_data, get_tree_key_for_basic_data,
        get_tree_key_for_code_hash, get_tree_key_for_storage_slot,
    };
    use ethrex_binary_trie::trie::EMPTY_TRIE_ROOT;
    use ethrex_common::constants::EMPTY_KECCAK_HASH;

    const ADDR_C: Address = Address::repeat_byte(0xcc);

    /// A block's update to a plain account: new nonce and balance, no
    /// code and no storage.
    fn eoa_update(address: Address, nonce: u64, balance: u64) -> AccountUpdate {
        AccountUpdate {
            info: Some(AccountInfo {
                code_hash: *EMPTY_KECCAK_HASH,
                balance: U256::from(balance),
                nonce,
            }),
            ..AccountUpdate::new(address)
        }
    }

    /// A block's update to an account's storage alone: no `info`, so
    /// nothing about the account header is rewritten.
    fn storage_update(address: Address, slots: &[(u64, u64)]) -> AccountUpdate {
        AccountUpdate {
            added_storage: storage_map(slots),
            ..AccountUpdate::new(address)
        }
    }

    fn contract_update(
        address: Address,
        nonce: u64,
        balance: u64,
        code: &Code,
        slots: &[(u64, u64)],
    ) -> AccountUpdate {
        AccountUpdate {
            info: Some(AccountInfo {
                code_hash: code.hash,
                balance: U256::from(balance),
                nonce,
            }),
            code: Some(code.clone()),
            added_storage: storage_map(slots),
            ..AccountUpdate::new(address)
        }
    }

    fn store_over(backend: Arc<dyn StorageBackend>) -> (Store, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::from_backend(
            backend,
            dir.path().to_path_buf(),
            1,
            DEFAULT_PERSIST_CHANNEL_CAPACITY,
        )
        .unwrap();
        (store, dir)
    }

    /// How many binary-trie nodes the database currently holds.
    fn nodes_on_disk(backend: &Arc<dyn StorageBackend>) -> usize {
        backend
            .begin_read()
            .unwrap()
            .prefix_iterator(BINARY_TRIE_NODES, &[])
            .unwrap()
            .count()
    }

    /// A backend counting the [`BINARY_TRIE_NODES`] entries written
    /// through it, so a test can state how much of the trie a block
    /// actually rewrote rather than assuming the dirty tracking works.
    ///
    /// Counted per entry, not per batch: what matters is how many nodes
    /// were touched, not how many transactions carried them. Deletes
    /// count too — a tombstone is a write.
    #[derive(Debug)]
    struct CountingBackend {
        inner: Arc<dyn StorageBackend>,
        node_writes: Arc<AtomicUsize>,
    }

    /// A fresh counting backend and the counter it reports into.
    fn counting_backend() -> (Arc<dyn StorageBackend>, Arc<AtomicUsize>) {
        let node_writes = Arc::new(AtomicUsize::new(0));
        let backend: Arc<dyn StorageBackend> = Arc::new(CountingBackend {
            inner: Arc::new(InMemoryBackend::open().unwrap()),
            node_writes: node_writes.clone(),
        });
        (backend, node_writes)
    }

    impl StorageBackend for CountingBackend {
        fn clear_table(&self, table: &'static str) -> Result<(), StoreError> {
            self.inner.clear_table(table)
        }

        fn begin_read(&self) -> Result<Arc<dyn StorageReadView>, StoreError> {
            self.inner.begin_read()
        }

        fn begin_write(&self) -> Result<Box<dyn StorageWriteBatch + 'static>, StoreError> {
            Ok(Box::new(CountingWriteBatch {
                inner: self.inner.begin_write()?,
                node_writes: self.node_writes.clone(),
            }))
        }

        fn begin_locked(
            &self,
            table_name: &'static str,
        ) -> Result<Box<dyn StorageLockedView + 'static>, StoreError> {
            self.inner.begin_locked(table_name)
        }

        fn create_checkpoint(&self, path: &Path) -> Result<(), StoreError> {
            self.inner.create_checkpoint(path)
        }

        fn flush(&self) -> Result<(), StoreError> {
            self.inner.flush()
        }
    }

    struct CountingWriteBatch {
        inner: Box<dyn StorageWriteBatch>,
        node_writes: Arc<AtomicUsize>,
    }

    impl StorageWriteBatch for CountingWriteBatch {
        fn put_batch(
            &mut self,
            table: &'static str,
            batch: Vec<(Vec<u8>, Vec<u8>)>,
        ) -> Result<(), StoreError> {
            if table == BINARY_TRIE_NODES {
                self.node_writes.fetch_add(batch.len(), Ordering::Relaxed);
            }
            self.inner.put_batch(table, batch)
        }

        fn delete(&mut self, table: &'static str, key: &[u8]) -> Result<(), StoreError> {
            if table == BINARY_TRIE_NODES {
                self.node_writes.fetch_add(1, Ordering::Relaxed);
            }
            self.inner.delete(table, key)
        }

        fn delete_range(
            &mut self,
            table: &'static str,
            start: &[u8],
            end: &[u8],
        ) -> Result<(), StoreError> {
            self.inner.delete_range(table, start, end)
        }

        fn merge(
            &mut self,
            table: &'static str,
            key: &[u8],
            operand: &[u8],
        ) -> Result<(), StoreError> {
            self.inner.merge(table, key, operand)
        }

        fn commit(&mut self) -> Result<(), StoreError> {
            self.inner.commit()
        }
    }

    #[tokio::test]
    async fn one_block_advances_the_root() {
        let (store, _backend, _dir) = test_store();
        let genesis_root = store
            .setup_genesis_binary_trie(alloc(vec![
                (ADDR_A, genesis_account(1, 100, vec![], &[])),
                (ADDR_B, genesis_account(2, 200, vec![], &[])),
            ]))
            .await
            .unwrap();

        let block_root = store
            .apply_account_updates_to_binary_trie(
                genesis_root,
                &[eoa_update(ADDR_A, 2, 150), eoa_update(ADDR_C, 0, 50)],
            )
            .await
            .unwrap();

        assert_ne!(block_root, genesis_root);
        // The real assertion: advancing the stored trie by a block must
        // land on the very trie the resulting state builds from scratch,
        // not merely on some root that changed.
        assert_eq!(
            block_root,
            plain_root(&[
                eoa_update(ADDR_A, 2, 150),
                eoa_update(ADDR_B, 2, 200),
                eoa_update(ADDR_C, 0, 50),
            ])
        );
    }

    #[tokio::test]
    async fn three_blocks_in_sequence_reach_the_final_state_root() {
        let (store, _backend, _dir) = test_store();
        let bytecode = vec![0x01u8; 31 * 3];
        let code = code_of(bytecode.clone());
        let mut root = store
            .setup_genesis_binary_trie(alloc(vec![
                (ADDR_A, genesis_account(1, 100, vec![], &[])),
                (ADDR_B, genesis_account(1, 7, bytecode, &[(5, 7)])),
            ]))
            .await
            .unwrap();

        // Each block opens at the root the previous one returned, which
        // is the whole point: nothing is carried over in memory.
        let blocks = vec![
            vec![eoa_update(ADDR_A, 2, 90), storage_update(ADDR_B, &[(5, 9)])],
            // Slot 100 is past the header stem, so it lands in the
            // overflow storage zone.
            vec![
                eoa_update(ADDR_A, 3, 80),
                storage_update(ADDR_B, &[(100, 3)]),
            ],
            // A zero-valued slot is a removal, so slot 5 leaves the trie.
            vec![storage_update(ADDR_B, &[(5, 0)]), eoa_update(ADDR_C, 0, 10)],
        ];
        let mut roots = Vec::new();
        for block in &blocks {
            root = store
                .apply_account_updates_to_binary_trie(root, block)
                .await
                .unwrap();
            roots.push(root);
        }
        assert_eq!(
            roots.iter().collect::<HashSet<_>>().len(),
            roots.len(),
            "each block changed the state, so each root should be new"
        );

        assert_eq!(
            root,
            plain_root(&[
                eoa_update(ADDR_A, 3, 80),
                contract_update(ADDR_B, 1, 7, &code, &[(100, 3)]),
                eoa_update(ADDR_C, 0, 10),
            ])
        );
    }

    #[tokio::test]
    async fn a_block_writes_far_fewer_nodes_than_the_trie_holds() {
        let (backend, node_writes) = counting_backend();
        let (store, _dir) = store_over(backend.clone());

        let accounts: Vec<Address> = (1..=200u64).map(Address::from_low_u64_be).collect();
        let genesis_root = store
            .setup_genesis_binary_trie(alloc(
                accounts
                    .iter()
                    .map(|address| (*address, genesis_account(1, 100, vec![], &[])))
                    .collect(),
            ))
            .await
            .unwrap();

        let trie_nodes = nodes_on_disk(&backend);
        node_writes.store(0, Ordering::Relaxed);

        let block_root = store
            .apply_account_updates_to_binary_trie(
                genesis_root,
                &[
                    eoa_update(accounts[3], 2, 150),
                    eoa_update(accounts[177], 5, 900),
                ],
            )
            .await
            .unwrap();
        assert_ne!(block_root, genesis_root);

        // Opening at a root loads nothing, applying loads only the
        // paths touched, and committing writes only what changed — the
        // property that makes per-block persistence affordable at all.
        // A block touching two of two hundred accounts must rewrite the
        // two root-to-leaf paths and nothing else, so an order of
        // magnitude below the trie's size is a generous ceiling.
        let written = node_writes.load(Ordering::Relaxed);
        assert!(
            written * 10 < trie_nodes,
            "block rewrote {written} of {trie_nodes} nodes"
        );
    }

    #[tokio::test]
    async fn an_empty_update_list_is_a_no_op() {
        let (backend, node_writes) = counting_backend();
        let (store, _dir) = store_over(backend.clone());
        let genesis_root = store
            .setup_genesis_binary_trie(alloc(vec![
                (ADDR_A, genesis_account(1, 100, vec![], &[])),
                (ADDR_B, genesis_account(2, 200, vec![], &[])),
            ]))
            .await
            .unwrap();

        node_writes.store(0, Ordering::Relaxed);
        assert_eq!(
            store
                .apply_account_updates_to_binary_trie(genesis_root, &[])
                .await
                .unwrap(),
            genesis_root
        );
        assert_eq!(node_writes.load(Ordering::Relaxed), 0);

        // And over an empty parent, where there is not even a root node
        // to leave alone.
        assert_eq!(
            store
                .apply_account_updates_to_binary_trie(EMPTY_TRIE_ROOT, &[])
                .await
                .unwrap(),
            EMPTY_TRIE_ROOT
        );
        assert_eq!(node_writes.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn removing_an_account_reaches_the_trie_of_the_survivors() {
        let (store, backend, _dir) = test_store();
        // Slot 5 lives in the header stem, one prefix removal; the
        // overflow zone, which takes a second one, is the case below.
        let genesis_root = store
            .setup_genesis_binary_trie(alloc(vec![
                (ADDR_A, genesis_account(1, 100, vec![], &[])),
                (ADDR_B, genesis_account(2, 200, vec![], &[(5, 7)])),
                (ADDR_C, genesis_account(3, 300, vec![], &[])),
            ]))
            .await
            .unwrap();

        let block_root = store
            .apply_account_updates_to_binary_trie(genesis_root, &[AccountUpdate::removed(ADDR_B)])
            .await
            .unwrap();

        assert_eq!(
            block_root,
            plain_root(&[eoa_update(ADDR_A, 1, 100), eoa_update(ADDR_C, 3, 300)])
        );

        let mut trie = reopen(&backend, block_root);
        let b32 = address20_to_address32(ADDR_B);
        assert_eq!(trie.get(&get_tree_key_for_basic_data(&b32)).unwrap(), None);
        assert_eq!(trie.get(&get_tree_key_for_code_hash(&b32)).unwrap(), None);
        assert_eq!(
            trie.get(&get_tree_key_for_storage_slot(&b32, U256::from(5)))
                .unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn removing_an_account_clears_its_overflow_storage_on_disk() {
        let (store, backend, _dir) = test_store();
        // ADDR_A's storage reaches past slot 63 into the overflow zone,
        // which no bounded loop over the header stem can clear: the
        // slots there are unenumerable from outside the trie, and this
        // removal was once refused outright for that reason.
        let overflow_slots: Vec<(u64, u64)> = [64u64, 255, 256, 5_000, 1_000_000]
            .iter()
            .map(|s| (*s, s + 1))
            .collect();
        let mut slots = vec![(5u64, 7u64)];
        slots.extend(overflow_slots.iter().copied());
        let genesis_root = store
            .setup_genesis_binary_trie(alloc(vec![
                (ADDR_A, genesis_account(1, 100, vec![], &slots)),
                (ADDR_B, genesis_account(2, 200, vec![], &slots)),
            ]))
            .await
            .unwrap();

        let block_root = store
            .apply_account_updates_to_binary_trie(genesis_root, &[AccountUpdate::removed(ADDR_A)])
            .await
            .unwrap();

        // The removed account leaves nothing behind, and the root is the
        // one a trie that never held it would have — the strong form,
        // which a stranded leaf anywhere would break.
        let survivor = store
            .setup_genesis_binary_trie(alloc(vec![(
                ADDR_B,
                genesis_account(2, 200, vec![], &slots),
            )]))
            .await
            .unwrap();
        assert_eq!(block_root, survivor);

        let a32 = address20_to_address32(ADDR_A);
        let b32 = address20_to_address32(ADDR_B);
        let mut trie = reopen(&backend, block_root);
        assert_eq!(trie.get(&get_tree_key_for_basic_data(&a32)).unwrap(), None);
        for (slot, value) in &slots {
            assert_eq!(
                trie.get(&get_tree_key_for_storage_slot(&a32, U256::from(*slot)))
                    .unwrap(),
                None,
                "slot {slot} was stranded"
            );
            assert_eq!(
                trie.get(&get_tree_key_for_storage_slot(&b32, U256::from(*slot)))
                    .unwrap(),
                Some(U256::from(*value).to_big_endian()),
                "slot {slot} of the other account must be untouched"
            );
        }
        // And the read path agrees: no orphaned storage under that
        // address for a later `CREATE` to collide with.
        assert_eq!(store.get_binary_account(block_root, ADDR_A).unwrap(), None);
        assert!(
            store
                .get_binary_account(block_root, ADDR_B)
                .unwrap()
                .expect("the survivor is still there")
                .has_storage
        );
    }

    #[tokio::test]
    async fn the_advanced_trie_is_readable_through_a_fresh_handle() {
        let (store, backend, _dir) = test_store();
        let genesis_root = store
            .setup_genesis_binary_trie(alloc(vec![(ADDR_A, genesis_account(1, 100, vec![], &[]))]))
            .await
            .unwrap();

        let bytecode = vec![0x60u8, 0x01, 0x60, 0x02, 0x01];
        let code = code_of(bytecode.clone());
        let block_root = store
            .apply_account_updates_to_binary_trie(
                genesis_root,
                &[
                    eoa_update(ADDR_A, 2, 150),
                    contract_update(ADDR_B, 1, 9, &code, &[(5, 42)]),
                ],
            )
            .await
            .unwrap();

        // A fresh handle inherits nothing but the bytes on disk.
        let mut trie = reopen(&backend, block_root);
        assert_eq!(trie.root(), block_root);

        let a32 = address20_to_address32(ADDR_A);
        assert_eq!(
            trie.get(&get_tree_key_for_basic_data(&a32)).unwrap(),
            Some(encode_basic_data(0, 2, U256::from(150u64)).unwrap())
        );

        let b32 = address20_to_address32(ADDR_B);
        assert_eq!(
            trie.get(&get_tree_key_for_basic_data(&b32)).unwrap(),
            Some(encode_basic_data(bytecode.len() as u32, 1, U256::from(9u64)).unwrap())
        );
        assert_eq!(
            trie.get(&get_tree_key_for_code_hash(&b32)).unwrap(),
            Some(code.hash.0)
        );
        assert_eq!(
            trie.get(&get_tree_key_for_storage_slot(&b32, U256::from(5)))
                .unwrap(),
            Some(U256::from(42u64).to_big_endian())
        );

        // The trie commits code as chunks, but the EVM fetches whole
        // bytecode by hash, and only the code table answers that.
        let stored = store
            .get_account_code(code.hash)
            .unwrap()
            .expect("the block's code is stored");
        assert_eq!(stored.code(), bytecode.as_slice());
        assert_eq!(stored.hash, code.hash);
    }
}

/// Regression tests for stale state-root reads.
///
/// ethrex keeps exactly one version of the state trie on disk — trie nodes are
/// keyed by *path*, not by node hash, so block N+1 overwrites block N's node at
/// the same path — plus a chain of in-memory diff layers keyed by state root
/// ([`TrieLayerCache`]). A read at a state root the store no longer holds
/// therefore falls through to whatever the on-disk trie currently is, and
/// neither [`Trie::open`] nor [`Trie::get`] validate the root against the data
/// they read. Left ungated, an account read at a pre-retention-window block
/// answers from the wrong state and reports success.
///
/// `eth_call` has never had this problem: `StoreVmDatabase::new` gates on
/// [`Store::has_state_root`] and errors. These tests pin the same contract on
/// the account read paths, so one node cannot answer the same question two
/// different ways.
///
/// The tests construct the failure directly rather than mining past
/// `DB_COMMIT_THRESHOLD`: the bug is "a root with no diff layer and no matching
/// on-disk root serves data anyway", and a fabricated root is exactly that
/// (the in-memory backend also raises the threshold to
/// `IN_MEMORY_COMMIT_THRESHOLD` precisely so tests can reach old state, which
/// would otherwise have to be defeated first).
#[cfg(test)]
mod stale_state_root_read_tests {
    use super::*;
    use bytes::Bytes;

    const ADDRESS: Address = Address::repeat_byte(0xa1);
    const SLOT: u64 = 7;
    const SLOT_VALUE: u64 = 0x1234;
    const BALANCE: u64 = 0xbeef;
    const NONCE: u64 = 3;
    const CODE: &[u8] = &[0x60, 0x00, 0x60, 0x00, 0xf3];

    /// A state root no block on this chain ever produced: the layer cache holds
    /// no layer for it and it is not the root of the on-disk trie. This is the
    /// same situation a caller lands in when it asks for a block whose state has
    /// fallen out of the retention window, or for a superseded fork's root.
    fn unheld_root() -> H256 {
        H256::repeat_byte(0xaa)
    }

    fn slot_key() -> H256 {
        H256::from_low_u64_be(SLOT)
    }

    async fn store_with_genesis_account() -> Store {
        let mut genesis = Genesis::default();
        genesis.alloc.insert(
            ADDRESS,
            GenesisAccount {
                code: Bytes::from_static(CODE),
                storage: BTreeMap::from([(U256::from(SLOT), U256::from(SLOT_VALUE))]),
                balance: U256::from(BALANCE),
                nonce: NONCE,
            },
        );
        let mut store = Store::new("", EngineType::InMemory).expect("open in-memory store");
        store.add_initial_state(genesis).await.expect("genesis");
        store
    }

    /// Appends a canonical block whose header *claims* `state_root` while no such
    /// state is ever written. This is what a node looks like when asked for a
    /// block past the retention window: the header is still on disk, the state
    /// behind it is not.
    async fn append_block_claiming_root(store: &Store, state_root: H256) -> BlockNumber {
        let genesis_hash = store
            .get_canonical_block_hash(0)
            .await
            .expect("canonical hash")
            .expect("genesis is canonical");
        let header = BlockHeader {
            number: 1,
            parent_hash: genesis_hash,
            state_root,
            ..Default::default()
        };
        let hash = header.hash();
        store
            .add_block(Block::new(header, BlockBody::default()))
            .await
            .expect("add block");
        store
            .forkchoice_update(vec![(1, hash)], 1, hash, None, None)
            .await
            .expect("forkchoice");
        1
    }

    #[track_caller]
    fn assert_missing_state_root<T: Debug>(result: Result<T, StoreError>, what: &str) {
        match result {
            Err(StoreError::MissingStateRoot { state_root, .. }) => {
                assert_eq!(state_root, unheld_root(), "{what}: wrong root reported");
            }
            Err(other) => panic!("{what}: expected MissingStateRoot, got {other:?}"),
            Ok(value) => panic!(
                "{what}: served state at a root the store does not hold: {value:?}. \
                 This is the stale-state-root read bug: the on-disk trie answered \
                 for a root it is not the root of."
            ),
        }
    }

    // ---------------------------------------------------------------------
    // 1. Reads at a root the store does not hold must error, not serve data.
    // ---------------------------------------------------------------------

    #[tokio::test]
    async fn get_account_info_errors_at_unheld_root() {
        let store = store_with_genesis_account().await;
        let block = append_block_claiming_root(&store, unheld_root()).await;
        assert_missing_state_root(
            store.get_account_info(block, ADDRESS).await,
            "eth_getBalance",
        );
    }

    #[tokio::test]
    async fn get_account_info_by_hash_errors_at_unheld_root() {
        let store = store_with_genesis_account().await;
        let block = append_block_claiming_root(&store, unheld_root()).await;
        let hash = store
            .get_canonical_block_hash(block)
            .await
            .unwrap()
            .unwrap();
        assert_missing_state_root(
            store.get_account_info_by_hash(hash, ADDRESS),
            "get_account_info_by_hash",
        );
    }

    #[tokio::test]
    async fn get_code_errors_at_unheld_root() {
        let store = store_with_genesis_account().await;
        let block = append_block_claiming_root(&store, unheld_root()).await;
        assert_missing_state_root(
            store.get_code_by_account_address(block, ADDRESS).await,
            "eth_getCode",
        );
    }

    #[tokio::test]
    async fn get_nonce_errors_at_unheld_root() {
        let store = store_with_genesis_account().await;
        let block = append_block_claiming_root(&store, unheld_root()).await;
        assert_missing_state_root(
            store.get_nonce_by_account_address(block, ADDRESS).await,
            "eth_getTransactionCount",
        );
    }

    #[tokio::test]
    async fn get_storage_at_errors_at_unheld_root() {
        let store = store_with_genesis_account().await;
        let block = append_block_claiming_root(&store, unheld_root()).await;
        assert_missing_state_root(
            store.get_storage_at(block, ADDRESS, slot_key()),
            "eth_getStorageAt",
        );
    }

    #[tokio::test]
    async fn get_account_state_errors_at_unheld_root() {
        let store = store_with_genesis_account().await;
        let block = append_block_claiming_root(&store, unheld_root()).await;
        assert_missing_state_root(
            store.get_account_state(block, ADDRESS).await,
            "get_account_state",
        );
    }

    // ---------------------------------------------------------------------
    // 3. eth_getProof must error rather than return an account object beside
    //    an empty proof array.
    // ---------------------------------------------------------------------

    /// Before the guard this returned `Ok(Some(..))` holding the *current*
    /// account (read through the unchecked `Trie::get`) next to an **empty**
    /// `proof` (`Trie::get_proof` does use the checked accessor and bails), i.e.
    /// a response that contradicts itself.
    #[tokio::test]
    async fn get_account_proof_errors_at_unheld_root() {
        let store = store_with_genesis_account().await;
        assert_missing_state_root(
            store
                .get_account_proof(unheld_root(), ADDRESS, &[slot_key()])
                .await,
            "eth_getProof",
        );
    }

    // ---------------------------------------------------------------------
    // 2. The happy path is untouched: reads at a root the store does hold
    //    return the real values.
    // ---------------------------------------------------------------------

    #[tokio::test]
    async fn reads_at_a_held_root_still_work() {
        let store = store_with_genesis_account().await;
        let genesis_hash = store.get_canonical_block_hash(0).await.unwrap().unwrap();
        let state_root = store
            .get_block_header_by_hash(genesis_hash)
            .unwrap()
            .unwrap()
            .state_root;

        let info = store
            .get_account_info(0, ADDRESS)
            .await
            .expect("balance read")
            .expect("account exists");
        assert_eq!(info.balance, U256::from(BALANCE));
        assert_eq!(info.nonce, NONCE);

        assert_eq!(
            store
                .get_account_info_by_hash(genesis_hash, ADDRESS)
                .unwrap()
                .unwrap()
                .balance,
            U256::from(BALANCE)
        );

        assert_eq!(
            store
                .get_nonce_by_account_address(0, ADDRESS)
                .await
                .unwrap()
                .unwrap(),
            NONCE
        );

        assert_eq!(
            store
                .get_code_by_account_address(0, ADDRESS)
                .await
                .unwrap()
                .unwrap()
                .code(),
            CODE
        );

        assert_eq!(
            store
                .get_storage_at(0, ADDRESS, slot_key())
                .unwrap()
                .unwrap(),
            U256::from(SLOT_VALUE)
        );

        assert_eq!(
            store
                .get_account_state(0, ADDRESS)
                .await
                .unwrap()
                .unwrap()
                .nonce,
            NONCE
        );

        let proof = store
            .get_account_proof(state_root, ADDRESS, &[slot_key()])
            .await
            .expect("proof read")
            .expect("state trie present");
        assert_eq!(proof.account.balance, U256::from(BALANCE));
        assert!(
            !proof.proof.is_empty(),
            "a held root must produce a non-empty account proof"
        );
        assert_eq!(proof.storage_proof.len(), 1);
        assert_eq!(proof.storage_proof[0].value, U256::from(SLOT_VALUE));
    }

    /// A block that exists but whose account is absent still reports "no such
    /// account" rather than an error: the guard must only fire on missing
    /// *state*, never on a missing account within present state.
    #[tokio::test]
    async fn absent_account_at_a_held_root_is_still_none() {
        let store = store_with_genesis_account().await;
        let other = Address::repeat_byte(0xb2);
        assert!(store.get_account_info(0, other).await.unwrap().is_none());
        assert!(
            store
                .get_storage_at(0, other, slot_key())
                .unwrap()
                .is_none()
        );
    }

    /// An unknown block still yields `Ok(None)` — "I have no such block" is a
    /// different answer from "I have the block but not its state".
    #[tokio::test]
    async fn unknown_block_is_still_none() {
        let store = store_with_genesis_account().await;
        assert!(store.get_account_info(99, ADDRESS).await.unwrap().is_none());
        assert!(
            store
                .get_account_info_by_hash(H256::repeat_byte(0xcd), ADDRESS)
                .unwrap()
                .is_none()
        );
        assert!(
            store
                .get_storage_at(99, ADDRESS, slot_key())
                .unwrap()
                .is_none()
        );
    }

    // ---------------------------------------------------------------------
    // 4. The deliberately unchecked variants stay unchecked.
    // ---------------------------------------------------------------------

    /// `get_account_state_by_root` is the per-account read `StoreVmDatabase`
    /// issues for every account an execution touches. Its caller has *already*
    /// gated on `has_state_root` once, at `StoreVmDatabase::new`
    /// (`crates/blockchain/vm.rs`), so re-verifying the root here would add a
    /// root-node read and a keccak to every single account access during block
    /// execution. It stays unchecked on purpose.
    #[tokio::test]
    async fn get_account_state_by_root_stays_unchecked() {
        let store = store_with_genesis_account().await;
        assert!(
            store
                .get_account_state_by_root(unheld_root(), ADDRESS)
                .unwrap()
                .is_some(),
            "the VM's hot per-account read must stay unguarded; \
             StoreVmDatabase::new checks the root once up front"
        );
    }

    /// `get_storage_at_root` is the root-addressed sibling of `get_storage_at`
    /// and is used by store-internal callers that address state by root directly
    /// (see the deep-reorg overlay tests, which read at roots the overlay
    /// deliberately does not serve). `get_storage_at`, the block-addressed entry
    /// point the RPC layer uses, is the guarded one.
    #[tokio::test]
    async fn get_storage_at_root_stays_unchecked() {
        let store = store_with_genesis_account().await;
        assert_eq!(
            store
                .get_storage_at_root(unheld_root(), ADDRESS, slot_key())
                .unwrap(),
            Some(U256::from(SLOT_VALUE)),
            "the root-addressed storage read must stay unguarded"
        );
    }

    /// `has_state_root`, the detector all of the above now share, must agree
    /// with them about what is and is not held.
    #[tokio::test]
    async fn has_state_root_agrees_with_the_guards() {
        let store = store_with_genesis_account().await;
        let genesis_root = store.get_block_header(0).unwrap().unwrap().state_root;
        assert!(store.has_state_root(genesis_root).unwrap());
        assert!(!store.has_state_root(unheld_root()).unwrap());
        assert!(
            store.has_state_root(EMPTY_TRIE_HASH).unwrap(),
            "the empty trie is always available"
        );
    }
}
