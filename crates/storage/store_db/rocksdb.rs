use crate::trie_db::rocksdb_locked::RocksDBLockedTrieDB;
use bytes::Bytes;
use ethrex_common::{
    H256, U256,
    types::{
        Block, BlockBody, BlockHash, BlockHeader, BlockNumber, ChainConfig, Index, Receipt,
        Transaction,
    },
    utils::u256_to_big_endian,
};
use ethrex_trie::{Nibbles, NodeHash, Trie};
use rocksdb::{
    BlockBasedOptions, BoundColumnFamily, Cache, ColumnFamilyDescriptor, MultiThreaded,
    OptimisticTransactionDB, Options, WriteBatchWithTransaction,
};
use std::sync::Arc;
use tracing::info;

use crate::{
    STATE_TRIE_SEGMENTS, UpdateBatch,
    api::StoreEngine,
    error::StoreError,
    rlp::{AccountCodeRLP, BlockBodyRLP, BlockHashRLP, BlockHeaderRLP, BlockRLP},
    trie_db::rocksdb::RocksDBTrieDB,
    utils::{ChainDataIndex, SnapStateIndex},
};
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};

/// Canonical block hashes column family: [`[u8; 8]`] => [`BlockHashRLP`]
/// - Key: `block_number.to_le_bytes()` (8 bytes, little-endian)
/// - Value: `BlockHashRLP::from(block_hash)` (RLP-encoded H256, ~33 bytes)
const CF_CANONICAL_BLOCK_HASHES: &str = "canonical_block_hashes";

/// Block numbers column family: [`BlockHashRLP`] => [`[u8; 8]`]
/// - Key: `BlockHashRLP::from(block_hash)` (RLP-encoded H256, ~33 bytes)
/// - Value: `block_number.to_le_bytes()` (8 bytes, little-endian)
const CF_BLOCK_NUMBERS: &str = "block_numbers";

/// Block headers column family: [`BlockHashRLP`] => [`BlockHeaderRLP`]
/// - Key: `BlockHashRLP::from(block_hash)` (RLP-encoded H256, ~33 bytes)
/// - Value: `BlockHeaderRLP::from(block_header)` (RLP-encoded BlockHeader, ~500-600 bytes)
const CF_HEADERS: &str = "headers";

/// Block bodies column family: [`BlockHashRLP`] => [`BlockBodyRLP`]
/// - Key: `BlockHashRLP::from(block_hash)` (RLP-encoded H256, ~33 bytes)
/// - Value: `BlockBodyRLP::from(block_body)` (RLP-encoded BlockBody, variable size)
const CF_BODIES: &str = "bodies";

/// Account codes column family: [`H256`] => [`AccountCodeRLP`]
/// - Key: `code_hash.as_bytes()` (32 bytes, H256 hash)
/// - Value: `AccountCodeRLP::from(code)` (RLP-encoded bytecode, variable size)
const CF_ACCOUNT_CODES: &str = "account_codes";

/// Receipts column family: [`(BlockHash, u64)`] => [`Receipt`]
/// - Key: `(block_hash, index).encode_to_vec()` (~40 bytes, RLP-encoded tuple)
/// - Value: `receipt.encode_to_vec()` (RLP-encoded Receipt, ~100-200 bytes)
const CF_RECEIPTS: &str = "receipts";

/// Transaction locations column family: [`H256 + H256`] => [`(BlockNumber, BlockHash, u64)`]
/// - Key: Composite key (64 bytes):
///   ```rust,no_run
///   let mut composite_key = Vec::with_capacity(64);
///   composite_key.extend_from_slice(transaction_hash.as_bytes()); // 32 bytes
///   composite_key.extend_from_slice(block_hash.as_bytes());       // 32 bytes
///   ```
/// - Value: `(block_number, block_hash, index).encode_to_vec()` (RLP-encoded tuple, ~48 bytes)
const CF_TRANSACTION_LOCATIONS: &str = "transaction_locations";

/// Chain data column family: [`ChainDataIndex`] => [`Variable`]
/// - Key: `Self::chain_data_key(index)` (1 byte, enum as u8)
/// - Value: Variable format depending on ChainDataIndex:
///   - ChainConfig: JSON string bytes
///   - Block numbers: `block_number.to_le_bytes()` (8 bytes)
const CF_CHAIN_DATA: &str = "chain_data";

/// Snap state column family: [`SnapStateIndex`] => [`Variable`]
/// - Key: `Self::snap_state_key(index)` (1 byte, enum as u8)
/// - Value: Variable format depending on SnapStateIndex:
///   - HeaderDownloadCheckpoint: `BlockHashRLP` (~33 bytes)
///   - StateTrieKeyCheckpoint: `Vec<H256>` (variable size)
///   - StateHealPaths: `Vec<(Nibbles, H256)>` (variable size)
const CF_SNAP_STATE: &str = "snap_state";

/// State trie nodes column family: [`NodeHash`] => [`Vec<u8>`]
/// - Key: `node_hash.as_ref()` (32 bytes, H256 hash of the node)
/// - Value: `node_data` (RLP-encoded trie node, variable size 32-2KB typically)
const CF_STATE_TRIE_NODES: &str = "state_trie_nodes";

/// Storage tries nodes column family: ([`H256`] + [`NodeHash`]) => [`Vec<u8>`]
/// - Key: Composite key (64 bytes):
///   ```rust,no_run
///   let mut key = Vec::with_capacity(64);
///   key.extend_from_slice(address_hash.as_bytes()); // 32 bytes (account hash)
///   key.extend_from_slice(node_hash.as_ref());      // 32 bytes (storage node hash)
///   ```
/// - Value: `node_data` (RLP-encoded trie node, variable size 32-2KB typically)
const CF_STORAGE_TRIES_NODES: &str = "storage_tries_nodes";

/// Pending blocks column family: [`BlockHashRLP`] => [`BlockRLP`]
/// - Key: `BlockHashRLP::from(block_hash)` (RLP-encoded H256, ~33 bytes)
/// - Value: `BlockRLP::from(block)` (RLP-encoded complete Block, variable size)
const CF_PENDING_BLOCKS: &str = "pending_blocks";

/// Storage snapshot column family: [`H256 + H256`] => [`U256`]
/// - Key: Composite key (64 bytes):
///   ```rust,no_run
///   let mut composite_key = Vec::with_capacity(64);
///   composite_key.extend_from_slice(account_hash.as_bytes()); // 32 bytes (account hash)
///   composite_key.extend_from_slice(storage_key.as_bytes());  // 32 bytes (storage slot hash)
///   ```
/// - Value: `u256_to_big_endian(value).to_vec()` (32 bytes, U256 in big-endian format)
const CF_STORAGE_SNAPSHOT: &str = "storage_snapshot";

/// Invalid ancestors column family: [`BlockHashRLP`] => [`BlockHashRLP`]
/// - Key: `BlockHashRLP::from(bad_block)` (RLP-encoded H256, ~33 bytes)
/// - Value: `BlockHashRLP::from(latest_valid)` (RLP-encoded H256, ~33 bytes)
const CF_INVALID_ANCESTORS: &str = "invalid_ancestors";

#[derive(Debug)]
pub struct Store {
    db: Arc<OptimisticTransactionDB<MultiThreaded>>,
}

impl Store {
    pub fn new(path: &str) -> Result<Self, StoreError> {
        // TODO: Review all the config values
        let mut db_options = Options::default();
        db_options.create_if_missing(true);
        db_options.create_missing_column_families(true);

        let cache = Cache::new_lru_cache(4 * 1024 * 1024 * 1024); // 4GB cache 

        db_options.set_max_open_files(-1);
        db_options.set_max_file_opening_threads(16);

        db_options.set_max_background_jobs(8);

        db_options.set_level_zero_file_num_compaction_trigger(2);
        db_options.set_level_zero_slowdown_writes_trigger(10);
        db_options.set_level_zero_stop_writes_trigger(16);
        db_options.set_target_file_size_base(512 * 1024 * 1024); // 512MB
        db_options.set_max_bytes_for_level_base(2 * 1024 * 1024 * 1024); // 2GB L1
        db_options.set_max_bytes_for_level_multiplier(10.0);
        db_options.set_level_compaction_dynamic_level_bytes(true);

        db_options.set_db_write_buffer_size(1024 * 1024 * 1024); // 1GB
        db_options.set_write_buffer_size(128 * 1024 * 1024); // 128MB
        db_options.set_max_write_buffer_number(4);
        db_options.set_min_write_buffer_number_to_merge(2);

        db_options.set_wal_recovery_mode(rocksdb::DBRecoveryMode::PointInTime);
        db_options.set_max_total_wal_size(2 * 1024 * 1024 * 1024); // 2GB
        db_options.set_wal_ttl_seconds(3600);
        db_options.set_wal_bytes_per_sync(32 * 1024 * 1024); // 32MB
        db_options.set_bytes_per_sync(32 * 1024 * 1024); // 32MB
        db_options.set_use_fsync(false); // fdatasync

        db_options.set_enable_pipelined_write(true);
        db_options.set_allow_concurrent_memtable_write(true);
        db_options.set_enable_write_thread_adaptive_yield(true);
        db_options.set_compaction_readahead_size(4 * 1024 * 1024); // 4MB
        db_options.set_advise_random_on_open(false);

        // Current column families that the code expects
        const EXPECTED_CFS: [&str; 14] = [
            CF_CANONICAL_BLOCK_HASHES,
            CF_BLOCK_NUMBERS,
            CF_HEADERS,
            CF_BODIES,
            CF_ACCOUNT_CODES,
            CF_RECEIPTS,
            CF_TRANSACTION_LOCATIONS,
            CF_CHAIN_DATA,
            CF_SNAP_STATE,
            CF_STATE_TRIE_NODES,
            CF_STORAGE_TRIES_NODES,
            CF_PENDING_BLOCKS,
            CF_STORAGE_SNAPSHOT,
            CF_INVALID_ANCESTORS,
        ];

        let mut cf_descriptors = Vec::new();
        for cf_name in EXPECTED_CFS {
            // TODO: Review all the config values
            let mut cf_opts = Options::default();

            cf_opts.set_level_zero_file_num_compaction_trigger(4);
            cf_opts.set_level_zero_slowdown_writes_trigger(20);
            cf_opts.set_level_zero_stop_writes_trigger(36);

            match cf_name {
                CF_HEADERS | CF_BODIES => {
                    cf_opts.set_compression_type(rocksdb::DBCompressionType::Zstd);
                    cf_opts.set_write_buffer_size(128 * 1024 * 1024); // 128MB
                    cf_opts.set_max_write_buffer_number(4);
                    cf_opts.set_target_file_size_base(256 * 1024 * 1024); // 256MB 

                    let mut block_opts = BlockBasedOptions::default();
                    block_opts.set_block_cache(&cache);
                    block_opts.set_block_size(32 * 1024); // 32KB blocks
                    block_opts.set_cache_index_and_filter_blocks(true);
                    cf_opts.set_block_based_table_factory(&block_opts);
                }
                CF_CANONICAL_BLOCK_HASHES | CF_BLOCK_NUMBERS => {
                    cf_opts.set_compression_type(rocksdb::DBCompressionType::Lz4);
                    cf_opts.set_write_buffer_size(64 * 1024 * 1024); // 64MB
                    cf_opts.set_max_write_buffer_number(3);
                    cf_opts.set_target_file_size_base(128 * 1024 * 1024); // 128MB 

                    let mut block_opts = BlockBasedOptions::default();
                    block_opts.set_block_cache(&cache);
                    block_opts.set_block_size(16 * 1024); // 16KB
                    block_opts.set_bloom_filter(10.0, false);
                    block_opts.set_cache_index_and_filter_blocks(true);
                    cf_opts.set_block_based_table_factory(&block_opts);
                }
                CF_STATE_TRIE_NODES | CF_STORAGE_TRIES_NODES => {
                    cf_opts.set_compression_type(rocksdb::DBCompressionType::Lz4);
                    cf_opts.set_write_buffer_size(512 * 1024 * 1024); // 512MB 
                    cf_opts.set_max_write_buffer_number(6);
                    cf_opts.set_min_write_buffer_number_to_merge(2);
                    cf_opts.set_target_file_size_base(256 * 1024 * 1024); // 256MB 
                    cf_opts.set_memtable_prefix_bloom_ratio(0.2); // Bloom filter 

                    let mut block_opts = BlockBasedOptions::default();
                    block_opts.set_block_size(16 * 1024); // 16KB 
                    block_opts.set_block_cache(&cache);
                    block_opts.set_bloom_filter(10.0, false); // 10 bits per key
                    block_opts.set_cache_index_and_filter_blocks(true);
                    block_opts.set_pin_l0_filter_and_index_blocks_in_cache(true);
                    cf_opts.set_block_based_table_factory(&block_opts);
                }
                CF_RECEIPTS | CF_ACCOUNT_CODES => {
                    cf_opts.set_compression_type(rocksdb::DBCompressionType::Lz4);
                    cf_opts.set_write_buffer_size(128 * 1024 * 1024); // 128MB
                    cf_opts.set_max_write_buffer_number(3);
                    cf_opts.set_target_file_size_base(256 * 1024 * 1024); // 256MB

                    let mut block_opts = BlockBasedOptions::default();
                    block_opts.set_block_cache(&cache);
                    block_opts.set_block_size(32 * 1024); // 32KB
                    cf_opts.set_block_based_table_factory(&block_opts);
                }
                _ => {
                    // Default for other CFs
                    cf_opts.set_compression_type(rocksdb::DBCompressionType::Lz4);
                    cf_opts.set_write_buffer_size(64 * 1024 * 1024); // 64MB
                    cf_opts.set_max_write_buffer_number(3);
                    cf_opts.set_target_file_size_base(128 * 1024 * 1024); // 128MB 

                    let mut block_opts = BlockBasedOptions::default();
                    block_opts.set_block_size(16 * 1024);
                    block_opts.set_block_cache(&cache);
                    cf_opts.set_block_based_table_factory(&block_opts);
                }
            }

            cf_descriptors.push(ColumnFamilyDescriptor::new(cf_name, cf_opts));
        }

        let db = OptimisticTransactionDB::<MultiThreaded>::open_cf_descriptors(
            &db_options,
            path,
            cf_descriptors,
        )
        .map_err(|e| StoreError::Custom(format!("Failed to open RocksDB: {e}")))?;

        Ok(Self { db: Arc::new(db) })
    }

    // Helper method for async writes
    async fn write_async<K, V>(&self, cf_name: &str, key: K, value: V) -> Result<(), StoreError>
    where
        K: AsRef<[u8]> + Send + 'static,
        V: AsRef<[u8]> + Send + 'static,
    {
        let db = self.db.clone();
        let cf_name = cf_name.to_string();

        tokio::task::spawn_blocking(move || {
            let cf = db
                .cf_handle(&cf_name)
                .ok_or_else(|| StoreError::Custom(format!("Column family not found: {cf_name}")))?;
            db.put_cf(&cf, key, value).map_err(StoreError::from)
        })
        .await
        .map_err(|e| StoreError::Custom(format!("Task panicked: {e}")))?
    }

    // Helper method for async reads
    async fn read_async<K>(&self, cf_name: &str, key: K) -> Result<Option<Vec<u8>>, StoreError>
    where
        K: AsRef<[u8]> + Send + 'static,
    {
        let db = self.db.clone();
        let cf_name = cf_name.to_string();

        tokio::task::spawn_blocking(move || {
            let cf = db
                .cf_handle(&cf_name)
                .ok_or_else(|| StoreError::Custom(format!("Column family not found: {cf_name}")))?;
            db.get_cf(&cf, key).map_err(StoreError::from)
        })
        .await
        .map_err(|e| StoreError::Custom(format!("Task panicked: {e}")))?
    }

    // Helper method for sync reads
    fn read_sync<K>(&self, cf_name: &str, key: K) -> Result<Option<Vec<u8>>, StoreError>
    where
        K: AsRef<[u8]>,
    {
        let cf = self
            .db
            .cf_handle(cf_name)
            .ok_or(StoreError::Custom(format!(
                "Column family not found: {cf_name}"
            )))?;
        self.db.get_cf(&cf, key).map_err(StoreError::from)
    }

    // Helper method for batch writes
    async fn write_batch_async(
        &self,
        batch_ops: Vec<(String, Vec<u8>, Vec<u8>)>,
    ) -> Result<(), StoreError> {
        let db = self.db.clone();

        tokio::task::spawn_blocking(move || {
            let mut batch = WriteBatchWithTransaction::<true>::default();

            for (cf_name, key, value) in batch_ops {
                let cf = db.cf_handle(&cf_name).ok_or_else(|| {
                    StoreError::Custom(format!("Column family not found: {cf_name}"))
                })?;
                batch.put_cf(&cf, key, value);
            }

            db.write(batch).map_err(StoreError::from)
        })
        .await
        .map_err(|e| StoreError::Custom(format!("Task panicked: {e}")))?
    }

    // Helper method to encode ChainDataIndex as key
    fn chain_data_key(index: ChainDataIndex) -> Vec<u8> {
        (index as u8).encode_to_vec()
    }

    // Helper method to encode SnapStateIndex as key
    fn snap_state_key(index: SnapStateIndex) -> Vec<u8> {
        (index as u8).encode_to_vec()
    }

    // Helper method for bulk reads - equivalent to LibMDBX read_bulk
    async fn read_bulk_async<K, V, F>(
        &self,
        cf_name: &str,
        keys: Vec<K>,
        deserialize_fn: F,
    ) -> Result<Vec<V>, StoreError>
    where
        K: AsRef<[u8]> + Send + 'static,
        V: Send + 'static,
        F: Fn(Vec<u8>) -> Result<V, StoreError> + Send + 'static,
    {
        let db = self.db.clone();
        let cf_name = cf_name.to_string();

        tokio::task::spawn_blocking(move || {
            let cf = db
                .cf_handle(&cf_name)
                .ok_or_else(|| StoreError::Custom(format!("Column family not found: {cf_name}")))?;

            let mut results = Vec::with_capacity(keys.len());

            for key in keys {
                let Some(bytes) = db.get_cf(&cf, key)? else {
                    return Err(StoreError::Custom("Key not found in bulk read".to_string()));
                };

                let value = deserialize_fn(bytes)?;
                results.push(value);
            }

            Ok(results)
        })
        .await
        .map_err(|e| StoreError::Custom(format!("Task panicked: {e}")))?
    }
}

#[async_trait::async_trait]
impl StoreEngine for Store {
    async fn apply_updates(&self, update_batch: UpdateBatch) -> Result<(), StoreError> {
        let db = self.db.clone();

        tokio::task::spawn_blocking(move || {
            let [
                cf_state,
                cf_storage,
                cf_receipts,
                cf_codes,
                cf_block_numbers,
                cf_tx_locations,
                cf_headers,
                cf_bodies,
            ] = open_cfs(
                &db,
                [
                    CF_STATE_TRIE_NODES,
                    CF_STORAGE_TRIES_NODES,
                    CF_RECEIPTS,
                    CF_ACCOUNT_CODES,
                    CF_BLOCK_NUMBERS,
                    CF_TRANSACTION_LOCATIONS,
                    CF_HEADERS,
                    CF_BODIES,
                ],
            )?;

            let _span = tracing::trace_span!("Block DB update").entered();
            let mut batch = WriteBatchWithTransaction::<true>::default();

            for (node_hash, node_data) in update_batch.account_updates {
                batch.put_cf(&cf_state, node_hash.as_ref(), node_data);
            }

            for (address_hash, storage_updates) in update_batch.storage_updates {
                for (node_hash, node_data) in storage_updates {
                    // Key: address_hash + node_hash
                    let mut key = Vec::with_capacity(64);
                    key.extend_from_slice(address_hash.as_bytes());
                    key.extend_from_slice(node_hash.as_ref());
                    batch.put_cf(&cf_storage, key, node_data);
                }
            }

            for block in update_batch.blocks {
                let block_number = block.header.number;
                let block_hash = block.hash();

                let hash_key_rlp = BlockHashRLP::from(block_hash);
                let header_value_rlp = BlockHeaderRLP::from(block.header.clone());
                batch.put_cf(&cf_headers, &hash_key_rlp, &header_value_rlp);

                let body_value = BlockBodyRLP::from_bytes(block.body.encode_to_vec());
                batch.put_cf(&cf_bodies, &hash_key_rlp, &body_value);

                batch.put_cf(&cf_block_numbers, &hash_key_rlp, block_number.to_le_bytes());

                for (index, transaction) in block.body.transactions.iter().enumerate() {
                    let tx_hash = transaction.hash();
                    // Key: tx_hash + block_hash
                    let mut composite_key = Vec::with_capacity(64);
                    composite_key.extend_from_slice(tx_hash.as_bytes());
                    composite_key.extend_from_slice(block_hash.as_bytes());
                    let location_value = (block_number, block_hash, index as u64).encode_to_vec();
                    batch.put_cf(&cf_tx_locations, composite_key, location_value);
                }
            }

            for (block_hash, receipts) in update_batch.receipts {
                for (index, receipt) in receipts.into_iter().enumerate() {
                    let key = (block_hash, index as u64).encode_to_vec();
                    let value = receipt.encode_to_vec();
                    batch.put_cf(&cf_receipts, key, value);
                }
            }

            for (code_hash, code) in update_batch.code_updates {
                let code_key = code_hash.as_bytes();
                let code_value = AccountCodeRLP::from(code);
                batch.put_cf(&cf_codes, code_key, &code_value);
            }

            // Single write operation
            db.write(batch).map_err(StoreError::from)
        })
        .await
        .map_err(|e| StoreError::Custom(format!("Task panicked: {e}")))?
    }

    /// Add a batch of blocks in a single transaction.
    /// This will store -> BlockHeader, BlockBody, BlockTransactions, BlockNumber.
    async fn add_blocks(&self, blocks: Vec<Block>) -> Result<(), StoreError> {
        let db = self.db.clone();

        tokio::task::spawn_blocking(move || {
            let mut batch = WriteBatchWithTransaction::<true>::default();

            let [cf_headers, cf_bodies, cf_block_numbers, cf_tx_locations] = open_cfs(
                &db,
                [
                    CF_HEADERS,
                    CF_BODIES,
                    CF_BLOCK_NUMBERS,
                    CF_TRANSACTION_LOCATIONS,
                ],
            )?;

            for block in blocks {
                let block_hash = block.hash();
                let block_number = block.header.number;

                // Reuse the same key for all operations
                let hash_key = BlockHashRLP::from(block_hash);
                let header_value = BlockHeaderRLP::from(block.header.clone());
                batch.put_cf(&cf_headers, &hash_key, &header_value);

                let body_value = BlockBodyRLP::from(block.body.clone());
                batch.put_cf(&cf_bodies, &hash_key, &body_value);

                batch.put_cf(&cf_block_numbers, &hash_key, block_number.to_le_bytes());

                for (index, transaction) in block.body.transactions.iter().enumerate() {
                    let tx_hash = transaction.hash();
                    // Key: tx_hash + block_hash
                    let mut composite_key = Vec::with_capacity(64);
                    composite_key.extend_from_slice(tx_hash.as_bytes());
                    composite_key.extend_from_slice(block_hash.as_bytes());
                    let location_value = (block_number, block_hash, index as u64).encode_to_vec();
                    batch.put_cf(&cf_tx_locations, composite_key, location_value);
                }
            }

            db.write(batch).map_err(StoreError::from)
        })
        .await
        .map_err(|e| StoreError::Custom(format!("Task panicked: {e}")))?
    }

    async fn add_block_header(
        &self,
        block_hash: BlockHash,
        block_header: BlockHeader,
    ) -> Result<(), StoreError> {
        let hash_key = BlockHashRLP::from(block_hash).as_ref().to_vec();
        let header_value = BlockHeaderRLP::from(block_header).as_ref().to_vec();
        self.write_async(CF_HEADERS, hash_key, header_value).await
    }

    async fn add_block_headers(&self, block_headers: Vec<BlockHeader>) -> Result<(), StoreError> {
        let mut batch_ops = Vec::new();

        for header in block_headers {
            let block_hash = header.hash();
            let hash_key = BlockHashRLP::from(block_hash);
            let header_value = BlockHeaderRLP::from(header.clone());

            batch_ops.push((
                CF_HEADERS.to_string(),
                hash_key.as_ref().to_vec(),
                header_value.as_ref().to_vec(),
            ));

            let number_key = header.number.to_le_bytes();
            batch_ops.push((
                CF_BLOCK_NUMBERS.to_string(),
                hash_key.as_ref().to_vec(),
                number_key.to_vec(),
            ));
        }

        self.write_batch_async(batch_ops).await
    }

    fn get_block_header(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<BlockHeader>, StoreError> {
        let Some(block_hash) = self.get_canonical_block_hash_sync(block_number)? else {
            return Ok(None);
        };

        self.get_block_header_by_hash(block_hash)
    }

    async fn add_block_body(
        &self,
        block_hash: BlockHash,
        block_body: BlockBody,
    ) -> Result<(), StoreError> {
        let hash_key = BlockHashRLP::from(block_hash).as_ref().to_vec();
        let body_value = BlockBodyRLP::from(block_body).as_ref().to_vec();
        self.write_async(CF_BODIES, hash_key, body_value).await
    }

    async fn get_block_body(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<BlockBody>, StoreError> {
        let Some(block_hash) = self.get_canonical_block_hash_sync(block_number)? else {
            return Ok(None);
        };

        self.get_block_body_by_hash(block_hash).await
    }

    async fn remove_block(&self, block_number: BlockNumber) -> Result<(), StoreError> {
        let mut batch = WriteBatchWithTransaction::<true>::default();

        let Some(hash) = self.get_canonical_block_hash_sync(block_number)? else {
            return Ok(());
        };

        let [cf_canonical, cf_bodies, cf_headers, cf_block_numbers] = open_cfs(
            &self.db,
            [
                CF_CANONICAL_BLOCK_HASHES,
                CF_BODIES,
                CF_HEADERS,
                CF_BLOCK_NUMBERS,
            ],
        )?;

        batch.delete_cf(&cf_canonical, block_number.to_le_bytes());
        batch.delete_cf(&cf_bodies, hash.as_bytes());
        batch.delete_cf(&cf_headers, hash.as_bytes());
        batch.delete_cf(&cf_block_numbers, hash.as_bytes());

        self.db.write(batch).map_err(StoreError::from)
    }

    async fn get_block_bodies(
        &self,
        from: BlockNumber,
        to: BlockNumber,
    ) -> Result<Vec<BlockBody>, StoreError> {
        let numbers: Vec<BlockNumber> = (from..=to).collect();
        let number_keys: Vec<[u8; 8]> = numbers.iter().map(|n| n.to_le_bytes()).collect();

        let hashes = self
            .read_bulk_async(CF_CANONICAL_BLOCK_HASHES, number_keys, |bytes| {
                BlockHashRLP::from_bytes(bytes)
                    .to()
                    .map_err(StoreError::from)
            })
            .await?;

        let hash_keys: Vec<Vec<u8>> = hashes
            .iter()
            .map(|hash| BlockHashRLP::from(*hash).as_ref().to_vec())
            .collect();

        let bodies = self
            .read_bulk_async(CF_BODIES, hash_keys, |bytes| {
                BlockBodyRLP::from_bytes(bytes)
                    .to()
                    .map_err(StoreError::from)
            })
            .await?;

        Ok(bodies)
    }

    async fn get_block_bodies_by_hash(
        &self,
        hashes: Vec<BlockHash>,
    ) -> Result<Vec<BlockBody>, StoreError> {
        let hash_keys: Vec<Vec<u8>> = hashes
            .iter()
            .map(|hash| BlockHashRLP::from(*hash).as_ref().to_vec())
            .collect();

        let bodies = self
            .read_bulk_async(CF_BODIES, hash_keys, |bytes| {
                BlockBodyRLP::from_bytes(bytes)
                    .to()
                    .map_err(StoreError::from)
            })
            .await?;

        Ok(bodies)
    }

    async fn get_block_body_by_hash(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockBody>, StoreError> {
        let hash_key = BlockHashRLP::from(block_hash).as_ref().to_vec();

        self.read_async(CF_BODIES, hash_key)
            .await?
            .map(|bytes| BlockBodyRLP::from_bytes(bytes).to())
            .transpose()
            .map_err(StoreError::from)
    }

    fn get_block_header_by_hash(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockHeader>, StoreError> {
        let hash_key = BlockHashRLP::from(block_hash).as_ref().to_vec();

        self.read_sync(CF_HEADERS, hash_key)?
            .map(|bytes| BlockHeaderRLP::from_bytes(bytes).to())
            .transpose()
            .map_err(StoreError::from)
    }

    async fn add_pending_block(&self, block: Block) -> Result<(), StoreError> {
        let hash_key = BlockHashRLP::from(block.hash()).as_ref().to_vec();
        let block_value = BlockRLP::from(block).as_ref().to_vec();
        self.write_async(CF_PENDING_BLOCKS, hash_key, block_value)
            .await
    }

    async fn get_pending_block(&self, block_hash: BlockHash) -> Result<Option<Block>, StoreError> {
        let hash_key = BlockHashRLP::from(block_hash).as_ref().to_vec();

        self.read_async(CF_PENDING_BLOCKS, hash_key)
            .await?
            .map(|bytes| BlockRLP::from_bytes(bytes).to())
            .transpose()
            .map_err(StoreError::from)
    }

    async fn add_block_number(
        &self,
        block_hash: BlockHash,
        block_number: BlockNumber,
    ) -> Result<(), StoreError> {
        let hash_key = BlockHashRLP::from(block_hash).as_ref().to_vec();
        let number_value = block_number.to_le_bytes();
        self.write_async(CF_BLOCK_NUMBERS, hash_key, number_value)
            .await
    }

    async fn get_block_number(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockNumber>, StoreError> {
        let hash_key = BlockHashRLP::from(block_hash).as_ref().to_vec();

        self.read_async(CF_BLOCK_NUMBERS, hash_key)
            .await?
            .map(|bytes| -> Result<BlockNumber, StoreError> {
                let array: [u8; 8] = bytes
                    .try_into()
                    .map_err(|_| StoreError::Custom("Invalid BlockNumber bytes".to_string()))?;
                Ok(BlockNumber::from_le_bytes(array))
            })
            .transpose()
    }

    async fn add_transaction_location(
        &self,
        transaction_hash: H256,
        block_number: BlockNumber,
        block_hash: BlockHash,
        index: Index,
    ) -> Result<(), StoreError> {
        // Key: tx_hash + block_hash
        let mut composite_key = Vec::with_capacity(64);
        composite_key.extend_from_slice(transaction_hash.as_bytes());
        composite_key.extend_from_slice(block_hash.as_bytes());

        let location_value = (block_number, block_hash, index).encode_to_vec();
        self.write_async(CF_TRANSACTION_LOCATIONS, composite_key, location_value)
            .await
    }

    // TODO: REVIEW LOGIC AGAINST LIBMDBX
    // Check also keys
    async fn add_transaction_locations(
        &self,
        locations: Vec<(H256, BlockNumber, BlockHash, Index)>,
    ) -> Result<(), StoreError> {
        let mut batch_ops = Vec::new();

        for (tx_hash, block_number, block_hash, index) in locations {
            // Key: tx_hash + block_hash
            let mut composite_key = Vec::with_capacity(64);
            composite_key.extend_from_slice(tx_hash.as_bytes());
            composite_key.extend_from_slice(block_hash.as_bytes());

            let location_value = (block_number, block_hash, index).encode_to_vec();
            batch_ops.push((
                CF_TRANSACTION_LOCATIONS.to_string(),
                composite_key,
                location_value,
            ));
        }

        self.write_batch_async(batch_ops).await
    }

    // TODO: REVIEW LOGIC AGAINST LIBMDBX
    // Check also keys
    async fn get_transaction_location(
        &self,
        transaction_hash: H256,
    ) -> Result<Option<(BlockNumber, BlockHash, Index)>, StoreError> {
        let db = self.db.clone();
        let tx_hash_key = transaction_hash.as_bytes().to_vec();

        tokio::task::spawn_blocking(move || {
            let [cf_transaction_locations, cf_canonical] =
                open_cfs(&db, [CF_TRANSACTION_LOCATIONS, CF_CANONICAL_BLOCK_HASHES])?;

            let mut iter = db.prefix_iterator_cf(&cf_transaction_locations, &tx_hash_key);
            let mut transaction_locations = Vec::new();

            while let Some(Ok((key, value))) = iter.next() {
                // Ensure key is exactly tx_hash + block_hash (32 + 32 = 64 bytes)
                // and starts with our exact tx_hash
                if key.len() == 64 && &key[0..32] == tx_hash_key.as_slice() {
                    transaction_locations.push(<(BlockNumber, BlockHash, Index)>::decode(&value)?);
                }
            }

            if transaction_locations.is_empty() {
                return Ok(None);
            }

            // If there are multiple locations, filter by the canonical chain
            for (block_number, block_hash, index) in transaction_locations {
                let canonical_hash = {
                    db.get_cf(&cf_canonical, block_number.to_le_bytes())?
                        .and_then(|bytes| BlockHashRLP::from_bytes(bytes).to().ok())
                };

                if canonical_hash == Some(block_hash) {
                    return Ok(Some((block_number, block_hash, index)));
                }
            }

            Ok(None)
        })
        .await
        .map_err(|e| StoreError::Custom(format!("Task panicked: {e}")))?
    }

    async fn add_receipt(
        &self,
        block_hash: BlockHash,
        index: Index,
        receipt: Receipt,
    ) -> Result<(), StoreError> {
        let key = (block_hash, index).encode_to_vec();
        let value = receipt.encode_to_vec();
        self.write_async(CF_RECEIPTS, key, value).await
    }

    async fn add_receipts(
        &self,
        block_hash: BlockHash,
        receipts: Vec<Receipt>,
    ) -> Result<(), StoreError> {
        let mut batch_ops = Vec::new();

        for (index, receipt) in receipts.into_iter().enumerate() {
            let key = (block_hash, index as u64).encode_to_vec();
            let value = receipt.encode_to_vec();
            batch_ops.push((CF_RECEIPTS.to_string(), key, value));
        }

        self.write_batch_async(batch_ops).await
    }

    // TODO: Check differences with libmdbx
    async fn get_receipt(
        &self,
        block_hash: BlockHash,
        index: Index,
    ) -> Result<Option<Receipt>, StoreError> {
        let key = (block_hash, index).encode_to_vec();

        self.read_async(CF_RECEIPTS, key)
            .await?
            .map(|bytes| Receipt::decode(bytes.as_slice()))
            .transpose()
            .map_err(StoreError::from)
    }

    async fn add_account_code(&self, code_hash: H256, code: Bytes) -> Result<(), StoreError> {
        let hash_key = code_hash.as_bytes().to_vec();
        let code_value = AccountCodeRLP::from(code).as_ref().to_vec();
        self.write_async(CF_ACCOUNT_CODES, hash_key, code_value)
            .await
    }

    async fn clear_snap_state(&self) -> Result<(), StoreError> {
        let db = self.db.clone();

        tokio::task::spawn_blocking(move || {
            let cf = db
                .cf_handle(CF_SNAP_STATE)
                .ok_or_else(|| StoreError::Custom("Column family not found".to_string()))?;

            let mut iter = db.iterator_cf(&cf, rocksdb::IteratorMode::Start);
            let mut batch = WriteBatchWithTransaction::<true>::default();

            while let Some(Ok((key, _))) = iter.next() {
                batch.delete_cf(&cf, key);
            }

            db.write(batch).map_err(StoreError::from)
        })
        .await
        .map_err(|e| StoreError::Custom(format!("Task panicked: {e}")))?
    }

    fn get_account_code(&self, code_hash: H256) -> Result<Option<Bytes>, StoreError> {
        let hash_key = code_hash.as_bytes();
        self.read_sync(CF_ACCOUNT_CODES, hash_key)?
            .map(|bytes| AccountCodeRLP::from_bytes(bytes).to())
            .transpose()
            .map_err(StoreError::from)
    }

    async fn get_transaction_by_hash(
        &self,
        transaction_hash: H256,
    ) -> Result<Option<Transaction>, StoreError> {
        info!(
            "[TRANSACTION BY HASH] Transaction hash: {:?}",
            transaction_hash
        );
        let (_block_number, block_hash, index) =
            match self.get_transaction_location(transaction_hash).await? {
                Some(location) => location,
                None => return Ok(None),
            };
        self.get_transaction_by_location(block_hash, index).await
    }

    async fn get_transaction_by_location(
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

    async fn get_block_by_hash(&self, block_hash: BlockHash) -> Result<Option<Block>, StoreError> {
        let Some(header) = self.get_block_header_by_hash(block_hash)? else {
            return Ok(None);
        };
        let Some(body) = self.get_block_body_by_hash(block_hash).await? else {
            return Ok(None);
        };
        Ok(Some(Block::new(header, body)))
    }

    async fn get_canonical_block_hash(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<BlockHash>, StoreError> {
        let number_key = block_number.to_le_bytes();

        self.read_async(CF_CANONICAL_BLOCK_HASHES, number_key)
            .await?
            .map(|bytes| BlockHashRLP::from_bytes(bytes).to())
            .transpose()
            .map_err(StoreError::from)
    }

    async fn set_chain_config(&self, chain_config: &ChainConfig) -> Result<(), StoreError> {
        let key = Self::chain_data_key(ChainDataIndex::ChainConfig);
        let value = serde_json::to_string(chain_config)
            .map_err(|_| StoreError::Custom("Failed to serialize chain config".to_string()))?
            .into_bytes();
        self.write_async(CF_CHAIN_DATA, key, value).await
    }

    async fn update_earliest_block_number(
        &self,
        block_number: BlockNumber,
    ) -> Result<(), StoreError> {
        let key = Self::chain_data_key(ChainDataIndex::EarliestBlockNumber);
        let value = block_number.to_le_bytes();
        self.write_async(CF_CHAIN_DATA, key, value).await
    }

    async fn get_earliest_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        let key = Self::chain_data_key(ChainDataIndex::EarliestBlockNumber);

        self.read_async(CF_CHAIN_DATA, key)
            .await?
            .map(|bytes| -> Result<BlockNumber, StoreError> {
                let array: [u8; 8] = bytes
                    .try_into()
                    .map_err(|_| StoreError::Custom("Invalid BlockNumber bytes".to_string()))?;
                Ok(BlockNumber::from_le_bytes(array))
            })
            .transpose()
    }

    async fn get_finalized_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        let key = Self::chain_data_key(ChainDataIndex::FinalizedBlockNumber);

        self.read_async(CF_CHAIN_DATA, key)
            .await?
            .map(|bytes| -> Result<BlockNumber, StoreError> {
                let array: [u8; 8] = bytes
                    .try_into()
                    .map_err(|_| StoreError::Custom("Invalid BlockNumber bytes".to_string()))?;
                Ok(BlockNumber::from_le_bytes(array))
            })
            .transpose()
    }

    async fn get_safe_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        let key = Self::chain_data_key(ChainDataIndex::SafeBlockNumber);

        self.read_async(CF_CHAIN_DATA, key)
            .await?
            .map(|bytes| -> Result<BlockNumber, StoreError> {
                let array: [u8; 8] = bytes
                    .try_into()
                    .map_err(|_| StoreError::Custom("Invalid BlockNumber bytes".to_string()))?;
                Ok(BlockNumber::from_le_bytes(array))
            })
            .transpose()
    }

    async fn get_latest_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        let key = Self::chain_data_key(ChainDataIndex::LatestBlockNumber);

        self.read_async(CF_CHAIN_DATA, key)
            .await?
            .map(|bytes| -> Result<BlockNumber, StoreError> {
                let array: [u8; 8] = bytes
                    .try_into()
                    .map_err(|_| StoreError::Custom("Invalid BlockNumber bytes".to_string()))?;
                Ok(BlockNumber::from_le_bytes(array))
            })
            .transpose()
    }

    async fn update_pending_block_number(
        &self,
        block_number: BlockNumber,
    ) -> Result<(), StoreError> {
        let key = Self::chain_data_key(ChainDataIndex::PendingBlockNumber);
        let value = block_number.to_le_bytes();
        self.write_async(CF_CHAIN_DATA, key, value).await
    }

    async fn get_pending_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        let key = Self::chain_data_key(ChainDataIndex::PendingBlockNumber);

        self.read_async(CF_CHAIN_DATA, key)
            .await?
            .map(|bytes| -> Result<BlockNumber, StoreError> {
                let array: [u8; 8] = bytes
                    .try_into()
                    .map_err(|_| StoreError::Custom("Invalid BlockNumber bytes".to_string()))?;
                Ok(BlockNumber::from_le_bytes(array))
            })
            .transpose()
    }

    fn open_storage_trie(
        &self,
        hashed_address: H256,
        storage_root: H256,
    ) -> Result<Trie, StoreError> {
        let db = Box::new(RocksDBTrieDB::new(
            self.db.clone(),
            CF_STORAGE_TRIES_NODES,
            Some(hashed_address),
        )?);
        Ok(Trie::open(db, storage_root))
    }

    fn open_state_trie(&self, state_root: H256) -> Result<Trie, StoreError> {
        let db = Box::new(RocksDBTrieDB::new(
            self.db.clone(),
            CF_STATE_TRIE_NODES,
            None,
        )?);
        Ok(Trie::open(db, state_root))
    }

    fn open_locked_state_trie(&self, state_root: H256) -> Result<Trie, StoreError> {
        let db = RocksDBLockedTrieDB::new(self.db.clone(), CF_STATE_TRIE_NODES, None)?;
        Ok(Trie::open(Box::new(db), state_root))
    }

    fn open_locked_storage_trie(
        &self,
        hashed_address: H256,
        storage_root: H256,
    ) -> Result<Trie, StoreError> {
        let db = RocksDBLockedTrieDB::new(
            self.db.clone(),
            CF_STORAGE_TRIES_NODES,
            Some(hashed_address),
        )?;
        Ok(Trie::open(Box::new(db), storage_root))
    }

    async fn forkchoice_update(
        &self,
        new_canonical_blocks: Option<Vec<(BlockNumber, BlockHash)>>,
        head_number: BlockNumber,
        head_hash: BlockHash,
        safe: Option<BlockNumber>,
        finalized: Option<BlockNumber>,
    ) -> Result<(), StoreError> {
        // Get current latest block number to know what to clean up
        let latest = self.get_latest_block_number().await?.unwrap_or(0);
        let db = self.db.clone();

        tokio::task::spawn_blocking(move || {
            let mut batch = WriteBatchWithTransaction::<true>::default();

            let [cf_canonical, cf_chain_data] =
                open_cfs(&db, [CF_CANONICAL_BLOCK_HASHES, CF_CHAIN_DATA])?;

            // Update canonical block hashes
            if let Some(canonical_blocks) = new_canonical_blocks {
                for (block_number, block_hash) in canonical_blocks {
                    let number_key = block_number.to_le_bytes();
                    let hash_value = BlockHashRLP::from(block_hash);
                    batch.put_cf(&cf_canonical, number_key, &hash_value);
                }
            }

            // Remove anything after the head from the canonical chain
            for number in (head_number + 1)..=(latest) {
                batch.delete_cf(&cf_canonical, number.to_le_bytes());
            }

            // Make head canonical
            let head_key = head_number.to_le_bytes();
            let head_value = BlockHashRLP::from(head_hash);
            batch.put_cf(&cf_canonical, head_key, &head_value);

            // Update chain data

            let latest_key = Self::chain_data_key(ChainDataIndex::LatestBlockNumber);
            batch.put_cf(&cf_chain_data, latest_key, head_number.to_le_bytes());

            if let Some(safe_number) = safe {
                let safe_key = Self::chain_data_key(ChainDataIndex::SafeBlockNumber);
                batch.put_cf(&cf_chain_data, safe_key, safe_number.to_le_bytes());
            }

            if let Some(finalized_number) = finalized {
                let finalized_key = Self::chain_data_key(ChainDataIndex::FinalizedBlockNumber);
                batch.put_cf(
                    &cf_chain_data,
                    finalized_key,
                    finalized_number.to_le_bytes(),
                );
            }

            db.write(batch).map_err(StoreError::from)
        })
        .await
        .map_err(|e| StoreError::Custom(format!("Task panicked: {}", e)))?
    }

    // TODO: REVIEW LOGIC AGAINST LIBMDBX
    fn get_receipts_for_block(&self, block_hash: &BlockHash) -> Result<Vec<Receipt>, StoreError> {
        let cf = self
            .db
            .cf_handle(CF_RECEIPTS)
            .ok_or(StoreError::Custom(format!(
                "Column family not found: {}",
                CF_RECEIPTS
            )))?;
        let mut receipts = Vec::new();
        let mut index = 0u64;

        loop {
            let key = (*block_hash, index).encode_to_vec();
            let Some(receipt_bytes) = self.db.get_cf(&cf, key)? else {
                break;
            };

            let receipt = Receipt::decode(receipt_bytes.as_slice())?;
            receipts.push(receipt);
            index += 1;
        }

        Ok(receipts)
    }

    async fn set_header_download_checkpoint(
        &self,
        block_hash: BlockHash,
    ) -> Result<(), StoreError> {
        let key = Self::snap_state_key(SnapStateIndex::HeaderDownloadCheckpoint);
        let value = BlockHashRLP::from(block_hash).as_ref().to_vec();
        self.write_async(CF_SNAP_STATE, key, value).await
    }

    async fn get_header_download_checkpoint(&self) -> Result<Option<BlockHash>, StoreError> {
        let key = Self::snap_state_key(SnapStateIndex::HeaderDownloadCheckpoint);

        self.read_async(CF_SNAP_STATE, key)
            .await?
            .map(|bytes| BlockHashRLP::from_bytes(bytes).to())
            .transpose()
            .map_err(StoreError::from)
    }

    async fn set_state_trie_key_checkpoint(
        &self,
        last_keys: [H256; STATE_TRIE_SEGMENTS],
    ) -> Result<(), StoreError> {
        let key = Self::snap_state_key(SnapStateIndex::StateTrieKeyCheckpoint);
        let value = last_keys.to_vec().encode_to_vec();
        self.write_async(CF_SNAP_STATE, key, value).await
    }

    async fn get_state_trie_key_checkpoint(
        &self,
    ) -> Result<Option<[H256; STATE_TRIE_SEGMENTS]>, StoreError> {
        let key = Self::snap_state_key(SnapStateIndex::StateTrieKeyCheckpoint);

        match self.read_async(CF_SNAP_STATE, key).await? {
            Some(keys_bytes) => {
                let keys_vec: Vec<H256> = Vec::<H256>::decode(keys_bytes.as_slice())?;
                if keys_vec.len() == STATE_TRIE_SEGMENTS {
                    let mut keys_array = [H256::zero(); STATE_TRIE_SEGMENTS];
                    keys_array.copy_from_slice(&keys_vec);
                    Ok(Some(keys_array))
                } else {
                    Err(StoreError::Custom("Invalid array size".to_string()))
                }
            }
            None => Ok(None),
        }
    }

    async fn set_state_heal_paths(&self, paths: Vec<(Nibbles, H256)>) -> Result<(), StoreError> {
        let key = Self::snap_state_key(SnapStateIndex::StateHealPaths);
        let value = paths.encode_to_vec();
        self.write_async(CF_SNAP_STATE, key, value).await
    }

    async fn get_state_heal_paths(&self) -> Result<Option<Vec<(Nibbles, H256)>>, StoreError> {
        let key = Self::snap_state_key(SnapStateIndex::StateHealPaths);

        self.read_async(CF_SNAP_STATE, key)
            .await?
            .map(|bytes| Vec::<(Nibbles, H256)>::decode(bytes.as_slice()))
            .transpose()
            .map_err(StoreError::from)
    }

    async fn write_snapshot_storage_batch(
        &self,
        account_hash: H256,
        storage_keys: Vec<H256>,
        storage_values: Vec<U256>,
    ) -> Result<(), StoreError> {
        if storage_keys.len() != storage_values.len() {
            return Err(StoreError::Custom(
                "Storage keys and values length mismatch".to_string(),
            ));
        }

        let mut batch_ops = Vec::new();

        for (key, value) in storage_keys.into_iter().zip(storage_values.into_iter()) {
            // Create composite key: account_hash + storage_key
            let mut composite_key = Vec::with_capacity(64);
            composite_key.extend_from_slice(account_hash.as_bytes());
            composite_key.extend_from_slice(key.as_bytes());

            // Convert U256 to bytes
            let value_bytes = u256_to_big_endian(value).to_vec();

            batch_ops.push((CF_STORAGE_SNAPSHOT.to_string(), composite_key, value_bytes));
        }

        self.write_batch_async(batch_ops).await
    }

    async fn write_snapshot_storage_batches(
        &self,
        account_hashes: Vec<H256>,
        storage_keys: Vec<Vec<H256>>,
        storage_values: Vec<Vec<U256>>,
    ) -> Result<(), StoreError> {
        if account_hashes.len() != storage_keys.len()
            || account_hashes.len() != storage_values.len()
        {
            return Err(StoreError::Custom(
                "Account hashes, keys, and values length mismatch".to_string(),
            ));
        }

        let mut batch_ops = Vec::new();

        for ((account_hash, keys), values) in account_hashes
            .into_iter()
            .zip(storage_keys.into_iter())
            .zip(storage_values.into_iter())
        {
            if keys.len() != values.len() {
                return Err(StoreError::Custom(
                    "Storage keys and values length mismatch for account".to_string(),
                ));
            }

            for (key, value) in keys.into_iter().zip(values.into_iter()) {
                // Create composite key: account_hash + storage_key
                let mut composite_key = Vec::with_capacity(64);
                composite_key.extend_from_slice(account_hash.as_bytes());
                composite_key.extend_from_slice(key.as_bytes());

                // Convert U256 to bytes
                let value_bytes = u256_to_big_endian(value).to_vec();

                batch_ops.push((CF_STORAGE_SNAPSHOT.to_string(), composite_key, value_bytes));
            }
        }

        self.write_batch_async(batch_ops).await
    }

    async fn set_state_trie_rebuild_checkpoint(
        &self,
        checkpoint: (H256, [H256; STATE_TRIE_SEGMENTS]),
    ) -> Result<(), StoreError> {
        let key = Self::snap_state_key(SnapStateIndex::StateTrieRebuildCheckpoint);
        let value = (checkpoint.0, checkpoint.1.to_vec()).encode_to_vec();
        self.write_async(CF_SNAP_STATE, key, value).await
    }

    async fn get_state_trie_rebuild_checkpoint(
        &self,
    ) -> Result<Option<(H256, [H256; STATE_TRIE_SEGMENTS])>, StoreError> {
        let key = Self::snap_state_key(SnapStateIndex::StateTrieRebuildCheckpoint);

        match self.read_async(CF_SNAP_STATE, key).await? {
            Some(checkpoint_bytes) => {
                let (root, keys_vec): (H256, Vec<H256>) =
                    <(H256, Vec<H256>)>::decode(checkpoint_bytes.as_slice())?;
                if keys_vec.len() == STATE_TRIE_SEGMENTS {
                    let mut keys_array = [H256::zero(); STATE_TRIE_SEGMENTS];
                    keys_array.copy_from_slice(&keys_vec);
                    Ok(Some((root, keys_array)))
                } else {
                    Err(StoreError::Custom("Invalid array size".to_string()))
                }
            }
            None => Ok(None),
        }
    }

    async fn set_storage_trie_rebuild_pending(
        &self,
        pending: Vec<(H256, H256)>,
    ) -> Result<(), StoreError> {
        let key = Self::snap_state_key(SnapStateIndex::StorageTrieRebuildPending);
        let value = pending.encode_to_vec();
        self.write_async(CF_SNAP_STATE, key, value).await
    }

    async fn get_storage_trie_rebuild_pending(
        &self,
    ) -> Result<Option<Vec<(H256, H256)>>, StoreError> {
        let key = Self::snap_state_key(SnapStateIndex::StorageTrieRebuildPending);

        self.read_async(CF_SNAP_STATE, key)
            .await?
            .map(|bytes| Vec::<(H256, H256)>::decode(bytes.as_slice()))
            .transpose()
            .map_err(StoreError::from)
    }

    // TODO: REVIEW LOGIC AGAINST LIBMDBX
    async fn read_storage_snapshot(
        &self,
        account_hash: H256,
        start: H256,
    ) -> Result<Vec<(H256, U256)>, StoreError> {
        let db = self.db.clone();
        let max_reads = crate::store::MAX_SNAPSHOT_READS;

        tokio::task::spawn_blocking(move || {
            let cf = db
                .cf_handle(CF_STORAGE_SNAPSHOT)
                .ok_or_else(|| StoreError::Custom("Column family not found".to_string()))?;

            // Create start key: account_hash + start
            let mut start_key = Vec::with_capacity(64);
            start_key.extend_from_slice(account_hash.as_bytes());
            start_key.extend_from_slice(start.as_bytes());

            let mut results = Vec::new();
            let mut iter = db.iterator_cf(
                &cf,
                rocksdb::IteratorMode::From(&start_key, rocksdb::Direction::Forward),
            );
            let mut count = 0;

            while let Some(Ok((key, value))) = iter.next() {
                if count >= max_reads {
                    break;
                }

                // Check if key still belongs to the same account
                if key.len() >= 32 && &key[0..32] == account_hash.as_bytes() {
                    // Extract storage key (last 32 bytes)
                    if key.len() >= 64 {
                        let storage_key = H256::from_slice(&key[32..64]);
                        let storage_value = U256::from_big_endian(&value);
                        results.push((storage_key, storage_value));
                        count += 1;
                    }
                } else {
                    // We've moved to a different account, stop
                    break;
                }
            }

            Ok(results)
        })
        .await
        .map_err(|e| StoreError::Custom(format!("Task panicked: {e}")))?
    }

    async fn set_latest_valid_ancestor(
        &self,
        bad_block: BlockHash,
        latest_valid: BlockHash,
    ) -> Result<(), StoreError> {
        let key = BlockHashRLP::from(bad_block).as_ref().to_vec();
        let value = BlockHashRLP::from(latest_valid).as_ref().to_vec();
        self.write_async(CF_INVALID_ANCESTORS, key, value).await
    }

    async fn get_latest_valid_ancestor(
        &self,
        block: BlockHash,
    ) -> Result<Option<BlockHash>, StoreError> {
        let key = BlockHashRLP::from(block).as_ref().to_vec();

        self.read_async(CF_INVALID_ANCESTORS, key)
            .await?
            .map(|bytes| BlockHashRLP::from_bytes(bytes).to())
            .transpose()
            .map_err(StoreError::from)
    }

    fn get_block_number_sync(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockNumber>, StoreError> {
        let hash_key = BlockHashRLP::from(block_hash).as_ref().to_vec();

        self.read_sync(CF_BLOCK_NUMBERS, hash_key)?
            .map(|bytes| -> Result<BlockNumber, StoreError> {
                let array: [u8; 8] = bytes
                    .try_into()
                    .map_err(|_| StoreError::Custom("Invalid BlockNumber bytes".to_string()))?;
                Ok(BlockNumber::from_le_bytes(array))
            })
            .transpose()
    }

    fn get_canonical_block_hash_sync(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<BlockHash>, StoreError> {
        let number_key = block_number.to_le_bytes();

        self.read_sync(CF_CANONICAL_BLOCK_HASHES, number_key)?
            .map(|bytes| BlockHashRLP::from_bytes(bytes).to())
            .transpose()
            .map_err(StoreError::from)
    }

    async fn write_storage_trie_nodes_batch(
        &self,
        storage_trie_nodes: Vec<(H256, Vec<(NodeHash, Vec<u8>)>)>,
    ) -> Result<(), StoreError> {
        let mut batch_ops = Vec::new();

        for (address_hash, nodes) in storage_trie_nodes {
            for (node_hash, node_data) in nodes {
                // Create composite key: address_hash + node_hash
                let mut key = Vec::with_capacity(64);
                key.extend_from_slice(address_hash.as_bytes());
                key.extend_from_slice(node_hash.as_ref());
                batch_ops.push((CF_STORAGE_TRIES_NODES.to_string(), key, node_data));
            }
        }

        self.write_batch_async(batch_ops).await
    }

    async fn write_account_code_batch(
        &self,
        account_codes: Vec<(H256, Bytes)>,
    ) -> Result<(), StoreError> {
        let mut batch_ops = Vec::new();

        for (code_hash, code) in account_codes {
            let key = code_hash.as_bytes().to_vec();
            let value = AccountCodeRLP::from(code).as_ref().to_vec();
            batch_ops.push((CF_ACCOUNT_CODES.to_string(), key, value));
        }

        self.write_batch_async(batch_ops).await
    }
}

/// Open column families
fn open_cfs<'a, const N: usize>(
    db: &'a Arc<OptimisticTransactionDB<MultiThreaded>>,
    names: [&str; N],
) -> Result<[Arc<BoundColumnFamily<'a>>; N], StoreError> {
    let mut handles = Vec::with_capacity(N);

    for name in names {
        handles
            .push(db.cf_handle(name).ok_or_else(|| {
                StoreError::Custom(format!("Column family '{}' not found", name))
            })?);
    }

    handles
        .try_into()
        .map_err(|_| StoreError::Custom("Unexpected number of column families".to_string()))
}
