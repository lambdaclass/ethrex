#[cfg(feature = "rocksdb")]
use crate::backend::rocksdb::RocksDBBackend;
use crate::{
    STORE_METADATA_FILENAME, STORE_SCHEMA_VERSION,
    api::{
        StorageBackend,
        tables::{
            ACCOUNT_CODE_METADATA, ACCOUNT_CODES, BLOCK_NUMBERS, BODIES, CANONICAL_BLOCK_HASHES,
            CHAIN_DATA, EXECUTION_WITNESSES, FULLSYNC_HEADERS, HEADERS, INVALID_CHAINS,
            MISC_VALUES, PENDING_BLOCKS, RECEIPTS, SNAP_STATE, STATE_BACKEND_FORMAT_KEY,
            TRANSACTION_LOCATIONS,
        },
    },
    backend::in_memory::InMemoryBackend,
    binary_wiring::{binary_commit_nodes_to_disk, build_binary_cache_layer},
    error::StoreError,
    layering::TrieLayerCache,
    mpt_wiring::{
        FKVGeneratorControlMessage, flatkeyvalue_generator, hash_address_fixed,
        mpt_commit_nodes_to_disk,
    },
    rlp::{BlockBodyRLP, BlockHeaderRLP, BlockRLP},
    utils::{ChainDataIndex, SnapStateIndex},
};
use ethrex_binary_trie::layer_cache::BinaryTrieLayerCache;

use bytes::Bytes;
use ethrex_common::{
    Address, H256, U256,
    types::{
        AccountInfo, AccountState, AccountUpdate, Block, BlockBody, BlockHash, BlockHeader,
        BlockNumber, ChainConfig, Code, CodeMetadata, ForkId, Index, Receipt, Transaction,
        block_execution_witness::RpcExecutionWitness,
    },
};
use ethrex_rlp::{
    decode::{RLPDecode, decode_bytes},
    encode::RLPEncode,
};
use ethrex_state_backend::{BackendKind, MerkleOutput, NodeUpdates, StateReader};
use ethrex_trie::EMPTY_TRIE_HASH;
use lru::LruCache;
use rustc_hash::FxBuildHasher;
use serde::{Deserialize, Serialize};
use std::{
    fmt::Debug,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, RwLock,
        atomic::{AtomicU8, Ordering},
        mpsc::{SyncSender, sync_channel},
    },
    thread::JoinHandle,
};
#[cfg(feature = "rocksdb")]
use tracing::warn;
use tracing::{debug, error, info};

pub use crate::mpt_wiring::{AccountProof, StorageSlotProof, StorageTrieNodes};

/// Maximum number of execution witnesses to keep in the database
pub const MAX_WITNESSES: u64 = 128;

// We use one constant for in-memory and another for on-disk backends.
// This is due to tests requiring state older than 128 blocks.
// TODO: unify these
#[allow(unused)]
const DB_COMMIT_THRESHOLD: usize = 128;
pub(crate) const IN_MEMORY_COMMIT_THRESHOLD: usize = 10000;

/// Commit threshold for batch (full sync) mode. Each batch layer holds ~1024
/// blocks of trie diffs (~1 GB), so we flush aggressively to bound memory.
const BATCH_COMMIT_THRESHOLD: usize = 4;

// 64mb
const CODE_CACHE_MAX_SIZE: u64 = 64 * 1024 * 1024;

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
/// The `Store` provides a high-level API for all blockchain data operations:
/// - Block storage and retrieval
/// - State trie management
/// - Account and storage queries
/// - Transaction indexing
///
/// # Thread Safety
///
/// `Store` is `Clone` and thread-safe. All clones share the same underlying
/// database connection and caches via `Arc`.
///
/// # Caching
///
/// The store maintains several caches for performance:
/// - **Trie Layer Cache**: Recent trie nodes for fast state access
/// - **Code Cache**: LRU cache for contract bytecode (64MB default)
/// - **Latest Block Cache**: Cached latest block header for RPC
///
/// # Example
///
/// ```ignore
/// let store = Store::new_mpt("./data", EngineType::RocksDB)?;
///
/// // Add a block
/// store.add_block(block).await?;
///
/// // Query account balance
/// let info = store.get_account_info(block_number, address)?;
/// let balance = info.map(|a| a.balance).unwrap_or_default();
/// ```
/// Everything needed to commit a block (or batch of blocks) to storage.
/// Produced by the blockchain pipeline and handed to [`Store::store_block_updates`].
pub struct UpdateBatch {
    /// Trie node diffs to write. The enum variant identifies the active backend.
    pub node_updates: NodeUpdates,
    /// Contract code updates (code hash -> bytecode).
    pub code_updates: Vec<(H256, Code)>,
    /// Blocks to store.
    pub blocks: Vec<Block>,
    /// Receipts to store, grouped by block hash.
    pub receipts: Vec<(H256, Vec<Receipt>)>,
    /// Whether this batch comes from full sync (batch execution mode).
    /// When true, uses `BATCH_COMMIT_THRESHOLD` (aggressive) instead of
    /// `DB_COMMIT_THRESHOLD` to bound memory during bulk block import.
    pub batch_mode: bool,
}

#[derive(Debug)]
pub struct Store {
    /// Path to the database directory.
    db_path: PathBuf,
    /// Storage backend (InMemory or RocksDB).
    pub(crate) backend: Arc<dyn StorageBackend>,
    /// Chain configuration (fork schedule, chain ID, etc.).
    chain_config: ChainConfig,
    /// Cache for trie nodes from recent blocks.
    pub(crate) trie_cache: Arc<RwLock<Arc<TrieLayerCache>>>,
    /// Channel for controlling the FlatKeyValue generator background task.
    flatkeyvalue_control_tx: std::sync::mpsc::SyncSender<FKVGeneratorControlMessage>,
    /// Channel for sending trie updates to the background worker.
    trie_update_worker_tx: std::sync::mpsc::SyncSender<TrieUpdate>,
    /// Cached latest canonical block header.
    ///
    /// Wrapped in Arc for cheap reads with infrequent writes.
    /// May be slightly out of date, which is acceptable for:
    /// - Caching frequently requested headers
    /// - RPC "latest" block queries (small delay acceptable)
    /// - Sync operations (must be idempotent anyway)
    pub(crate) latest_block_header: LatestBlockHeaderCache,
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

    background_threads: Arc<ThreadList>,

    /// Which trie backend is active for this store. Stored as an atomic byte
    /// so `set_backend_kind` can update it in-process without requiring a
    /// restart (hot-swap). The byte encoding is via `backend_kind_to_byte` /
    /// `byte_to_backend_kind`.
    ///
    /// Wrapped in `Arc` so all `Store` clones share the same atomic — a
    /// hot-swap via `set_backend_kind` is therefore visible to every live
    /// handle (RPC handlers, engine API, SyncManager, …) without a restart.
    pub(crate) backend_kind: Arc<AtomicU8>,

    /// Cache for binary trie leaf diffs (FKV), one layer per block.
    ///
    /// Independent from `trie_cache` (MPT node cache). During `Transition` mode
    /// both caches coexist, each keyed by its own root hash; they never cross-read.
    /// For `BackendKind::Mpt` this cache is present but always empty.
    pub(crate) binary_trie_cache: Arc<RwLock<Arc<BinaryTrieLayerCache>>>,

    /// Transition metadata loaded at startup when `backend_kind == Transition`.
    ///
    /// Contains `(switch_block, frozen_mpt_root, binary_root)` as persisted by
    /// `Store::persist_transition_metadata`. Present only for `Transition` stores;
    /// `None` for `Mpt` and `Binary` stores.
    ///
    /// Wrapped in `Arc<RwLock<…>>` so all `Store` clones share the same lock —
    /// a write by `persist_transition_metadata` (called during hot-swap) is
    /// immediately visible to every live clone (RPC, engine API, SyncManager).
    pub(crate) transition_metadata: Arc<RwLock<Option<(u64, H256, H256)>>>,

    /// Shared mutex that serialises block execution against activation.
    ///
    /// Both `execute_block_pipeline` (in the blockchain crate) and
    /// `TransitionActivator::activate` acquire this lock.  The activation
    /// write is therefore exclusive with any concurrent block commit, which
    /// prevents a race between writing the format byte and applying trie
    /// updates.
    ///
    /// The lock itself carries no data (`()`); it is purely a coordination
    /// primitive.
    activation_lock: Arc<std::sync::Mutex<()>>,

    /// Current binary trie head root.
    ///
    /// Advances per block in `apply_trie_updates` (Phase 1) when a binary
    /// `TrieUpdate` is processed: set to `child_state_root` of the just-applied
    /// layer. Read by `new_transition_state_reader` so each block's overlay
    /// reads see the live binary trie head, not the frozen activation snapshot.
    ///
    /// Why not derive from `transition_metadata.binary_root` or on-disk
    /// `META_ROOT_HASH`: the former is frozen at activation and never updated;
    /// the latter is updated only on `BinaryTrieLayerCache` Phase-2 disk
    /// commits which fire at the 128-layer threshold, so during catchup the
    /// on-disk root lags ≥ 1 block behind the in-memory layer cache. Reading
    /// either leads to overlay reads that miss the latest block's writes
    /// (Bug 4, hoodi 2026-05-05).
    ///
    /// Default: `EMPTY_BINARY_ROOT` (`H256::zero()`).
    pub(crate) current_binary_root: Arc<RwLock<H256>>,
}

impl Clone for Store {
    fn clone(&self) -> Self {
        Store {
            db_path: self.db_path.clone(),
            backend: self.backend.clone(),
            chain_config: self.chain_config,
            latest_block_header: self.latest_block_header.clone(),
            trie_cache: self.trie_cache.clone(),
            flatkeyvalue_control_tx: self.flatkeyvalue_control_tx.clone(),
            trie_update_worker_tx: self.trie_update_worker_tx.clone(),
            last_computed_flatkeyvalue: self.last_computed_flatkeyvalue.clone(),
            account_code_cache: self.account_code_cache.clone(),
            code_metadata_cache: self.code_metadata_cache.clone(),
            background_threads: self.background_threads.clone(),
            backend_kind: self.backend_kind.clone(),
            binary_trie_cache: self.binary_trie_cache.clone(),
            transition_metadata: self.transition_metadata.clone(),
            activation_lock: self.activation_lock.clone(),
            current_binary_root: self.current_binary_root.clone(),
        }
    }
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

impl Store {
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

            // TODO: Same logic in apply_updates
            for block in blocks {
                let block_number = block.header.number;
                let block_hash = block.hash();
                let hash_key = block_hash.encode_to_vec();

                let header_value_rlp = BlockHeaderRLP::from(block.header.clone());
                tx.put(HEADERS, &hash_key, header_value_rlp.bytes())?;

                let body_value = BlockBodyRLP::from_bytes(block.body.encode_to_vec());
                tx.put(BODIES, &hash_key, body_value.bytes())?;

                tx.put(BLOCK_NUMBERS, &hash_key, &block_number.to_le_bytes())?;

                for (index, transaction) in block.body.transactions.iter().enumerate() {
                    let tx_hash = transaction.hash();
                    // Key: tx_hash + block_hash
                    let mut composite_key = Vec::with_capacity(64);
                    composite_key.extend_from_slice(tx_hash.as_bytes());
                    composite_key.extend_from_slice(block_hash.as_bytes());
                    let location_value = (block_number, block_hash, index as u64).encode_to_vec();
                    tx.put(TRANSACTION_LOCATIONS, &composite_key, &location_value)?;
                }
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
        self.load_block_header(block_number)
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
        let backend = self.backend.clone();
        // TODO: Implement read bulk
        tokio::task::spawn_blocking(move || {
            let txn = backend.begin_read()?;
            let mut block_bodies = Vec::new();
            for hash in hashes {
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
        // FIXME: Use dupsort table
        let mut composite_key = Vec::with_capacity(64);
        composite_key.extend_from_slice(transaction_hash.as_bytes());
        composite_key.extend_from_slice(block_hash.as_bytes());
        let location_value = (block_number, block_hash, index).encode_to_vec();

        self.write_async(TRANSACTION_LOCATIONS, composite_key, location_value)
            .await
    }

    /// Store transaction locations in batch (one db transaction for all)
    pub async fn add_transaction_locations(
        &self,
        locations: Vec<(H256, BlockNumber, BlockHash, Index)>,
    ) -> Result<(), StoreError> {
        let batch_items: Vec<_> = locations
            .iter()
            .map(|(tx_hash, block_number, block_hash, index)| {
                let mut composite_key = Vec::with_capacity(64);
                composite_key.extend_from_slice(tx_hash.as_bytes());
                composite_key.extend_from_slice(block_hash.as_bytes());
                let location_value = (*block_number, *block_hash, *index).encode_to_vec();
                (composite_key, location_value)
            })
            .collect();

        self.write_batch_async(TRANSACTION_LOCATIONS, batch_items)
            .await
    }

    /// Obtain transaction location (block hash and index)
    pub async fn get_transaction_location(
        &self,
        transaction_hash: H256,
    ) -> Result<Option<(BlockNumber, BlockHash, Index)>, StoreError> {
        let db = self.backend.clone();
        tokio::task::spawn_blocking(move || {
            let tx_hash_bytes = transaction_hash.as_bytes();
            let tx = db.begin_read()?;

            // Use prefix iterator to find all entries with this transaction hash
            let mut iter = tx.prefix_iterator(TRANSACTION_LOCATIONS, tx_hash_bytes)?;
            let mut transaction_locations = Vec::new();

            while let Some(Ok((key, value))) = iter.next() {
                // Ensure key is exactly tx_hash + block_hash (32 + 32 = 64 bytes)
                // and starts with our exact tx_hash
                if key.len() == 64 && &key[0..32] == tx_hash_bytes {
                    transaction_locations.push(<(BlockNumber, BlockHash, Index)>::decode(&value)?);
                }
            }

            if transaction_locations.is_empty() {
                return Ok(None);
            }

            // If there are multiple locations, filter by the canonical chain
            for (block_number, block_hash, index) in transaction_locations {
                let canonical_hash = {
                    tx.get(
                        CANONICAL_BLOCK_HASHES,
                        block_number.to_le_bytes().as_slice(),
                    )?
                    .map(|bytes| H256::decode(bytes.as_slice()))
                    .transpose()?
                };

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
        // FIXME: Use dupsort table
        let key = (block_hash, index).encode_to_vec();
        let value = receipt.encode_to_vec();
        self.write_async(RECEIPTS, key, value).await
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
                let key = (block_hash, index as u64).encode_to_vec();
                let value = receipt.encode_to_vec();
                (key, value)
            })
            .collect();
        self.write_batch_async(RECEIPTS, batch_items).await
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
        let key = (block_hash, index).encode_to_vec();
        self.read_async(RECEIPTS, key)
            .await?
            .map(|bytes| Receipt::decode(bytes.as_slice()))
            .transpose()
            .map_err(StoreError::from)
    }

    /// Get account code by its hash.
    ///
    /// Check if the code exists in the cache (attribute `account_code_cache`), if not,
    /// reads the database, and if it exists, decodes and returns it.
    pub fn get_account_code(&self, code_hash: H256) -> Result<Option<Code>, StoreError> {
        use ethrex_common::constants::EMPTY_KECCACK_HASH;

        // Empty-code special case: every empty-coded account hashes to
        // EMPTY_KECCACK_HASH. We don't store that entry in `ACCOUNT_CODES`
        // explicitly (saves space across millions of empty accounts), but
        // callers — notably the snap-server's `GetByteCodes` handler — must
        // see a `Some(empty)` to satisfy the eth/devp2p protocol expectation.
        // Mirrors the `get_code_metadata` empty-code special case.
        if code_hash == *EMPTY_KECCACK_HASH {
            return Ok(Some(Code {
                hash: code_hash,
                bytecode: Bytes::new(),
                jump_targets: Vec::new(),
            }));
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
        let bytes = Bytes::from_owner(bytes);
        let (bytecode_slice, targets) = decode_bytes(&bytes)?;
        let bytecode = bytes.slice_ref(bytecode_slice);

        let code = Code {
            hash: code_hash,
            bytecode,
            jump_targets: <Vec<_>>::decode(targets)?,
        };

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
        use ethrex_common::constants::EMPTY_KECCACK_HASH;

        // Empty code special case
        if code_hash == *EMPTY_KECCACK_HASH {
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
                length: code.bytecode.len() as u64,
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
        let metadata_buf = (code.bytecode.len() as u64).to_be_bytes();

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

    pub async fn forkchoice_update_inner(
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
        self.get_receipts_for_block_from_index(block_hash, 0).await
    }

    /// Retrieves receipts for a block starting from the given index.
    /// Used by eth/70 partial receipt requests (EIP-7975).
    pub async fn get_receipts_for_block_from_index(
        &self,
        block_hash: &BlockHash,
        start_index: u64,
    ) -> Result<Vec<Receipt>, StoreError> {
        let mut receipts = Vec::new();
        let mut index = start_index;

        let txn = self.backend.begin_read()?;
        loop {
            let key = (*block_hash, index).encode_to_vec();
            match txn.get(RECEIPTS, key.as_slice())? {
                Some(receipt_bytes) => {
                    let receipt = Receipt::decode(receipt_bytes.as_slice())?;
                    receipts.push(receipt);
                    index += 1;
                }
                None => break,
            }
        }

        Ok(receipts)
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

    /// Write trie node diffs directly to disk, bypassing TrieLayerCache.
    /// Dispatches to the backend-specific writer based on the [`NodeUpdates`] variant.
    ///
    /// For `NodeUpdates::Binary`, writes trie nodes, stem tombstones, and FKV
    /// leaf entries to disk in a single atomic transaction via
    /// [`binary_commit_nodes_to_disk`].
    pub(crate) fn write_node_updates_direct(
        &self,
        node_updates: NodeUpdates,
    ) -> Result<(), StoreError> {
        match node_updates {
            NodeUpdates::Mpt {
                state_updates,
                storage_updates,
            } => self.write_mpt_node_updates(state_updates, storage_updates),
            NodeUpdates::Binary {
                node_diffs,
                deleted_stems,
                fkv_entries,
            } => binary_commit_nodes_to_disk(
                self.backend.as_ref(),
                node_diffs,
                deleted_stems,
                fkv_entries,
            ),
        }
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
            let metadata_buf = (code.bytecode.len() as u64).to_be_bytes().to_vec();
            code_batch_items.push((code_hash.as_bytes().to_vec(), buf));
            metadata_batch_items.push((code_hash.as_bytes().to_vec(), metadata_buf));
        }

        // Write both batches
        self.write_batch_async(ACCOUNT_CODES, code_batch_items)
            .await?;
        self.write_batch_async(ACCOUNT_CODE_METADATA, metadata_batch_items)
            .await
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

    fn apply_updates(&self, update_batch: UpdateBatch) -> Result<(), StoreError> {
        let db = self.backend.clone();
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
        let trie_upd_worker_tx = self.trie_update_worker_tx.clone();

        // Capacity one ensures sender just notifies and goes on
        let (notify_tx, notify_rx) = sync_channel(1);
        let wait_for_new_layer = notify_rx;
        let trie_update = TrieUpdate {
            parent_state_root,
            node_updates: update_batch.node_updates,
            result_sender: notify_tx,
            child_state_root: last_state_root,
            is_batch: update_batch.batch_mode,
        };
        trie_upd_worker_tx.send(trie_update).map_err(|e| {
            StoreError::Custom(format!("failed to read new trie layer notification: {e}"))
        })?;
        let mut tx = db.begin_write()?;

        for block in update_batch.blocks {
            let block_number = block.header.number;
            let block_hash = block.hash();
            let hash_key = block_hash.encode_to_vec();

            let header_value_rlp = BlockHeaderRLP::from(block.header.clone());
            tx.put(HEADERS, &hash_key, header_value_rlp.bytes())?;

            let body_value = BlockBodyRLP::from_bytes(block.body.encode_to_vec());
            tx.put(BODIES, &hash_key, body_value.bytes())?;

            tx.put(BLOCK_NUMBERS, &hash_key, &block_number.to_le_bytes())?;

            for (index, transaction) in block.body.transactions.iter().enumerate() {
                let tx_hash = transaction.hash();
                // Key: tx_hash + block_hash
                let mut composite_key = Vec::with_capacity(64);
                composite_key.extend_from_slice(tx_hash.as_bytes());
                composite_key.extend_from_slice(block_hash.as_bytes());
                let location_value = (block_number, block_hash, index as u64).encode_to_vec();
                tx.put(TRANSACTION_LOCATIONS, &composite_key, &location_value)?;
            }
        }

        for (block_hash, receipts) in update_batch.receipts {
            for (index, receipt) in receipts.into_iter().enumerate() {
                let key = (block_hash, index as u64).encode_to_vec();
                let value = receipt.encode_to_vec();
                tx.put(RECEIPTS, &key, &value)?;
            }
        }

        for (code_hash, code) in update_batch.code_updates {
            let buf = encode_code(&code);
            let metadata_buf = (code.bytecode.len() as u64).to_be_bytes();
            tx.put(ACCOUNT_CODES, code_hash.as_ref(), &buf)?;
            tx.put(ACCOUNT_CODE_METADATA, code_hash.as_ref(), &metadata_buf)?;
        }

        // Wait for an updated top layer so every caller afterwards sees a consistent view.
        // Specifically, the next block produced MUST see this upper layer.
        wait_for_new_layer
            .recv()
            .map_err(|e| StoreError::Custom(format!("recv failed: {e}")))??;
        // After top-level is added, we can make the rest of the changes visible.
        tx.commit()?;

        Ok(())
    }

    pub fn new(
        path: impl AsRef<Path>,
        engine_type: EngineType,
        backend_kind: BackendKind,
    ) -> Result<Self, StoreError> {
        let db_path = path.as_ref().to_path_buf();

        if engine_type != EngineType::InMemory {
            let version = read_store_schema_version(&db_path)?;

            match version {
                None if db_path.exists() && !dir_is_empty(&db_path)? => {
                    // Pre-metadata DB — cannot migrate safely
                    return Err(StoreError::NotFoundDBVersion);
                }
                None => {
                    // Fresh / empty directory — write initial metadata
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
                    // Open backend, run migrations, then proceed with the same Arc
                    let backend: Arc<dyn crate::api::StorageBackend> =
                        Arc::new(RocksDBBackend::open(&path)?);
                    crate::migrations::run_pending_migrations(backend.as_ref(), &db_path, v)?;
                    return Self::from_backend(backend, db_path, DB_COMMIT_THRESHOLD, backend_kind);
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
                let backend = Arc::new(RocksDBBackend::open(path)?);
                Self::from_backend(backend, db_path, DB_COMMIT_THRESHOLD, backend_kind)
            }
            EngineType::InMemory => {
                let backend = Arc::new(InMemoryBackend::open()?);
                Self::from_backend(backend, db_path, IN_MEMORY_COMMIT_THRESHOLD, backend_kind)
            }
        }
    }

    /// Transitional convenience wrapper that creates a store using the MPT
    /// trie backend. To be removed when a second `BackendKind` variant lands;
    /// at that point every caller must name `BackendKind` explicitly so the
    /// choice is visible at the call site.
    pub fn new_mpt(path: impl AsRef<Path>, engine_type: EngineType) -> Result<Self, StoreError> {
        Self::new(path, engine_type, BackendKind::Mpt)
    }

    /// Read the raw flat-state format marker byte from `MISC_VALUES`.
    /// Returns `None` when the marker has not been written yet (should not
    /// happen after a successful `Store::new`).
    pub fn read_state_backend_format_byte(&self) -> Result<Option<u8>, StoreError> {
        let tx = self.backend.begin_read()?;
        let bytes = tx.get(MISC_VALUES, STATE_BACKEND_FORMAT_KEY)?;
        match bytes {
            None => Ok(None),
            Some(b) if b.len() == 1 => Ok(Some(b[0])),
            Some(b) => Err(StoreError::Custom(format!(
                "state backend format marker has unexpected length {} (expected 1)",
                b.len()
            ))),
        }
    }

    /// Opens the backend just enough to read the `STATE_BACKEND_FORMAT_KEY` byte,
    /// without running any schema migrations or mismatch checks.
    ///
    /// Returns `None` if the key is absent (fresh DB or in-memory path).
    /// Used by the CLI to determine the correct `BackendKind` before calling
    /// `Store::new`, so the Store is opened with the right variant from the start.
    pub fn peek_backend_format_byte(
        #[allow(unused_variables)] path: impl AsRef<std::path::Path>,
        engine_type: EngineType,
    ) -> Result<Option<u8>, StoreError> {
        match engine_type {
            EngineType::InMemory => {
                // In-memory stores are never persisted; always fresh.
                return Ok(None);
            }
            #[cfg(feature = "rocksdb")]
            EngineType::RocksDB => {}
        }
        #[cfg(feature = "rocksdb")]
        {
            // Opening RocksDB on a fresh datadir writes bootstrap files
            // (CURRENT, MANIFEST, OPTIONS-*).  Subsequently `Store::new`'s
            // schema-version check would see a non-empty dir without
            // metadata.json and fail with NotFoundDBVersion.  Skip the peek
            // when the datadir has no valid DB; a fresh datadir cannot be in
            // transition mode by definition.
            if !has_valid_db(path.as_ref()) {
                return Ok(None);
            }
            let backend: Arc<dyn StorageBackend> = Arc::new(RocksDBBackend::open(path.as_ref())?);
            let tx = backend.begin_read()?;
            let bytes = tx.get(MISC_VALUES, STATE_BACKEND_FORMAT_KEY)?;
            return match bytes {
                None => Ok(None),
                Some(b) if b.len() == 1 => Ok(Some(b[0])),
                Some(b) => Err(StoreError::Custom(format!(
                    "state backend format marker has unexpected length {} (expected 1)",
                    b.len()
                ))),
            };
        }
        #[allow(unreachable_code)]
        Ok(None)
    }

    pub(crate) fn from_backend(
        backend: Arc<dyn StorageBackend>,
        db_path: PathBuf,
        commit_threshold: usize,
        backend_kind: BackendKind,
    ) -> Result<Self, StoreError> {
        debug!("Initializing Store with {commit_threshold} in-memory diff-layers");
        let (fkv_tx, fkv_rx) = std::sync::mpsc::sync_channel(0);
        let (trie_upd_tx, trie_upd_rx) = std::sync::mpsc::sync_channel(0);

        let last_written = {
            let tx = backend.begin_read()?;
            let last_written = tx
                .get(MISC_VALUES, "last_written".as_bytes())?
                .unwrap_or_else(|| vec![0u8; 64]);
            if last_written == [0xff] {
                vec![0xff; 64]
            } else {
                last_written
            }
        };

        // Flat-state tables (`ACCOUNT_FLATKEYVALUE`, `STORAGE_FLATKEYVALUE`)
        // store raw key bytes whose meaning depends on the backend (MPT writes
        // keccak-nibble paths; a future binary backend would write a different
        // key derivation). Opening a DB with the wrong backend would silently
        // return garbage. To prevent that, stamp a single-byte format marker
        // in `MISC_VALUES` on first open, and fail fast on any subsequent open
        // whose configured `BackendKind` disagrees with the marker.
        {
            let on_disk = backend
                .begin_read()?
                .get(MISC_VALUES, STATE_BACKEND_FORMAT_KEY)?;
            match on_disk {
                None => {
                    // First open: write the marker.
                    let mut write_tx = backend.begin_write()?;
                    write_tx.put(
                        MISC_VALUES,
                        STATE_BACKEND_FORMAT_KEY,
                        &[backend_kind_to_byte(backend_kind)],
                    )?;
                    write_tx.commit()?;
                }
                Some(bytes) => {
                    let byte = match bytes.as_slice() {
                        [b] => *b,
                        other => {
                            return Err(StoreError::Custom(format!(
                                "state backend format marker has unexpected length {} (expected 1)",
                                other.len()
                            )));
                        }
                    };
                    let on_disk_kind = byte_to_backend_kind(byte)?;
                    if on_disk_kind != backend_kind {
                        // Provide a clear, actionable error for the most common
                        // mismatch: DB has been transitioned (format byte 2) but
                        // the node was restarted without --binary-transition.
                        if byte == 2 && backend_kind == BackendKind::Mpt {
                            return Err(StoreError::Custom(
                                "database has format byte 2 (transition) but --binary-transition \
                                 was not passed. Restart with --binary-transition to resume in \
                                 transition mode, or point ethrex at a different datadir."
                                    .to_string(),
                            ));
                        }
                        return Err(StoreError::Custom(format!(
                            "state backend format mismatch: on-disk={on_disk_kind:?}, configured={backend_kind:?}"
                        )));
                    }
                }
            }
        }

        // For Transition mode, load the three metadata keys from MISC_VALUES.
        // Fail fast if any key is missing — the store cannot operate as a
        // transition backend without them (they are written atomically on activation).
        let transition_metadata = if backend_kind == BackendKind::Transition {
            let tx = backend.begin_read()?;
            use crate::api::tables::{
                TRANSITION_BINARY_ROOT_KEY, TRANSITION_MPT_FROZEN_ROOT_KEY,
                TRANSITION_SWITCH_BLOCK_KEY,
            };
            let switch_block_bytes = tx
                .get(MISC_VALUES, TRANSITION_SWITCH_BLOCK_KEY)?
                .ok_or_else(|| {
                    StoreError::Custom(
                        "transition store is missing TRANSITION_SWITCH_BLOCK_KEY".to_string(),
                    )
                })?;
            let mpt_root_bytes = tx
                .get(MISC_VALUES, TRANSITION_MPT_FROZEN_ROOT_KEY)?
                .ok_or_else(|| {
                    StoreError::Custom(
                        "transition store is missing TRANSITION_MPT_FROZEN_ROOT_KEY".to_string(),
                    )
                })?;
            let binary_root_bytes = tx
                .get(MISC_VALUES, TRANSITION_BINARY_ROOT_KEY)?
                .ok_or_else(|| {
                    StoreError::Custom(
                        "transition store is missing TRANSITION_BINARY_ROOT_KEY".to_string(),
                    )
                })?;
            let switch_block = if switch_block_bytes.len() == 8 {
                u64::from_be_bytes(
                    switch_block_bytes
                        .as_slice()
                        .try_into()
                        .expect("length checked above"),
                )
            } else {
                return Err(StoreError::Custom(format!(
                    "TRANSITION_SWITCH_BLOCK_KEY has unexpected length {} (expected 8)",
                    switch_block_bytes.len()
                )));
            };
            let mpt_root = if mpt_root_bytes.len() == 32 {
                H256::from_slice(&mpt_root_bytes)
            } else {
                return Err(StoreError::Custom(format!(
                    "TRANSITION_MPT_FROZEN_ROOT_KEY has unexpected length {} (expected 32)",
                    mpt_root_bytes.len()
                )));
            };
            let binary_root = if binary_root_bytes.len() == 32 {
                H256::from_slice(&binary_root_bytes)
            } else {
                return Err(StoreError::Custom(format!(
                    "TRANSITION_BINARY_ROOT_KEY has unexpected length {} (expected 32)",
                    binary_root_bytes.len()
                )));
            };
            Some((switch_block, mpt_root, binary_root))
        } else {
            None
        };

        // Load the in-memory binary head root from disk-persisted META_ROOT_HASH.
        // The worker advances this per-block as new layers land in
        // `binary_trie_cache`; we seed it here so it survives restarts.
        // For fresh DBs (no binary commits yet), defaults to EMPTY_BINARY_ROOT.
        let initial_binary_root = {
            use crate::api::tables::BINARY_TRIE_NODES;
            use ethrex_binary_trie::META_ROOT_HASH;
            let tx = backend.begin_read()?;
            match tx.get(BINARY_TRIE_NODES, META_ROOT_HASH)? {
                Some(b) if b.len() == 32 => H256::from_slice(&b),
                Some(b) => {
                    return Err(StoreError::Custom(format!(
                        "META_ROOT_HASH has unexpected length {} (expected 32)",
                        b.len()
                    )));
                }
                None => H256::zero(),
            }
        };

        let mut background_threads = Vec::new();
        let mut store = Self {
            db_path,
            backend,
            chain_config: Default::default(),
            latest_block_header: Default::default(),
            trie_cache: Arc::new(RwLock::new(Arc::new(TrieLayerCache::new(commit_threshold)))),
            flatkeyvalue_control_tx: fkv_tx,
            trie_update_worker_tx: trie_upd_tx,
            last_computed_flatkeyvalue: Arc::new(RwLock::new(last_written)),
            account_code_cache: Arc::new(Mutex::new(CodeCache::default())),
            code_metadata_cache: Arc::new(Mutex::new(rustc_hash::FxHashMap::default())),
            background_threads: Default::default(),
            backend_kind: Arc::new(AtomicU8::new(backend_kind_to_byte(backend_kind))),
            binary_trie_cache: Arc::new(RwLock::new(Arc::new(BinaryTrieLayerCache::new(
                commit_threshold,
            )))),
            transition_metadata: Arc::new(RwLock::new(transition_metadata)),
            activation_lock: Arc::new(std::sync::Mutex::new(())),
            // Initialise the in-memory binary head root from disk-persisted
            // META_ROOT_HASH (last-flushed binary root). For fresh activation
            // (no binary commits yet), this is `EMPTY_BINARY_ROOT`. The worker
            // advances this per-block as new layers land in `binary_trie_cache`.
            current_binary_root: Arc::new(RwLock::new(initial_binary_root)),
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

            match backend_kind {
                BackendKind::Mpt => {
                    let _ = flatkeyvalue_generator(&backend_clone, &last_computed_fkv, &rx)
                        .inspect_err(|err| error!("Error while generating FlatKeyValue: {err}"));
                }
                // Binary FKV is populated inline on commit (no background generator).
                // The FKV generator thread exits immediately for Binary/Transition;
                // the MPT fkv_ctl channel remains present but carries no traffic.
                BackendKind::Binary | BackendKind::Transition => {
                    info!(
                        "Binary/Transition mode: no FKV background generator \
                         (FKV populated inline on commit)."
                    );
                }
            }
        }));
        let backend = store.backend.clone();
        let flatkeyvalue_control_tx = store.flatkeyvalue_control_tx.clone();
        let trie_cache = store.trie_cache.clone();
        let binary_trie_cache = store.binary_trie_cache.clone();
        let current_binary_root = store.current_binary_root.clone();
        /*
            When a block is executed, the write of the bottom-most diff layer to disk is done in the background through this thread.
            This is to improve block execution times, since it's not necessary when executing the next block to have this layer flushed to disk.

            This background thread receives messages through a channel to apply new trie updates and does three things:

            - First, it updates the top-most in-memory diff layer and notifies the process that sent the message (i.e. the
            block production thread) so it can continue with block execution (block execution cannot proceed without the
            diff layers updated, otherwise it would see wrong state when reading from the trie). This section is done in an RCU manner:
            a shared pointer with the trie is kept behind a lock. This thread first acquires the lock, then copies the pointer and drops the lock;
            afterwards it makes a deep copy of the trie layer and mutates it, then takes the lock again, replaces the pointer with the updated copy,
            then drops the lock again.

            - Second, it performs the logic of persisting the bottom-most diff layer to disk. This is the part of the logic that block execution does not
            need to proceed. What does need to be aware of this section is the process in charge of generating the snapshot (a.k.a. FlatKeyValue).
            Because of this, this section first sends a message to pause the FlatKeyValue generation, then persists the diff layer to disk, then notifies
            again for FlatKeyValue generation to continue.

            - Third, it removes the (no longer needed) bottom-most diff layer from the trie layers in the same way as the first step.
        */
        background_threads.push(std::thread::spawn(move || {
            let rx = trie_upd_rx;
            loop {
                match rx.recv() {
                    Ok(trie_update) => {
                        // FIXME: what should we do on error?
                        let _ = apply_trie_updates(
                            backend.as_ref(),
                            &flatkeyvalue_control_tx,
                            &trie_cache,
                            &binary_trie_cache,
                            &current_binary_root,
                            trie_update,
                        )
                        .inspect_err(|err| error!("apply_trie_updates failed: {err}"));
                    }
                    Err(err) => {
                        debug!("Trie update sender disconnected: {err}");
                        return;
                    }
                }
            }
        }));
        store.background_threads = Arc::new(ThreadList {
            list: background_threads,
        });
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
        let Some(header) = self.get_block_header_by_hash(block_hash)? else {
            return Ok(None);
        };
        self.new_state_reader(header.state_root)?
            .account(address)
            .map_err(StoreError::from)
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
        let Some(header) = self.get_block_header_by_hash(block_hash)? else {
            return Ok(None);
        };
        let Some(account_info) = self
            .new_state_reader(header.state_root)?
            .account(address)
            .map_err(StoreError::from)?
        else {
            return Ok(None);
        };
        self.get_account_code(account_info.code_hash)
    }

    pub async fn get_nonce_by_account_address(
        &self,
        block_number: BlockNumber,
        address: Address,
    ) -> Result<Option<u64>, StoreError> {
        let Some(block_hash) = self.get_canonical_block_hash(block_number).await? else {
            return Ok(None);
        };
        let Some(header) = self.get_block_header_by_hash(block_hash)? else {
            return Ok(None);
        };
        let account_info = self
            .new_state_reader(header.state_root)?
            .account(address)
            .map_err(StoreError::from)?;
        Ok(account_info.map(|info| info.nonce))
    }

    /// Applies account updates based on the block's latest storage state
    /// and returns the new state root after the updates have been applied.
    ///
    /// Dispatches the underlying state reader on the active `backend_kind` so
    /// `Blockchain::add_block` (the non-pipelined path that calls this) does
    /// not silently route through the MPT-only reader after Transition
    /// activation. Same Bug 0 / Bug 5 family — if this used `new_state_reader`
    /// unconditionally, post-switch state writes here would never reach the
    /// binary overlay.
    pub fn apply_account_updates_batch(
        &self,
        block_hash: BlockHash,
        account_updates: &[AccountUpdate],
    ) -> Result<Option<MerkleOutput>, StoreError> {
        let Some(header) = self.get_block_header_by_hash(block_hash)? else {
            return Ok(None);
        };
        let backend = match self.backend_kind() {
            BackendKind::Mpt => self.new_state_reader(header.state_root)?,
            BackendKind::Transition => {
                let (switch_block, frozen_mpt_root, binary_root) =
                    self.transition_metadata().ok_or_else(|| {
                        StoreError::Custom(
                            "Transition mode requires transition_metadata; not loaded".to_string(),
                        )
                    })?;
                self.new_transition_state_reader(switch_block, frozen_mpt_root, binary_root)?
            }
            BackendKind::Binary => self.new_binary_state_reader(header.state_root)?,
        };
        Ok(Some(
            backend
                .apply_account_updates(account_updates)
                .map_err(StoreError::from)?,
        ))
    }

    // Key format: block_number (8 bytes, big-endian) + block_hash (32 bytes)
    pub(crate) fn make_witness_key(block_number: u64, block_hash: &BlockHash) -> Vec<u8> {
        let mut composite_key = Vec::with_capacity(8 + 32);
        composite_key.extend_from_slice(&block_number.to_be_bytes());
        composite_key.extend_from_slice(block_hash.as_bytes());
        composite_key
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

    pub async fn load_initial_state(&self) -> Result<(), StoreError> {
        info!("Loading initial state from DB");
        let Some(number) = self.load_latest_block_number().await? else {
            return Err(StoreError::MissingLatestBlockNumber);
        };
        let latest_block_header = self
            .load_block_header(number)?
            .ok_or_else(|| StoreError::Custom("latest block header is missing".to_string()))?;
        self.latest_block_header.update(latest_block_header);
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
        let last_written = self.last_written()?;
        let use_fkv = flatkeyvalue_computed_with_last_written(account_hash, &last_written);

        // When FKV has already been populated for this account, the account's
        // on-trie `storage_root` field may be stale relative to the flat-state
        // tables. Skip reading the account and open the storage trie at
        // `EMPTY_TRIE_HASH` so `Trie::get` falls straight to the FKV lookup
        // via `flatkeyvalue_computed`.
        let storage_root = if use_fkv {
            *EMPTY_TRIE_HASH
        } else {
            let state_trie = self.open_state_trie(state_root)?;
            let Some(encoded_account) = state_trie.get(account_hash.as_bytes())? else {
                return Ok(None);
            };
            AccountState::decode(&encoded_account)?.storage_root
        };

        let storage_trie = self.open_storage_trie(account_hash, state_root, storage_root)?;
        let hashed_key = ethrex_trie::hash_key(&storage_key);
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
        // Updates first the latest_block_header to avoid nonce inconsistencies #3927.
        let new_head = self
            .load_block_header_by_hash(head_hash)?
            .ok_or_else(|| StoreError::MissingLatestBlockNumber)?;
        self.latest_block_header.update(new_head);
        self.forkchoice_update_inner(
            new_canonical_blocks,
            head_number,
            head_hash,
            safe,
            finalized,
        )
        .await?;

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

    // -------------------------------------------------------------------------
    // Activation-lock and flush helpers (Phase 7 — binary trie transition)
    // -------------------------------------------------------------------------

    /// Returns a clone of the `Arc<Mutex<()>>` that serialises block execution
    /// against the binary-trie activation write.
    ///
    /// Both `execute_block_pipeline` (blockchain crate) and
    /// `TransitionActivator::activate` acquire this lock; the activation
    /// metadata write is therefore atomic w.r.t. concurrent block commits.
    pub fn activation_lock(&self) -> Arc<std::sync::Mutex<()>> {
        self.activation_lock.clone()
    }

    /// Sends `FKVGeneratorControlMessage::Stop` to the MPT FKV generator.
    ///
    /// Best-effort: if the channel is already closed (e.g. for Binary/Transition
    /// stores that never started the generator), the error is swallowed.
    pub fn stop_fkv_generator(&self) {
        let _ = self
            .flatkeyvalue_control_tx
            .send(FKVGeneratorControlMessage::Stop);
    }

    /// Force-flushes all in-memory MPT `TrieLayerCache` layers to disk.
    ///
    /// Used by the activation sequence to ensure the frozen MPT root on disk
    /// matches the block header's `state_root` before the format byte is
    /// written.
    ///
    /// This bypasses the normal commit-threshold logic and writes every
    /// accumulated layer, newest-to-oldest, in a single pass.
    /// Force-flushes all in-memory MPT `TrieLayerCache` layers reachable from
    /// `from_root` (oldest-first) to disk.
    ///
    /// `from_root` MUST be the most recent state_root we want frozen (typically
    /// the state_root of the block we just committed via `execute_block_pipeline`).
    /// `Store::latest_block_header` is NOT a safe source here because it is
    /// advanced by `apply_fork_choice` (engine_forkchoiceUpdated from the CL),
    /// not by block execution; during catchup it lags by ≥ 1 block, and a
    /// walk from the stale root skips the most recent layer (it is keyed by
    /// `from_root` and a child of the stale root, not an ancestor).
    pub fn force_commit_layers(&self, from_root: H256) -> Result<(), StoreError> {
        // Snapshot the current cache so we operate on a consistent view.
        let cache = self
            .trie_cache
            .read()
            .map_err(|_| StoreError::LockError)?
            .clone();

        let mut cache_mut = (*cache).clone();
        // TEMP (Bug 4 diagnosis): log the activation flush so we can see
        // how many layers + leaves are actually being persisted at switch time.
        let mut total_committed = 0usize;
        let mut total_leaves = 0usize;

        // Drain until no more layers are found.  We use a temporary threshold
        // of 1 so every layer (not just those older than 128 blocks) is eligible.
        while let Some(commitable_root) = cache_mut.get_commitable_with_threshold(from_root, 1) {
            let last_written = self
                .backend
                .begin_read()?
                .get(MISC_VALUES, "last_written".as_bytes())?
                .unwrap_or_default();

            let nodes = cache_mut.commit(commitable_root).ok_or_else(|| {
                StoreError::Custom(
                    "force_commit_layers: layer vanished from cloned cache after \
                     get_commitable_with_threshold returned Some — internal bug"
                        .to_string(),
                )
            })?;
            // TEMP (Bug 4 diagnosis): tally writes for the activation summary.
            total_committed += nodes.len();
            for (k, _) in &nodes {
                if k.len() == crate::mpt_wiring::MPT_ACCOUNT_LEAF_KEY_LEN
                    || k.len() == crate::mpt_wiring::MPT_STORAGE_LEAF_KEY_LEN
                {
                    total_leaves += 1;
                }
            }
            // bypass_fkv_cursor=true: this path runs only during the binary
            // transition activation freeze (FKV generator has been stopped
            // permanently by activate() step 2). Leaves past the FKV cursor
            // would otherwise be silently dropped — see mpt_commit_nodes_to_disk
            // doc and Bug 3 v3 in phase-7-handoff.md.
            mpt_commit_nodes_to_disk(self.backend.as_ref(), nodes, last_written, true)?;
        }

        // Write the drained cache back.
        *self.trie_cache.write().map_err(|_| StoreError::LockError)? = Arc::new(cache_mut);

        // TEMP (Bug 4 diagnosis): summary of what we actually persisted.
        tracing::info!(
            "[BINARY-DEBUG] force_commit_layers from {:?}: committed {} nodes ({} leaves)",
            from_root,
            total_committed,
            total_leaves,
        );

        Ok(())
    }

    /// Drains any pending trie-update-worker item by sending a no-op TrieUpdate
    /// and waiting for acknowledgement.
    ///
    /// Because the `trie_update_worker` channel has capacity 0 (rendezvous),
    /// a pending send from a prior block commit will have already been received
    /// by the worker.  However the worker's Phase-2 disk write may still be
    /// running.  Sending a no-op TrieUpdate and waiting for its Phase-1 ack
    /// ensures the worker is idle before we proceed with activation.
    pub fn drain_trie_update_worker(&self) -> Result<(), StoreError> {
        let (notify_tx, notify_rx) = std::sync::mpsc::sync_channel(1);
        let latest_root = self.latest_block_header.get().state_root;
        // A no-op update: same parent and child root, empty diffs.
        // The worker will update the cache (put_batch becomes a no-op for
        // equal roots), signal us, then find nothing to commit to disk.
        let noop_update = TrieUpdate {
            result_sender: notify_tx,
            parent_state_root: latest_root,
            child_state_root: latest_root,
            node_updates: NodeUpdates::Mpt {
                state_updates: vec![],
                storage_updates: vec![],
            },
            is_batch: false,
        };
        self.trie_update_worker_tx
            .send(noop_update)
            .map_err(|_| StoreError::Custom("trie_update_worker disconnected".to_string()))?;
        notify_rx
            .recv()
            .map_err(|_| StoreError::Custom("trie_update_worker ack recv failed".to_string()))??;
        Ok(())
    }

    /// Returns the trie backend kind active for this store.
    ///
    /// Used by callers outside this crate (e.g. the blockchain crate) to gate
    /// operations that are only meaningful for a specific backend — for example,
    /// MPT state-root validation must be skipped for `Binary` and `Transition`
    /// stores because those backends produce a different root hash that does not
    /// match the MPT-format header field.
    pub fn backend_kind(&self) -> BackendKind {
        let byte = self.backend_kind.load(Ordering::Acquire);
        byte_to_backend_kind(byte).expect("backend_kind byte was set via backend_kind_to_byte")
    }

    /// Atomically updates the in-memory backend kind.
    ///
    /// Called by `TransitionActivator::activate` after `persist_transition_metadata`
    /// writes the format byte to disk, completing the in-process hot-swap.
    /// Uses `Release` ordering so all subsequent `Acquire` loads see the new value.
    pub fn set_backend_kind(&self, kind: BackendKind) {
        self.backend_kind
            .store(backend_kind_to_byte(kind), Ordering::Release);
    }

    /// Returns the in-memory binary trie head root (live, advances per-block).
    ///
    /// Defaults to `H256::zero()` (`EMPTY_BINARY_ROOT`) on a fresh store. The
    /// worker advances this in `apply_trie_updates` after each binary
    /// `TrieUpdate`'s Phase-1 layer write. Used by `new_transition_state_reader`
    /// so each block's overlay reads start from the live binary head, not the
    /// disk-persisted `META_ROOT_HASH` which lags behind in-memory layers.
    pub fn current_binary_root(&self) -> H256 {
        *self
            .current_binary_root
            .read()
            .expect("current_binary_root RwLock poisoned")
    }

    /// Returns the transition metadata loaded at startup (or updated by activation).
    ///
    /// Returns `None` for `Mpt` and `Binary` stores, or before activation fires.
    /// After `persist_transition_metadata` succeeds during hot-swap, returns `Some`.
    pub fn transition_metadata(&self) -> Option<(u64, H256, H256)> {
        *self
            .transition_metadata
            .read()
            .expect("transition_metadata RwLock poisoned")
    }

    /// Returns the latest cached block header (not an async call).
    ///
    /// Returns `Ok(None)` when no canonical block has ever been committed to
    /// the database (i.e. `LatestBlockNumber` is absent from `CHAIN_DATA`).
    /// The `LatestBlockHeaderCache` starts at a zero-initialized default; we
    /// cannot distinguish "genesis block stored" from "never stored" by
    /// inspecting the cache alone, so we do a cheap synchronous DB peek
    /// instead.
    ///
    /// Used by the `TransitionActivator` to read `frozen_mpt_root` without
    /// blocking the tokio runtime on an async DB call.
    pub fn get_latest_canonical_block_header(
        &self,
    ) -> Result<Option<ethrex_common::types::BlockHeader>, StoreError> {
        let key = chain_data_key(ChainDataIndex::LatestBlockNumber);
        if self.read(CHAIN_DATA, key)?.is_none() {
            // `LatestBlockNumber` is absent only on a bare backend that has never
            // had `add_initial_state` called (possible in unit tests; unreachable
            // in production, where genesis init writes it for block 0).
            // Its presence means the cache holds at least the genesis header.
            return Ok(None);
        }
        let header = self.latest_block_header.get();
        Ok(Some((*header).clone()))
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
    pub(crate) async fn load_latest_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
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

    pub(crate) fn load_block_header(
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
}

/// Returns `true` when the FKV generator has already flattened every entry up
/// to and including the given account's nibble path. When true, the account's
/// storage values live in the flat-state tables and can be read directly
/// without walking the state trie.
fn flatkeyvalue_computed_with_last_written(account: H256, last_written: &[u8]) -> bool {
    let account_nibbles = ethrex_trie::Nibbles::from_bytes(account.as_bytes());
    &last_written[0..64] > account_nibbles.as_ref()
}

struct TrieUpdate {
    result_sender: std::sync::mpsc::SyncSender<Result<(), StoreError>>,
    parent_state_root: H256,
    child_state_root: H256,
    node_updates: NodeUpdates,
    is_batch: bool,
}

// NOTE: we don't receive `Store` here to avoid cyclic dependencies
// with the other end of `fkv_ctl`
fn apply_trie_updates(
    backend: &dyn StorageBackend,
    fkv_ctl: &SyncSender<FKVGeneratorControlMessage>,
    trie_cache: &Arc<RwLock<Arc<TrieLayerCache>>>,
    binary_trie_cache: &Arc<RwLock<Arc<BinaryTrieLayerCache>>>,
    current_binary_root: &Arc<RwLock<H256>>,
    trie_update: TrieUpdate,
) -> Result<(), StoreError> {
    let TrieUpdate {
        result_sender,
        parent_state_root,
        child_state_root,
        node_updates,
        is_batch,
    } = trie_update;

    // Extract binary FKV entries before consuming node_updates.
    // For MPT this is always empty; for Binary it carries the FKV leaf diffs.
    // Note: Transition mode also emits NodeUpdates::Binary (overlay writes only),
    // so `backend_kind` here is always either Mpt or Binary — never Transition.
    // The store-level BackendKind::Transition is irrelevant to the NodeUpdates discriminant.
    let (binary_fkv_entries, backend_kind) = match &node_updates {
        NodeUpdates::Mpt { .. } => (Vec::new(), BackendKind::Mpt),
        NodeUpdates::Binary { fkv_entries, .. } => (fkv_entries.clone(), BackendKind::Binary),
    };

    // Dispatch on backend variant to build the byte-keyed node cache layer.
    let new_layer = match node_updates {
        NodeUpdates::Mpt {
            state_updates,
            storage_updates,
        } => crate::mpt_wiring::build_mpt_cache_layer(state_updates, storage_updates),
        NodeUpdates::Binary {
            node_diffs,
            deleted_stems,
            fkv_entries: _,
        } => build_binary_cache_layer(node_diffs, deleted_stems),
    };

    // Read-Copy-Update the MPT trie cache with a new layer.
    // For Binary nodes, the framed key-value pairs go into the same TrieLayerCache
    // (keyed by state root). Binary FKV leaf diffs go into the separate BinaryTrieLayerCache.
    let trie = trie_cache
        .read()
        .map_err(|_| StoreError::LockError)?
        .clone();
    let mut trie_mut = (*trie).clone();
    trie_mut.put_batch(parent_state_root, child_state_root, new_layer);
    let trie = Arc::new(trie_mut);
    *trie_cache.write().map_err(|_| StoreError::LockError)? = trie.clone();

    // For Binary (which covers Transition too — see comment above): update the
    // BinaryTrieLayerCache with FKV leaf diffs.
    if backend_kind == BackendKind::Binary && !binary_fkv_entries.is_empty() {
        let bin_cache = binary_trie_cache
            .read()
            .map_err(|_| StoreError::LockError)?
            .clone();
        let mut bin_mut = (*bin_cache).clone();
        bin_mut.put_batch(parent_state_root.0, child_state_root.0, binary_fkv_entries);
        *binary_trie_cache
            .write()
            .map_err(|_| StoreError::LockError)? = Arc::new(bin_mut);
    }

    // For Binary (Transition): advance the in-memory current binary head root.
    // Subsequent block reads use this to walk the binary_trie_cache layers from
    // the live head, not from the disk-persisted META_ROOT_HASH (which only
    // updates on Phase-2 flushes that fire at the 128-layer threshold).
    if backend_kind == BackendKind::Binary {
        *current_binary_root
            .write()
            .map_err(|_| StoreError::LockError)? = child_state_root;
        // TEMP (Bug 4 diagnosis): log each advance to verify the cache + head
        // are tracking the same root the EVM-side merkleizer produced.
        tracing::info!(
            "[BINARY-DEBUG] advanced current_binary_root: parent={:?} child={:?}",
            parent_state_root,
            child_state_root,
        );
    }

    // Update finished, signal block processing.
    result_sender
        .send(Ok(()))
        .map_err(|_| StoreError::LockError)?;

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
    // Stop the flat-key-value generator thread, as the underlying trie is about to change.
    // Ignore the error, if the channel is closed it means there is no worker to notify.
    let _ = fkv_ctl.send(FKVGeneratorControlMessage::Stop);

    // RCU to remove the bottom layer: update step needs to happen after disk layer is updated.
    let mut trie_mut = (*trie).clone();

    let last_written = backend
        .begin_read()?
        .get(MISC_VALUES, "last_written".as_bytes())?
        .unwrap_or_default();

    // Commit removes the bottom layer from the node cache and returns the merged diffs.
    // This is the mutation step.
    let nodes = trie_mut.commit(root).unwrap_or_default();
    // backend_kind here can only be Mpt or Binary: NodeUpdates::Mpt maps to Mpt
    // and NodeUpdates::Binary maps to Binary. Transition stores always emit
    // NodeUpdates::Binary (overlay writes only), so BackendKind::Transition is
    // unreachable from this match arm.
    let write_result = match backend_kind {
        // bypass_fkv_cursor=false: normal worker Phase 2 commit path; the FKV
        // generator may still be running and will rewrite leaves past the cursor.
        BackendKind::Mpt => mpt_commit_nodes_to_disk(backend, nodes, last_written, false),
        BackendKind::Transition => {
            // NodeUpdates::Binary is the only path here, which maps to BackendKind::Binary above.
            // Transition stores never produce BackendKind::Transition from NodeUpdates.
            unreachable!("BackendKind::Transition cannot arise from NodeUpdates matching")
        }
        BackendKind::Binary => {
            // Decode framed node bytes from the committed layer and separate
            // tombstones (key prefix 0xFE) from regular node diffs.
            let mut node_diffs: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
            let mut deleted_stems: Vec<[u8; 31]> = Vec::new();
            for (key, framed_value) in nodes {
                if key.first() == Some(&0xFE) && key.len() == 32 {
                    // Tombstone key: decode the stem from bytes 1..32.
                    let mut stem = [0u8; 31];
                    stem.copy_from_slice(&key[1..32]);
                    deleted_stems.push(stem);
                } else {
                    // Regular node: unframe the value.
                    match crate::binary_wiring::decode_binary_cache_value(&framed_value) {
                        Ok(Some(value)) => node_diffs.push((key, value)),
                        Ok(None) => {
                            // Deletion-framed value on a non-tombstone key: treat as deletion.
                            node_diffs.push((key, vec![]));
                        }
                        Err(e) => {
                            return Err(StoreError::Custom(format!(
                                "binary cache decode error during commit: {e}"
                            )));
                        }
                    }
                }
            }

            // Drain committed FKV diffs from the BinaryTrieLayerCache.
            let fkv_entries: Vec<([u8; 32], Option<[u8; 32]>)> = {
                let bin_cache = binary_trie_cache
                    .read()
                    .map_err(|_| StoreError::LockError)?
                    .clone();
                let mut bin_mut = (*bin_cache).clone();
                let committed_fkv = bin_mut
                    .commit(root.0)
                    .map(|(_roots, diffs)| diffs)
                    .unwrap_or_default();
                *binary_trie_cache
                    .write()
                    .map_err(|_| StoreError::LockError)? = Arc::new(bin_mut);
                committed_fkv
            };

            binary_commit_nodes_to_disk(backend, node_diffs, deleted_stems, fkv_entries)
        }
    };
    // We want to send this message even if there was an error during the batch write
    let _ = fkv_ctl.send(FKVGeneratorControlMessage::Continue);
    write_result?;
    // Phase 3: update diff layers with the removal of bottom layer.
    *trie_cache.write().map_err(|_| StoreError::LockError)? = Arc::new(trie_mut);
    Ok(())
}

pub struct AncestorIterator {
    store: Store,
    next_hash: BlockHash,
}

impl Iterator for AncestorIterator {
    type Item = Result<(BlockHash, BlockHeader), StoreError>;

    fn next(&mut self) -> Option<Self::Item> {
        let next_hash = self.next_hash;
        match self.store.load_block_header_by_hash(next_hash) {
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

fn chain_data_key(index: ChainDataIndex) -> Vec<u8> {
    (index as u8).encode_to_vec()
}

fn snap_state_key(index: SnapStateIndex) -> Vec<u8> {
    (index as u8).encode_to_vec()
}

pub(crate) fn encode_code(code: &Code) -> Vec<u8> {
    let mut buf = Vec::with_capacity(
        6 + code.bytecode.len() + std::mem::size_of_val(code.jump_targets.as_slice()),
    );
    code.bytecode.encode(&mut buf);
    code.jump_targets.encode(&mut buf);
    buf
}

#[derive(Debug, Default, Clone)]
pub(crate) struct LatestBlockHeaderCache {
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

fn dir_is_empty(path: &Path) -> Result<bool, StoreError> {
    let is_empty = std::fs::read_dir(path)?.next().is_none();
    Ok(is_empty)
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
        let backend = match RocksDBBackend::open(path) {
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

/// Encode a `BackendKind` as the single byte stored in `MISC_VALUES` under
/// `STATE_BACKEND_FORMAT_KEY`. Each variant must have a unique, stable value.
///
/// Current mapping:
/// - `0` = `BackendKind::Mpt`
/// - `1` = `BackendKind::Binary`
/// - `2` = `BackendKind::Transition`
pub(crate) fn backend_kind_to_byte(k: BackendKind) -> u8 {
    match k {
        BackendKind::Mpt => 0,
        BackendKind::Binary => 1,
        BackendKind::Transition => 2,
    }
}

/// Decode the single byte stored in `MISC_VALUES` under `STATE_BACKEND_FORMAT_KEY`
/// back into a `BackendKind`. Returns `StoreError::Custom` for unknown values.
pub(crate) fn byte_to_backend_kind(b: u8) -> Result<BackendKind, StoreError> {
    match b {
        0 => Ok(BackendKind::Mpt),
        1 => Ok(BackendKind::Binary),
        2 => Ok(BackendKind::Transition),
        other => Err(StoreError::Custom(format!(
            "unknown state backend format byte: {other:#04x}"
        ))),
    }
}

/// Test-only helpers exposed as `pub` so external test crates (e.g.
/// `ethrex-blockchain`) can simulate store state without a full async block
/// pipeline.  These are not part of the stable API.
impl Store {
    /// Writes the `LatestBlockNumber` entry in `CHAIN_DATA` synchronously,
    /// simulating what `forkchoice_update_inner` does when the first canonical
    /// block is committed.  Used by tests that need to exercise code paths
    /// downstream of `get_latest_canonical_block_header`.
    ///
    /// **Not for production use.**
    pub fn test_set_latest_block_number(&self, number: u64) -> Result<(), StoreError> {
        let key = chain_data_key(ChainDataIndex::LatestBlockNumber);
        self.write(CHAIN_DATA, key, number.to_le_bytes().to_vec())
    }
}

#[cfg(test)]
mod backend_format_tests {
    use super::*;
    use ethrex_state_backend::BackendKind;

    #[test]
    fn mpt_round_trips() {
        assert_eq!(backend_kind_to_byte(BackendKind::Mpt), 0);
        assert!(matches!(byte_to_backend_kind(0), Ok(BackendKind::Mpt)));
    }

    #[test]
    fn binary_round_trips() {
        assert_eq!(backend_kind_to_byte(BackendKind::Binary), 1);
        assert!(matches!(byte_to_backend_kind(1), Ok(BackendKind::Binary)));
    }

    #[test]
    fn transition_round_trips() {
        assert_eq!(backend_kind_to_byte(BackendKind::Transition), 2);
        assert!(matches!(
            byte_to_backend_kind(2),
            Ok(BackendKind::Transition)
        ));
    }

    #[test]
    fn unknown_byte_returns_err() {
        assert!(byte_to_backend_kind(0xFF).is_err());
        assert!(byte_to_backend_kind(3).is_err());
    }

    /// `get_latest_canonical_block_header` must return `Ok(None)` on a fresh
    /// store (no block ever committed) and `Ok(Some(_))` after the canonical
    /// chain data is written.
    ///
    /// This tests that the function properly detects the "no block committed"
    /// state rather than silently returning the zero-initialized default header.
    #[test]
    fn get_latest_canonical_block_header_returns_none_on_fresh_store() {
        let store = Store::new(".", EngineType::InMemory, BackendKind::Mpt)
            .expect("failed to open in-memory store");

        // Fresh store: LatestBlockNumber absent → must return None.
        let result = store
            .get_latest_canonical_block_header()
            .expect("must not error on fresh store");
        assert!(
            result.is_none(),
            "fresh store must return None, not the zero-initialized default header"
        );

        // Simulate the first canonical block commit by writing LatestBlockNumber.
        let key = chain_data_key(ChainDataIndex::LatestBlockNumber);
        store
            .write(CHAIN_DATA, key, 1u64.to_le_bytes().to_vec())
            .expect("write LatestBlockNumber failed");

        // Now it must return Some (the cache still holds the default header, but
        // that's acceptable — the activator only cares that Some is returned).
        let result2 = store
            .get_latest_canonical_block_header()
            .expect("must not error after writing LatestBlockNumber");
        assert!(
            result2.is_some(),
            "after writing LatestBlockNumber, must return Some"
        );
    }

    /// Plan §6 Task 7.9 — `binary_transition_locked_without_flag`.
    ///
    /// Persist format byte 2 + valid transition metadata to an in-memory backend,
    /// then attempt to re-open the same backend with `BackendKind::Mpt`
    /// (simulating a restart without `--binary-transition`).
    ///
    /// `Store::from_backend` must return `Err(StoreError::Custom(_))` with a
    /// message containing "format byte 2 (transition) but --binary-transition was
    /// not passed", and must NOT construct a usable `Store`.
    #[test]
    fn binary_transition_locked_without_flag() {
        use crate::api::StorageBackend;
        use crate::backend::in_memory::InMemoryBackend;
        use std::sync::Arc;

        let backend_arc: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::open().unwrap());

        // Step 1: open as MPT, write format byte 2 via persist_transition_metadata.
        {
            let store1 = Store::from_backend(
                Arc::clone(&backend_arc),
                std::path::PathBuf::from("."),
                IN_MEMORY_COMMIT_THRESHOLD,
                BackendKind::Mpt,
            )
            .unwrap();
            store1
                .persist_transition_metadata(5, Default::default(), Default::default())
                .unwrap();
        }

        // Step 2: re-open with BackendKind::Mpt (flag absent). Must error.
        let result = Store::from_backend(
            Arc::clone(&backend_arc),
            std::path::PathBuf::from("."),
            IN_MEMORY_COMMIT_THRESHOLD,
            BackendKind::Mpt,
        );

        match result {
            Err(StoreError::Custom(msg)) => {
                assert!(
                    msg.contains(
                        "format byte 2 (transition) but --binary-transition was not passed"
                    ),
                    "error message must explain the mismatch; got: {msg:?}"
                );
            }
            Err(other) => panic!("expected StoreError::Custom, got: {other:?}"),
            Ok(_) => panic!("expected Err but Store::from_backend succeeded"),
        }
    }

    /// Regression test: `peek_backend_format_byte` must NOT create RocksDB
    /// bootstrap files in a fresh datadir.  Earlier behavior opened RocksDB
    /// unconditionally, populating the dir with CURRENT/MANIFEST/OPTIONS-* and
    /// causing `Store::new` to bail with `NotFoundDBVersion` on the next call.
    #[cfg(feature = "rocksdb")]
    #[test]
    fn peek_backend_format_byte_does_not_populate_fresh_datadir() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path();

        let result = Store::peek_backend_format_byte(path, EngineType::RocksDB)
            .expect("peek must not error on fresh datadir");
        assert!(
            result.is_none(),
            "peek on fresh datadir must return None, got {result:?}"
        );

        let entries: Vec<_> = std::fs::read_dir(path)
            .expect("read_dir")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect entries");
        assert!(
            entries.is_empty(),
            "datadir must remain empty after peek on fresh dir; found: {:?}",
            entries.iter().map(|e| e.file_name()).collect::<Vec<_>>()
        );
    }
}

#[cfg(test)]
mod code_lookup_tests {
    use super::*;
    use ethrex_common::constants::EMPTY_KECCACK_HASH;
    use ethrex_state_backend::BackendKind;

    /// Regression test for hive devp2p `GetByteCodes` Test 3: a snap-protocol
    /// query for `EMPTY_KECCACK_HASH` must return an empty-bytecode entry, not
    /// `None`. The empty-code entry is never written to `ACCOUNT_CODES` (would
    /// duplicate across every empty-coded account); we must synthesize it.
    #[test]
    fn get_account_code_empty_hash_returns_empty_bytecode() {
        let store = Store::new(".", EngineType::InMemory, BackendKind::Mpt).unwrap();
        let code = store
            .get_account_code(*EMPTY_KECCACK_HASH)
            .unwrap()
            .expect("EMPTY_KECCACK_HASH must yield Some(empty Code)");
        assert_eq!(code.hash, *EMPTY_KECCACK_HASH);
        assert_eq!(code.bytecode.len(), 0, "empty bytecode");
        assert!(code.jump_targets.is_empty(), "no jump targets");
    }
}

#[cfg(test)]
mod hot_swap_clone_tests {
    use super::*;
    use ethrex_common::H256;
    use ethrex_state_backend::BackendKind;

    /// Verifies that `Store::clone` shares the `Arc<AtomicU8>` backing
    /// `backend_kind`, so a hot-swap via `set_backend_kind` is immediately
    /// visible to all live clones (RPC handlers, engine API, SyncManager, …).
    ///
    /// This test would have FAILED under the old by-value Clone where each clone
    /// held its own independent `AtomicU8`.
    #[test]
    fn store_clone_shares_backend_kind_after_hot_swap() {
        let store = Store::new(".", EngineType::InMemory, BackendKind::Mpt).unwrap();
        let clone = store.clone();
        assert_eq!(store.backend_kind(), BackendKind::Mpt);
        assert_eq!(clone.backend_kind(), BackendKind::Mpt);

        store.set_backend_kind(BackendKind::Transition);

        assert_eq!(store.backend_kind(), BackendKind::Transition);
        assert_eq!(
            clone.backend_kind(),
            BackendKind::Transition,
            "Store clones must observe hot-swap (shared Arc<AtomicU8>)"
        );
    }

    /// Verifies that `Store::clone` shares the `Arc<RwLock<…>>` backing
    /// `transition_metadata`, so a write by `persist_transition_metadata` is
    /// immediately visible to all live clones without a restart.
    ///
    /// This test would have FAILED under the old by-value Clone where each clone
    /// held its own independent `RwLock`.
    #[test]
    fn store_clone_shares_transition_metadata_after_persist() {
        let store = Store::new(".", EngineType::InMemory, BackendKind::Mpt).unwrap();
        let clone = store.clone();
        assert!(store.transition_metadata().is_none());
        assert!(clone.transition_metadata().is_none());

        let frozen = H256::from([0xAA; 32]);
        let binary = H256::zero();
        store
            .persist_transition_metadata(42, frozen, binary)
            .unwrap();

        let meta = clone
            .transition_metadata()
            .expect("Store clones must observe persisted metadata (shared Arc<RwLock>)");
        assert_eq!(meta, (42, frozen, binary));
    }
}
