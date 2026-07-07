#[cfg(feature = "rocksdb")]
use crate::backend::rocksdb::RocksDBBackend;
use crate::{
    STORE_METADATA_FILENAME, STORE_SCHEMA_VERSION,
    api::{
        StorageBackend, StorageReadView, StorageWriteBatch,
        tables::{
            ACCOUNT_CODE_METADATA, ACCOUNT_CODES, ACCOUNT_FLATKEYVALUE, ACCOUNT_TRIE_NODES,
            BLOCK_ACCESS_LISTS, BLOCK_NUMBERS, BODIES, CANONICAL_BLOCK_HASHES, CHAIN_DATA,
            EXECUTION_WITNESSES, FULLSYNC_HEADERS, HEADERS, INVALID_CHAINS, MISC_VALUES,
            PENDING_BLOCKS, RECEIPTS_V2, SNAP_STATE, STORAGE_FLATKEYVALUE, STORAGE_TRIE_NODES,
            TRANSACTION_LOCATIONS,
        },
    },
    apply_prefix,
    backend::in_memory::InMemoryBackend,
    block_data_buffer::BlockDataBuffer,
    error::StoreError,
    layering::{TrieLayerCache, TrieWrapper},
    rlp::{BlockBodyRLP, BlockHeaderRLP, BlockRLP},
    trie::{BackendTrieDB, BackendTrieDBLocked},
    utils::{ChainDataIndex, SnapStateIndex},
};

use ethrex_common::{
    Address, H256, U256,
    types::{
        AccountInfo, AccountState, AccountUpdate, Block, BlockBody, BlockHash, BlockHeader,
        BlockNumber, ChainConfig, Code, CodeMetadata, ForkId, Genesis, GenesisAccount, Index,
        Receipt, Transaction,
        block_access_list::BlockAccessList,
        block_execution_witness::{ExecutionWitness, RpcExecutionWitness},
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
#[allow(unused)]
const DB_COMMIT_THRESHOLD: usize = 128;
const IN_MEMORY_COMMIT_THRESHOLD: usize = 10000;

/// Commit threshold for batch (full sync) mode. Each batch layer holds ~1024
/// blocks of trie diffs (~1 GB), so we flush aggressively to bound memory.
const BATCH_COMMIT_THRESHOLD: usize = 4;

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
    /// Whether this batch comes from full sync (batch execution mode).
    /// When true, uses `BATCH_COMMIT_THRESHOLD` (aggressive) instead of
    /// `DB_COMMIT_THRESHOLD` to bound memory during bulk block import.
    pub batch_mode: bool,
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
                txn.put(CHAIN_DATA, &finalized_key, &finalized.to_le_bytes())?;
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

    /// Compute `(parent_state_root, last_state_root)` for a batch's trie update:
    /// the state root of the first block's parent and the last block's own state
    /// root. Used by `apply_updates` for both the live and full-sync paths (which
    /// share the single persist worker).
    fn batch_state_roots(&self, update_batch: &UpdateBatch) -> Result<(H256, H256), StoreError> {
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
        let last_state_root = update_batch
            .blocks
            .last()
            .ok_or(StoreError::UpdateBatchNoBlocks)?
            .header
            .state_root;
        Ok((parent_state_root, last_state_root))
    }

    /// Single path for both live (`batch_mode == false`) and full-sync
    /// (`batch_mode == true`) updates. Both hand the whole unit (block data +
    /// one aggregate trie diff) to the SINGLE persist worker and wait for its ack;
    /// `wait_for_flush` (= `batch_mode`) selects when the worker acks.
    fn apply_updates(&self, update_batch: UpdateBatch) -> Result<(), StoreError> {
        let (parent_state_root, last_state_root) = self.batch_state_roots(&update_batch)?;

        let UpdateBatch {
            account_updates,
            storage_updates,
            blocks,
            receipts,
            code_updates,
            batch_mode,
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
        // LIVE (wait_for_flush=false): worker acks after staging; the ack carries
        //   the PRIOR flush result so a disk error surfaces on the next call.
        // BATCH (wait_for_flush=true): worker acks after flush, bounding
        //   in-flight batches to ~1.
        let (ack_tx, ack_rx) = sync_channel(1);
        self.persist_tx
            .send(PersistMessage::Block(BlockPersist {
                blocks: blocks_with_receipts,
                codes: code_updates,
                parent_state_root,
                child_state_root: last_state_root,
                account_updates,
                storage_updates,
                wait_for_flush: batch_mode,
                ack: ack_tx,
            }))
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
        let (fkv_tx, fkv_rx) = std::sync::mpsc::sync_channel(0);
        let persist_cap = persist_channel_capacity.max(1); // clamp: 0 would be a rendezvous channel
        let (persist_tx, persist_rx) = std::sync::mpsc::sync_channel(persist_cap);

        let (last_written, initial_flushed_upto) = {
            let tx = backend.begin_read()?;
            let last_written = tx
                .get(MISC_VALUES, "last_written".as_bytes())?
                .unwrap_or_else(|| vec![0u8; 64]);
            let last_written = if last_written == [0xff] {
                vec![0xff; 64]
            } else {
                last_written
            };
            let initial_flushed_upto = match tx.get(MISC_VALUES, FLUSHED_UPTO_KEY)? {
                Some(bytes) => decode_flushed_upto(&bytes)?,
                None => 0,
            };
            (last_written, initial_flushed_upto)
        };
        let mut initial_buffer = BlockDataBuffer::new();
        initial_buffer.set_flushed_upto(initial_flushed_upto);

        let mut background_threads = Vec::new();
        let mut store = Self {
            db_path,
            backend,
            chain_config: Default::default(),
            latest_block_header: Default::default(),
            trie_cache: Arc::new(RwLock::new(Arc::new(TrieLayerCache::new(commit_threshold)))),
            flatkeyvalue_control_tx: fkv_tx,
            block_data_buffer: Arc::new(RwLock::new(Arc::new(initial_buffer))),
            persist_tx,
            pending_trie_roots: Arc::new(PendingTrieRoots::default()),
            last_computed_flatkeyvalue: Arc::new(RwLock::new(last_written)),
            account_code_cache: Arc::new(Mutex::new(CodeCache::default())),
            code_metadata_cache: Arc::new(Mutex::new(rustc_hash::FxHashMap::default())),
            fcu_lock: Arc::new(tokio::sync::Mutex::new(())),
            background_threads: Default::default(),
        };
        let backend_clone = store.backend.clone();
        let last_computed_fkv = store.last_computed_flatkeyvalue.clone();
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
                        // LIVE: ack after staging; carries prior flush result.
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
                            bp.account_updates,
                            bp.storage_updates,
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
                                    bp.wait_for_flush,
                                )
                            });
                        // BATCH: ack after flush (bounds in-flight batches to ~1),
                        // folding in any prior live-path error. LIVE: stash result.
                        if bp.wait_for_flush {
                            let prior = std::mem::replace(&mut last_flush_result, Ok(()));
                            let _ = bp.ack.send(prior.and(flushed));
                        } else {
                            last_flush_result = flushed;
                        }
                    }
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

    pub fn get_account_info_by_hash(
        &self,
        block_hash: BlockHash,
        address: Address,
    ) -> Result<Option<AccountInfo>, StoreError> {
        let Some(state_trie) = self.state_trie(block_hash)? else {
            return Ok(None);
        };
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

    pub async fn get_code_by_account_address(
        &self,
        block_number: BlockNumber,
        address: Address,
    ) -> Result<Option<Code>, StoreError> {
        let Some(block_hash) = self.get_canonical_block_hash(block_number).await? else {
            return Ok(None);
        };
        let Some(state_trie) = self.state_trie(block_hash)? else {
            return Ok(None);
        };
        let hashed_address = hash_address_fixed(&address);
        let Some(encoded_state) = state_trie.get(hashed_address.as_bytes())? else {
            return Ok(None);
        };
        let account_state = AccountState::decode(&encoded_state)?;
        self.get_account_code(account_state.code_hash)
    }

    pub async fn get_nonce_by_account_address(
        &self,
        block_number: BlockNumber,
        address: Address,
    ) -> Result<Option<u64>, StoreError> {
        let Some(block_hash) = self.get_canonical_block_hash(block_number).await? else {
            return Ok(None);
        };
        let Some(state_trie) = self.state_trie(block_hash)? else {
            return Ok(None);
        };
        let hashed_address = hash_address_fixed(&address);
        let Some(encoded_state) = state_trie.get(hashed_address.as_bytes())? else {
            return Ok(None);
        };
        let account_state = AccountState::decode(&encoded_state)?;
        Ok(Some(account_state.nonce))
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
                account_state.storage_root = *EMPTY_TRIE_HASH;
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
                account_state.storage_root = *EMPTY_TRIE_HASH;
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
        let mut genesis_state_trie = self.open_direct_state_trie(*EMPTY_TRIE_HASH)?;
        for (address, account) in genesis_accounts {
            let hashed_address = hash_address(&address);
            let h256_hashed_address = H256::from_slice(&hashed_address);

            // Store account code (as this won't be stored in the trie)
            let code = Code::from_bytecode(account.code, &NativeCrypto);
            let code_hash = code.hash;
            self.add_account_code(code).await?;

            // Store the account's storage in a clean storage trie and compute its root
            let mut storage_trie =
                self.open_direct_storage_trie(h256_hashed_address, *EMPTY_TRIE_HASH)?;
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
        let genesis_state_root = self.setup_genesis_state_trie(genesis.alloc).await?;
        debug_assert_eq!(genesis_state_root, genesis_block.header.state_root);

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

    pub fn get_storage_at(
        &self,
        block_number: BlockNumber,
        address: Address,
        storage_key: H256,
    ) -> Result<Option<U256>, StoreError> {
        match self.get_block_header(block_number)? {
            Some(header) => self.get_storage_at_root(header.state_root, address, storage_key),
            None => Ok(None),
        }
    }

    pub fn get_storage_at_root(
        &self,
        state_root: H256,
        address: Address,
        storage_key: H256,
    ) -> Result<Option<U256>, StoreError> {
        let account_hash = hash_address_fixed(&address);

        // Pre-acquire shared resources once for both trie opens
        let read_view = self.backend.begin_read()?;
        let cache = self.gated_snapshot(state_root)?;
        let last_written = self.last_written()?;
        let use_fkv = Self::flatkeyvalue_computed_with_last_written(account_hash, &last_written);

        let storage_root = if use_fkv {
            // We will use FKVs, we don't need the root
            *EMPTY_TRIE_HASH
        } else {
            let state_trie = self.open_state_trie_shared(
                state_root,
                read_view.clone(),
                cache.clone(),
                last_written.clone(),
            )?;
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
        // so open_storage_trie_shared falls through to the FKV path.
        let storage_root =
            if Self::flatkeyvalue_computed_with_last_written(account_hash, &last_written) {
                *EMPTY_TRIE_HASH
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

        Ok(())
    }

    /// Obtain the storage trie for the given block
    pub fn state_trie(&self, block_hash: BlockHash) -> Result<Option<Trie>, StoreError> {
        let Some(header) = self.get_block_header_by_hash(block_hash)? else {
            return Ok(None);
        };
        Ok(Some(self.open_state_trie(header.state_root)?))
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

    pub async fn get_account_state(
        &self,
        block_number: BlockNumber,
        address: Address,
    ) -> Result<Option<AccountState>, StoreError> {
        let Some(block_hash) = self.get_canonical_block_hash(block_number).await? else {
            return Ok(None);
        };
        let Some(state_trie) = self.state_trie(block_hash)? else {
            return Ok(None);
        };
        self.get_account_state_from_trie(&state_trie, address)
    }

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
        let fkv_cursor: &[u8] = last_written.as_slice();
        for (i, path) in leaf_paths.iter().enumerate() {
            if let Some(value) = trie_cache.get(state_root, path.as_slice()) {
                if !value.is_empty() {
                    results[i] = Some(AccountState::decode(&value)?);
                }
                continue;
            }
            if fkv_cursor >= path.as_slice() {
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
        // TODO: check state root
        // let Some(state_trie) = self.open_state_trie(state_trie)? else {
        //     return Ok(None);
        // };
        let state_trie = self.open_state_trie(state_root)?;
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
        if state_root == *EMPTY_TRIE_HASH {
            return Ok(true);
        }
        let trie = self.open_state_trie(state_root)?;
        // NOTE: here we hash the root because the trie doesn't check the state root is correct
        let Some(root) = trie.db().get(Nibbles::default())? else {
            return Ok(false);
        };
        let root_hash = ethrex_trie::Node::decode(&root)?
            .compute_hash(&NativeCrypto)
            .finalize(&NativeCrypto);
        Ok(state_root == root_hash)
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

/// One unit of work for the persist worker: stage block(s), build the trie
/// diff-layer, flush to disk. `wait_for_flush` selects the ack point: `false`
/// (live) acks after staging carrying the prior flush result; `true` (batch)
/// acks after flush.
struct BlockPersist {
    blocks: Vec<(Block, Vec<Receipt>)>,
    codes: Vec<(H256, Code)>,
    parent_state_root: H256,
    child_state_root: H256,
    account_updates: TrieNodesUpdate,
    storage_updates: Vec<(H256, TrieNodesUpdate)>,
    wait_for_flush: bool,
    ack: std::sync::mpsc::SyncSender<Result<(), StoreError>>,
}

/// Messages for the persist worker. `Ping(ack)` is the idle handshake for
/// [`Store::wait_for_persistence_idle`]: the FIFO worker handles it only after
/// all earlier `Block` messages are fully processed.
enum PersistMessage {
    Block(BlockPersist),
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
fn apply_trie_phase1(
    trie_cache: &Arc<RwLock<Arc<TrieLayerCache>>>,
    pending_roots: &PendingTrieRoots,
    parent_state_root: H256,
    child_state_root: H256,
    account_updates: TrieNodesUpdate,
    storage_updates: Vec<(H256, TrieNodesUpdate)>,
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
        trie_mut.put_batch(parent_state_root, child_state_root, new_layer);
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

/// When the diff-layer chain is deep enough, flush the bottom layer to disk and
/// RCU-evict it. `is_batch` selects `BATCH_COMMIT_THRESHOLD` (full sync) over
/// the default per-block threshold. No-ops when nothing is committable.
fn commit_trie_if_due(
    backend: &dyn StorageBackend,
    trie_cache: &Arc<RwLock<Arc<TrieLayerCache>>>,
    fkv_ctl: &SyncSender<FKVGeneratorControlMessage>,
    parent_state_root: H256,
    is_batch: bool,
) -> Result<(), StoreError> {
    let trie = trie_cache
        .read()
        .map_err(|_| StoreError::LockError)?
        .clone();
    // Phase 2: update disk layer.
    let commitable = if is_batch {
        trie.get_commitable_with_threshold(parent_state_root, BATCH_COMMIT_THRESHOLD)
    } else {
        trie.get_commitable(parent_state_root)
    };
    let Some(root) = commitable else {
        // Nothing to commit to disk, move on.
        return Ok(());
    };
    commit_trie_layers(backend, trie_cache, fkv_ctl, &trie, root)
}

/// Writes the layer at `root` and all of its ancestors to disk in one tx, then
/// RCU-evicts them from the cache. Used by the per-block "commit when due" path
/// ([`commit_trie_if_due`]). `trie` is the caller's snapshot of the cache;
/// `root` must be one of its layer keys.
fn commit_trie_layers(
    backend: &dyn StorageBackend,
    trie_cache: &Arc<RwLock<Arc<TrieLayerCache>>>,
    fkv_ctl: &SyncSender<FKVGeneratorControlMessage>,
    trie: &Arc<TrieLayerCache>,
    root: H256,
) -> Result<(), StoreError> {
    // Stop the flat-key-value generator thread, as the underlying trie is about to change.
    // Ignore the error, if the channel is closed it means there is no worker to notify.
    let _ = fkv_ctl.send(FKVGeneratorControlMessage::Stop);

    // RCU to remove the bottom layer: update step needs to happen after disk layer is updated.
    let mut trie_mut = (**trie).clone();

    let last_written = backend
        .begin_read()?
        .get(MISC_VALUES, "last_written".as_bytes())?
        .unwrap_or_default();

    let mut write_tx = backend.begin_write()?;

    // Before encoding, accounts have only the account address as their path, while storage keys have
    // the account address (32 bytes) + storage path (up to 32 bytes).

    // Commit removes the bottom layer and returns it, this is the mutation step.
    let nodes = trie_mut.commit(root).unwrap_or_default();
    let mut result = Ok(());
    for (key, value) in nodes {
        let is_leaf = key.len() == 65 || key.len() == 131;
        let is_account = key.len() <= 65;

        if is_leaf && key > last_written {
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
        if value.is_empty() {
            result = write_tx.delete(table, &key);
        } else {
            result = write_tx.put(table, &key, &value);
        }
        if result.is_err() {
            break;
        }
    }
    if result.is_ok() {
        result = write_tx.commit();
    }
    // We want to send this message even if there was an error during the batch write
    let _ = fkv_ctl.send(FKVGeneratorControlMessage::Continue);
    result?;
    // Phase 3: update diff layers with the removal of bottom layer.
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

pub struct AccountProof {
    pub proof: Vec<NodeRLP>,
    pub account: AccountState,
    pub storage_proof: Vec<StorageSlotProof>,
}

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
        // RocksDB only ever writes these as plain files.
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("CURRENT")).unwrap();
        fs::create_dir(dir.path().join("MANIFEST-000001")).unwrap();
        assert!(!dir_contains_legacy_db(dir.path()).unwrap());
    }
}
