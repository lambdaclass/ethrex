use crate::{
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
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{
        Arc, Mutex,
        mpsc::{SyncSender, sync_channel},
    },
};
use tracing::{debug, error, info, trace};

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

/// State trie nodes column family: [`Nibbles`] => [`Vec<u8>`]
/// - [`Nibbles`] = `node_hash.as_ref()`
/// - [`Vec<u8>`] = `node_data`
pub(crate) const CF_TRIE_NODES: &str = "trie_nodes";

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

pub const CF_FLATKEYVALUE: &str = "flatkeyvalue";

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

#[derive(Debug, Clone)]
pub struct Store {
    db: canopydb::Environment,
    dbs: Arc<BTreeMap<String, canopydb::Database>>,
    trie_cache: Arc<Mutex<Arc<TrieLayerCache>>>,
    flatkeyvalue_control_tx: std::sync::mpsc::SyncSender<FKVGeneratorControlMessage>,
    trie_update_worker_tx: TriedUpdateWorkerTx,
    last_computed_flatkeyvalue: Arc<Mutex<Vec<u8>>>,
    writer_tx: std::sync::mpsc::SyncSender<(
        WriterMessage,
        std::sync::mpsc::SyncSender<Result<(), StoreError>>,
    )>,
}

impl Store {
    pub fn new(path: &Path) -> Result<Self, StoreError> {
        std::fs::create_dir_all(path).unwrap();
        let environment = canopydb::Environment::new(path).unwrap();

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
            CF_TRIE_NODES,
            CF_PENDING_BLOCKS,
            CF_INVALID_ANCESTORS,
            CF_FULLSYNC_HEADERS,
            CF_FLATKEYVALUE,
            CF_MISC_VALUES,
        ];

        let mut dbs = BTreeMap::new();

        // Add all existing CFs (we must open them to be able to drop obsolete ones later)
        for cf in expected_column_families {
            // default is handled automatically
            let db_handle = environment.get_or_create_database(cf).unwrap();
            // Write outside Writer, because we still didn't start that
            let tx = db_handle.begin_write().unwrap();
            tx.get_or_create_tree(b"").unwrap();
            tx.commit().unwrap();
            dbs.insert(cf.to_string(), db_handle);
        }

        let (fkv_tx, fkv_rx) = std::sync::mpsc::sync_channel(0);
        let (trie_upd_tx, trie_upd_rx) = std::sync::mpsc::sync_channel(0);
        let (writer_tx, writer_rx) = std::sync::mpsc::sync_channel(0);

        let last_written = {
            let db = dbs.get(CF_MISC_VALUES).unwrap();
            let transaction = db.begin_read().unwrap();
            let mut last_written = transaction
                .get_tree(b"")
                .unwrap()
                .unwrap()
                .get(b"last_written")
                .unwrap()
                .map(|b| b.to_vec())
                .unwrap_or_else(|| vec![0u8; 64]);
            if last_written == vec![0xff] {
                last_written = vec![0xff; 64];
            }
            last_written
        };

        let store = Self {
            db: environment,
            dbs: Arc::new(dbs),
            trie_cache: Default::default(),
            flatkeyvalue_control_tx: fkv_tx,
            trie_update_worker_tx: trie_upd_tx,
            last_computed_flatkeyvalue: Arc::new(Mutex::new(last_written)),
            writer_tx,
        };
        let store_clone = store.clone();
        std::thread::spawn(move || {
            let mut rx = fkv_rx;
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
            match store_clone.flatkeyvalue_generator(&mut rx) {
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

        let writer = Writer {
            db: store.db.clone(),
            dbs: store.dbs.clone(),
            trie_update_worker_tx: store.trie_update_worker_tx.clone(),
        };

        std::thread::spawn(move || {
            writer.run(writer_rx);
        });
        Ok(store)
    }

    // Helper method for async writes
    async fn write_async<K, V>(
        &self,
        cf_name: &'static str,
        key: K,
        value: V,
    ) -> Result<(), StoreError>
    where
        K: AsRef<[u8]> + Send + 'static,
        V: AsRef<[u8]> + Send + 'static,
    {
        let writer_tx = self.writer_tx.clone();

        tokio::task::spawn_blocking(move || {
            let msg = WriterMessage::WriteAsync {
                cf_name,
                key: key.as_ref().to_vec(),
                value: value.as_ref().to_vec(),
            };
            let (tx, rx) = std::sync::mpsc::sync_channel(0);
            writer_tx.send((msg, tx)).unwrap();
            rx.recv().unwrap()?;
            Ok(())
        })
        .await
        .map_err(|e| StoreError::Custom(format!("Task panicked: {}", e)))?
    }

    // Helper method for async reads
    async fn read_async<K>(&self, cf_name: &str, key: K) -> Result<Option<Vec<u8>>, StoreError>
    where
        K: AsRef<[u8]> + Send + 'static,
    {
        let dbs = self.dbs.clone();
        let cf_name = cf_name.to_string();

        tokio::task::spawn_blocking(move || {
            let transaction = dbs.get(&cf_name).unwrap().begin_read().unwrap();
            let tree = transaction.get_tree(b"").unwrap().unwrap();
            tree.get(key.as_ref())
                .map(|b| b.map(|b| b.to_vec()))
                .map_err(|e| StoreError::Custom(format!("CanopyDB read error: {}", e)))
        })
        .await
        .map_err(|e| StoreError::Custom(format!("Task panicked: {}", e)))?
    }

    // Helper method for sync reads
    fn read_sync<K>(&self, cf_name: &str, key: K) -> Result<Option<Vec<u8>>, StoreError>
    where
        K: AsRef<[u8]>,
    {
        let transaction = self.dbs.get(cf_name).unwrap().begin_read().unwrap();
        let tree = transaction.get_tree(b"").unwrap().unwrap();
        tree.get(key.as_ref())
            .map(|b| b.map(|b| b.to_vec()))
            .map_err(|e| StoreError::Custom(format!("CanopyDB read error: {}", e)))
    }

    // Helper method for batch writes
    async fn write_batch_async(
        &self,
        batch_ops: Vec<(&'static str, Vec<u8>, Vec<u8>)>,
    ) -> Result<(), StoreError> {
        let writer_tx = self.writer_tx.clone();
        tokio::task::spawn_blocking(move || {
            let msg = WriterMessage::WriteBatchAsync { batch_ops };
            let (tx, rx) = std::sync::mpsc::sync_channel(0);
            writer_tx.send((msg, tx)).unwrap();
            rx.recv().unwrap()?;
            Ok(())
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
    ) -> Result<Vec<V>, StoreError>
    where
        K: AsRef<[u8]> + Send + 'static,
        V: Send + 'static,
        F: Fn(Vec<u8>) -> Result<V, StoreError> + Send + 'static,
    {
        let dbs = self.dbs.clone();
        let cf_name = cf_name.to_string();

        tokio::task::spawn_blocking(move || {
            let cf = dbs.get(&cf_name).unwrap();
            let transaction = cf.begin_read().unwrap();
            let tree = transaction.get_tree(b"").unwrap().unwrap();
            let mut results = Vec::with_capacity(keys.len());

            for key in keys {
                match tree.get(key.as_ref())? {
                    Some(bytes) => {
                        let value = deserialize_fn(bytes.to_vec())?;
                        results.push(value);
                    }
                    None => {
                        return Err(StoreError::Custom("Key not found in bulk read".to_string()));
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
        control_rx: &mut std::sync::mpsc::Receiver<FKVGeneratorControlMessage>,
    ) -> Result<(), StoreError> {
        {
            let cf_misc = self.dbs.get(CF_MISC_VALUES).unwrap().begin_read().unwrap();
            // We use sequential write here, so it should be deadlock-free
            let cf_flatkeyvalue = self
                .dbs
                .get(CF_FLATKEYVALUE)
                .unwrap()
                .begin_write()
                .unwrap();
            let misc_tree = cf_misc.get_tree(b"").unwrap().unwrap();
            let mut flatkeyvalue = cf_flatkeyvalue.get_tree(b"").unwrap().unwrap();
            let last_written = {
                misc_tree
                    .get(b"last_written")
                    .unwrap()
                    .map(|b| b.to_vec())
                    .unwrap_or_else(|| vec![0u8; 64])
            };
            if last_written == vec![0xff] {
                return Ok(());
            }

            flatkeyvalue.delete_range(last_written..vec![0xff])?;
        }

        loop {
            let root = self
                .read_sync(CF_TRIE_NODES, [])?
                .ok_or(StoreError::MissingLatestBlockNumber)?;
            let root: Node = ethrex_trie::Node::decode(&root)?;
            let state_root = root.compute_hash().finalize();

            let mut last_written = self
                .read_sync(CF_MISC_VALUES, b"last_written")
                .unwrap()
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
            let mut batch = Vec::new();

            let mut iter = {
                let db = Box::new(
                    RocksDBTrieDB::new(
                        self.db.clone(),
                        self.dbs.clone(),
                        self.writer_tx.clone(),
                        CF_TRIE_NODES,
                        None,
                        self.last_written()?,
                    )
                    .unwrap(),
                );
                Trie::open(db, state_root)
            }
            .into_iter();
            if last_written_account > Nibbles::default() {
                iter.advance(last_written_account.to_bytes())?;
            }
            let mut res = Ok(());
            'outer: for (path, node) in iter {
                let Node::Leaf(node) = node else {
                    continue;
                };
                let account_state = AccountState::decode(&node.value)?;
                let account_hash = H256::from_slice(&path.to_bytes());
                last_written = path.clone().into_vec();
                batch.push((path.into_vec(), node.value));
                ctr += 1;
                if ctr > 10_000 {
                    let msg = WriterMessage::WriteFKVBatch {
                        last_written: last_written.clone(),
                        batch: std::mem::take(&mut batch),
                    };
                    let (tx, rx) = std::sync::mpsc::sync_channel(0);
                    self.writer_tx.send((msg, tx)).unwrap();
                    rx.recv().unwrap()?;

                    *self
                        .last_computed_flatkeyvalue
                        .lock()
                        .map_err(|_| StoreError::LockError)? = last_written.clone();
                }

                let mut iter_inner = {
                    let db = Box::new(
                        RocksDBTrieDB::new(
                            self.db.clone(),
                            self.dbs.clone(),
                            self.writer_tx.clone(),
                            CF_TRIE_NODES,
                            Some(account_hash),
                            self.last_written()?,
                        )
                        .unwrap(),
                    );
                    Trie::open(db, account_state.storage_root)
                }
                .into_iter();
                if last_written_storage > Nibbles::default() {
                    iter_inner.advance(last_written_storage.to_bytes())?;
                    last_written_storage = Nibbles::default();
                }
                for (path, node) in iter_inner {
                    let Node::Leaf(node) = node else {
                        continue;
                    };
                    let key = apply_prefix(Some(account_hash), path);
                    last_written = key.clone().into_vec();
                    batch.push((key.into_vec(), node.value));
                    ctr += 1;
                    if ctr > 10_000 {
                        let msg = WriterMessage::WriteFKVBatch {
                            last_written: last_written.clone(),
                            batch: std::mem::take(&mut batch),
                        };
                        let (tx, rx) = std::sync::mpsc::sync_channel(0);
                        self.writer_tx.send((msg, tx)).unwrap();
                        rx.recv().unwrap()?;

                        *self
                            .last_computed_flatkeyvalue
                            .lock()
                            .map_err(|_| StoreError::LockError)? = last_written.clone();
                    }
                    if let Ok(value) = control_rx.try_recv() {
                        match value {
                            FKVGeneratorControlMessage::Stop => {
                                res = Err(StoreError::PivotChanged);
                                break 'outer;
                            }
                            _ => {
                                res = Err(StoreError::Custom("Unexpected message".to_string()));
                                break 'outer;
                            }
                        }
                    }
                }
                if let Ok(value) = control_rx.try_recv() {
                    match value {
                        FKVGeneratorControlMessage::Stop => {
                            res = Err(StoreError::PivotChanged);
                            break 'outer;
                        }
                        _ => {
                            res = Err(StoreError::Custom("Unexpected message".to_string()));
                            break 'outer;
                        }
                    }
                }
            }
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
                    let msg = WriterMessage::WriteFKVBatch {
                        last_written: vec![0xff],
                        batch: std::mem::take(&mut batch),
                    };
                    let (tx, rx) = std::sync::mpsc::sync_channel(0);
                    self.writer_tx.send((msg, tx)).unwrap();
                    rx.recv().unwrap()?;

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
        let [cf_misc] = open_cfs(&self.dbs, [CF_MISC_VALUES])?;

        let mut batch_ops = Vec::new();
        {
            let last_written = {
                cf_misc
                    .begin_read()
                    .unwrap()
                    .get_tree(b"")
                    .unwrap()
                    .unwrap()
                    .get(b"last_written")
                    .unwrap()
                    .map(|b| b.to_vec())
                    .unwrap_or_default()
            };
            // Commit removes the bottom layer and returns it, this is the mutation step.
            let nodes = trie_mut.commit(root).unwrap_or_default();
            for (key, value) in nodes {
                let is_leaf = key.len() == 65 || key.len() == 131;

                if is_leaf && key > last_written {
                    continue;
                }
                let cf = if is_leaf {
                    CF_FLATKEYVALUE
                } else {
                    CF_TRIE_NODES
                };
                if value.is_empty() {
                    batch_ops.push((cf, key, vec![]));
                } else {
                    batch_ops.push((cf, key, value));
                }
            }
        }
        let msg = WriterMessage::WriteBatchAsync { batch_ops };
        let (tx, rx) = std::sync::mpsc::sync_channel(0);
        self.writer_tx.send((msg, tx)).unwrap();
        let result = rx.recv().unwrap();
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

        let msg = WriterMessage::ApplyUpdateBatch {
            parent_state_root,
            update_batch,
        };
        let (tx, rx) = std::sync::mpsc::sync_channel(0);
        self.writer_tx.send((msg, tx)).unwrap();
        rx.recv().unwrap()
    }

    /// Add a batch of blocks in a single transaction.
    /// This will store -> BlockHeader, BlockBody, BlockTransactions, BlockNumber.
    async fn add_blocks(&self, blocks: Vec<Block>) -> Result<(), StoreError> {
        let mut batch_ops = Vec::new();

        for block in blocks {
            let block_hash = block.hash();
            let block_number = block.header.number;

            let hash_key = BlockHashRLP::from(block_hash).bytes().clone();
            let header_value = BlockHeaderRLP::from(block.header.clone()).bytes().clone();
            batch_ops.push((CF_HEADERS, hash_key, header_value));

            let hash_key = BlockHashRLP::from(block_hash).bytes().clone();
            let body_value = BlockBodyRLP::from(block.body.clone()).bytes().clone();
            batch_ops.push((CF_BODIES, hash_key, body_value));

            let hash_key = BlockHashRLP::from(block_hash).bytes().clone();
            batch_ops.push((
                CF_BLOCK_NUMBERS,
                hash_key,
                block_number.to_le_bytes().to_vec(),
            ));

            for (index, transaction) in block.body.transactions.iter().enumerate() {
                let tx_hash = transaction.hash();
                // Key: tx_hash + block_hash
                let mut composite_key = Vec::with_capacity(64);
                composite_key.extend_from_slice(tx_hash.as_bytes());
                composite_key.extend_from_slice(block_hash.as_bytes());
                let location_value = (block_number, block_hash, index as u64).encode_to_vec();
                batch_ops.push((CF_TRANSACTION_LOCATIONS, composite_key, location_value));
            }
        }

        self.write_batch_async(batch_ops)
            .await
            .map_err(|e| StoreError::Custom(format!("CanopyDB batch write error: {}", e)))
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

            batch_ops.push((CF_HEADERS, hash_key, header_value));

            let number_key = header.number.to_le_bytes().to_vec();
            batch_ops.push((
                CF_BLOCK_NUMBERS,
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
        let Some(hash) = self.get_canonical_block_hash_sync(block_number)? else {
            return Ok(());
        };

        let mut batch_ops = vec![];

        batch_ops.push((
            CF_CANONICAL_BLOCK_HASHES,
            block_number.to_le_bytes().to_vec(),
            vec![],
        ));
        batch_ops.push((CF_BODIES, hash.as_bytes().to_vec(), vec![]));
        batch_ops.push((CF_HEADERS, hash.as_bytes().to_vec(), vec![]));
        batch_ops.push((CF_BLOCK_NUMBERS, hash.as_bytes().to_vec(), vec![]));

        self.write_batch_async(batch_ops)
            .await
            .map_err(|e| StoreError::Custom(format!("RocksDB batch write error: {}", e)))
    }

    async fn get_block_bodies(
        &self,
        from: BlockNumber,
        to: BlockNumber,
    ) -> Result<Vec<BlockBody>, StoreError> {
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

    async fn get_block_bodies_by_hash(
        &self,
        hashes: Vec<BlockHash>,
    ) -> Result<Vec<BlockBody>, StoreError> {
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

        let msg = WriterMessage::WriteAsync {
            cf_name: CF_PENDING_BLOCKS,
            key: hash_key,
            value: block_value,
        };
        let (tx, rx) = std::sync::mpsc::sync_channel(0);
        self.writer_tx.send((msg, tx)).unwrap();
        rx.recv().unwrap()?;
        Ok(())
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
        let dbs = self.dbs.clone();
        let tx_hash_key = transaction_hash.as_bytes().to_vec();

        tokio::task::spawn_blocking(move || {
            let [cf_transaction_locations, cf_canonical] =
                open_cfs(&dbs, [CF_TRANSACTION_LOCATIONS, CF_CANONICAL_BLOCK_HASHES])?;

            let tl_tx = cf_transaction_locations.begin_read().unwrap();
            let canonical_tx = cf_canonical.begin_read().unwrap();

            let tl_tree = tl_tx.get_tree(b"").unwrap().unwrap();

            let mut iter = tl_tree.prefix(&tx_hash_key).unwrap();
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

            let canonical_tree = canonical_tx.get_tree(b"").unwrap().unwrap();

            // If there are multiple locations, filter by the canonical chain
            for (block_number, block_hash, index) in transaction_locations {
                let canonical_hash = {
                    canonical_tree
                        .get(&block_number.to_le_bytes())?
                        .and_then(|bytes| BlockHashRLP::from_bytes(bytes.to_vec()).to().ok())
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
            batch_ops.push((CF_RECEIPTS, key, value));
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
        let mut buf = Vec::with_capacity(6 + code.bytecode.len() + 2 * code.jump_targets.len());
        code.bytecode.encode(&mut buf);
        code.jump_targets
            .into_iter()
            .flat_map(|t| t.to_le_bytes())
            .collect::<Vec<u8>>()
            .as_slice()
            .encode(&mut buf);
        self.write_async(CF_ACCOUNT_CODES, hash_key, buf).await
    }

    async fn clear_snap_state(&self) -> Result<(), StoreError> {
        let writer_tx = self.writer_tx.clone();

        tokio::task::spawn_blocking(move || {
            let msg = WriterMessage::ClearSnapState;
            let (tx, rx) = std::sync::mpsc::sync_channel(0);
            writer_tx.send((msg, tx)).unwrap();
            rx.recv().unwrap()?;
            Ok(())
        })
        .await
        .map_err(|e| StoreError::Custom(format!("Task panicked: {}", e)))?
    }

    fn get_account_code(&self, code_hash: H256) -> Result<Option<Code>, StoreError> {
        let hash_key = code_hash.as_bytes().to_vec();
        let Some(bytes) = self.read_sync(CF_ACCOUNT_CODES, hash_key)? else {
            return Ok(None);
        };
        let bytes = Bytes::from_owner(bytes);
        let (bytecode, targets) = decode_bytes(&bytes)?;
        let (targets, rest) = decode_bytes(targets)?;
        if !rest.is_empty() || !targets.len().is_multiple_of(2) {
            return Err(StoreError::DecodeError);
        }
        let code = Code {
            hash: code_hash,
            bytecode: Bytes::copy_from_slice(bytecode),
            jump_targets: targets
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect(),
        };
        Ok(Some(code))
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
            self.dbs.clone(),
            self.writer_tx.clone(),
            CF_TRIE_NODES,
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
            self.dbs.clone(),
            self.writer_tx.clone(),
            CF_TRIE_NODES,
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
            self.dbs.clone(),
            self.writer_tx.clone(),
            CF_TRIE_NODES,
            Some(hashed_address),
            self.last_written()?,
        )?);
        Ok(Trie::open(db, storage_root))
    }

    fn open_direct_state_trie(&self, state_root: H256) -> Result<Trie, StoreError> {
        let db = Box::new(RocksDBTrieDB::new(
            self.db.clone(),
            self.dbs.clone(),
            self.writer_tx.clone(),
            CF_TRIE_NODES,
            None,
            self.last_written()?,
        )?);
        Ok(Trie::open(db, state_root))
    }

    fn open_locked_state_trie(&self, state_root: H256) -> Result<Trie, StoreError> {
        let db = Box::new(RocksDBLockedTrieDB::new(
            self.dbs.clone(),
            CF_TRIE_NODES,
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
            self.dbs.clone(),
            CF_TRIE_NODES,
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

        let mut batch_ops = Vec::new();

        // Update canonical block hashes
        if let Some(canonical_blocks) = new_canonical_blocks {
            for (block_number, block_hash) in canonical_blocks {
                let number_key = block_number.to_le_bytes();
                let hash_value = BlockHashRLP::from(block_hash).bytes().clone();
                batch_ops.push((CF_CANONICAL_BLOCK_HASHES, number_key.to_vec(), hash_value));
            }
        }

        // Remove anything after the head from the canonical chain
        for number in (head_number + 1)..=(latest) {
            batch_ops.push((
                CF_CANONICAL_BLOCK_HASHES,
                number.to_le_bytes().to_vec(),
                vec![],
            ));
        }

        // Make head canonical
        let head_key = head_number.to_le_bytes();
        let head_value = BlockHashRLP::from(head_hash).bytes().clone();
        batch_ops.push((CF_CANONICAL_BLOCK_HASHES, head_key.to_vec(), head_value));

        // Update chain data

        let latest_key = Self::chain_data_key(ChainDataIndex::LatestBlockNumber);
        batch_ops.push((
            CF_CHAIN_DATA,
            latest_key,
            head_number.to_le_bytes().to_vec(),
        ));

        if let Some(safe_number) = safe {
            let safe_key = Self::chain_data_key(ChainDataIndex::SafeBlockNumber);
            batch_ops.push((CF_CHAIN_DATA, safe_key, safe_number.to_le_bytes().to_vec()));
        }

        if let Some(finalized_number) = finalized {
            let finalized_key = Self::chain_data_key(ChainDataIndex::FinalizedBlockNumber);
            batch_ops.push((
                CF_CHAIN_DATA,
                finalized_key,
                finalized_number.to_le_bytes().to_vec(),
            ));
        }
        self.write_batch_async(batch_ops).await
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
        let mut batch_ops = vec![];
        for (address_hash, nodes) in storage_trie_nodes {
            for (node_hash, node_data) in nodes {
                let key = apply_prefix(Some(address_hash), node_hash).into_vec();
                if node_data.is_empty() {
                    batch_ops.push((CF_TRIE_NODES, key, Vec::new()));
                } else {
                    batch_ops.push((CF_TRIE_NODES, key, node_data));
                }
            }
        }
        self.write_batch_async(batch_ops).await
    }

    async fn write_account_code_batch(
        &self,
        account_codes: Vec<(H256, Code)>,
    ) -> Result<(), StoreError> {
        let mut batch_ops = Vec::new();

        for (code_hash, code) in account_codes {
            let key = code_hash.as_bytes().to_vec();
            let mut buf = Vec::with_capacity(6 + code.bytecode.len() + 2 * code.jump_targets.len());
            code.bytecode.encode(&mut buf);
            code.jump_targets
                .into_iter()
                .flat_map(|t| t.to_le_bytes())
                .collect::<Vec<u8>>()
                .as_slice()
                .encode(&mut buf);
            batch_ops.push((CF_ACCOUNT_CODES, key, buf));
        }

        self.write_batch_async(batch_ops).await
    }

    async fn add_fullsync_batch(&self, headers: Vec<BlockHeader>) -> Result<(), StoreError> {
        let mut batch_ops = Vec::new();

        for header in headers {
            let number_value = header.number.to_le_bytes().to_vec();
            let header_value = BlockHeaderRLP::from(header).bytes().clone();

            batch_ops.push((CF_FULLSYNC_HEADERS, number_value, header_value));
        }

        self.write_batch_async(batch_ops).await
    }

    async fn read_fullsync_batch(
        &self,
        start: BlockNumber,
        limit: u64,
    ) -> Result<Vec<BlockHeader>, StoreError> {
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
        let writer_tx = self.writer_tx.clone();

        tokio::task::spawn_blocking(move || {
            let msg = WriterMessage::ClearFullsyncHeaders;
            let (tx, rx) = std::sync::mpsc::sync_channel(0);
            writer_tx.send((msg, tx)).unwrap();
            rx.recv().unwrap()?;
            Ok(())
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
        todo!("not implemented because used only for L2")

        // let checkpoint = Checkpoint::new(&self.db)
        //     .map_err(|e| StoreError::Custom(format!("Failed to create checkpoint: {e}")))?;

        // checkpoint.create_checkpoint(path).map_err(|e| {
        //     StoreError::Custom(format!(
        //         "Failed to create RocksDB checkpoint at {path:?}: {e}"
        //     ))
        // })?;

        // Ok(())
    }

    fn flatkeyvalue_computed(&self, account: H256) -> Result<bool, StoreError> {
        let account_nibbles = Nibbles::from_bytes(account.as_bytes());
        let last_computed_flatkeyvalue = self.last_written()?;
        Ok(&last_computed_flatkeyvalue[0..64] > account_nibbles.as_ref())
    }
}

/// Open column families
fn open_cfs<'a, const N: usize>(
    dbs: &'a BTreeMap<String, canopydb::Database>,
    names: [&str; N],
) -> Result<[&'a canopydb::Database; N], StoreError> {
    let mut handles = Vec::with_capacity(N);

    for name in names {
        handles
            .push(dbs.get(name).ok_or_else(|| {
                StoreError::Custom(format!("Column family '{}' not found", name))
            })?);
    }

    handles
        .try_into()
        .map_err(|_| StoreError::Custom("Unexpected number of column families".to_string()))
}

struct Writer {
    db: canopydb::Environment,
    dbs: Arc<BTreeMap<String, canopydb::Database>>,
    trie_update_worker_tx: TriedUpdateWorkerTx,
}

pub(crate) enum WriterMessage {
    WriteAsync {
        cf_name: &'static str,
        key: Vec<u8>,
        value: Vec<u8>,
    },
    WriteBatchAsync {
        batch_ops: Vec<(&'static str, Vec<u8>, Vec<u8>)>,
    },
    WriteFKVBatch {
        last_written: Vec<u8>,
        batch: Vec<(Vec<u8>, Vec<u8>)>,
    },
    ApplyUpdateBatch {
        parent_state_root: H256,
        update_batch: UpdateBatch,
    },
    ClearSnapState,
    ClearFullsyncHeaders,
}

impl Writer {
    fn run(
        self,
        receiver: std::sync::mpsc::Receiver<(
            WriterMessage,
            std::sync::mpsc::SyncSender<Result<(), StoreError>>,
        )>,
    ) {
        for (message, resp_tx) in receiver {
            let result = match message {
                WriterMessage::WriteAsync {
                    cf_name,
                    key,
                    value,
                } => self.write_async(cf_name, key, value),
                WriterMessage::WriteBatchAsync { batch_ops } => self.write_batch_async(batch_ops),
                WriterMessage::WriteFKVBatch {
                    last_written,
                    batch,
                } => self.write_fkv_batch(last_written, batch),
                WriterMessage::ApplyUpdateBatch {
                    parent_state_root,
                    update_batch,
                } => self.apply_update_batch(parent_state_root, update_batch),
                WriterMessage::ClearSnapState => self.clear_snap_state(),
                WriterMessage::ClearFullsyncHeaders => self.clear_fullsync_headers(),
            };
            let _ = resp_tx.send(result);
        }
    }

    // Helper method for async writes
    fn write_async<K, V>(&self, cf_name: &str, key: K, value: V) -> Result<(), StoreError>
    where
        K: AsRef<[u8]> + Send + 'static,
        V: AsRef<[u8]> + Send + 'static,
    {
        trace!(%cf_name, "Write async called");
        let dbs = self.dbs.clone();
        let cf_name = cf_name.to_string();

        let transaction = dbs.get(&cf_name).unwrap().begin_write().unwrap();
        {
            let mut tree = transaction.get_tree(b"").unwrap().unwrap();
            tree.insert(key.as_ref(), value.as_ref())
                .map_err(|e| StoreError::Custom(format!("CanopyDB write error: {}", e)))?;
        }
        transaction
            .commit()
            .map_err(|e| StoreError::Custom(format!("CanopyDB commit error: {}", e)))?;

        trace!(%cf_name, "Write async finished");
        Ok(())
    }

    fn write_batch_async(
        &self,
        batch_ops: Vec<(&'static str, Vec<u8>, Vec<u8>)>,
    ) -> Result<(), StoreError> {
        trace!("Write batch async called");
        let db = self.db.clone();
        let dbs = self.dbs.clone();
        let mut transactions = BTreeMap::new();

        for (db_name, key, value) in batch_ops {
            let transaction = transactions
                .entry(db_name)
                .or_insert_with_key(|name| dbs.get(*name).unwrap().begin_write().unwrap());

            let mut tree = transaction.get_tree(b"").unwrap().unwrap();
            if value.is_empty() {
                tree.delete(key.as_ref()).unwrap();
            } else {
                tree.insert(key.as_ref(), value.as_ref()).unwrap();
            }
        }
        let cfs_len = transactions.len();
        db.group_commit(transactions.into_values(), false).unwrap();
        trace!(%cfs_len, "Write batch async finished");
        Ok(())
    }

    fn write_fkv_batch(
        &self,
        last_written: Vec<u8>,
        batch: Vec<(Vec<u8>, Vec<u8>)>,
    ) -> Result<(), StoreError> {
        trace!("Write FKV batch called");
        let cf_misc = self.dbs.get(CF_MISC_VALUES).unwrap().begin_write().unwrap();
        let cf_flatkeyvalue = self
            .dbs
            .get(CF_FLATKEYVALUE)
            .unwrap()
            .begin_write()
            .unwrap();
        {
            let mut misc_tree = cf_misc.get_tree(b"").unwrap().unwrap();
            let mut flatkeyvalue_tree = cf_flatkeyvalue.get_tree(b"").unwrap().unwrap();
            for (key, value) in batch {
                flatkeyvalue_tree
                    .insert(key.as_ref(), value.as_ref())
                    .map_err(|e| StoreError::Custom(format!("CanopyDB write error: {}", e)))?;
            }
            misc_tree.insert(b"last_written", &last_written).unwrap();
        }
        self.db
            .group_commit([cf_flatkeyvalue, cf_misc], false)
            .unwrap();

        trace!("Write FKV batch finished");
        Ok(())
    }

    fn apply_update_batch(
        &self,
        parent_state_root: H256,
        update_batch: UpdateBatch,
    ) -> Result<(), StoreError> {
        trace!("Apply update batch called");
        let last_state_root = update_batch
            .blocks
            .last()
            .ok_or(StoreError::UpdateBatchNoBlocks)?
            .header
            .state_root;

        let _span = tracing::trace_span!("Block DB update").entered();

        let [
            cf_receipts,
            cf_codes,
            cf_block_numbers,
            cf_tx_locations,
            cf_headers,
            cf_bodies,
        ] = open_cfs(
            &self.dbs,
            [
                CF_RECEIPTS,
                CF_ACCOUNT_CODES,
                CF_BLOCK_NUMBERS,
                CF_TRANSACTION_LOCATIONS,
                CF_HEADERS,
                CF_BODIES,
            ],
        )?;

        let receipts_transaction = cf_receipts.begin_write().unwrap();
        let codes_transaction = cf_codes.begin_write().unwrap();
        let block_numbers_transaction = cf_block_numbers.begin_write().unwrap();
        let txl_transaction = cf_tx_locations.begin_write().unwrap();
        let headers_transaction = cf_headers.begin_write().unwrap();
        let bodies_transaction = cf_bodies.begin_write().unwrap();
        {
            let mut receipts_tree = receipts_transaction.get_tree(b"").unwrap().unwrap();
            let mut codes_tree = codes_transaction.get_tree(b"").unwrap().unwrap();
            let mut block_numbers_tree = block_numbers_transaction.get_tree(b"").unwrap().unwrap();
            let mut txl_tree = txl_transaction.get_tree(b"").unwrap().unwrap();
            let mut headers_tree = headers_transaction.get_tree(b"").unwrap().unwrap();
            let mut bodies_tree = bodies_transaction.get_tree(b"").unwrap().unwrap();

            let UpdateBatch {
                account_updates,
                storage_updates,
                ..
            } = update_batch;

            // Capacity one ensures sender just notifies and goes on
            let (notify_tx, notify_rx) = sync_channel(1);
            let wait_for_new_layer = notify_rx;
            self.trie_update_worker_tx
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
                headers_tree
                    .insert(hash_key_rlp.bytes(), header_value_rlp.bytes())
                    .unwrap();

                let hash_key: AccountCodeHashRLP = block_hash.into();
                let body_value = BlockBodyRLP::from_bytes(block.body.encode_to_vec());
                bodies_tree
                    .insert(hash_key.bytes(), body_value.bytes())
                    .unwrap();

                let hash_key = BlockHashRLP::from(block_hash).bytes().clone();
                block_numbers_tree
                    .insert(&hash_key, &block_number.to_le_bytes())
                    .unwrap();

                for (index, transaction) in block.body.transactions.iter().enumerate() {
                    let tx_hash = transaction.hash();
                    // Key: tx_hash + block_hash
                    let mut composite_key = Vec::with_capacity(64);
                    composite_key.extend_from_slice(tx_hash.as_bytes());
                    composite_key.extend_from_slice(block_hash.as_bytes());
                    let location_value = (block_number, block_hash, index as u64).encode_to_vec();
                    txl_tree.insert(&composite_key, &location_value).unwrap();
                }
            }

            for (block_hash, receipts) in update_batch.receipts {
                for (index, receipt) in receipts.into_iter().enumerate() {
                    let key = (block_hash, index as u64).encode_to_vec();
                    let value = receipt.encode_to_vec();
                    receipts_tree.insert(&key, &value).unwrap();
                }
            }

            for (code_hash, code) in update_batch.code_updates {
                let mut buf =
                    Vec::with_capacity(6 + code.bytecode.len() + 2 * code.jump_targets.len());
                code.bytecode.encode(&mut buf);
                code.jump_targets
                    .into_iter()
                    .flat_map(|t| t.to_le_bytes())
                    .collect::<Vec<u8>>()
                    .as_slice()
                    .encode(&mut buf);
                codes_tree.insert(&code_hash.0, &buf).unwrap();
            }

            // Wait for an updated top layer so every caller afterwards sees a consistent view.
            // Specifically, the next block produced MUST see this upper layer.
            wait_for_new_layer
                .recv()
                .map_err(|e| StoreError::Custom(format!("recv failed: {e}")))??;
        }
        // After top-level is added, we can make the rest of the changes visible.
        self.db
            .group_commit(
                [
                    codes_transaction,
                    block_numbers_transaction,
                    bodies_transaction,
                    headers_transaction,
                    receipts_transaction,
                    txl_transaction,
                ],
                false,
            )
            .map_err(|e| StoreError::Custom(format!("CanopyDB batch write error: {}", e)))?;

        trace!("Apply update batch finished");
        Ok(())
    }

    fn clear_snap_state(&self) -> Result<(), StoreError> {
        trace!("Clear snap state called");
        let dbs = self.dbs.clone();

        let cf = dbs
            .get(CF_SNAP_STATE)
            .ok_or_else(|| StoreError::Custom("Column family not found".to_string()))?;

        let tx = cf.begin_write().unwrap();
        tx.get_tree(b"").unwrap().unwrap().clear().unwrap();
        tx.commit().unwrap();
        trace!("Clear snap state finished");
        Ok(())
    }

    fn clear_fullsync_headers(&self) -> Result<(), StoreError> {
        trace!("Clear fullsync headers called");
        let dbs = self.dbs.clone();

        let cf = dbs
            .get(CF_FULLSYNC_HEADERS)
            .ok_or_else(|| StoreError::Custom("Column family not found".to_string()))?;

        let tx = cf.begin_write().unwrap();
        tx.get_tree(b"").unwrap().unwrap().clear().unwrap();
        tx.commit().unwrap();
        trace!("Clear fullsync headers finished");
        Ok(())
    }
}
