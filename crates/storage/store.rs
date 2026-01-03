#[cfg(feature = "rocksdb")]
use crate::backend::rocksdb::RocksDBBackend;
use crate::{
    STORE_METADATA_FILENAME, STORE_SCHEMA_VERSION,
    api::{
        StorageBackend,
        tables::{
            ACCOUNT_CODES, ACCOUNT_FLATKEYVALUE, ACCOUNT_TRIE_NODES, BLOCK_NUMBERS, BODIES,
            CANONICAL_BLOCK_HASHES, CHAIN_DATA, FULLSYNC_HEADERS, HEADERS, INVALID_CHAINS,
            MISC_VALUES, PENDING_BLOCKS, RECEIPTS, SNAP_STATE, STORAGE_FLATKEYVALUE,
            STORAGE_TRIE_NODES, TRANSACTION_LOCATIONS,
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
        BlockNumber, ChainConfig, Code, ForkId, Genesis, GenesisAccount, Index, Receipt,
        Transaction,
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
};
use tracing::{debug, error, info};
/// Number of state trie segments to fetch concurrently during state sync
pub const STATE_TRIE_SEGMENTS: usize = 2;
/// Maximum amount of reads from the snapshot in a single transaction to avoid performance hits due to long-living reads
/// This will always be the amount yielded by snapshot reads unless there are less elements left
pub const MAX_SNAPSHOT_READS: usize = 100;

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

/// Inner cache structure with size tracking (avoids atomic operations)
#[derive(Debug)]
struct CodeCacheInner {
    cache: LruCache<H256, Code, FxBuildHasher>,
    current_size: u64,
}

impl Default for CodeCacheInner {
    fn default() -> Self {
        Self {
            cache: LruCache::unbounded_with_hasher(FxBuildHasher),
            current_size: 0,
        }
    }
}

/// Thread-safe code cache with size-based eviction.
/// Size tracking is done inside the mutex to avoid atomic contention.
#[derive(Debug, Default)]
struct CodeCache {
    inner: Mutex<CodeCacheInner>,
}

impl CodeCache {
    fn get(&self, code_hash: &H256) -> Result<Option<Code>, StoreError> {
        let mut inner = self.inner.lock().map_err(|_| StoreError::LockError)?;
        Ok(inner.cache.get(code_hash).cloned())
    }

    fn insert(&self, code: &Code) -> Result<(), StoreError> {
        let mut inner = self.inner.lock().map_err(|_| StoreError::LockError)?;

        // Check if already present to avoid duplicate size tracking
        if inner.cache.contains(&code.hash) {
            return Ok(());
        }

        let code_size = code.size() as u64;
        inner.current_size += code_size;

        debug!(
            "[ACCOUNT CODE CACHE] cache elements: {}, total size: {} bytes",
            inner.cache.len() + 1,
            inner.current_size
        );

        // Evict LRU entries until we're under the size limit
        while inner.current_size > CODE_CACHE_MAX_SIZE {
            if let Some((_, evicted_code)) = inner.cache.pop_lru() {
                inner.current_size = inner.current_size.saturating_sub(evicted_code.size() as u64);
            } else {
                break;
            }
        }

        inner.cache.put(code.hash, code.clone());
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct Store {
    db_path: PathBuf,
    backend: Arc<dyn StorageBackend>,
    chain_config: ChainConfig,
    trie_cache: Arc<Mutex<Arc<TrieLayerCache>>>,
    flatkeyvalue_control_tx: std::sync::mpsc::SyncSender<FKVGeneratorControlMessage>,
    trie_update_worker_tx: std::sync::mpsc::SyncSender<TrieUpdate>,
    /// Keeps the latest canonical block header
    /// It's wrapped in an Arc to allow for cheap reads with infrequent writes
    /// Reading an out-of-date value is acceptable, since it's only used as:
    /// - a cache of the (frequently requested) header
    /// - a Latest tag for RPC, where a small extra delay before the newest block is expected
    /// - sync-related operations, which must be idempotent in order to handle reorgs
    latest_block_header: LatestBlockHeaderCache,
    last_computed_flatkeyvalue: Arc<Mutex<Vec<u8>>>,

    /// Cache for account bytecodes, keyed by the bytecode hash.
    /// Note that we don't remove entries on account code changes, since
    /// those changes already affect the code hash stored in the account, and only
    /// may result in this cache having useless data.
    account_code_cache: Arc<CodeCache>,
}

pub type StorageTrieNodes = Vec<(H256, Vec<(Nibbles, Vec<u8>)>)>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineType {
    InMemory,
    #[cfg(feature = "rocksdb")]
    RocksDB,
}

pub struct UpdateBatch {
    /// Nodes to be added to the state trie
    pub account_updates: Vec<TrieNode>,
    /// Storage tries updated and their new nodes
    pub storage_updates: Vec<(H256, Vec<TrieNode>)>,
    /// Blocks to be added
    pub blocks: Vec<Block>,
    /// Receipts added per block
    pub receipts: Vec<(H256, Vec<Receipt>)>,
    /// Code updates
    pub code_updates: Vec<(H256, Code)>,
}

pub type StorageUpdates = Vec<(H256, Vec<(Nibbles, Vec<u8>)>)>;

pub struct AccountUpdatesList {
    pub state_trie_hash: H256,
    pub state_updates: Vec<(Nibbles, Vec<u8>)>,
    pub storage_updates: StorageUpdates,
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
        if let Some(code) = self.account_code_cache.get(&code_hash)? {
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

        // Decode as Vec and convert to FxHashSet for O(1) lookup performance
        let jump_targets_vec: Vec<u32> = <Vec<_>>::decode(targets)?;
        let code = Code {
            hash: code_hash,
            bytecode,
            jump_targets: jump_targets_vec.into_iter().collect(),
        };

        // insert into cache and evict if needed
        self.account_code_cache.insert(&code)?;

        Ok(Some(code))
    }

    /// Add account code
    pub async fn add_account_code(&self, code: Code) -> Result<(), StoreError> {
        let hash_key = code.hash.0.to_vec();
        let buf = encode_code(&code);
        self.write_async(ACCOUNT_CODES, hash_key, buf).await
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
        let mut batch_items = Vec::new();
        for (code_hash, code) in account_codes {
            let buf = encode_code(&code);
            batch_items.push((code_hash.as_bytes().to_vec(), buf));
        }

        self.write_batch_async(ACCOUNT_CODES, batch_items).await
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
            tx.put(ACCOUNT_CODES, code_hash.as_ref(), &buf)?;
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
        let store = Self {
            db_path,
            backend,
            chain_config: Default::default(),
            latest_block_header: Default::default(),
            trie_cache: Arc::new(Mutex::new(Arc::new(TrieLayerCache::new(commit_threshold)))),
            flatkeyvalue_control_tx: fkv_tx,
            trie_update_worker_tx: trie_upd_tx,
            last_computed_flatkeyvalue: Arc::new(Mutex::new(last_written)),
            account_code_cache: Arc::new(CodeCache::default()),
        };
        let backend_clone = store.backend.clone();
        let last_computed_fkv = store.last_computed_flatkeyvalue.clone();
        std::thread::spawn(move || {
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
        });
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
        std::thread::spawn(move || {
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
        let hashed_address = hash_address(&address);

        let Some(encoded_state) = state_trie.get(&hashed_address)? else {
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
        let Some(encoded_state) = state_trie.get(&account_hash.to_fixed_bytes().to_vec())? else {
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
        let hashed_address = hash_address(&address);
        let Some(encoded_state) = state_trie.get(&hashed_address)? else {
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
        let hashed_address = hash_address(&address);
        let Some(encoded_state) = state_trie.get(&hashed_address)? else {
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
                let mut storage_trie = self.open_storage_trie(
                    H256::from_slice(&hashed_address),
                    state_root,
                    account_state.storage_root,
                )?;
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

        Ok(AccountUpdatesList {
            state_trie_hash,
            state_updates,
            storage_updates: ret_storage_updates,
            code_updates,
        })
    }

    /// Performs the same actions as apply_account_updates_from_trie
    ///  but also returns the used storage tries with witness recorded
    pub async fn apply_account_updates_from_trie_with_witness(
        &self,
        mut state_trie: Trie,
        account_updates: &[AccountUpdate],
        mut storage_tries: HashMap<Address, (TrieWitness, Trie)>,
    ) -> Result<(HashMap<Address, (TrieWitness, Trie)>, AccountUpdatesList), StoreError> {
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
        let hashed_address = hash_address(&address);
        let account_hash = H256::from_slice(&hashed_address);
        let storage_root = if self.flatkeyvalue_computed(account_hash)? {
            // We will use FKVs, we don't need the root
            *EMPTY_TRIE_HASH
        } else {
            let state_trie = self.open_state_trie(state_root)?;
            let Some(encoded_account) = state_trie.get(&hashed_address)? else {
                return Ok(None);
            };
            let account = AccountState::decode(&encoded_account)?;
            account.storage_root
        };
        let storage_trie = self.open_storage_trie(account_hash, state_root, storage_root)?;

        let hashed_key = hash_key(&storage_key);
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
        let hashed_address = hash_address(&address);
        let Some(encoded_account) = state_trie.get(&hashed_address)? else {
            return Ok(None);
        };
        let account = AccountState::decode(&encoded_account)?;
        // Open storage_trie
        let storage_root = account.storage_root;
        Ok(Some(self.open_storage_trie(
            H256::from_slice(&hashed_address),
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
        let hashed_address = hash_address(&address);
        let Some(encoded_state) = state_trie.get(&hashed_address)? else {
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
        let hashed_address = hash_address_fixed(&address);
        let address_path = hashed_address.0.to_vec();
        let proof = state_trie.get_proof(&address_path)?;
        let account_opt = state_trie
            .get(&address_path)?
            .map(|encoded_state| AccountState::decode(&encoded_state))
            .transpose()?;

        let mut storage_proof = Vec::with_capacity(storage_keys.len());

        if let Some(account) = &account_opt {
            let storage_trie =
                self.open_storage_trie(hashed_address, state_root, account.storage_root)?;

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
        let Some(account_rlp) = state_trie.get(&hashed_address.as_bytes().to_vec())? else {
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
        let mut proof = state_trie.get_proof(&starting_hash.as_bytes().to_vec())?;
        if let Some(last_hash) = last_hash {
            proof.extend_from_slice(&state_trie.get_proof(&last_hash.as_bytes().to_vec())?);
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
        let Some(account_rlp) = state_trie.get(&hashed_address.as_bytes().to_vec())? else {
            return Ok(None);
        };
        let storage_root = AccountState::decode(&account_rlp)?.storage_root;
        let storage_trie = self.open_storage_trie(hashed_address, state_root, storage_root)?;
        let mut proof = storage_trie.get_proof(&starting_hash.as_bytes().to_vec())?;
        if let Some(last_hash) = last_hash {
            proof.extend_from_slice(&storage_trie.get_proof(&last_hash.as_bytes().to_vec())?);
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

fn chain_data_key(index: ChainDataIndex) -> Vec<u8> {
    (index as u8).encode_to_vec()
}

fn snap_state_key(index: SnapStateIndex) -> Vec<u8> {
    (index as u8).encode_to_vec()
}

fn encode_code(code: &Code) -> Vec<u8> {
    // Convert FxHashSet to Vec for RLP encoding (storage format compatibility)
    let jump_targets_vec: Vec<u32> = code.jump_targets.iter().copied().collect();
    let mut buf = Vec::with_capacity(6 + code.bytecode.len() + jump_targets_vec.len() * 4);
    code.bytecode.encode(&mut buf);
    jump_targets_vec.encode(&mut buf);
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

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use ethereum_types::{H256, U256};
    use ethrex_common::{
        Bloom, H160,
        constants::EMPTY_KECCACK_HASH,
        types::{Transaction, TxType},
        utils::keccak,
    };
    use ethrex_rlp::decode::RLPDecode;
    use std::{fs, str::FromStr};

    use super::*;

    #[tokio::test]
    async fn test_in_memory_store() {
        test_store_suite(EngineType::InMemory).await;
    }

    #[cfg(feature = "rocksdb")]
    #[tokio::test]
    async fn test_rocksdb_store() {
        test_store_suite(EngineType::RocksDB).await;
    }

    // Creates an empty store, runs the test and then removes the store (if needed)
    async fn run_test<F, Fut>(test_func: F, engine_type: EngineType)
    where
        F: FnOnce(Store) -> Fut,
        Fut: std::future::Future<Output = ()>,
    {
        let nonce: u64 = H256::random().to_low_u64_be();
        let path = format!("store-test-db-{nonce}");
        // Remove preexistent DBs in case of a failed previous test
        if !matches!(engine_type, EngineType::InMemory) {
            remove_test_dbs(&path);
        };
        // Build a new store
        let store = Store::new(&path, engine_type).expect("Failed to create test db");
        // Run the test
        test_func(store).await;
        // Remove store (if needed)
        if !matches!(engine_type, EngineType::InMemory) {
            remove_test_dbs(&path);
        };
    }

    async fn test_store_suite(engine_type: EngineType) {
        run_test(test_store_block, engine_type).await;
        run_test(test_store_block_number, engine_type).await;
        run_test(test_store_block_receipt, engine_type).await;
        run_test(test_store_account_code, engine_type).await;
        run_test(test_store_block_tags, engine_type).await;
        run_test(test_chain_config_storage, engine_type).await;
        run_test(test_genesis_block, engine_type).await;
        run_test(test_iter_accounts, engine_type).await;
        run_test(test_iter_storage, engine_type).await;
    }

    async fn test_iter_accounts(store: Store) {
        let mut accounts: Vec<_> = (0u64..1_000)
            .map(|i| {
                (
                    keccak(i.to_be_bytes()),
                    AccountState {
                        nonce: 2 * i,
                        balance: U256::from(3 * i),
                        code_hash: *EMPTY_KECCACK_HASH,
                        storage_root: *EMPTY_TRIE_HASH,
                    },
                )
            })
            .collect();
        accounts.sort_by_key(|a| a.0);
        let mut trie = store.open_direct_state_trie(*EMPTY_TRIE_HASH).unwrap();
        for (address, state) in &accounts {
            trie.insert(address.0.to_vec(), state.encode_to_vec())
                .unwrap();
        }
        let state_root = trie.hash().unwrap();
        let pivot = H256::random();
        let pos = accounts.partition_point(|(key, _)| key < &pivot);
        let account_iter = store.iter_accounts_from(state_root, pivot).unwrap();
        for (expected, actual) in std::iter::zip(accounts.drain(pos..), account_iter) {
            assert_eq!(expected, actual);
        }
    }

    async fn test_iter_storage(store: Store) {
        let address = keccak(12345u64.to_be_bytes());
        let mut slots: Vec<_> = (0u64..1_000)
            .map(|i| (keccak(i.to_be_bytes()), U256::from(2 * i)))
            .collect();
        slots.sort_by_key(|a| a.0);
        let mut trie = store
            .open_direct_storage_trie(address, *EMPTY_TRIE_HASH)
            .unwrap();
        for (slot, value) in &slots {
            trie.insert(slot.0.to_vec(), value.encode_to_vec()).unwrap();
        }
        let storage_root = trie.hash().unwrap();
        let mut trie = store.open_direct_state_trie(*EMPTY_TRIE_HASH).unwrap();
        trie.insert(
            address.0.to_vec(),
            AccountState {
                nonce: 1,
                balance: U256::zero(),
                storage_root,
                code_hash: *EMPTY_KECCACK_HASH,
            }
            .encode_to_vec(),
        )
        .unwrap();
        let state_root = trie.hash().unwrap();
        let pivot = H256::random();
        let pos = slots.partition_point(|(key, _)| key < &pivot);
        let storage_iter = store
            .iter_storage_from(state_root, address, pivot)
            .unwrap()
            .unwrap();
        for (expected, actual) in std::iter::zip(slots.drain(pos..), storage_iter) {
            assert_eq!(expected, actual);
        }
    }

    async fn test_genesis_block(mut store: Store) {
        const GENESIS_KURTOSIS: &str = include_str!("../../fixtures/genesis/kurtosis.json");
        const GENESIS_HIVE: &str = include_str!("../../fixtures/genesis/hive.json");
        assert_ne!(GENESIS_KURTOSIS, GENESIS_HIVE);
        let genesis_kurtosis: Genesis =
            serde_json::from_str(GENESIS_KURTOSIS).expect("deserialize kurtosis.json");
        let genesis_hive: Genesis =
            serde_json::from_str(GENESIS_HIVE).expect("deserialize hive.json");
        store
            .add_initial_state(genesis_kurtosis.clone())
            .await
            .expect("first genesis");
        store
            .add_initial_state(genesis_kurtosis)
            .await
            .expect("second genesis with same block");
        let result = store.add_initial_state(genesis_hive).await;
        assert!(result.is_err());
        assert!(matches!(result, Err(StoreError::IncompatibleChainConfig)));
    }

    fn remove_test_dbs(path: &str) {
        // Removes all test databases from filesystem
        if std::path::Path::new(path).exists() {
            fs::remove_dir_all(path).expect("Failed to clean test db dir");
        }
    }

    async fn test_store_block(store: Store) {
        let (block_header, block_body) = create_block_for_testing();
        let block_number = 6;
        let hash = block_header.hash();

        store
            .add_block_header(hash, block_header.clone())
            .await
            .unwrap();
        store
            .add_block_body(hash, block_body.clone())
            .await
            .unwrap();
        store
            .forkchoice_update(vec![], block_number, hash, None, None)
            .await
            .unwrap();

        let stored_header = store.get_block_header(block_number).unwrap().unwrap();
        let stored_body = store.get_block_body(block_number).await.unwrap().unwrap();

        // Ensure both headers have their hashes computed for comparison
        let _ = stored_header.hash();
        let _ = block_header.hash();
        assert_eq!(stored_header, block_header);
        assert_eq!(stored_body, block_body);
    }

    fn create_block_for_testing() -> (BlockHeader, BlockBody) {
        let block_header = BlockHeader {
            parent_hash: H256::from_str(
                "0x1ac1bf1eef97dc6b03daba5af3b89881b7ae4bc1600dc434f450a9ec34d44999",
            )
            .unwrap(),
            ommers_hash: H256::from_str(
                "0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347",
            )
            .unwrap(),
            coinbase: Address::from_str("0x2adc25665018aa1fe0e6bc666dac8fc2697ff9ba").unwrap(),
            state_root: H256::from_str(
                "0x9de6f95cb4ff4ef22a73705d6ba38c4b927c7bca9887ef5d24a734bb863218d9",
            )
            .unwrap(),
            transactions_root: H256::from_str(
                "0x578602b2b7e3a3291c3eefca3a08bc13c0d194f9845a39b6f3bcf843d9fed79d",
            )
            .unwrap(),
            receipts_root: H256::from_str(
                "0x035d56bac3f47246c5eed0e6642ca40dc262f9144b582f058bc23ded72aa72fa",
            )
            .unwrap(),
            logs_bloom: Bloom::from([0; 256]),
            difficulty: U256::zero(),
            number: 1,
            gas_limit: 0x016345785d8a0000,
            gas_used: 0xa8de,
            timestamp: 0x03e8,
            extra_data: Bytes::new(),
            prev_randao: H256::zero(),
            nonce: 0x0000000000000000,
            base_fee_per_gas: Some(0x07),
            withdrawals_root: Some(
                H256::from_str(
                    "0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421",
                )
                .unwrap(),
            ),
            blob_gas_used: Some(0x00),
            excess_blob_gas: Some(0x00),
            parent_beacon_block_root: Some(H256::zero()),
            requests_hash: Some(*EMPTY_KECCACK_HASH),
            ..Default::default()
        };
        let block_body = BlockBody {
            transactions: vec![Transaction::decode(&hex::decode("b86f02f86c8330182480114e82f618946177843db3138ae69679a54b95cf345ed759450d870aa87bee53800080c080a0151ccc02146b9b11adf516e6787b59acae3e76544fdcd75e77e67c6b598ce65da064c5dd5aae2fbb535830ebbdad0234975cd7ece3562013b63ea18cc0df6c97d4").unwrap()).unwrap(),
            Transaction::decode(&hex::decode("f86d80843baa0c4082f618946177843db3138ae69679a54b95cf345ed759450d870aa87bee538000808360306ba0151ccc02146b9b11adf516e6787b59acae3e76544fdcd75e77e67c6b598ce65da064c5dd5aae2fbb535830ebbdad0234975cd7ece3562013b63ea18cc0df6c97d4").unwrap()).unwrap()],
            ommers: Default::default(),
            withdrawals: Default::default(),
        };
        (block_header, block_body)
    }

    async fn test_store_block_number(store: Store) {
        let block_hash = H256::random();
        let block_number = 6;

        store
            .add_block_number(block_hash, block_number)
            .await
            .unwrap();

        let stored_number = store.get_block_number(block_hash).await.unwrap().unwrap();

        assert_eq!(stored_number, block_number);
    }

    async fn test_store_block_receipt(store: Store) {
        let receipt = Receipt {
            tx_type: TxType::EIP2930,
            succeeded: true,
            cumulative_gas_used: 1747,
            logs: vec![],
        };
        let block_number = 6;
        let index = 4;
        let block_header = BlockHeader::default();

        store
            .add_receipt(block_header.hash(), index, receipt.clone())
            .await
            .unwrap();

        store
            .add_block_header(block_header.hash(), block_header.clone())
            .await
            .unwrap();

        store
            .forkchoice_update(vec![], block_number, block_header.hash(), None, None)
            .await
            .unwrap();

        let stored_receipt = store
            .get_receipt(block_number, index)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(stored_receipt, receipt);
    }

    async fn test_store_account_code(store: Store) {
        let code = Code::from_bytecode(Bytes::from("kiwi"));
        let code_hash = code.hash;

        store.add_account_code(code.clone()).await.unwrap();

        let stored_code = store.get_account_code(code_hash).unwrap().unwrap();

        assert_eq!(stored_code, code);
    }

    async fn test_store_block_tags(store: Store) {
        let earliest_block_number = 0;
        let finalized_block_number = 7;
        let safe_block_number = 6;
        let latest_block_number = 8;
        let pending_block_number = 9;

        let (mut block_header, block_body) = create_block_for_testing();
        block_header.number = latest_block_number;
        let hash = block_header.hash();

        store
            .add_block_header(hash, block_header.clone())
            .await
            .unwrap();
        store
            .add_block_body(hash, block_body.clone())
            .await
            .unwrap();

        store
            .update_earliest_block_number(earliest_block_number)
            .await
            .unwrap();
        store
            .update_pending_block_number(pending_block_number)
            .await
            .unwrap();
        store
            .forkchoice_update(
                vec![],
                latest_block_number,
                hash,
                Some(safe_block_number),
                Some(finalized_block_number),
            )
            .await
            .unwrap();

        let stored_earliest_block_number = store.get_earliest_block_number().await.unwrap();
        let stored_finalized_block_number =
            store.get_finalized_block_number().await.unwrap().unwrap();
        let stored_latest_block_number = store.get_latest_block_number().await.unwrap();
        let stored_safe_block_number = store.get_safe_block_number().await.unwrap().unwrap();
        let stored_pending_block_number = store.get_pending_block_number().await.unwrap().unwrap();

        assert_eq!(earliest_block_number, stored_earliest_block_number);
        assert_eq!(finalized_block_number, stored_finalized_block_number);
        assert_eq!(safe_block_number, stored_safe_block_number);
        assert_eq!(latest_block_number, stored_latest_block_number);
        assert_eq!(pending_block_number, stored_pending_block_number);
    }

    async fn test_chain_config_storage(mut store: Store) {
        let chain_config = example_chain_config();
        store.set_chain_config(&chain_config).await.unwrap();
        let retrieved_chain_config = store.get_chain_config();
        assert_eq!(chain_config, retrieved_chain_config);
    }

    fn example_chain_config() -> ChainConfig {
        ChainConfig {
            chain_id: 3151908_u64,
            homestead_block: Some(0),
            eip150_block: Some(0),
            eip155_block: Some(0),
            eip158_block: Some(0),
            byzantium_block: Some(0),
            constantinople_block: Some(0),
            petersburg_block: Some(0),
            istanbul_block: Some(0),
            berlin_block: Some(0),
            london_block: Some(0),
            merge_netsplit_block: Some(0),
            shanghai_time: Some(0),
            cancun_time: Some(0),
            prague_time: Some(1718232101),
            terminal_total_difficulty: Some(58750000000000000000000),
            terminal_total_difficulty_passed: true,
            deposit_contract_address: H160::from_str("0x4242424242424242424242424242424242424242")
                .unwrap(),
            ..Default::default()
        }
    }
}
