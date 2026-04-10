#[cfg(feature = "rocksdb")]
use crate::backend::rocksdb::RocksDBBackend;
use crate::{
    STORE_METADATA_FILENAME, STORE_SCHEMA_VERSION,
    api::{
        StorageBackend,
        tables::{
            ACCOUNT_CODE_METADATA, ACCOUNT_CODES, ACCOUNT_FLATKEYVALUE, BLOCK_NUMBERS, BODIES,
            CANONICAL_BLOCK_HASHES, CHAIN_DATA, EXECUTION_WITNESSES, FULLSYNC_HEADERS, HEADERS,
            INVALID_CHAINS, MISC_VALUES, PENDING_BLOCKS, RECEIPTS, SNAP_STATE,
            STORAGE_FLATKEYVALUE, TRANSACTION_LOCATIONS,
        },
    },
    backend::in_memory::InMemoryBackend,
    error::StoreError,
    rlp::{BlockBodyRLP, BlockHeaderRLP, BlockRLP},
    utils::{ChainDataIndex, SnapStateIndex},
};

use bytes::Bytes;
use ethrex_common::{
    Address, H256, U256,
    constants::EMPTY_TRIE_HASH,
    types::{
        AccountInfo, AccountState, AccountUpdate, Block, BlockBody, BlockHash, BlockHeader,
        BlockNumber, ChainConfig, Code, CodeMetadata, ForkId, Genesis, GenesisAccount, Index,
        Receipt, Transaction,
        block_execution_witness::{ExecutionWitness, RpcExecutionWitness},
    },
    utils::keccak,
};
use ethrex_crypto::{NativeCrypto, keccak::keccak_hash};
use ethrex_rlp::{
    decode::{RLPDecode, decode_bytes},
    encode::RLPEncode,
};
use lru::LruCache;
use rustc_hash::{FxBuildHasher, FxHashMap};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fmt::Debug,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, RwLock,
        mpsc::{SyncSender, sync_channel},
    },
    thread::JoinHandle,
};
use tracing::{debug, error, info};

/// Maximum number of execution witnesses to keep in the database
pub const MAX_WITNESSES: u64 = 128;

// ---------------------------------------------------------------------------
// TrieBackend adapter for the storage backend
// ---------------------------------------------------------------------------

/// Wraps the storage backend to implement `TrieBackend` for the binary trie.
struct StorageTrieBackend {
    backend: Arc<dyn StorageBackend>,
}

impl ethrex_binary_trie::db::TrieBackend for StorageTrieBackend {
    fn get(
        &self,
        table: &'static str,
        key: &[u8],
    ) -> Result<Option<Vec<u8>>, ethrex_binary_trie::BinaryTrieError> {
        self.backend
            .begin_read()
            .map_err(|e| ethrex_binary_trie::BinaryTrieError::StoreError(e.to_string()))?
            .get(table, key)
            .map_err(|e| ethrex_binary_trie::BinaryTrieError::StoreError(e.to_string()))
    }

    fn write_batch(
        &self,
        ops: Vec<ethrex_binary_trie::db::WriteOp>,
    ) -> Result<(), ethrex_binary_trie::BinaryTrieError> {
        let mut tx = self
            .backend
            .begin_write()
            .map_err(|e| ethrex_binary_trie::BinaryTrieError::StoreError(e.to_string()))?;
        for op in ops {
            match op {
                ethrex_binary_trie::db::WriteOp::Put { table, key, value } => {
                    tx.put(table, &key, &value).map_err(|e| {
                        ethrex_binary_trie::BinaryTrieError::StoreError(e.to_string())
                    })?;
                }
                ethrex_binary_trie::db::WriteOp::Delete { table, key } => {
                    tx.delete(table, &key).map_err(|e| {
                        ethrex_binary_trie::BinaryTrieError::StoreError(e.to_string())
                    })?;
                }
            }
        }
        tx.commit()
            .map_err(|e| ethrex_binary_trie::BinaryTrieError::StoreError(e.to_string()))
    }

    fn full_iterator(
        &self,
        table: &'static str,
    ) -> Result<Box<dyn Iterator<Item = (Vec<u8>, Vec<u8>)>>, ethrex_binary_trie::BinaryTrieError>
    {
        let read_view = self
            .backend
            .begin_read()
            .map_err(|e| ethrex_binary_trie::BinaryTrieError::StoreError(e.to_string()))?;
        let entries: Vec<(Vec<u8>, Vec<u8>)> = read_view
            .prefix_iterator(table, &[])
            .map_err(|e| ethrex_binary_trie::BinaryTrieError::StoreError(e.to_string()))?
            .filter_map(|r| r.ok().map(|(k, v)| (k.to_vec(), v.to_vec())))
            .collect();
        Ok(Box::new(entries.into_iter()))
    }
}

// We use one constant for in-memory and another for on-disk backends.
// This is due to tests requiring state older than 128 blocks.
// TODO: unify these
#[allow(unused)]
const DB_COMMIT_THRESHOLD: usize = 128;
const IN_MEMORY_COMMIT_THRESHOLD: usize = 10000;

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
/// - **Code Cache**: LRU cache for contract bytecode (64MB default)
/// - **Latest Block Cache**: Cached latest block header for RPC
///
/// # Example
///
/// ```ignore
/// let store = Store::new("./data", EngineType::RocksDB)?;
///
/// // Add a block
/// store.add_block(block).await?;
///
/// // Query account balance
/// let info = store.get_account_info(block_number, address)?;
/// let balance = info.map(|a| a.balance).unwrap_or_default();
/// ```
#[derive(Debug, Clone)]
pub struct Store {
    /// Path to the database directory.
    db_path: PathBuf,
    /// Storage backend (InMemory or RocksDB).
    backend: Arc<dyn StorageBackend>,
    /// Chain configuration (fork schedule, chain ID, etc.).
    chain_config: ChainConfig,
    /// Cached latest canonical block header.
    ///
    /// Wrapped in Arc for cheap reads with infrequent writes.
    /// May be slightly out of date, which is acceptable for:
    /// - Caching frequently requested headers
    /// - RPC "latest" block queries (small delay acceptable)
    /// - Sync operations (must be idempotent anyway)
    latest_block_header: LatestBlockHeaderCache,
    /// Last computed FlatKeyValue for incremental updates.
    last_computed_flatkeyvalue: Arc<RwLock<Vec<u8>>>,

    /// Binary trie state for EIP-7864 state reads.
    ///
    /// When set, account and storage reads delegate to the binary trie instead
    /// of the MPT. Set via `set_binary_trie_state` after the store is created.
    binary_trie_state: Option<Arc<RwLock<ethrex_binary_trie::state::BinaryTrieState>>>,

    /// Per-block leaf diff cache for the binary trie. Enables reorg support
    /// by keeping uncommitted state in memory as layers.
    binary_trie_layer_cache: Arc<RwLock<ethrex_binary_trie::layer_cache::BinaryTrieLayerCache>>,

    /// Maps `block_hash -> binary_trie_root` for blocks in the layer cache.
    /// Binary trie roots are NOT stored in block headers (which have MPT roots).
    binary_trie_root_map: Arc<RwLock<FxHashMap<BlockHash, [u8; 32]>>>,

    /// Pending FKV updates per binary trie state root, written to disk at
    /// layer commit time. Each entry holds the flat_updates and code_updates
    /// for one block (or batch).
    pending_fkv_updates: Arc<RwLock<FxHashMap<[u8; 32], (Vec<AccountUpdate>, Vec<(H256, Code)>)>>>,

    /// Channel for sending flush work (trie nodes + FKV) to the background thread.
    flush_work_tx: SyncSender<FlushWork>,

    /// Cache for account bytecodes, keyed by the bytecode hash.
    /// Note that we don't remove entries on account code changes, since
    /// those changes already affect the code hash stored in the account, and only
    /// may result in this cache having useless data.
    account_code_cache: Arc<Mutex<CodeCache>>,

    /// Cache for code metadata (code length), keyed by the bytecode hash.
    /// Uses FxHashMap for efficient lookups, much smaller than code cache.
    code_metadata_cache: Arc<Mutex<rustc_hash::FxHashMap<H256, CodeMetadata>>>,

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
    /// Blocks to store.
    pub blocks: Vec<Block>,
    /// Receipts to store, grouped by block hash.
    pub receipts: Vec<(H256, Vec<Receipt>)>,
    /// Contract code updates (code hash -> bytecode).
    pub code_updates: Vec<(H256, Code)>,
}

/// Collection of account state changes from block execution.
///
/// Returned by `apply_account_updates_batch` after applying updates to the
/// binary trie. Contains the data needed by `store_block` to write FKV
/// tables and contract code.
pub struct AccountUpdatesList {
    /// New contract bytecode deployments.
    pub code_updates: Vec<(H256, Code)>,
    /// Account updates to write to FKV tables for O(1) state reads.
    pub flat_updates: Vec<AccountUpdate>,
}

impl AccountUpdatesList {
    /// Extract code updates and flat updates from a slice of AccountUpdates.
    pub fn from_updates(updates: &[AccountUpdate]) -> Self {
        let code_updates = updates
            .iter()
            .filter_map(|u| {
                u.info
                    .as_ref()
                    .and_then(|info| u.code.as_ref().map(|code| (info.code_hash, code.clone())))
            })
            .collect();
        Self {
            code_updates,
            flat_updates: updates.to_vec(),
        }
    }
}

/// Return type for state trie hash updates.
pub struct MptUpdatesList {
    pub state_trie_hash: H256,
    pub code_updates: Vec<(H256, Code)>,
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

    /// Obtain latest block number synchronously (reads from in-memory cache).
    pub fn get_latest_block_number_sync(&self) -> BlockNumber {
        self.latest_block_header.get().number
    }

    /// Obtain latest block hash synchronously (reads from in-memory cache).
    pub fn get_latest_block_hash_sync(&self) -> BlockHash {
        self.latest_block_header.get().hash()
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

    /// Returns the lowest and highest block numbers in the fullsync headers table.
    /// Scans all entries since keys are LE-encoded and byte order != numeric order.
    pub async fn fullsync_header_range(
        &self,
    ) -> Result<Option<(BlockNumber, BlockNumber)>, StoreError> {
        let read_tx = self.backend.begin_read()?;
        let mut lowest = u64::MAX;
        let mut highest = 0u64;
        let mut found = false;
        for item in read_tx.prefix_iterator(FULLSYNC_HEADERS, &[])? {
            let (key, _) = item?;
            if key.len() >= 8 {
                let num = u64::from_le_bytes(key[..8].try_into().unwrap());
                lowest = lowest.min(num);
                highest = highest.max(num);
                found = true;
            }
        }
        if found {
            Ok(Some((lowest, highest)))
        } else {
            Ok(None)
        }
    }

    /// Returns the block header at the given number from the fullsync headers table.
    pub async fn get_fullsync_header(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<BlockHeader>, StoreError> {
        let read_tx = self.backend.begin_read()?;
        match read_tx.get(FULLSYNC_HEADERS, &block_number.to_le_bytes())? {
            Some(bytes) => Ok(Some(BlockHeader::decode(&bytes)?)),
            None => Ok(None),
        }
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

        let UpdateBatch {
            blocks,
            receipts,
            code_updates,
        } = update_batch;

        let mut tx = db.begin_write()?;

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

        for (block_hash, block_receipts) in receipts {
            for (index, receipt) in block_receipts.into_iter().enumerate() {
                let key = (block_hash, index as u64).encode_to_vec();
                let value = receipt.encode_to_vec();
                tx.put(RECEIPTS, &key, &value)?;
            }
        }

        for (code_hash, code) in code_updates {
            let buf = encode_code(&code);
            let metadata_buf = (code.bytecode.len() as u64).to_be_bytes();
            tx.put(ACCOUNT_CODES, code_hash.as_ref(), &buf)?;
            tx.put(ACCOUNT_CODE_METADATA, code_hash.as_ref(), &metadata_buf)?;
        }

        tx.commit()?;

        Ok(())
    }

    pub fn new(path: impl AsRef<Path>, engine_type: EngineType) -> Result<Self, StoreError> {
        // Ignore unused variable warning when compiling without DB features
        let db_path = path.as_ref().to_path_buf();

        if engine_type != EngineType::InMemory {
            // Check that the last used DB version matches the current version
            validate_store_schema_version(&db_path)?;
        }

        match engine_type {
            #[cfg(feature = "rocksdb")]
            EngineType::RocksDB => {
                let backend = Arc::new(RocksDBBackend::open(path)?);
                Self::from_backend(backend, db_path, DB_COMMIT_THRESHOLD)
            }
            EngineType::InMemory => {
                let backend = Arc::new(InMemoryBackend::open()?);
                Self::from_backend(backend, db_path, IN_MEMORY_COMMIT_THRESHOLD)
            }
        }
    }

    fn from_backend(
        backend: Arc<dyn StorageBackend>,
        db_path: PathBuf,
        commit_threshold: usize,
    ) -> Result<Self, StoreError> {
        debug!("Initializing Store with {commit_threshold} in-memory diff-layers");

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
        let (flush_work_tx, flush_work_rx) = sync_channel::<FlushWork>(1);
        // Background flush thread: writes trie ops + FKV updates to disk.
        let flush_backend = backend.clone();
        let flush_trie_backend = Arc::new(StorageTrieBackend {
            backend: backend.clone(),
        });
        let flush_thread = std::thread::spawn(move || {
            loop {
                match flush_work_rx.recv() {
                    Ok(work) => {
                        if !work.trie_ops.is_empty() {
                            if let Err(e) = ethrex_binary_trie::db::TrieBackend::write_batch(
                                flush_trie_backend.as_ref(),
                                work.trie_ops,
                            ) {
                                error!("Background trie flush failed: {e}");
                            }
                        }
                        if let Err(e) = write_fkv_updates_static(
                            flush_backend.as_ref(),
                            &work.flat_updates,
                            &work.code_updates,
                        ) {
                            error!("Background FKV flush failed: {e}");
                        }
                        if let Some(tx) = work.done_tx {
                            let _ = tx.try_send(());
                        }
                    }
                    Err(err) => {
                        debug!("Flush worker sender disconnected: {err}");
                        return;
                    }
                }
            }
        });
        let store = Self {
            db_path,
            backend,
            chain_config: Default::default(),
            latest_block_header: Default::default(),
            last_computed_flatkeyvalue: Arc::new(RwLock::new(last_written)),
            binary_trie_state: None,
            binary_trie_layer_cache: Arc::new(RwLock::new(
                ethrex_binary_trie::layer_cache::BinaryTrieLayerCache::new(commit_threshold),
            )),
            binary_trie_root_map: Arc::new(RwLock::new(FxHashMap::default())),
            pending_fkv_updates: Arc::new(RwLock::new(FxHashMap::default())),
            flush_work_tx,
            account_code_cache: Arc::new(Mutex::new(CodeCache::default())),
            code_metadata_cache: Arc::new(Mutex::new(rustc_hash::FxHashMap::default())),
            background_threads: Arc::new(ThreadList {
                list: vec![flush_thread],
            }),
        };
        Ok(store)
    }

    /// Create a `TrieBackend` backed by this store's storage backend.
    ///
    /// Used to open a `BinaryTrieState` that shares the same underlying storage.
    pub fn create_trie_backend(&self) -> Arc<dyn ethrex_binary_trie::db::TrieBackend> {
        Arc::new(StorageTrieBackend {
            backend: self.backend.clone(),
        })
    }

    pub async fn new_from_genesis(
        store_path: &Path,
        engine_type: EngineType,
        genesis_path: &str,
    ) -> Result<Self, StoreError> {
        let file = std::fs::File::open(genesis_path)
            .map_err(|error| StoreError::Custom(format!("Failed to open genesis file: {error}")))?;
        let reader = std::io::BufReader::new(file);
        let genesis: Genesis = serde_json::from_reader(reader)
            .map_err(|e| StoreError::Custom(format!("Failed to deserialize genesis file: {e}")))?;
        let mut store = Self::new(store_path, engine_type)?;
        store.add_initial_state(genesis).await?;
        Ok(store)
    }

    /// Attach a binary trie state so that account/storage reads delegate to it
    /// instead of the MPT. Must be called before any state reads.
    pub fn set_binary_trie_state(
        &mut self,
        state: Arc<RwLock<ethrex_binary_trie::state::BinaryTrieState>>,
    ) {
        self.binary_trie_state = Some(state);
    }

    /// Returns the binary trie state if set.
    pub fn binary_trie_state(
        &self,
    ) -> Option<Arc<RwLock<ethrex_binary_trie::state::BinaryTrieState>>> {
        self.binary_trie_state.clone()
    }

    /// Record a block's binary trie state root for layer cache lookups.
    pub fn set_binary_trie_root(&self, block_hash: BlockHash, root: [u8; 32]) {
        if let Ok(mut map) = self.binary_trie_root_map.write() {
            map.insert(block_hash, root);
        }
    }

    /// Look up the binary trie state root for a given block hash.
    pub fn get_binary_trie_root(&self, block_hash: BlockHash) -> Option<[u8; 32]> {
        self.binary_trie_root_map
            .read()
            .ok()?
            .get(&block_hash)
            .copied()
    }

    /// Reload the binary trie from its last disk checkpoint, discarding
    /// all in-memory mutations. Also clears the layer cache and root map.
    /// Returns the checkpoint block number.
    ///
    /// On non-RocksDB backends, clears layers/root map and returns 0
    /// (the trie has no persistent checkpoint).
    pub fn reload_binary_trie(&self) -> Result<u64, StoreError> {
        // Synchronous flush: ensure any in-flight background write completes before
        // we reload from the checkpoint. We create a completion channel and send a
        // no-op work item; the background thread signals done after processing it.
        // The sync_channel(1) ensures we block until the previous work item has been
        // picked up, and the done_rx.recv() waits for the no-op to be processed,
        // guaranteeing all prior disk writes are complete.
        let (done_tx, done_rx) = std::sync::mpsc::sync_channel::<()>(1);
        let _ = self.flush_work_tx.send(FlushWork {
            trie_ops: Vec::new(),
            flat_updates: Vec::new(),
            code_updates: Vec::new(),
            done_tx: Some(done_tx),
        });
        // Wait for background thread to finish processing.
        let _ = done_rx.recv();

        let bts = self
            .binary_trie_state
            .as_ref()
            .ok_or_else(|| StoreError::Custom("binary trie state not initialized".to_string()))?;
        let mut state = bts
            .write()
            .map_err(|_| StoreError::Custom("binary trie lock poisoned".to_string()))?;
        let checkpoint = state
            .reload_from_checkpoint()
            .map_err(|e| StoreError::Custom(format!("binary trie reload failed: {e}")))?;

        // Clear all in-memory state from the old fork.
        if let Ok(mut cache) = self.binary_trie_layer_cache.write() {
            cache.clear();
        }
        if let Ok(mut map) = self.binary_trie_root_map.write() {
            map.clear();
        }
        if let Ok(mut pending) = self.pending_fkv_updates.write() {
            pending.clear();
        }

        Ok(checkpoint)
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
        Ok(self
            .get_account_state_by_block_hash(block_hash, address)?
            .map(|s| AccountInfo {
                code_hash: s.code_hash,
                balance: s.balance,
                nonce: s.nonce,
            }))
    }

    /// Read account state, checking the binary trie layer cache and trie state
    /// first, then falling through to FKV on disk for committed state.
    pub fn get_account_state_by_block_hash(
        &self,
        block_hash: BlockHash,
        address: Address,
    ) -> Result<Option<AccountState>, StoreError> {
        use crate::binary_trie_read::BinaryTrieWrapper;

        // Check in-memory binary trie state + layer cache.
        // Lock order: BinaryTrieState before LayerCache (matches handle_merkleization).
        if let Some(bts) = &self.binary_trie_state {
            let state = bts
                .read()
                .map_err(|_| StoreError::Custom("binary trie lock poisoned".to_string()))?;
            let cache = self
                .binary_trie_layer_cache
                .read()
                .map_err(|_| StoreError::Custom("layer cache lock poisoned".to_string()))?;

            // Use trie root from the root map if available (for layer cache lookups).
            // If not in the map (e.g., after restart), use a zero root — the wrapper
            // will skip layer cache and read directly from trie state.
            let trie_root = self.get_binary_trie_root(block_hash).unwrap_or([0u8; 32]);

            let wrapper = BinaryTrieWrapper {
                trie_root,
                layer_cache: &cache,
                trie_state: &state,
            };

            if let Some(result) = wrapper.get_account_state(&address) {
                return Ok(result);
            }
            // Fall through to FKV if not found in any in-memory layer or trie state.
        }

        // Fall through to FKV for committed state.
        let hashed_address = hash_address_fixed(&address);
        let read_tx = self.backend.begin_read()?;
        match read_tx.get(ACCOUNT_FLATKEYVALUE, hashed_address.as_bytes())? {
            Some(bytes) => Ok(Some(AccountState::decode(&bytes)?)),
            None => Ok(None),
        }
    }

    /// Read a storage slot, checking the binary trie layer cache and trie state
    /// first, then falling through to FKV on disk for committed state.
    pub fn get_storage_at_by_block_hash(
        &self,
        block_hash: BlockHash,
        address: Address,
        storage_key: H256,
    ) -> Result<Option<U256>, StoreError> {
        use crate::binary_trie_read::BinaryTrieWrapper;

        // Check in-memory binary trie state + layer cache.
        // Lock order: BinaryTrieState before LayerCache (matches handle_merkleization).
        if let Some(bts) = &self.binary_trie_state {
            let state = bts
                .read()
                .map_err(|_| StoreError::Custom("binary trie lock poisoned".to_string()))?;
            let cache = self
                .binary_trie_layer_cache
                .read()
                .map_err(|_| StoreError::Custom("layer cache lock poisoned".to_string()))?;

            let trie_root = self.get_binary_trie_root(block_hash).unwrap_or([0u8; 32]);

            let wrapper = BinaryTrieWrapper {
                trie_root,
                layer_cache: &cache,
                trie_state: &state,
            };

            if let Some(result) = wrapper.get_storage_slot(&address, storage_key) {
                return Ok(result);
            }
            // Fall through to FKV if not found in any in-memory layer.
        }

        // Fall through to FKV for committed state.
        let hashed_address = hash_address_fixed(&address);
        let hashed_key = hash_key_fixed(&storage_key);
        let mut fkv_key = Vec::with_capacity(64);
        fkv_key.extend_from_slice(hashed_address.as_bytes());
        fkv_key.extend_from_slice(&hashed_key);
        let read_tx = self.backend.begin_read()?;
        match read_tx.get(STORAGE_FLATKEYVALUE, &fkv_key)? {
            Some(bytes) => Ok(Some(U256::decode(&bytes)?)),
            None => Ok(None),
        }
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
        use ethrex_common::constants::EMPTY_KECCACK_HASH;
        let Some(block_hash) = self.get_canonical_block_hash(block_number).await? else {
            return Ok(None);
        };
        let account = self.get_account_state_by_block_hash(block_hash, address)?;
        let Some(account) = account else {
            return Ok(None);
        };
        if account.code_hash == *EMPTY_KECCACK_HASH {
            return Ok(None);
        }
        self.get_account_code(account.code_hash)
    }

    pub async fn get_nonce_by_account_address(
        &self,
        block_number: BlockNumber,
        address: Address,
    ) -> Result<Option<u64>, StoreError> {
        let Some(block_hash) = self.get_canonical_block_hash(block_number).await? else {
            return Ok(None);
        };
        Ok(self
            .get_account_state_by_block_hash(block_hash, address)?
            .map(|s| s.nonce))
    }

    /// Applies account updates to the binary trie, computes state root, and flushes if needed.
    ///
    /// Used by non-pipeline paths (`add_block`, `add_blocks_in_batch`).
    /// The pipeline path calls `handle_merkleization` + `flush_binary_trie_if_needed` separately.
    pub fn apply_account_updates_batch(
        &self,
        block_hash: BlockHash,
        block_number: u64,
        block_count: u64,
        account_updates: &[AccountUpdate],
    ) -> Result<AccountUpdatesList, StoreError> {
        let (result, trie_root) = self.handle_merkleization(account_updates)?;
        self.set_binary_trie_root(block_hash, trie_root);
        self.flush_binary_trie_if_needed(block_number, block_hash, block_count)?;
        Ok(result)
    }

    /// Apply account updates to the binary trie and compute the state root.
    ///
    /// Called from the merkleizer pipeline thread during pipelined block execution.
    /// Does NOT flush -- call `flush_binary_trie_if_needed` separately after storing.
    /// Apply account updates to the binary trie, compute state root, and record
    /// the block's leaf diffs into the layer cache.
    ///
    /// Returns `(AccountUpdatesList, binary_trie_root)`. The caller should pass
    /// `binary_trie_root` to [`set_binary_trie_root`] with the block hash.
    pub fn handle_merkleization(
        &self,
        account_updates: &[AccountUpdate],
    ) -> Result<(AccountUpdatesList, [u8; 32]), StoreError> {
        let bts = self
            .binary_trie_state
            .as_ref()
            .ok_or_else(|| StoreError::Custom("binary trie state not initialized".to_string()))?;
        let mut state = bts
            .write()
            .map_err(|_| StoreError::Custom("binary trie lock poisoned".to_string()))?;
        let mut code_updates = Vec::new();
        for update in account_updates {
            state
                .apply_account_update(update)
                .map_err(|e| StoreError::Custom(format!("binary trie update error: {e}")))?;
            if let Some(info) = &update.info {
                if let Some(code) = &update.code {
                    code_updates.push((info.code_hash, code.clone()));
                }
            }
        }
        let root = state.state_root();
        tracing::debug!("Binary trie root: {}", ethrex_common::H256::from(root));

        // Record the block's leaf diffs into the layer cache.
        let (parent_root, diffs) = state.take_block_diffs(root);
        if !diffs.is_empty() {
            self.binary_trie_layer_cache
                .write()
                .map_err(|_| StoreError::Custom("layer cache lock poisoned".to_string()))?
                .put_batch(parent_root, root, diffs);
        }

        // Clone account updates once, share between pending FKV and return value.
        let flat_updates = account_updates.to_vec();
        if let Ok(mut pending) = self.pending_fkv_updates.write() {
            pending.insert(root, (flat_updates.clone(), code_updates.clone()));
        }

        Ok((
            AccountUpdatesList {
                code_updates,
                flat_updates,
            },
            root,
        ))
    }

    /// Flushes the binary trie to disk if the flush interval has been reached.
    ///
    /// Used by the pipeline path after the merkleizer thread has already applied
    /// account updates and computed the state root in-thread.
    pub fn flush_binary_trie_if_needed(
        &self,
        block_number: u64,
        block_hash: BlockHash,
        block_count: u64,
    ) -> Result<(), StoreError> {
        let bts = self
            .binary_trie_state
            .as_ref()
            .ok_or_else(|| StoreError::Custom("binary trie state not initialized".to_string()))?;
        let mut state = bts
            .write()
            .map_err(|_| StoreError::Custom("binary trie lock poisoned".to_string()))?;

        // Check if flush threshold reached.
        if !state.tick_and_check_flush(block_count) {
            return Ok(());
        }

        // Collect write ops (fast: collects dirty nodes, rotates generations).
        let trie_ops = state
            .prepare_flush(block_number, block_hash)
            .map_err(|e| StoreError::Custom(format!("binary trie flush error: {e}")))?;

        drop(state); // Release the trie lock before background send.

        // Collect pending FKV updates.
        let (all_flat, all_code) = self.drain_pending_fkv()?;

        // Send both trie ops + FKV to background thread.
        self.flush_work_tx
            .send(FlushWork {
                trie_ops,
                flat_updates: all_flat,
                code_updates: all_code,
                done_tx: None,
            })
            .map_err(|_| StoreError::Custom("flush worker disconnected".to_string()))?;

        // Commit old layers and prune the root map.
        // This mirrors main's TrieLayerCache commit: when enough layers accumulate,
        // the oldest are merged and removed. Reads for committed blocks fall through
        // to FKV on disk.
        if let Some(current_root) = self.get_binary_trie_root(block_hash) {
            if let Ok(mut cache) = self.binary_trie_layer_cache.write() {
                if let Some(commit_root) = cache.get_commitable(current_root) {
                    cache.commit(commit_root);
                }
            }
        }
        if let (Ok(cache), Ok(mut map)) = (
            self.binary_trie_layer_cache.read(),
            self.binary_trie_root_map.write(),
        ) {
            map.retain(|_block_hash, trie_root| cache.contains_root(*trie_root));
        }

        Ok(())
    }

    /// Drain all pending FKV updates and return them.
    fn drain_pending_fkv(&self) -> Result<(Vec<AccountUpdate>, Vec<(H256, Code)>), StoreError> {
        let all_pending: Vec<(Vec<AccountUpdate>, Vec<(H256, Code)>)> = {
            let mut pending = self
                .pending_fkv_updates
                .write()
                .map_err(|_| StoreError::Custom("pending FKV lock poisoned".to_string()))?;
            pending.drain().map(|(_, v)| v).collect()
        };

        let mut all_flat: Vec<AccountUpdate> = Vec::new();
        let mut all_code: Vec<(H256, Code)> = Vec::new();
        for (flat, code) in all_pending {
            all_flat.extend(flat);
            all_code.extend(code);
        }

        Ok((all_flat, all_code))
    }

    // NOTE: remaining impl Store methods continue below, after write_fkv_updates_static.
}

/// Write FKV updates to disk. Standalone function usable from background threads.
fn write_fkv_updates_static(
    db: &dyn StorageBackend,
    flat_updates: &[AccountUpdate],
    code_updates: &[(H256, Code)],
) -> Result<(), StoreError> {
    if flat_updates.is_empty() && code_updates.is_empty() {
        return Ok(());
    }

    // Pre-collect storage keys to delete.
    let storage_keys_to_delete: FxHashMap<H256, Vec<Vec<u8>>> = {
        let read_view = db.begin_read()?;
        flat_updates
            .iter()
            .filter(|u| u.removed_storage || u.removed)
            .map(|u| {
                let hashed_address = hash_address_fixed(&u.address);
                let prefix = hashed_address.as_bytes().to_vec();
                let keys: Vec<Vec<u8>> = read_view
                    .prefix_iterator(STORAGE_FLATKEYVALUE, &prefix)?
                    .filter_map(|item| item.ok().map(|(k, _)| k.to_vec()))
                    .collect();
                Ok((hashed_address, keys))
            })
            .collect::<Result<_, StoreError>>()?
    };

    let fkv_read = db.begin_read()?;
    let mut tx = db.begin_write()?;

    for (code_hash, code) in code_updates {
        let buf = encode_code(code);
        let metadata_buf = (code.bytecode.len() as u64).to_be_bytes();
        tx.put(ACCOUNT_CODES, code_hash.as_ref(), &buf)?;
        tx.put(ACCOUNT_CODE_METADATA, code_hash.as_ref(), &metadata_buf)?;
    }

    for update in flat_updates {
        let hashed_address = hash_address_fixed(&update.address);
        let acct_fkv_key = hashed_address.as_bytes();

        if update.removed {
            tx.delete(ACCOUNT_FLATKEYVALUE, acct_fkv_key)?;
            if let Some(keys) = storage_keys_to_delete.get(&hashed_address) {
                for key in keys {
                    tx.delete(STORAGE_FLATKEYVALUE, key)?;
                }
            }
            continue;
        }

        if update.removed_storage {
            if let Some(keys) = storage_keys_to_delete.get(&hashed_address) {
                for key in keys {
                    tx.delete(STORAGE_FLATKEYVALUE, key)?;
                }
            }
        }

        if let Some(ref info) = update.info {
            let storage_root = if update.removed_storage && update.added_storage.is_empty() {
                *EMPTY_TRIE_HASH
            } else if !update.added_storage.is_empty() {
                H256::from_low_u64_be(1)
            } else {
                fkv_read
                    .get(ACCOUNT_FLATKEYVALUE, acct_fkv_key)
                    .ok()
                    .flatten()
                    .and_then(|bytes| AccountState::decode(&bytes).ok())
                    .map(|prev| prev.storage_root)
                    .unwrap_or(*EMPTY_TRIE_HASH)
            };
            let account_state = AccountState {
                nonce: info.nonce,
                balance: info.balance,
                code_hash: info.code_hash,
                storage_root,
            };
            tx.put(
                ACCOUNT_FLATKEYVALUE,
                acct_fkv_key,
                &account_state.encode_to_vec(),
            )?;
        }

        for (slot_key, value) in &update.added_storage {
            let hashed_key = hash_key_fixed(slot_key);
            let mut fkv_key = Vec::with_capacity(64);
            fkv_key.extend_from_slice(acct_fkv_key);
            fkv_key.extend_from_slice(&hashed_key);
            if value.is_zero() {
                tx.delete(STORAGE_FLATKEYVALUE, &fkv_key)?;
            } else {
                tx.put(STORAGE_FLATKEYVALUE, &fkv_key, &value.encode_to_vec())?;
            }
        }

        if update.info.is_none() && !update.added_storage.is_empty() && !update.removed {
            if let Some(bytes) = fkv_read.get(ACCOUNT_FLATKEYVALUE, acct_fkv_key)? {
                let mut account_state = AccountState::decode(&bytes)?;
                if account_state.storage_root == *EMPTY_TRIE_HASH {
                    account_state.storage_root = H256::from_low_u64_be(1);
                    tx.put(
                        ACCOUNT_FLATKEYVALUE,
                        acct_fkv_key,
                        &account_state.encode_to_vec(),
                    )?;
                }
            }
        }
    }

    tx.commit()?;
    Ok(())
}

impl Store {
    /// Adds all genesis accounts and returns the genesis block's state_root
    /// Initialize the binary trie from genesis accounts.
    ///
    /// Creates a BinaryTrieState from Store's DB handle, applies genesis,
    /// flushes to disk, and sets the binary trie state on this Store.
    fn setup_genesis_binary_trie(
        &mut self,
        genesis_accounts: &BTreeMap<Address, GenesisAccount>,
        genesis_hash: H256,
    ) -> Result<(), StoreError> {
        use crate::api::tables::{BINARY_TRIE_NODES, BINARY_TRIE_STORAGE_KEYS};
        use ethrex_binary_trie::state::BinaryTrieState;

        let backend = Arc::new(StorageTrieBackend {
            backend: self.backend.clone(),
        });

        let mut state = BinaryTrieState::open(backend, BINARY_TRIE_NODES, BINARY_TRIE_STORAGE_KEYS)
            .map_err(|e| StoreError::Custom(format!("Failed to open binary trie: {e}")))?;

        // In-memory backend: the binary trie tables won't persist across restarts,
        // so genesis is applied here (same as before).
        state.apply_genesis(genesis_accounts).map_err(|e| {
            StoreError::Custom(format!("Failed to apply genesis to binary trie: {e}"))
        })?;

        state
            .flush(0, genesis_hash)
            .map_err(|e| StoreError::Custom(format!("Failed to flush binary trie: {e}")))?;

        self.set_binary_trie_state(Arc::new(RwLock::new(state)));

        Ok(())
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
        let rpc_witness = RpcExecutionWitness::from(witness);
        let key = Self::make_witness_key(block_number, &block_hash);
        let value = serde_json::to_vec(&rpc_witness)?;
        self.write(EXECUTION_WITNESSES, key, value)?;
        // Clean up old witnesses (keep only last 128)
        self.cleanup_old_witnesses(block_number)
    }

    /// Stores pre-serialized JSON witness bytes for a block.
    ///
    /// Use this when the witness is already serialized (e.g., binary trie witnesses
    /// that derive `serde::Serialize` and are serialized by the blockchain layer).
    pub fn store_witness_bytes(
        &self,
        block_hash: BlockHash,
        block_number: u64,
        json_bytes: Vec<u8>,
    ) -> Result<(), StoreError> {
        let key = Self::make_witness_key(block_number, &block_hash);
        self.write(EXECUTION_WITNESSES, key, json_bytes)?;
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

    pub async fn add_initial_state(&mut self, genesis: Genesis) -> Result<(), StoreError> {
        debug!("Storing initial state from genesis");

        // Obtain genesis block
        let genesis_block = genesis.get_block();
        let genesis_block_number = genesis_block.header.number;

        let genesis_hash = genesis_block.hash();

        // Set chain config
        self.set_chain_config(&genesis.config).await?;

        // The cache can't be empty
        if let Some(number) = self.load_latest_block_number().await? {
            let latest_block_header = self
                .load_block_header(number)?
                .ok_or_else(|| StoreError::MissingLatestBlockNumber)?;
            self.latest_block_header.update(latest_block_header);
        }

        match self.load_block_header(genesis_block_number)? {
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
        // Populate FKV tables so state reads can use O(1) RocksDB gets.
        self.populate_fkv_from_genesis(&genesis.alloc)?;

        // Store genesis account code (system contracts, etc.)
        for account in genesis.alloc.values() {
            if !account.code.is_empty() {
                let code = Code::from_bytecode(account.code.clone(), &NativeCrypto);
                self.add_account_code(code).await?;
            }
        }

        // Initialize binary trie from genesis.
        self.setup_genesis_binary_trie(&genesis.alloc, genesis_hash)?;

        // Store genesis block
        info!(hash = %genesis_hash, "Storing genesis block");

        self.add_block(genesis_block).await?;
        self.update_earliest_block_number(genesis_block_number)
            .await?;
        self.forkchoice_update(vec![], genesis_block_number, genesis_hash, None, None)
            .await?;
        Ok(())
    }

    /// Populate FKV tables from the genesis allocation.
    ///
    /// Must be called after genesis state is applied to the binary trie and
    /// before any blocks are processed. Safe to call on an already-populated
    /// FKV (it will overwrite with identical data).
    pub fn populate_fkv_from_genesis(
        &self,
        alloc: &std::collections::BTreeMap<Address, GenesisAccount>,
    ) -> Result<(), StoreError> {
        let mut tx = self.backend.begin_write()?;
        for (address, account) in alloc {
            let hashed_address = hash_address_fixed(address);
            let code_hash = if account.code.is_empty() {
                *ethrex_common::constants::EMPTY_KECCACK_HASH
            } else {
                keccak(account.code.as_ref())
            };
            // Use H256::from_low_u64_be(1) sentinel when account has genesis storage,
            // otherwise EMPTY_TRIE_HASH so the VM skips storage reads for storage-less accounts.
            let storage_root = if account.storage.is_empty() {
                *EMPTY_TRIE_HASH
            } else {
                H256::from_low_u64_be(1)
            };
            let account_state = AccountState {
                nonce: account.nonce,
                balance: account.balance,
                code_hash,
                storage_root,
            };
            tx.put(
                ACCOUNT_FLATKEYVALUE,
                hashed_address.as_bytes(),
                &account_state.encode_to_vec(),
            )?;

            for (slot, value) in &account.storage {
                if !value.is_zero() {
                    let slot_h256 = H256(slot.to_big_endian());
                    let hashed_key = hash_key_fixed(&slot_h256);
                    let mut fkv_key = Vec::with_capacity(64);
                    fkv_key.extend_from_slice(hashed_address.as_bytes());
                    fkv_key.extend_from_slice(&hashed_key);
                    tx.put(STORAGE_FLATKEYVALUE, &fkv_key, &value.encode_to_vec())?;
                }
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Returns true if the FKV account table has no entries (i.e. genesis has not been populated).
    pub fn fkv_account_table_is_empty(&self) -> Result<bool, StoreError> {
        let read_tx = self.backend.begin_read()?;
        // Use a prefix scan over the full range (empty prefix matches all).
        let mut iter = read_tx.prefix_iterator(ACCOUNT_FLATKEYVALUE, &[])?;
        Ok(iter.next().is_none())
    }

    /// Iterate all entries in a FKV table with the given prefix, collecting them eagerly.
    ///
    /// Returns a `Vec<(key, value)>` so callers do not need to hold a read transaction.
    /// Use an empty prefix to scan the entire table.
    /// Iterate all entries in a table with the given prefix, calling `f` for each.
    ///
    /// Streams entries without loading them all into memory.
    pub fn fkv_for_each(
        &self,
        table: &'static str,
        prefix: &[u8],
        mut f: impl FnMut(&[u8], &[u8]) -> Result<(), StoreError>,
    ) -> Result<u64, StoreError> {
        let read_tx = self.backend.begin_read()?;
        let mut count = 0u64;
        for result in read_tx.prefix_iterator(table, prefix)? {
            let (key, value) = result?;
            f(&key, &value)?;
            count += 1;
        }
        Ok(count)
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
        let Some(header) = self.get_block_header(block_number)? else {
            return Ok(None);
        };
        let block_hash = header.hash();
        self.get_storage_at_by_block_hash(block_hash, address, storage_key)
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
}

/// Work item for the background flush thread.
/// Carries both the trie node write ops and FKV updates.
struct FlushWork {
    /// Pre-collected write ops for binary trie nodes + storage_keys + metadata.
    trie_ops: Vec<ethrex_binary_trie::db::WriteOp>,
    flat_updates: Vec<AccountUpdate>,
    code_updates: Vec<(H256, Code)>,
    /// Optional completion signal. When set, the background thread sends a unit
    /// value after processing so the caller can wait for synchronous completion.
    /// Used during reorg to ensure all in-flight writes are on disk before reload.
    done_tx: Option<std::sync::mpsc::SyncSender<()>>,
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

fn encode_code(code: &Code) -> Vec<u8> {
    let mut buf = Vec::with_capacity(
        6 + code.bytecode.len() + std::mem::size_of_val(code.jump_targets.as_slice()),
    );
    code.bytecode.encode(&mut buf);
    code.jump_targets.encode(&mut buf);
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
struct StoreMetadata {
    schema_version: u64,
}

impl StoreMetadata {
    fn new(schema_version: u64) -> Self {
        Self { schema_version }
    }
}

fn validate_store_schema_version(path: &Path) -> Result<(), StoreError> {
    let metadata_path = path.join(STORE_METADATA_FILENAME);
    // If metadata file does not exist, try to create it
    if !metadata_path.exists() {
        // If datadir exists but is not empty, this is probably a DB for an
        // old ethrex version and we should return an error
        if path.exists() && !dir_is_empty(path)? {
            return Err(StoreError::NotFoundDBVersion {
                expected: STORE_SCHEMA_VERSION,
            });
        }
        init_metadata_file(path)?;
        return Ok(());
    }
    if !metadata_path.is_file() {
        return Err(StoreError::Custom(
            "store schema path exists but is not a file".to_string(),
        ));
    }
    let file_contents = std::fs::read_to_string(metadata_path)?;
    let metadata: StoreMetadata = serde_json::from_str(&file_contents)?;

    // Check schema version matches the expected one
    if metadata.schema_version != STORE_SCHEMA_VERSION {
        return Err(StoreError::IncompatibleDBVersion {
            found: metadata.schema_version,
            expected: STORE_SCHEMA_VERSION,
        });
    }
    Ok(())
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

/// Checks whether a valid database exists at the given path by looking for
/// a metadata.json file with a matching schema version.
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
    metadata.schema_version == STORE_SCHEMA_VERSION
}

/// Reads the chain ID from an existing database without performing a full
/// store initialization. Returns `None` if the database doesn't exist or
/// the chain config can't be read. Always returns `None` when compiled
/// without the `rocksdb` feature.
pub fn read_chain_id_from_db(path: &Path) -> Option<u64> {
    if !has_valid_db(path) {
        return None;
    }
    #[cfg(feature = "rocksdb")]
    {
        let backend = RocksDBBackend::open(path).ok()?;
        let read = backend.begin_read().ok()?;
        let key = chain_data_key(ChainDataIndex::ChainConfig);
        let bytes = read.get(CHAIN_DATA, &key).ok()??;
        let config: ethrex_common::types::ChainConfig = serde_json::from_slice(&bytes).ok()?;
        Some(config.chain_id)
    }
    #[cfg(not(feature = "rocksdb"))]
    {
        let _ = path;
        None
    }
}
