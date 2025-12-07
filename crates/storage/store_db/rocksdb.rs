use crate::{
    STORE_SCHEMA_VERSION,
    rlp::AccountCodeHashRLP,
    trie_db::{
        layering::{TrieLayerCache, TrieWrapper, apply_prefix},
        rocksdb_locked::RocksDBLockedTrieDB,
    },
};
use bytes::Bytes;
use ethrex_common::{
    H256,
    types::{
        AccountState, Block, BlockBody, BlockHash, BlockHeader, BlockNumber, ChainConfig, Code,
        Index, Receipt, Transaction,
    },
};
use ethrex_trie::{Nibbles, Node, Trie};
use lru::LruCache;
use rocksdb::{
    BlockBasedOptions, BoundColumnFamily, ColumnFamilyDescriptor, DBWithThreadMode, MultiThreaded,
    Options, WriteBatch, checkpoint::Checkpoint,
};
use rustc_hash::FxBuildHasher;
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::AtomicU64,
        mpsc::{SyncSender, sync_channel},
    },
};
use tracing::{debug, error, info};

use crate::{
    STATE_TRIE_SEGMENTS, UpdateBatch,
    api::StoreEngine,
    error::StoreError,
    rlp::{BlockBodyRLP, BlockHashRLP, BlockHeaderRLP, BlockRLP},
    trie_db::rocksdb::RocksDBTrieDB,
    utils::{ChainDataIndex, SnapStateIndex},
};
use ethrex_rlp::{
    decode::{RLPDecode, decode_bytes},
    encode::RLPEncode,
};
use std::fmt::Debug;

// TODO: use finalized hash to determine when to commit
const COMMIT_THRESHOLD: usize = 128;

/// Canonical block hashes column family: [`u8;_`] => [`Vec<u8>`]
/// - [`u8;_`] = `block_number.to_le_bytes()`
/// - [`Vec<u8>`] = `BlockHashRLP::from(block_hash).bytes().clone()`
const CF_CANONICAL_BLOCK_HASHES: &str = "canonical_block_hashes";

/// Block numbers column family: [`Vec<u8>`] => [`u8;_`]
/// - [`Vec<u8>`] = `BlockHashRLP::from(block_hash).bytes().clone()`
/// - [`u8;_`] = `block_number.to_le_bytes()`
const CF_BLOCK_NUMBERS: &str = "block_numbers";

/// Block headers column family: [`Vec<u8>`] => [`Vec<u8>`]
/// - [`Vec<u8>`] = `BlockHashRLP::from(block_hash).bytes().clone()`
/// - [`Vec<u8>`] = `BlockHeaderRLP::from(block.header.clone()).bytes().clone()`
const CF_HEADERS: &str = "headers";

/// Block bodies column family: [`Vec<u8>`] => [`Vec<u8>`]
/// - [`Vec<u8>`] = `BlockHashRLP::from(block_hash).bytes().clone();`
/// - [`Vec<u8>`] = `BlockBodyRLP::from(block.body.clone()).bytes().clone()`
const CF_BODIES: &str = "bodies";

/// Account codes column family: [`Vec<u8>`] => [`Vec<u8>`]
/// - [`Vec<u8>`] = `code_hash.as_bytes().to_vec()`
/// - [`Vec<u8>`] = `AccountCodeRLP::from(code).bytes().clone()`
const CF_ACCOUNT_CODES: &str = "account_codes";

/// Receipts column family: [`Vec<u8>`] => [`Vec<u8>`]
/// - [`Vec<u8>`] = `(block_hash, index).encode_to_vec()`
/// - [`Vec<u8>`] = `receipt.encode_to_vec()`
const CF_RECEIPTS: &str = "receipts";

/// Transaction locations column family: [`Vec<u8>`] => [`Vec<u8>`]
/// - [`Vec<u8>`] = Composite key
///    ```rust,no_run
///     // let mut composite_key = Vec::with_capacity(64);
///     // composite_key.extend_from_slice(transaction_hash.as_bytes());
///     // composite_key.extend_from_slice(block_hash.as_bytes());
///    ```
/// - [`Vec<u8>`] = `(block_number, block_hash, index).encode_to_vec()`
const CF_TRANSACTION_LOCATIONS: &str = "transaction_locations";

/// Chain data column family: [`Vec<u8>`] => [`Vec<u8>`]
/// - [`Vec<u8>`] = `Self::chain_data_key(ChainDataIndex::ChainConfig)`
/// - [`Vec<u8>`] = `serde_json::to_string(chain_config)`
const CF_CHAIN_DATA: &str = "chain_data";

/// Snap state column family: [`Vec<u8>`] => [`Vec<u8>`]
/// - [`Vec<u8>`] = `Self::snap_state_key(SnapStateIndex::HeaderDownloadCheckpoint)`
/// - [`Vec<u8>`] = `BlockHashRLP::from(block_hash).bytes().clone()`
const CF_SNAP_STATE: &str = "snap_state";

/// Account State trie nodes column family: [`Nibbles`] => [`Vec<u8>`]
/// - [`Nibbles`] = `node_hash.as_ref()`
/// - [`Vec<u8>`] = `node_data`
const CF_ACCOUNT_TRIE_NODES: &str = "account_trie_nodes";

/// Storage trie nodes column family: [`Nibbles`] => [`Vec<u8>`]
/// - [`Nibbles`] = `node_hash.as_ref()`
/// - [`Vec<u8>`] = `node_data`
const CF_STORAGE_TRIE_NODES: &str = "storage_trie_nodes";

/// Pending blocks column family: [`Vec<u8>`] => [`Vec<u8>`]
/// - [`Vec<u8>`] = `BlockHashRLP::from(block.hash()).bytes().clone()`
/// - [`Vec<u8>`] = `BlockRLP::from(block).bytes().clone()`
const CF_PENDING_BLOCKS: &str = "pending_blocks";

/// Invalid ancestors column family: [`Vec<u8>`] => [`Vec<u8>`]
/// - [`Vec<u8>`] = `BlockHashRLP::from(bad_block).bytes().clone()`
/// - [`Vec<u8>`] = `BlockHashRLP::from(latest_valid).bytes().clone()`
const CF_INVALID_ANCESTORS: &str = "invalid_ancestors";

/// Block headers downloaded during fullsync column family: [`u8;_`] => [`Vec<u8>`]
/// - [`u8;_`] = `block_number.to_le_bytes()`
/// - [`Vec<u8>`] = `BlockHeaderRLP::from(block.header.clone()).bytes().clone()`
const CF_FULLSYNC_HEADERS: &str = "fullsync_headers";

/// Account sate flat key-value store: [`Nibbles`] => [`Vec<u8>`]
/// - [`Nibbles`] = `node_hash.as_ref()`
/// - [`Vec<u8>`] = `node_data`
pub const CF_ACCOUNT_FLATKEYVALUE: &str = "account_flatkeyvalue";

/// Storage slots key-value store: [`Nibbles`] => [`Vec<u8>`]
/// - [`Nibbles`] = `node_hash.as_ref()`
/// - [`Vec<u8>`] = `node_data`
pub const CF_STORAGE_FLATKEYVALUE: &str = "storage_flatkeyvalue";

pub const CF_MISC_VALUES: &str = "misc_values";

pub type StorageUpdates = Vec<(H256, Vec<(Nibbles, Vec<u8>)>)>;

pub type TriedUpdateWorkerTx = std::sync::mpsc::SyncSender<(
    std::sync::mpsc::SyncSender<Result<(), StoreError>>,
    H256,
    H256,
    Vec<(Nibbles, Vec<u8>)>,
    Vec<(H256, Vec<(Nibbles, Vec<u8>)>)>,
)>;

/// Control messages for the FlatKeyValue generator
#[derive(Debug, PartialEq)]
enum FKVGeneratorControlMessage {
    Stop,
    Continue,
}

// 64mb
const CODE_CACHE_MAX_SIZE: u64 = 64 * 1024 * 1024;
type CodeCache = LruCache<H256, Code, FxBuildHasher>;

#[derive(Debug)]
pub struct Store {
    db: Arc<DBWithThreadMode<MultiThreaded>>,
    trie_cache: Arc<Mutex<Arc<TrieLayerCache>>>,
    flatkeyvalue_control_tx: std::sync::mpsc::SyncSender<FKVGeneratorControlMessage>,
    trie_update_worker_tx: TriedUpdateWorkerTx,
    last_computed_flatkeyvalue: Arc<Mutex<Vec<u8>>>,
    /// Cache for account bytecodes, keyed by the bytecode hash.
    /// Note that we don't remove entries on account code changes, since
    /// those changes already affect the code hash stored in the account, and only
    /// may result in this cache having useless data.
    account_code_cache: Arc<Mutex<CodeCache>>,
    account_code_cache_size: AtomicU64,
}

impl Clone for Store {
    fn clone(&self) -> Self {
        Self {
            db: self.db.clone(),
            trie_cache: self.trie_cache.clone(),
            flatkeyvalue_control_tx: self.flatkeyvalue_control_tx.clone(),
            trie_update_worker_tx: self.trie_update_worker_tx.clone(),
            last_computed_flatkeyvalue: self.last_computed_flatkeyvalue.clone(),
            account_code_cache: self.account_code_cache.clone(),
            account_code_cache_size: AtomicU64::new(
                self.account_code_cache_size
                    .load(std::sync::atomic::Ordering::Relaxed),
            ),
        }
    }
}

impl Store {
    pub fn new(path: &Path) -> Result<Self, StoreError> {
        let mut db_options = Options::default();
        db_options.create_if_missing(true);
        db_options.create_missing_column_families(true);

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
        db_options.set_wal_bytes_per_sync(32 * 1024 * 1024); // 32MB
        db_options.set_bytes_per_sync(32 * 1024 * 1024); // 32MB
        db_options.set_use_fsync(false); // fdatasync

        db_options.set_enable_pipelined_write(true);
        db_options.set_allow_concurrent_memtable_write(true);
        db_options.set_enable_write_thread_adaptive_yield(true);
        db_options.set_compaction_readahead_size(4 * 1024 * 1024); // 4MB
        db_options.set_advise_random_on_open(false);
        db_options.set_compression_type(rocksdb::DBCompressionType::None);

        // db_options.enable_statistics();
        // db_options.set_stats_dump_period_sec(600);

        // Current column families that the code expects
        let expected_column_families = vec![
            CF_CANONICAL_BLOCK_HASHES,
            CF_BLOCK_NUMBERS,
            CF_HEADERS,
            CF_BODIES,
            CF_ACCOUNT_CODES,
            CF_RECEIPTS,
            CF_TRANSACTION_LOCATIONS,
            CF_CHAIN_DATA,
            CF_SNAP_STATE,
            CF_ACCOUNT_TRIE_NODES,
            CF_STORAGE_TRIE_NODES,
            CF_PENDING_BLOCKS,
            CF_INVALID_ANCESTORS,
            CF_FULLSYNC_HEADERS,
            CF_ACCOUNT_FLATKEYVALUE,
            CF_STORAGE_FLATKEYVALUE,
            CF_MISC_VALUES,
        ];
        let compressible_cfs = [
            CF_BLOCK_NUMBERS,
            CF_HEADERS,
            CF_BODIES,
            CF_RECEIPTS,
            CF_TRANSACTION_LOCATIONS,
            CF_FULLSYNC_HEADERS,
        ];

        // Get existing column families to know which ones to drop later
        let existing_cfs = match DBWithThreadMode::<MultiThreaded>::list_cf(&db_options, path) {
            Ok(cfs) => {
                info!("Found existing column families: {:?}", cfs);
                cfs
            }
            Err(_) => {
                // Database doesn't exist yet
                info!("Database doesn't exist, will create with expected column families");
                vec!["default".to_string()]
            }
        };

        // Create descriptors for ALL existing CFs + expected ones (RocksDB requires opening all existing CFs)
        let mut all_cfs_to_open = HashSet::new();

        // Add all expected CFs
        for cf in &expected_column_families {
            all_cfs_to_open.insert(cf.to_string());
        }

        // Add all existing CFs (we must open them to be able to drop obsolete ones later)
        for cf in &existing_cfs {
            if cf != "default" {
                // default is handled automatically
                all_cfs_to_open.insert(cf.clone());
            }
        }

        let mut cf_descriptors = Vec::new();
        for cf_name in &all_cfs_to_open {
            let mut cf_opts = Options::default();

            cf_opts.set_level_zero_file_num_compaction_trigger(4);
            cf_opts.set_level_zero_slowdown_writes_trigger(20);
            cf_opts.set_level_zero_stop_writes_trigger(36);

            if compressible_cfs.contains(&cf_name.as_str()) {
                cf_opts.set_compression_type(rocksdb::DBCompressionType::Lz4);
            } else {
                cf_opts.set_compression_type(rocksdb::DBCompressionType::None);
            }

            match cf_name.as_str() {
                CF_HEADERS | CF_BODIES => {
                    cf_opts.set_write_buffer_size(128 * 1024 * 1024); // 128MB
                    cf_opts.set_max_write_buffer_number(4);
                    cf_opts.set_target_file_size_base(256 * 1024 * 1024); // 256MB

                    let mut block_opts = BlockBasedOptions::default();
                    block_opts.set_block_size(32 * 1024); // 32KB blocks
                    cf_opts.set_block_based_table_factory(&block_opts);
                }
                CF_CANONICAL_BLOCK_HASHES | CF_BLOCK_NUMBERS => {
                    cf_opts.set_write_buffer_size(64 * 1024 * 1024); // 64MB
                    cf_opts.set_max_write_buffer_number(3);
                    cf_opts.set_target_file_size_base(128 * 1024 * 1024); // 128MB

                    let mut block_opts = BlockBasedOptions::default();
                    block_opts.set_block_size(16 * 1024); // 16KB
                    block_opts.set_bloom_filter(10.0, false);
                    cf_opts.set_block_based_table_factory(&block_opts);
                }
                CF_ACCOUNT_TRIE_NODES | CF_STORAGE_TRIE_NODES => {
                    cf_opts.set_write_buffer_size(512 * 1024 * 1024); // 512MB
                    cf_opts.set_max_write_buffer_number(6);
                    cf_opts.set_min_write_buffer_number_to_merge(2);
                    cf_opts.set_target_file_size_base(256 * 1024 * 1024); // 256MB
                    cf_opts.set_memtable_prefix_bloom_ratio(0.2); // Bloom filter

                    let mut block_opts = BlockBasedOptions::default();
                    block_opts.set_block_size(16 * 1024); // 16KB
                    block_opts.set_bloom_filter(10.0, false); // 10 bits per key
                    cf_opts.set_block_based_table_factory(&block_opts);
                }
                CF_ACCOUNT_FLATKEYVALUE | CF_STORAGE_FLATKEYVALUE => {
                    cf_opts.set_write_buffer_size(512 * 1024 * 1024); // 512MB
                    cf_opts.set_max_write_buffer_number(6);
                    cf_opts.set_min_write_buffer_number_to_merge(2);
                    cf_opts.set_target_file_size_base(256 * 1024 * 1024); // 256MB
                    cf_opts.set_memtable_prefix_bloom_ratio(0.2); // Bloom filter

                    let mut block_opts = BlockBasedOptions::default();
                    block_opts.set_block_size(16 * 1024); // 16KB
                    block_opts.set_bloom_filter(10.0, false); // 10 bits per key
                    cf_opts.set_block_based_table_factory(&block_opts);
                }
                CF_ACCOUNT_CODES => {
                    cf_opts.set_write_buffer_size(128 * 1024 * 1024); // 128MB
                    cf_opts.set_max_write_buffer_number(3);
                    cf_opts.set_target_file_size_base(256 * 1024 * 1024); // 256MB

                    cf_opts.set_enable_blob_files(true);
                    // Small bytecodes should go inline (mainly for delegation indicators)
                    cf_opts.set_min_blob_size(32);
                    cf_opts.set_blob_compression_type(rocksdb::DBCompressionType::Lz4);

                    let mut block_opts = BlockBasedOptions::default();
                    block_opts.set_block_size(32 * 1024); // 32KB
                    cf_opts.set_block_based_table_factory(&block_opts);
                }
                CF_RECEIPTS => {
                    cf_opts.set_write_buffer_size(128 * 1024 * 1024); // 128MB
                    cf_opts.set_max_write_buffer_number(3);
                    cf_opts.set_target_file_size_base(256 * 1024 * 1024); // 256MB

                    let mut block_opts = BlockBasedOptions::default();
                    block_opts.set_block_size(32 * 1024); // 32KB
                    cf_opts.set_block_based_table_factory(&block_opts);
                }
                _ => {
                    // Default for other CFs
                    cf_opts.set_write_buffer_size(64 * 1024 * 1024); // 64MB
                    cf_opts.set_max_write_buffer_number(3);
                    cf_opts.set_target_file_size_base(128 * 1024 * 1024); // 128MB

                    let mut block_opts = BlockBasedOptions::default();
                    block_opts.set_block_size(16 * 1024);
                    cf_opts.set_block_based_table_factory(&block_opts);
                }
            }

            cf_descriptors.push(ColumnFamilyDescriptor::new(cf_name, cf_opts));
        }

        // Note: we are not using transactions on our Rocksdb instance.
        // This is safe as long as two conditions are met:
        // - We never write to the same table from two different places concurrently.
        // - We always use batch writes. This guarantees atomicity in rocksdb.
        //
        // For the first point, we know that all writes to the state and storage tries are
        // done through the `apply_updates` function, called only after block execution.
        // There is only one other place where we write to the tries, and that's during snap
        // sync, through the `write_storage_trie_nodes_batch` function (and similarly for state trie nodes);
        // this does not pose a problem because there is no block execution until snap sync is done.
        //
        // Regardless of transactionality, all writes go through a WAL, which ensures
        // we get durability (i.e. crash recovery).
        //
        // For other less crucial tables refer to the db_safety documentation.
        let db = DBWithThreadMode::<MultiThreaded>::open_cf_descriptors(
            &db_options,
            path,
            cf_descriptors,
        )
        .map_err(|e| StoreError::Custom(format!("Failed to open RocksDB: {}", e)))?;

        // Clean up obsolete column families
        for cf_name in &existing_cfs {
            if cf_name != "default" && !expected_column_families.contains(&cf_name.as_str()) {
                info!("Dropping obsolete column family: {}", cf_name);
                match db.drop_cf(cf_name) {
                    Ok(_) => info!("Successfully dropped column family: {}", cf_name),
                    Err(e) => {
                        // Log error but don't fail initialization - the database is still usable
                        tracing::warn!(
                            "Failed to drop obsolete column family '{}': {}",
                            cf_name,
                            e
                        );
                    }
                }
            }
        }
        let (fkv_tx, fkv_rx) = std::sync::mpsc::sync_channel(0);
        let (trie_upd_tx, trie_upd_rx) = std::sync::mpsc::sync_channel(0);

        let cf_misc = db
            .cf_handle(CF_MISC_VALUES)
            .ok_or_else(|| StoreError::Custom("column not found".to_string()))?;
        let mut last_written = db
            .get_cf(&cf_misc, "last_written")?
            .unwrap_or_else(|| vec![0u8; 64]);
        if last_written == vec![0xff] {
            last_written = vec![0xff; 64];
        }
        drop(cf_misc); // dropped to remove borrow on db

        let store = Self {
            db: Arc::new(db),
            trie_cache: Default::default(),
            flatkeyvalue_control_tx: fkv_tx,
            trie_update_worker_tx: trie_upd_tx,
            last_computed_flatkeyvalue: Arc::new(Mutex::new(last_written)),
            account_code_cache: Arc::new(Mutex::new(LruCache::unbounded_with_hasher(
                FxBuildHasher,
            ))),
            account_code_cache_size: AtomicU64::new(0),
        };
        let store_clone = store.clone();
        std::thread::spawn(move || {
            let rx = fkv_rx;
            loop {
                match rx.recv() {
                    Ok(FKVGeneratorControlMessage::Continue) => break,
                    Ok(FKVGeneratorControlMessage::Stop) => {}
                    Err(_) => {
                        debug!("Closing FlatKeyValue generator.");
                        return;
                    }
                }
            }
            info!("Generation of FlatKeyValue started.");
            match store_clone.flatkeyvalue_generator(&rx) {
                Ok(_) => info!("FlatKeyValue generation finished."),
                Err(err) => error!("Error while generating FlatKeyValue: {err}"),
            }
            // rx channel is dropped, closing it
        });
        let store_clone = store.clone();
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
                    Ok((
                        notify,
                        parent_state_root,
                        child_state_root,
                        account_updates,
                        storage_updates,
                    )) => {
                        // FIXME: what should we do on error?
                        let _ = store_clone
                            .apply_trie_updates(
                                notify,
                                parent_state_root,
                                child_state_root,
                                account_updates,
                                storage_updates,
                            )
                            .inspect_err(|err| error!("apply_trie_updates failed: {err}"));
                    }
                    Err(err) => error!("Error while reading diff layer: {err}"),
                }
            }
        });
        Ok(store)
    }

    // Helper method to get column family handle
    fn cf_handle(
        &self,
        cf_name: &str,
    ) -> Result<std::sync::Arc<BoundColumnFamily<'_>>, StoreError> {
        self.db
            .cf_handle(cf_name)
            .ok_or_else(|| StoreError::Custom(format!("Column family not found: {}", cf_name)))
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
            let cf = db.cf_handle(&cf_name).ok_or_else(|| {
                StoreError::Custom(format!("Column family not found: {}", cf_name))
            })?;
            db.put_cf(&cf, key, value)
                .map_err(|e| StoreError::Custom(format!("RocksDB write error: {}", e)))
        })
        .await
        .map_err(|e| StoreError::Custom(format!("Task panicked: {}", e)))?
    }

    // Helper method for sync writes
    fn write_sync<K, V>(&self, cf_name: &str, key: K, value: V) -> Result<(), StoreError>
    where
        K: AsRef<[u8]> + Send + 'static,
        V: AsRef<[u8]> + Send + 'static,
    {
        let cf = self.cf_handle(cf_name)?;
        self.db
            .put_cf(&cf, key, value)
            .map_err(|e| StoreError::Custom(format!("RocksDB write error: {}", e)))
    }

    // Helper method for async reads
    async fn read_async<K>(&self, cf_name: &str, key: K) -> Result<Option<Vec<u8>>, StoreError>
    where
        K: AsRef<[u8]> + Send + 'static,
    {
        let db = self.db.clone();
        let cf_name = cf_name.to_string();

        tokio::task::spawn_blocking(move || {
            let cf = db.cf_handle(&cf_name).ok_or_else(|| {
                StoreError::Custom(format!("Column family not found: {}", cf_name))
            })?;
            db.get_cf(&cf, key)
                .map_err(|e| StoreError::Custom(format!("RocksDB read error: {}", e)))
        })
        .await
        .map_err(|e| StoreError::Custom(format!("Task panicked: {}", e)))?
    }

    // Helper method for sync reads
    fn read_sync<K>(&self, cf_name: &str, key: K) -> Result<Option<Vec<u8>>, StoreError>
    where
        K: AsRef<[u8]>,
    {
        let cf = self.cf_handle(cf_name)?;
        self.db
            .get_cf(&cf, key)
            .map_err(|e| StoreError::Custom(format!("RocksDB read error: {}", e)))
    }

    // Helper method for batch writes
    async fn write_batch_async(
        &self,
        batch_ops: Vec<(String, Vec<u8>, Vec<u8>)>,
    ) -> Result<(), StoreError> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            let mut batch = WriteBatch::default();

            for (cf_name, key, value) in batch_ops {
                let cf = db.cf_handle(&cf_name).ok_or_else(|| {
                    StoreError::Custom(format!("Column family not found: {}", cf_name))
                })?;
                batch.put_cf(&cf, key, value);
            }

            db.write(batch)
                .map_err(|e| StoreError::Custom(format!("RocksDB batch write error: {}", e)))
        })
        .await
        .map_err(|e| StoreError::Custom(format!("Task panicked: {}", e)))?
    }

    // Helper method to encode ChainDataIndex as key
    fn chain_data_key(index: ChainDataIndex) -> Vec<u8> {
        (index as u8).encode_to_vec()
    }

    // Helper method to encode SnapStateIndex as key
    fn snap_state_key(index: SnapStateIndex) -> Vec<u8> {
        (index as u8).encode_to_vec()
    }

    // Helper method for bulk reads
    async fn read_bulk_async<K, V, F>(
        &self,
        cf_name: &str,
        keys: Vec<K>,
        deserialize_fn: F,
    ) -> Result<Vec<Option<V>>, StoreError>
    where
        K: AsRef<[u8]> + Send + 'static,
        V: Send + 'static,
        F: Fn(Vec<u8>) -> Result<V, StoreError> + Send + 'static,
    {
        let db = self.db.clone();
        let cf_name = cf_name.to_string();

        tokio::task::spawn_blocking(move || {
            let cf = db.cf_handle(&cf_name).ok_or_else(|| {
                StoreError::Custom(format!("Column family not found: {}", cf_name))
            })?;

            let mut results = Vec::with_capacity(keys.len());

            for key in keys {
                match db.get_cf(&cf, key)? {
                    Some(bytes) => {
                        let value = deserialize_fn(bytes)?;
                        results.push(Some(value));
                    }
                    None => {
                        results.push(None);
                    }
                }
            }

            Ok(results)
        })
        .await
        .map_err(|e| StoreError::Custom(format!("Task panicked: {}", e)))?
    }

    fn flatkeyvalue_generator(
        &self,
        control_rx: &std::sync::mpsc::Receiver<FKVGeneratorControlMessage>,
    ) -> Result<(), StoreError> {
        let cf_misc = self.cf_handle(CF_MISC_VALUES)?;
        let cf_accounts_fkv = self.cf_handle(CF_ACCOUNT_FLATKEYVALUE)?;
        let cf_storage_fkv = self.cf_handle(CF_STORAGE_FLATKEYVALUE)?;

        let last_written = self
            .db
            .get_cf(&cf_misc, "last_written")?
            .unwrap_or_default();
        if last_written == vec![0xff] {
            return Ok(());
        }

        self.db
            .delete_range_cf(&cf_accounts_fkv, &last_written, vec![0xff].as_ref())?;
        self.db
            .delete_range_cf(&cf_storage_fkv, &last_written, vec![0xff].as_ref())?;

        loop {
            let root = self
                .read_sync(CF_ACCOUNT_TRIE_NODES, [])?
                .ok_or(StoreError::MissingLatestBlockNumber)?;
            let root: Node = ethrex_trie::Node::decode(&root)?;
            let state_root = root.compute_hash().finalize();

            let last_written = self
                .db
                .get_cf(&cf_misc, "last_written")?
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
            let mut batch = WriteBatch::default();
            let mut account_iter = self.open_direct_state_trie(state_root)?.into_iter();
            if last_written_account > Nibbles::default() {
                account_iter.advance(last_written_account.to_bytes())?;
            }
            let res = account_iter.try_for_each(|(path, account_node)| -> Result<(), StoreError> {
                let Node::Leaf(node) = account_node else {
                    return Ok(());
                };
                let account_state = AccountState::decode(&node.value)?;
                let account_hash = H256::from_slice(&path.to_bytes());
                batch.put_cf(&cf_misc, "last_written", path.as_ref());
                batch.put_cf(&cf_accounts_fkv, path.as_ref(), node.value);
                ctr += 1;
                if ctr > 10_000 {
                    self.db.write(std::mem::take(&mut batch))?;
                    *self
                        .last_computed_flatkeyvalue
                        .lock()
                        .map_err(|_| StoreError::LockError)? = path.as_ref().to_vec();
                    ctr = 0;
                }

                let mut storage_iter = self
                    .open_direct_storage_trie(account_hash, account_state.storage_root)?
                    .into_iter();
                if last_written_storage > Nibbles::default() {
                    storage_iter.advance(last_written_storage.to_bytes())?;
                    last_written_storage = Nibbles::default();
                }
                storage_iter.try_for_each(|(path, storage_node)| -> Result<(), StoreError> {
                    let Node::Leaf(node) = storage_node else {
                        return Ok(());
                    };
                    let key = apply_prefix(Some(account_hash), path);
                    batch.put_cf(&cf_misc, "last_written", key.as_ref());
                    batch.put_cf(&cf_storage_fkv, key.as_ref(), node.value);
                    ctr += 1;
                    if ctr > 10_000 {
                        self.db.write(std::mem::take(&mut batch))?;
                        *self
                            .last_computed_flatkeyvalue
                            .lock()
                            .map_err(|_| StoreError::LockError)? = key.as_ref().to_vec();
                        ctr = 0;
                    }
                    if let Ok(value) = control_rx.try_recv() {
                        match value {
                            FKVGeneratorControlMessage::Stop => {
                                return Err(StoreError::PivotChanged);
                            }
                            _ => {
                                return Err(StoreError::Custom("Unexpected message".to_string()));
                            }
                        }
                    }
                    Ok(())
                })?;
                if let Ok(value) = control_rx.try_recv() {
                    match value {
                        FKVGeneratorControlMessage::Stop => return Err(StoreError::PivotChanged),
                        _ => {
                            return Err(StoreError::Custom("Unexpected message".to_string()));
                        }
                    }
                }
                Ok(())
            });
            match res {
                Err(StoreError::PivotChanged) => {
                    if let Ok(value) = control_rx.recv() {
                        match value {
                            FKVGeneratorControlMessage::Continue => {}
                            _ => {
                                return Err(StoreError::Custom("Unexpected messafe".to_string()));
                            }
                        }
                    }
                }
                Err(err) => return Err(err),
                Ok(()) => {
                    batch.put_cf(&cf_misc, "last_written", [0xff]);
                    self.db.write(batch)?;
                    *self
                        .last_computed_flatkeyvalue
                        .lock()
                        .map_err(|_| StoreError::LockError)? = vec![0xff; 64];
                    return Ok(());
                }
            };
        }
    }

    fn apply_trie_updates(
        &self,
        notify: SyncSender<Result<(), StoreError>>,
        parent_state_root: H256,
        child_state_root: H256,
        account_updates: Vec<(Nibbles, Vec<u8>)>,
        storage_updates: StorageUpdates,
    ) -> Result<(), StoreError> {
        let db = &*self.db;
        let fkv_ctl = &self.flatkeyvalue_control_tx;
        let trie_cache = &self.trie_cache;

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
        notify.send(Ok(())).map_err(|_| StoreError::LockError)?;

        // Phase 2: update disk layer.
        let Some(root) = trie.get_commitable(parent_state_root, COMMIT_THRESHOLD) else {
            // Nothing to commit to disk, move on.
            return Ok(());
        };
        // Stop the flat-key-value generator thread, as the underlying trie is about to change.
        // Ignore the error, if the channel is closed it means there is no worker to notify.
        let _ = fkv_ctl.send(FKVGeneratorControlMessage::Stop);

        // RCU to remove the bottom layer: update step needs to happen after disk layer is updated.
        let mut trie_mut = (*trie).clone();
        let mut batch = WriteBatch::default();
        let [
            cf_accounts_trie_nodes,
            cf_accounts_flatkeyvalue,
            cf_storage_trie_nodes,
            cf_storage_flatkeyvalue,
            cf_misc,
        ] = open_cfs(
            db,
            [
                CF_ACCOUNT_TRIE_NODES,
                CF_ACCOUNT_FLATKEYVALUE,
                CF_STORAGE_TRIE_NODES,
                CF_STORAGE_FLATKEYVALUE,
                CF_MISC_VALUES,
            ],
        )?;

        let last_written = db.get_cf(&cf_misc, "last_written")?.unwrap_or_default();

        // Before encoding, accounts have only the account address as their path, while storage keys have
        // the account address (32 bytes) + storage path (up to 32 bytes).

        // Commit removes the bottom layer and returns it, this is the mutation step.
        let nodes = trie_mut.commit(root).unwrap_or_default();
        for (key, value) in nodes {
            let is_leaf = key.len() == 65 || key.len() == 131;
            let is_account = key.len() <= 65;

            if is_leaf && key > last_written {
                continue;
            }
            let cf = if is_leaf {
                if is_account {
                    &cf_accounts_flatkeyvalue
                } else {
                    &cf_storage_flatkeyvalue
                }
            } else if is_account {
                &cf_accounts_trie_nodes
            } else {
                &cf_storage_trie_nodes
            };
            if value.is_empty() {
                batch.delete_cf(cf, key);
            } else {
                batch.put_cf(cf, key, value);
            }
        }
        let result = db.write(batch);
        // We want to send this message even if there was an error during the batch write
        let _ = fkv_ctl.send(FKVGeneratorControlMessage::Continue);
        result?;
        // Phase 3: update diff layers with the removal of bottom layer.
        *trie_cache.lock().map_err(|_| StoreError::LockError)? = Arc::new(trie_mut);
        Ok(())
    }

    fn last_written(&self) -> Result<Vec<u8>, StoreError> {
        let last_computed_flatkeyvalue = self
            .last_computed_flatkeyvalue
            .lock()
            .map_err(|_| StoreError::LockError)?;
        Ok(last_computed_flatkeyvalue.clone())
    }
}

#[async_trait::async_trait]
impl StoreEngine for Store {
    fn apply_updates(&self, update_batch: UpdateBatch) -> Result<(), StoreError> {
        let db = self.db.clone();
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

        let [
            cf_receipts,
            cf_codes,
            cf_block_numbers,
            cf_tx_locations,
            cf_headers,
            cf_bodies,
        ] = open_cfs(
            &db,
            [
                CF_RECEIPTS,
                CF_ACCOUNT_CODES,
                CF_BLOCK_NUMBERS,
                CF_TRANSACTION_LOCATIONS,
                CF_HEADERS,
                CF_BODIES,
            ],
        )?;

        let mut batch = WriteBatch::default();

        let UpdateBatch {
            account_updates,
            storage_updates,
            ..
        } = update_batch;

        // Capacity one ensures sender just notifies and goes on
        let (notify_tx, notify_rx) = sync_channel(1);
        let wait_for_new_layer = notify_rx;
        trie_upd_worker_tx
            .send((
                notify_tx,
                parent_state_root,
                last_state_root,
                account_updates,
                storage_updates,
            ))
            .map_err(|e| {
                StoreError::Custom(format!("failed to read new trie layer notification: {e}"))
            })?;

        for block in update_batch.blocks {
            let block_number = block.header.number;
            let block_hash = block.hash();

            let hash_key_rlp = BlockHashRLP::from(block_hash);
            let header_value_rlp = BlockHeaderRLP::from(block.header.clone());
            batch.put_cf(&cf_headers, hash_key_rlp.bytes(), header_value_rlp.bytes());

            let hash_key: AccountCodeHashRLP = block_hash.into();
            let body_value = BlockBodyRLP::from_bytes(block.body.encode_to_vec());
            batch.put_cf(&cf_bodies, hash_key.bytes(), body_value.bytes());

            let hash_key = BlockHashRLP::from(block_hash).bytes().clone();
            batch.put_cf(&cf_block_numbers, hash_key, block_number.to_le_bytes());

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
            let mut buf = Vec::with_capacity(
                6 + code.bytecode.len()
                    + code
                        .jump_targets
                        .iter()
                        .map(std::mem::size_of_val)
                        .sum::<usize>(),
            );
            code.bytecode.encode(&mut buf);
            code.jump_targets.encode(&mut buf);
            batch.put_cf(&cf_codes, code_hash.0, buf);
        }

        // Wait for an updated top layer so every caller afterwards sees a consistent view.
        // Specifically, the next block produced MUST see this upper layer.
        wait_for_new_layer
            .recv()
            .map_err(|e| StoreError::Custom(format!("recv failed: {e}")))??;
        // After top-level is added, we can make the rest of the changes visible.
        db.write(batch)
            .map_err(|e| StoreError::Custom(format!("RocksDB batch write error: {}", e)))
    }

    /// Add a batch of blocks in a single transaction.
    /// This will store -> BlockHeader, BlockBody, BlockTransactions, BlockNumber.
    async fn add_blocks(&self, blocks: Vec<Block>) -> Result<(), StoreError> {
        let db = self.db.clone();

        tokio::task::spawn_blocking(move || {
            let mut batch = WriteBatch::default();

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

                let hash_key = BlockHashRLP::from(block_hash).bytes().clone();
                let header_value = BlockHeaderRLP::from(block.header.clone()).bytes().clone();
                batch.put_cf(&cf_headers, hash_key, header_value);

                let hash_key = BlockHashRLP::from(block_hash).bytes().clone();
                let body_value = BlockBodyRLP::from(block.body.clone()).bytes().clone();
                batch.put_cf(&cf_bodies, hash_key, body_value);

                let hash_key = BlockHashRLP::from(block_hash).bytes().clone();
                batch.put_cf(&cf_block_numbers, hash_key, block_number.to_le_bytes());

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

            db.write(batch)
                .map_err(|e| StoreError::Custom(format!("RocksDB batch write error: {}", e)))
        })
        .await
        .map_err(|e| StoreError::Custom(format!("Task panicked: {}", e)))?
    }

    async fn add_block_header(
        &self,
        block_hash: BlockHash,
        block_header: BlockHeader,
    ) -> Result<(), StoreError> {
        let hash_key = BlockHashRLP::from(block_hash).bytes().clone();
        let header_value = BlockHeaderRLP::from(block_header).bytes().clone();
        self.write_async(CF_HEADERS, hash_key, header_value).await
    }

    async fn add_block_headers(&self, block_headers: Vec<BlockHeader>) -> Result<(), StoreError> {
        let mut batch_ops = Vec::new();

        for header in block_headers {
            let block_hash = header.hash();
            let hash_key = BlockHashRLP::from(block_hash).bytes().clone();
            let header_value = BlockHeaderRLP::from(header.clone()).bytes().clone();

            batch_ops.push((CF_HEADERS.to_string(), hash_key, header_value));

            let number_key = header.number.to_le_bytes().to_vec();
            batch_ops.push((
                CF_BLOCK_NUMBERS.to_string(),
                BlockHashRLP::from(block_hash).bytes().clone(),
                number_key,
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
        let hash_key = BlockHashRLP::from(block_hash).bytes().clone();
        let body_value = BlockBodyRLP::from(block_body).bytes().clone();
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
        let mut batch = WriteBatch::default();

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

        self.db
            .write(batch)
            .map_err(|e| StoreError::Custom(format!("RocksDB batch write error: {}", e)))
    }

    async fn get_block_bodies(
        &self,
        from: BlockNumber,
        to: BlockNumber,
    ) -> Result<Vec<Option<BlockBody>>, StoreError> {
        let numbers: Vec<BlockNumber> = (from..=to).collect();
        let number_keys: Vec<Vec<u8>> = numbers.iter().map(|n| n.to_le_bytes().to_vec()).collect();

        let hashes = self
            .read_bulk_async(CF_CANONICAL_BLOCK_HASHES, number_keys, |bytes| {
                BlockHashRLP::from_bytes(bytes)
                    .to()
                    .map_err(StoreError::from)
            })
            .await?;

        let hash_keys: Vec<Vec<u8>> = hashes
            .iter()
            .flat_map(|hash_opt| hash_opt.map(|h| BlockHashRLP::from(h).bytes().clone()))
            .collect();

        let mut bodies = self
            .read_bulk_async(CF_BODIES, hash_keys, |bytes| {
                BlockBodyRLP::from_bytes(bytes)
                    .to()
                    .map_err(StoreError::from)
            })
            .await?;

        let mut i = 0;

        // Fill in with None for missing bodies
        let bodies = hashes
            .into_iter()
            .map(|opt| {
                opt.and_then(|_| {
                    let body_ref = bodies
                        .get_mut(i)
                        .expect("bodies length is equal to number of Somes in hashes");
                    let body = std::mem::take(body_ref);
                    i += 1;
                    body
                })
            })
            .collect();

        Ok(bodies)
    }

    async fn get_block_bodies_by_hash(
        &self,
        hashes: Vec<BlockHash>,
    ) -> Result<Vec<Option<BlockBody>>, StoreError> {
        let hash_keys: Vec<Vec<u8>> = hashes
            .iter()
            .map(|hash| BlockHashRLP::from(*hash).bytes().clone())
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
        let hash_key = BlockHashRLP::from(block_hash).bytes().clone();

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
        let hash_key = BlockHashRLP::from(block_hash).bytes().clone();

        self.read_sync(CF_HEADERS, hash_key)?
            .map(|bytes| BlockHeaderRLP::from_bytes(bytes).to())
            .transpose()
            .map_err(StoreError::from)
    }

    fn add_pending_block(&self, block: Block) -> Result<(), StoreError> {
        let hash_key = BlockHashRLP::from(block.hash()).bytes().clone();
        let block_value = BlockRLP::from(block).bytes().clone();
        let cf = self.cf_handle(CF_PENDING_BLOCKS)?;
        self.db
            .put_cf(&cf, hash_key, block_value)
            .map_err(StoreError::RocksdbError)
    }

    async fn get_pending_block(&self, block_hash: BlockHash) -> Result<Option<Block>, StoreError> {
        let hash_key = BlockHashRLP::from(block_hash).bytes().clone();

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
        let hash_key = BlockHashRLP::from(block_hash).bytes().clone();
        let number_value = block_number.to_le_bytes();
        self.write_async(CF_BLOCK_NUMBERS, hash_key, number_value)
            .await
    }

    async fn get_block_number(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockNumber>, StoreError> {
        let hash_key = BlockHashRLP::from(block_hash).bytes().clone();

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
        .map_err(|e| StoreError::Custom(format!("Task panicked: {}", e)))?
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

    async fn add_account_code(&self, code: Code) -> Result<(), StoreError> {
        let hash_key = code.hash.0.to_vec();
        let mut buf = Vec::with_capacity(
            6 + code.bytecode.len()
                + code
                    .jump_targets
                    .iter()
                    .map(std::mem::size_of_val)
                    .sum::<usize>(),
        );
        code.bytecode.encode(&mut buf);
        code.jump_targets.encode(&mut buf);
        self.write_async(CF_ACCOUNT_CODES, hash_key, buf).await
    }

    async fn clear_snap_state(&self) -> Result<(), StoreError> {
        let db = self.db.clone();

        tokio::task::spawn_blocking(move || {
            let cf = db
                .cf_handle(CF_SNAP_STATE)
                .ok_or_else(|| StoreError::Custom("Column family not found".to_string()))?;

            let mut iter = db.iterator_cf(&cf, rocksdb::IteratorMode::Start);
            let mut batch = WriteBatch::default();

            while let Some(Ok((key, _))) = iter.next() {
                batch.delete_cf(&cf, key);
            }

            db.write(batch)
                .map_err(|e| StoreError::Custom(format!("RocksDB batch write error: {}", e)))
        })
        .await
        .map_err(|e| StoreError::Custom(format!("Task panicked: {}", e)))?
    }

    /// Get account code by its hash.
    ///
    /// Check if the code exists in the cache (attribute `account_code_cache`), if not,
    /// reads the database, and if it exists, decodes and returns it.
    fn get_account_code(&self, code_hash: H256) -> Result<Option<Code>, StoreError> {
        // check cache first
        if let Some(code) = self
            .account_code_cache
            .lock()
            .map_err(|_| StoreError::LockError)?
            .get(&code_hash)
        {
            return Ok(Some(code.clone()));
        }

        let cf = self.cf_handle(CF_ACCOUNT_CODES)?;
        let Some(bytes) = self
            .db
            .get_pinned_cf(&cf, code_hash.as_bytes())
            .map_err(|e| StoreError::Custom(format!("RocksDB read error: {}", e)))?
        else {
            return Ok(None);
        };
        let (bytecode, targets) = decode_bytes(&bytes)?;
        let code = Code {
            hash: code_hash,
            bytecode: Bytes::copy_from_slice(bytecode),
            jump_targets: <Vec<_>>::decode(targets)?,
        };

        // insert into cache and evict if needed
        {
            let mut cache = self
                .account_code_cache
                .lock()
                .map_err(|_| StoreError::LockError)?;
            let code_size = code.size();
            self.account_code_cache_size
                .fetch_add(code_size as u64, std::sync::atomic::Ordering::SeqCst);
            let cache_len = cache.len() + 1;
            let mut current_size = self
                .account_code_cache_size
                .load(std::sync::atomic::Ordering::SeqCst);
            debug!(
                "[ACCOUNT CODE CACHE] cache elements (): {cache_len}, total size: {current_size} bytes"
            );

            while current_size > CODE_CACHE_MAX_SIZE {
                if let Some((_, code)) = cache.pop_lru() {
                    self.account_code_cache_size
                        .fetch_sub(code.size() as u64, std::sync::atomic::Ordering::SeqCst);
                    current_size = self
                        .account_code_cache_size
                        .load(std::sync::atomic::Ordering::SeqCst);
                } else {
                    break;
                }
            }

            cache.get_or_insert(code_hash, || code.clone());
        }

        Ok(Some(code))
    }

    async fn get_transaction_by_hash(
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

    async fn get_canonical_block_hash(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<BlockHash>, StoreError> {
        let number_key = block_number.to_le_bytes().to_vec();

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
        state_root: H256,
    ) -> Result<Trie, StoreError> {
        // FIXME: use a DB snapshot here
        let db = Box::new(RocksDBTrieDB::new(
            self.db.clone(),
            CF_STORAGE_TRIE_NODES,
            CF_STORAGE_FLATKEYVALUE,
            None,
            self.last_written()?,
        )?);
        let wrap_db = Box::new(TrieWrapper {
            state_root,
            inner: self
                .trie_cache
                .lock()
                .map_err(|_| StoreError::LockError)?
                .clone(),
            db,
            prefix: Some(hashed_address),
        });
        Ok(Trie::open(wrap_db, storage_root))
    }

    fn open_state_trie(&self, state_root: H256) -> Result<Trie, StoreError> {
        // FIXME: use a DB snapshot here
        let db = Box::new(RocksDBTrieDB::new(
            self.db.clone(),
            CF_ACCOUNT_TRIE_NODES,
            CF_ACCOUNT_FLATKEYVALUE,
            None,
            self.last_written()?,
        )?);
        let wrap_db = Box::new(TrieWrapper {
            state_root,
            inner: self
                .trie_cache
                .lock()
                .map_err(|_| StoreError::LockError)?
                .clone(),
            db,
            prefix: None,
        });
        Ok(Trie::open(wrap_db, state_root))
    }

    fn open_direct_storage_trie(
        &self,
        hashed_address: H256,
        storage_root: H256,
    ) -> Result<Trie, StoreError> {
        let db = Box::new(RocksDBTrieDB::new(
            self.db.clone(),
            CF_STORAGE_TRIE_NODES,
            CF_STORAGE_FLATKEYVALUE,
            Some(hashed_address),
            self.last_written()?,
        )?);
        Ok(Trie::open(db, storage_root))
    }

    fn open_direct_state_trie(&self, state_root: H256) -> Result<Trie, StoreError> {
        let db = Box::new(RocksDBTrieDB::new(
            self.db.clone(),
            CF_ACCOUNT_TRIE_NODES,
            CF_ACCOUNT_FLATKEYVALUE,
            None,
            self.last_written()?,
        )?);
        Ok(Trie::open(db, state_root))
    }

    fn open_locked_state_trie(&self, state_root: H256) -> Result<Trie, StoreError> {
        let db = Box::new(RocksDBLockedTrieDB::new(
            self.db.clone(),
            CF_ACCOUNT_TRIE_NODES,
            CF_ACCOUNT_FLATKEYVALUE,
            None,
            self.last_written()?,
        )?);
        let wrap_db = Box::new(TrieWrapper {
            state_root,
            inner: self
                .trie_cache
                .lock()
                .map_err(|_| StoreError::LockError)?
                .clone(),
            db,
            prefix: None,
        });
        Ok(Trie::open(wrap_db, state_root))
    }

    fn open_locked_storage_trie(
        &self,
        hashed_address: H256,
        storage_root: H256,
        state_root: H256,
    ) -> Result<Trie, StoreError> {
        let db = Box::new(RocksDBLockedTrieDB::new(
            self.db.clone(),
            CF_STORAGE_TRIE_NODES,
            CF_STORAGE_FLATKEYVALUE,
            None,
            self.last_written()?,
        )?);
        let wrap_db = Box::new(TrieWrapper {
            state_root,
            inner: self
                .trie_cache
                .lock()
                .map_err(|_| StoreError::LockError)?
                .clone(),
            db,
            prefix: Some(hashed_address),
        });
        Ok(Trie::open(wrap_db, storage_root))
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
            let mut batch = WriteBatch::default();

            let [cf_canonical, cf_chain_data] =
                open_cfs(&db, [CF_CANONICAL_BLOCK_HASHES, CF_CHAIN_DATA])?;

            // Update canonical block hashes
            if let Some(canonical_blocks) = new_canonical_blocks {
                for (block_number, block_hash) in canonical_blocks {
                    let number_key = block_number.to_le_bytes();
                    let hash_value = BlockHashRLP::from(block_hash).bytes().clone();
                    batch.put_cf(&cf_canonical, number_key, hash_value);
                }
            }

            // Remove anything after the head from the canonical chain
            for number in (head_number + 1)..=(latest) {
                batch.delete_cf(&cf_canonical, number.to_le_bytes());
            }

            // Make head canonical
            let head_key = head_number.to_le_bytes();
            let head_value = BlockHashRLP::from(head_hash).bytes().clone();
            batch.put_cf(&cf_canonical, head_key, head_value);

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

            db.write(batch)
                .map_err(|e| StoreError::Custom(format!("RocksDB batch write error: {}", e)))
        })
        .await
        .map_err(|e| StoreError::Custom(format!("Task panicked: {}", e)))?
    }

    async fn get_receipts_for_block(
        &self,
        block_hash: &BlockHash,
    ) -> Result<Vec<Receipt>, StoreError> {
        let mut receipts = Vec::new();
        let mut index = 0u64;

        loop {
            let key = (*block_hash, index).encode_to_vec();
            match self.read_async(CF_RECEIPTS, key).await? {
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

    async fn set_header_download_checkpoint(
        &self,
        block_hash: BlockHash,
    ) -> Result<(), StoreError> {
        let key = Self::snap_state_key(SnapStateIndex::HeaderDownloadCheckpoint);
        let value = BlockHashRLP::from(block_hash).bytes().clone();
        self.write_async(CF_SNAP_STATE, key, value).await
    }

    /// Gets the hash of the last header downloaded during a snap sync
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

    async fn set_latest_valid_ancestor(
        &self,
        bad_block: BlockHash,
        latest_valid: BlockHash,
    ) -> Result<(), StoreError> {
        let key = BlockHashRLP::from(bad_block).bytes().clone();
        let value = BlockHashRLP::from(latest_valid).bytes().clone();
        self.write_async(CF_INVALID_ANCESTORS, key, value).await
    }

    async fn get_latest_valid_ancestor(
        &self,
        block: BlockHash,
    ) -> Result<Option<BlockHash>, StoreError> {
        let key = BlockHashRLP::from(block).bytes().clone();

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
        let hash_key = BlockHashRLP::from(block_hash).bytes().clone();

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
        let number_key = block_number.to_le_bytes().to_vec();

        self.read_sync(CF_CANONICAL_BLOCK_HASHES, number_key)?
            .map(|bytes| BlockHashRLP::from_bytes(bytes).to())
            .transpose()
            .map_err(StoreError::from)
    }

    async fn write_storage_trie_nodes_batch(
        &self,
        storage_trie_nodes: Vec<(H256, Vec<(Nibbles, Vec<u8>)>)>,
    ) -> Result<(), StoreError> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            let mut batch = WriteBatch::default();
            let cf = db.cf_handle(CF_STORAGE_TRIE_NODES).ok_or_else(|| {
                StoreError::Custom("Column family not found: CF_STORAGE_TRIE_NODES".to_string())
            })?;

            for (address_hash, nodes) in storage_trie_nodes {
                for (node_hash, node_data) in nodes {
                    let key = apply_prefix(Some(address_hash), node_hash);
                    if node_data.is_empty() {
                        batch.delete_cf(&cf, key.as_ref());
                    } else {
                        batch.put_cf(&cf, key.as_ref(), node_data);
                    }
                }
            }

            db.write(batch)
                .map_err(|e| StoreError::Custom(format!("RocksDB batch write error: {}", e)))
        })
        .await
        .map_err(|e| StoreError::Custom(format!("Task panicked: {}", e)))?
    }

    async fn write_account_code_batch(
        &self,
        account_codes: Vec<(H256, Code)>,
    ) -> Result<(), StoreError> {
        let mut batch_ops = Vec::new();

        for (code_hash, code) in account_codes {
            let key = code_hash.as_bytes().to_vec();
            let mut buf = Vec::with_capacity(
                6 + code.bytecode.len()
                    + code
                        .jump_targets
                        .iter()
                        .map(std::mem::size_of_val)
                        .sum::<usize>(),
            );
            code.bytecode.encode(&mut buf);
            code.jump_targets.encode(&mut buf);
            batch_ops.push((CF_ACCOUNT_CODES.to_string(), key, buf));
        }

        self.write_batch_async(batch_ops).await
    }

    async fn add_fullsync_batch(&self, headers: Vec<BlockHeader>) -> Result<(), StoreError> {
        let mut batch_ops = Vec::new();

        for header in headers {
            let number_value = header.number.to_le_bytes().to_vec();
            let header_value = BlockHeaderRLP::from(header).bytes().clone();

            batch_ops.push((CF_FULLSYNC_HEADERS.to_string(), number_value, header_value));
        }

        self.write_batch_async(batch_ops).await
    }

    async fn read_fullsync_batch(
        &self,
        start: BlockNumber,
        limit: u64,
    ) -> Result<Vec<Option<BlockHeader>>, StoreError> {
        self.read_bulk_async(
            CF_FULLSYNC_HEADERS,
            (start..start + limit).map(|n| n.to_le_bytes()).collect(),
            |bytes| {
                BlockHeaderRLP::from_bytes(bytes)
                    .to()
                    .map_err(StoreError::from)
            },
        )
        .await
    }

    async fn clear_fullsync_headers(&self) -> Result<(), StoreError> {
        let db = self.db.clone();

        tokio::task::spawn_blocking(move || {
            let cf = db
                .cf_handle(CF_FULLSYNC_HEADERS)
                .ok_or_else(|| StoreError::Custom("Column family not found".to_string()))?;

            let mut iter = db.iterator_cf(&cf, rocksdb::IteratorMode::Start);
            let mut batch = WriteBatch::default();

            while let Some(Ok((key, _))) = iter.next() {
                batch.delete_cf(&cf, key);
            }

            db.write(batch)
                .map_err(|e| StoreError::Custom(format!("RocksDB batch write error: {}", e)))
        })
        .await
        .map_err(|e| StoreError::Custom(format!("Task panicked: {}", e)))?
    }

    fn generate_flatkeyvalue(&self) -> Result<(), StoreError> {
        self.flatkeyvalue_control_tx
            .send(FKVGeneratorControlMessage::Continue)
            .map_err(|_| StoreError::Custom("FlatKeyValue thread disconnected.".to_string()))
    }

    async fn create_checkpoint(&self, path: &Path) -> Result<(), StoreError> {
        let checkpoint = Checkpoint::new(&self.db)
            .map_err(|e| StoreError::Custom(format!("Failed to create checkpoint: {e}")))?;

        checkpoint.create_checkpoint(path).map_err(|e| {
            StoreError::Custom(format!(
                "Failed to create RocksDB checkpoint at {path:?}: {e}"
            ))
        })?;

        Ok(())
    }

    fn get_store_directory(&self) -> Result<PathBuf, StoreError> {
        let path = self.db.path();
        Ok(path.to_path_buf())
    }

    fn flatkeyvalue_computed(&self, account: H256) -> Result<bool, StoreError> {
        let account_nibbles = Nibbles::from_bytes(account.as_bytes());
        let last_computed_flatkeyvalue = self.last_written()?;
        Ok(&last_computed_flatkeyvalue[0..64] > account_nibbles.as_ref())
    }

    fn set_store_schema_version(&self) -> Result<(), StoreError> {
        self.write_sync(
            CF_MISC_VALUES,
            "store_schema_version",
            STORE_SCHEMA_VERSION.to_le_bytes(),
        )
    }

    fn get_store_schema_version(&self) -> Result<Option<u64>, StoreError> {
        self.read_sync(CF_MISC_VALUES, "store_schema_version")?
            .map(|bytes| -> Result<u64, StoreError> {
                let array: [u8; 8] = bytes.try_into().map_err(|_| {
                    StoreError::Custom("Invalid store schema version bytes".to_string())
                })?;
                Ok(u64::from_le_bytes(array))
            })
            .transpose()
            .map_err(|_| StoreError::DecodeError)
    }
}

/// Open column families
fn open_cfs<'a, const N: usize>(
    db: &'a DBWithThreadMode<MultiThreaded>,
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
