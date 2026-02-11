#[cfg(feature = "rocksdb")]
use crate::backend::rocksdb::RocksDBBackend;
use crate::{
    STORE_METADATA_FILENAME, STORE_SCHEMA_VERSION,
    api::{
        StorageBackend,
        tables::{
            ACCOUNT_CODE_METADATA, ACCOUNT_CODES, ACCOUNT_FLATKEYVALUE, ACCOUNT_TRIE_NODES,
            BLOCK_NUMBERS, BODIES, CANONICAL_BLOCK_HASHES, CHAIN_DATA, EXECUTION_WITNESSES,
            FULLSYNC_HEADERS, HEADERS, INVALID_CHAINS, MISC_VALUES, PENDING_BLOCKS, RECEIPTS,
            SNAP_STATE, STORAGE_FLATKEYVALUE, STORAGE_TRIE_NODES, TRANSACTION_LOCATIONS,
        },
    },
    apply_prefix,
    backend::in_memory::InMemoryBackend,
    error::StoreError,
    layering::{TrieLayerCache, TrieWrapper},
    rlp::{BlockBodyRLP, BlockHeaderRLP, BlockRLP},
    trie::{BackendTrieDB, BackendTrieDBLocked},
    utils::{ChainDataIndex, SnapStateIndex},
};

use bytes::Bytes;
use ethrex_common::{
    Address, H256, U256,
    types::{
        AccountInfo, AccountState, AccountUpdate, Block, BlockBody, BlockHash, BlockHeader,
        BlockNumber, ChainConfig, Code, CodeMetadata, ForkId, Genesis, GenesisAccount, Index,
        Receipt, Transaction,
        block_execution_witness::{ExecutionWitness, RpcExecutionWitness},
    },
    utils::keccak,
};
use ethrex_crypto::keccak::keccak_hash;
use ethrex_rlp::{
    decode::{RLPDecode, decode_bytes},
    encode::RLPEncode,
};
use ethrex_trie::{EMPTY_TRIE_HASH, Nibbles, Trie, TrieLogger, TrieNode, TrieWitness};
use ethrex_trie::{Node, NodeRLP};
use lru::LruCache;
use rustc_hash::FxBuildHasher;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, hash_map::Entry},
    fmt::Debug,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        mpsc::{SyncSender, TryRecvError, sync_channel},
    },
    thread::JoinHandle,
};
use tracing::{debug, error, info};

/// Maximum number of execution witnesses to keep in the database
pub const MAX_WITNESSES: u64 = 128;

// We use one constant for in-memory and another for on-disk backends.
// This is due to tests requiring state older than 128 blocks.
// TODO: unify these
#[allow(unused)]
const DB_COMMIT_THRESHOLD: usize = 128;
const IN_MEMORY_COMMIT_THRESHOLD: usize = 10000;

/// Control messages for the FlatKeyValue generator
#[derive(Debug, PartialEq)]
enum FKVGeneratorControlMessage {
    Stop,
    Continue,
}

// 64mb
const CODE_CACHE_MAX_SIZE: u64 = 64 * 1024 * 1024;
const BLOCK_HASH_CACHE_SIZE: usize = 256;

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
    /// Cache for trie nodes from recent blocks.
    trie_cache: Arc<Mutex<Arc<TrieLayerCache>>>,
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
    latest_block_header: LatestBlockHeaderCache,
    /// Last computed FlatKeyValue for incremental updates.
    last_computed_flatkeyvalue: Arc<Mutex<Vec<u8>>>,

    /// Cache for account bytecodes, keyed by the bytecode hash.
    /// Note that we don't remove entries on account code changes, since
    /// those changes already affect the code hash stored in the account, and only
    /// may result in this cache having useless data.
    account_code_cache: Arc<Mutex<CodeCache>>,

    /// Cache for block hashes, keyed by block number.
    /// Stores up to 256 recently accessed entries to speed up block hash lookups.
    /// Helps avoid repeated backend reads for frequently requested block hashes.
    block_hash_cache: Arc<Mutex<LruCache<BlockNumber, BlockHash>>>,

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
        let block_hash_cache = self.block_hash_cache.clone();

        // Collect blocks to cache for updating after DB write
        let blocks_to_cache: Vec<(BlockNumber, BlockHash)> = new_canonical_blocks
            .iter()
            .copied()
            .chain(std::iter::once((head_number, head_hash)))
            .collect();

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

            txn.commit()?;

            // Update block hash cache with new canonical blocks
            if let Ok(mut cache) = block_hash_cache.lock() {
                for (block_number, block_hash) in blocks_to_cache {
                    cache.put(block_number, block_hash);
                }
            }

            Ok(())
        })
        .await
        .map_err(|e| StoreError::Custom(format!("Task panicked: {}", e)))?
    }

    pub async fn get_receipts_for_block(
        &self,
        block_hash: &BlockHash,
    ) -> Result<Vec<Receipt>, StoreError> {
        let mut receipts = Vec::new();
        let mut index = 0u64;

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

        // Check the LRU cache first
        {
            let mut cache = self
                .block_hash_cache
                .lock()
                .map_err(|_| StoreError::LockError)?;
            if let Some(hash) = cache.get(&block_number) {
                return Ok(Some(*hash));
            }
        }

        // Cache miss: read from database
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

        let UpdateBatch {
            account_updates,
            storage_updates,
            ..
        } = update_batch;

        // Capacity one ensures sender just notifies and goes on
        let (notify_tx, notify_rx) = sync_channel(1);
        let wait_for_new_layer = notify_rx;
        let trie_update = TrieUpdate {
            parent_state_root,
            account_updates,
            storage_updates,
            result_sender: notify_tx,
            child_state_root: last_state_root,
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
        let mut background_threads = Vec::new();
        let mut store = Self {
            db_path,
            backend,
            chain_config: Default::default(),
            latest_block_header: Default::default(),
            trie_cache: Arc::new(Mutex::new(Arc::new(TrieLayerCache::new(commit_threshold)))),
            flatkeyvalue_control_tx: fkv_tx,
            trie_update_worker_tx: trie_upd_tx,
            last_computed_flatkeyvalue: Arc::new(Mutex::new(last_written)),
            account_code_cache: Arc::new(Mutex::new(CodeCache::default())),
            block_hash_cache: Arc::new(Mutex::new(LruCache::new(
                std::num::NonZeroUsize::new(BLOCK_HASH_CACHE_SIZE).unwrap(),
            ))),
            code_metadata_cache: Arc::new(Mutex::new(rustc_hash::FxHashMap::default())),
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
        let backend = store.backend.clone();
        let flatkeyvalue_control_tx = store.flatkeyvalue_control_tx.clone();
        let trie_cache = store.trie_cache.clone();
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

    pub async fn new_from_genesis(
        store_path: &Path,
        engine_type: EngineType,
        genesis_path: &str,
    ) -> Result<Self, StoreError> {
        let file = std::fs::File::open(genesis_path)
            .map_err(|error| StoreError::Custom(format!("Failed to open genesis file: {error}")))?;
        let reader = std::io::BufReader::new(file);
        let genesis: Genesis =
            serde_json::from_reader(reader).expect("Failed to deserialize genesis file");
        let mut store = Self::new(store_path, engine_type)?;
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
        let state_root = state_trie.hash_no_commit();
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
                    storage_trie.collect_changes_since_last_hash();
                account_state.storage_root = storage_hash;
                ret_storage_updates.push((hashed_address, storage_updates));
            }
            state_trie.insert(
                hashed_address.as_bytes().to_vec(),
                account_state.encode_to_vec(),
            )?;
        }
        let (state_trie_hash, state_updates) = state_trie.collect_changes_since_last_hash();

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

        let state_root = state_trie.hash_no_commit();

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
                    storage_trie.collect_changes_since_last_hash();

                account_state.storage_root = storage_hash;

                ret_storage_updates.push((H256::from_slice(&hashed_address), storage_updates));
            }

            state_trie.insert(hashed_address, account_state.encode_to_vec())?;
        }

        let (state_trie_hash, state_updates) = state_trie.collect_changes_since_last_hash();

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
            let code = Code::from_bytecode(account.code);
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

            let (storage_root, storage_nodes) = storage_trie.collect_changes_since_last_hash();

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

        let (state_root, account_trie_nodes) = genesis_state_trie.collect_changes_since_last_hash();
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
        let storage_root = if self.flatkeyvalue_computed(account_hash)? {
            // We will use FKVs, we don't need the root
            *EMPTY_TRIE_HASH
        } else {
            let state_trie = self.open_state_trie(state_root)?;
            let Some(encoded_account) = state_trie.get(account_hash.as_bytes())? else {
                return Ok(None);
            };
            let account = AccountState::decode(&encoded_account)?;
            account.storage_root
        };
        let storage_trie = self.open_storage_trie(account_hash, state_root, storage_root)?;

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

    /// Creates a new state trie with an empty state root, for testing purposes only
    pub fn new_state_trie_for_test(&self) -> Result<Trie, StoreError> {
        self.open_state_trie(*EMPTY_TRIE_HASH)
    }

    // Methods exclusive for trie management during snap-syncing

    /// Obtain a state trie from the given state root
    /// Doesn't check if the state root is valid
    /// Used for internal store operations
    pub fn open_state_trie(&self, state_root: H256) -> Result<Trie, StoreError> {
        let trie_db = TrieWrapper {
            state_root,
            inner: self
                .trie_cache
                .lock()
                .map_err(|_| StoreError::LockError)?
                .clone(),
            db: Box::new(BackendTrieDB::new_for_accounts(
                self.backend.clone(),
                self.last_written()?,
            )?),
            prefix: None,
        };
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
        let trie_db = TrieWrapper {
            state_root,
            inner: self
                .trie_cache
                .lock()
                .map_err(|_| StoreError::LockError)?
                .clone(),
            db: Box::new(state_trie_locked_backend(
                self.backend.as_ref(),
                self.last_written()?,
            )?),
            prefix: None,
        };
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
        let trie_db = TrieWrapper {
            state_root,
            inner: self
                .trie_cache
                .lock()
                .map_err(|_| StoreError::LockError)?
                .clone(),
            db: Box::new(BackendTrieDB::new_for_storages(
                self.backend.clone(),
                self.last_written()?,
            )?),
            prefix: Some(account_hash),
        };
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
        let trie_db = TrieWrapper {
            state_root,
            inner: self
                .trie_cache
                .lock()
                .map_err(|_| StoreError::LockError)?
                .clone(),
            db: Box::new(state_trie_locked_backend(
                self.backend.as_ref(),
                self.last_written()?,
            )?),
            prefix: Some(account_hash),
        };
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
        let root_hash = ethrex_trie::Node::decode(&root)?.compute_hash().finalize();
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

    fn last_written(&self) -> Result<Vec<u8>, StoreError> {
        let last_computed_flatkeyvalue = self
            .last_computed_flatkeyvalue
            .lock()
            .map_err(|_| StoreError::LockError)?;
        Ok(last_computed_flatkeyvalue.clone())
    }

    fn flatkeyvalue_computed(&self, account: H256) -> Result<bool, StoreError> {
        let account_nibbles = Nibbles::from_bytes(account.as_bytes());
        let last_computed_flatkeyvalue = self.last_written()?;
        Ok(&last_computed_flatkeyvalue[0..64] > account_nibbles.as_ref())
    }
}

type TrieNodesUpdate = Vec<(Nibbles, Vec<u8>)>;

struct TrieUpdate {
    result_sender: std::sync::mpsc::SyncSender<Result<(), StoreError>>,
    parent_state_root: H256,
    child_state_root: H256,
    account_updates: TrieNodesUpdate,
    storage_updates: Vec<(H256, TrieNodesUpdate)>,
}

// NOTE: we don't receive `Store` here to avoid cyclic dependencies
// with the other end of `fkv_ctl`
fn apply_trie_updates(
    backend: &dyn StorageBackend,
    fkv_ctl: &SyncSender<FKVGeneratorControlMessage>,
    trie_cache: &Arc<Mutex<Arc<TrieLayerCache>>>,
    trie_update: TrieUpdate,
) -> Result<(), StoreError> {
    let TrieUpdate {
        result_sender,
        parent_state_root,
        child_state_root,
        account_updates,
        storage_updates,
    } = trie_update;

    // Phase 1: update the in-memory diff-layers only, then notify block production.
    let new_layer = storage_updates
        .into_iter()
        .flat_map(|(account_hash, nodes)| {
            nodes
                .into_iter()
                .map(move |(path, node)| (apply_prefix(Some(account_hash), path), node))
        })
        .chain(account_updates)
        .collect();
    // Read-Copy-Update the trie cache with a new layer.
    let trie = trie_cache
        .lock()
        .map_err(|_| StoreError::LockError)?
        .clone();
    let mut trie_mut = (*trie).clone();
    trie_mut.put_batch(parent_state_root, child_state_root, new_layer);
    let trie = Arc::new(trie_mut);
    *trie_cache.lock().map_err(|_| StoreError::LockError)? = trie.clone();
    // Update finished, signal block processing.
    result_sender
        .send(Ok(()))
        .map_err(|_| StoreError::LockError)?;

    // Phase 2: update disk layer.
    let Some(root) = trie.get_commitable(parent_state_root) else {
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
    *trie_cache.lock().map_err(|_| StoreError::LockError)? = Arc::new(trie_mut);
    Ok(())
}

// NOTE: we don't receive `Store` here to avoid cyclic dependencies
// with the other end of `control_rx`
fn flatkeyvalue_generator(
    backend: &Arc<dyn StorageBackend>,
    last_computed_fkv: &Mutex<Vec<u8>>,
    control_rx: &std::sync::mpsc::Receiver<FKVGeneratorControlMessage>,
) -> Result<(), StoreError> {
    info!("Generation of FlatKeyValue started.");
    let read_tx = backend.begin_read()?;
    let last_written = read_tx
        .get(MISC_VALUES, "last_written".as_bytes())?
        .unwrap_or_default();

    if last_written.is_empty() {
        // First time generating the FKV. Remove all FKV entries just in case
        backend.clear_table(ACCOUNT_FLATKEYVALUE)?;
        backend.clear_table(STORAGE_FLATKEYVALUE)?;
    } else if last_written == [0xff] {
        // FKV was already generated
        info!("FlatKeyValue already generated. Skipping.");
        return Ok(());
    }

    loop {
        let root = read_tx
            .get(ACCOUNT_TRIE_NODES, &[])?
            .ok_or(StoreError::MissingLatestBlockNumber)?;
        let root: Node = ethrex_trie::Node::decode(&root)?;
        let state_root = root.compute_hash().finalize();

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
            Box::new(BackendTrieDB::new_for_accounts(
                backend.clone(),
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
                    .lock()
                    .map_err(|_| StoreError::LockError)? = path.as_ref().to_vec();
                ctr = 0;
            }

            let mut iter_inner = Trie::open(
                Box::new(BackendTrieDB::new_for_account_storage(
                    backend.clone(),
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
                        .lock()
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
                    .lock()
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
