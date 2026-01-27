//! Hybrid backend using ethrex_db for state/storage tries and RocksDB for other data.
//!
//! This backend combines:
//! - ethrex_db's `Blockchain` for state trie and storage trie operations (hot + cold storage)
//! - RocksDB for blocks, headers, receipts, and other blockchain data
//!
//! The hybrid approach leverages ethrex_db's optimized trie storage (memory-mapped pages,
//! Copy-on-Write concurrency) while keeping RocksDB for data that benefits from its
//! compression and indexing capabilities.
//!
//! ## Key Translation
//!
//! ethrex uses nibble-based paths for trie keys:
//! - Account trie leaf: 65 nibbles = keccak256(address)[64] + terminator[1]
//! - Storage trie leaf: 131 nibbles = address_hash[64] + separator(17)[1] + slot_hash[64] + terminator[2]
//!
//! ethrex_db uses direct address/slot access:
//! - `get_finalized_account_by_hash(&[u8; 32])` for account queries
//! - `get_finalized_storage_by_hash(&[u8; 32], &[u8; 32])` for storage queries
//!
//! This module translates between these formats for leaf data queries.

use crate::api::tables::{
    ACCOUNT_FLATKEYVALUE, ACCOUNT_TRIE_NODES, STORAGE_FLATKEYVALUE, STORAGE_TRIE_NODES,
};
use crate::api::{
    PrefixResult, StorageBackend, StorageLockedView, StorageReadView, StorageWriteBatch,
};
use crate::error::StoreError;
use ethrex_db::chain::Blockchain;
use ethrex_db::store::{AccountData, PagedDb};
use primitive_types::U256;
use std::collections::HashMap;
use std::fmt;
use std::path::Path;
use std::sync::{Arc, RwLock};

use super::rocksdb::RocksDBBackend;

// =============================================================================
// Path Translation Utilities
// =============================================================================
//
// These utilities convert between ethrex's nibble-based trie keys and
// ethrex_db's native account/storage address format.

/// Represents a parsed nibble key from ethrex's trie.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedTrieKey {
    /// Account trie leaf: contains the keccak256 hash of the address (32 bytes).
    AccountLeaf { address_hash: [u8; 32] },
    /// Storage trie leaf: contains address hash and storage slot hash.
    StorageLeaf {
        address_hash: [u8; 32],
        slot_hash: [u8; 32],
    },
    /// Intermediate trie node (not a leaf).
    IntermediateNode,
    /// Invalid or unrecognized key format.
    Invalid,
}

/// Converts 64 nibbles to a 32-byte hash.
///
/// Each byte in the input represents a single nibble (0-15).
/// Two nibbles combine to form one output byte.
/// Returns None if the input is not exactly 64 nibbles or contains invalid values.
#[inline]
fn nibbles_to_hash(nibbles: &[u8]) -> Option<[u8; 32]> {
    if nibbles.len() != 64 {
        return None;
    }

    let mut result = [0u8; 32];
    for (i, chunk) in nibbles.chunks_exact(2).enumerate() {
        let high = chunk[0];
        let low = chunk[1];
        if high >= 16 || low >= 16 {
            return None;
        }
        result[i] = (high << 4) | low;
    }
    Some(result)
}

/// Parses an ethrex nibble key into its semantic components.
///
/// Key formats:
/// - Account leaf (ACCOUNT_FLATKEYVALUE): 65 nibbles
///   - nibbles[0..64]: keccak256(address) as nibbles
///   - nibbles[64]: terminator (16)
///
/// - Storage leaf (STORAGE_FLATKEYVALUE): 131 nibbles
///   - nibbles[0..64]: keccak256(address) as nibbles
///   - nibbles[64]: separator (17)
///   - nibbles[65..129]: keccak256(slot) as nibbles
///   - nibbles[129..131]: terminator
///
/// - Intermediate nodes: any other length
pub fn parse_trie_key(key: &[u8], table: &str) -> ParsedTrieKey {
    match table {
        t if t == ACCOUNT_FLATKEYVALUE => {
            // Account leaf: 65 nibbles = 64 nibbles for hash + 1 terminator
            if key.len() == 65 && key[64] == 16 {
                if let Some(address_hash) = nibbles_to_hash(&key[0..64]) {
                    return ParsedTrieKey::AccountLeaf { address_hash };
                }
            }
            // Might be an intermediate account trie node
            if key.len() < 65 {
                return ParsedTrieKey::IntermediateNode;
            }
            ParsedTrieKey::Invalid
        }
        t if t == STORAGE_FLATKEYVALUE => {
            // Storage leaf: 131 nibbles = 64 + 1 (separator) + 64 + 2 (terminator)
            // Note: actual format may vary, check for separator at position 64
            if key.len() == 131 && key[64] == 17 {
                if let (Some(address_hash), Some(slot_hash)) =
                    (nibbles_to_hash(&key[0..64]), nibbles_to_hash(&key[65..129]))
                {
                    return ParsedTrieKey::StorageLeaf {
                        address_hash,
                        slot_hash,
                    };
                }
            }
            // Might be an intermediate storage trie node
            if key.len() < 131 {
                return ParsedTrieKey::IntermediateNode;
            }
            ParsedTrieKey::Invalid
        }
        t if t == ACCOUNT_TRIE_NODES || t == STORAGE_TRIE_NODES => {
            // Trie node tables always contain intermediate nodes (or leaf nodes)
            // These are RLP-encoded trie nodes, not flat key-value pairs
            ParsedTrieKey::IntermediateNode
        }
        _ => ParsedTrieKey::Invalid,
    }
}

/// Encodes an ethrex_db Account for storage in the trie.
///
/// The trie stores account data as RLP-encoded [nonce, balance, storage_root, code_hash].
/// This matches Ethereum's account state trie format.
fn encode_account_for_trie(account: &ethrex_db::chain::Account) -> Vec<u8> {
    use ethrex_rlp::encode::RLPEncode;

    // Build the account RLP: [nonce, balance, storage_root, code_hash]
    let mut buf = Vec::new();

    // Calculate the header for a list with 4 elements
    let nonce_encoded = {
        let mut b = Vec::new();
        account.nonce.encode(&mut b);
        b
    };
    let balance_encoded = {
        let mut b = Vec::new();
        // Convert U256 to bytes for encoding
        let balance_bytes: [u8; 32] = account.balance.to_big_endian();
        // Trim leading zeros for canonical RLP
        let trimmed = trim_leading_zeros(&balance_bytes);
        trimmed.to_vec().encode(&mut b);
        b
    };
    let storage_root_encoded = {
        let mut b = Vec::new();
        account.storage_root.as_bytes().encode(&mut b);
        b
    };
    let code_hash_encoded = {
        let mut b = Vec::new();
        account.code_hash.as_bytes().encode(&mut b);
        b
    };

    let total_len = nonce_encoded.len()
        + balance_encoded.len()
        + storage_root_encoded.len()
        + code_hash_encoded.len();

    // RLP list header
    if total_len < 56 {
        buf.push(0xc0 + total_len as u8);
    } else {
        let len_bytes = encode_length(total_len);
        buf.push(0xf7 + len_bytes.len() as u8);
        buf.extend_from_slice(&len_bytes);
    }

    buf.extend_from_slice(&nonce_encoded);
    buf.extend_from_slice(&balance_encoded);
    buf.extend_from_slice(&storage_root_encoded);
    buf.extend_from_slice(&code_hash_encoded);

    buf
}

/// Encodes a storage value (U256) for storage in the trie.
///
/// Storage values are RLP-encoded U256 values.
fn encode_storage_value_for_trie(value: &U256) -> Vec<u8> {
    use ethrex_rlp::encode::RLPEncode;

    let mut buf = Vec::new();
    let value_bytes: [u8; 32] = value.to_big_endian();

    // Trim leading zeros for canonical RLP
    let trimmed = trim_leading_zeros(&value_bytes);
    trimmed.to_vec().encode(&mut buf);
    buf
}

/// Trims leading zeros from a byte slice.
fn trim_leading_zeros(bytes: &[u8]) -> &[u8] {
    let first_nonzero = bytes.iter().position(|&b| b != 0).unwrap_or(bytes.len());
    if first_nonzero == bytes.len() {
        // All zeros, return empty or single zero
        &[]
    } else {
        &bytes[first_nonzero..]
    }
}

/// Encodes a length as big-endian bytes (for RLP headers).
fn encode_length(len: usize) -> Vec<u8> {
    if len == 0 {
        return vec![];
    }
    let mut bytes = Vec::new();
    let mut n = len;
    while n > 0 {
        bytes.push((n & 0xff) as u8);
        n >>= 8;
    }
    bytes.reverse();
    bytes
}

/// Decodes an RLP-encoded storage value to a 32-byte array.
///
/// Storage values in the trie are stored as RLP-encoded bytes with leading
/// zeros trimmed. This function decodes them back to a padded 32-byte array.
fn decode_storage_value_from_rlp(rlp_bytes: &[u8]) -> [u8; 32] {
    use ethrex_rlp::decode::RLPDecode;

    // Empty input means zero value
    if rlp_bytes.is_empty() {
        return [0u8; 32];
    }

    // Try to decode as RLP bytes
    let decoded: Result<Vec<u8>, _> = Vec::<u8>::decode(rlp_bytes);

    let value_bytes = match decoded {
        Ok(bytes) => bytes,
        Err(_) => {
            // If RLP decoding fails, treat the input as raw bytes
            rlp_bytes.to_vec()
        }
    };

    // Pad to 32 bytes (right-aligned, big-endian)
    let mut result = [0u8; 32];
    let offset = 32usize.saturating_sub(value_bytes.len());
    let copy_len = value_bytes.len().min(32);
    result[offset..offset + copy_len].copy_from_slice(&value_bytes[..copy_len]);
    result
}

/// Tables that are handled by ethrex_db (state and storage tries).
const ETHREX_DB_TABLES: [&str; 4] = [
    ACCOUNT_TRIE_NODES,
    STORAGE_TRIE_NODES,
    ACCOUNT_FLATKEYVALUE,
    STORAGE_FLATKEYVALUE,
];

/// Check if a table should be routed to ethrex_db.
fn is_ethrex_db_table(table: &str) -> bool {
    ETHREX_DB_TABLES.contains(&table)
}

/// Hybrid backend combining ethrex_db and RocksDB.
///
/// State and storage trie operations are routed to ethrex_db's `Blockchain`,
/// while all other operations go to RocksDB.
pub struct EthrexDbBackend {
    /// ethrex_db blockchain for state/storage trie management.
    /// Handles hot (unfinalized) and cold (finalized) state storage.
    blockchain: Arc<RwLock<Blockchain>>,

    /// RocksDB backend for blocks, headers, receipts, and other data.
    auxiliary: Arc<RocksDBBackend>,

    /// In-memory cache for trie data written but not yet committed to ethrex_db.
    /// Maps table -> key -> value.
    /// This bridges the gap between ethrex's write batch model and ethrex_db's block model.
    pending_trie_writes: Arc<RwLock<HashMap<&'static str, HashMap<Vec<u8>, Vec<u8>>>>>,
}

impl fmt::Debug for EthrexDbBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EthrexDbBackend")
            .field("blockchain", &"<Blockchain>")
            .field("auxiliary", &self.auxiliary)
            .field("pending_trie_writes", &"<pending>")
            .finish()
    }
}

impl EthrexDbBackend {
    /// Opens or creates a hybrid backend at the given path.
    ///
    /// Creates:
    /// - `state.db` file for ethrex_db (PagedDb)
    /// - `auxiliary/` directory for RocksDB
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let path = path.as_ref();

        // Create parent directory if needed
        std::fs::create_dir_all(path)
            .map_err(|e| StoreError::Custom(format!("Failed to create data directory: {}", e)))?;

        // Paths for the two storage components
        let state_path = path.join("state.db");
        let auxiliary_path = path.join("auxiliary");

        std::fs::create_dir_all(&auxiliary_path).map_err(|e| {
            StoreError::Custom(format!("Failed to create auxiliary directory: {}", e))
        })?;

        // Open PagedDb for state storage (expects a file path)
        let paged_db = PagedDb::open(&state_path)
            .map_err(|e| StoreError::Custom(format!("Failed to open PagedDb: {}", e)))?;

        // Create Blockchain on top of PagedDb
        let blockchain = Blockchain::new(paged_db);

        // Open RocksDB for auxiliary storage
        let auxiliary = RocksDBBackend::open(&auxiliary_path)?;

        Ok(Self {
            blockchain: Arc::new(RwLock::new(blockchain)),
            auxiliary: Arc::new(auxiliary),
            pending_trie_writes: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    /// Returns a reference to the underlying blockchain for direct state operations.
    ///
    /// This is useful for higher-level operations that need to interact with
    /// ethrex_db's native API (e.g., forkchoice updates, state root computation).
    pub fn blockchain(&self) -> Arc<RwLock<Blockchain>> {
        self.blockchain.clone()
    }

    /// Returns a reference to the auxiliary RocksDB backend.
    pub fn auxiliary(&self) -> Arc<RocksDBBackend> {
        self.auxiliary.clone()
    }

    /// Flushes pending trie writes to the ethrex_db state trie.
    ///
    /// This method:
    /// 1. Takes and clears pending trie writes from memory
    /// 2. Parses leaf nodes (accounts and storage) from the pending writes
    /// 3. Inserts them into PagedStateTrie via ethrex-db's native API
    /// 4. Persists to PagedDb via persist_state_trie_checkpoint()
    ///
    /// Intermediate trie nodes (ACCOUNT_TRIE_NODES, STORAGE_TRIE_NODES) are discarded
    /// because ethrex-db recomputes the trie structure internally.
    ///
    /// Returns the number of leaf entries that were flushed.
    pub fn flush_pending_trie_writes(&self) -> Result<usize, StoreError> {
        use primitive_types::H256;

        let mut flushed = 0;

        // Take and clear pending writes atomically
        let pending = {
            let mut p = self.pending_trie_writes.write().map_err(|_| {
                StoreError::Custom("Failed to acquire write lock on pending trie writes".into())
            })?;
            std::mem::take(&mut *p)
        };

        // Check if there's anything to flush
        if pending.is_empty() {
            return Ok(0);
        }

        let blockchain = self
            .blockchain
            .write()
            .map_err(|_| StoreError::Custom("Failed to acquire write lock on blockchain".into()))?;
        let mut state_trie = blockchain.state_trie_mut();

        // Process ACCOUNT_FLATKEYVALUE leaves
        if let Some(accounts) = pending.get(ACCOUNT_FLATKEYVALUE) {
            for (key, value) in accounts {
                if let ParsedTrieKey::AccountLeaf { address_hash } =
                    parse_trie_key(key, ACCOUNT_FLATKEYVALUE)
                {
                    // Decode the RLP-encoded account data
                    let account_data = AccountData::decode(value);
                    state_trie.set_account_by_hash(&address_hash, account_data);
                    flushed += 1;
                }
            }
        }

        // Process STORAGE_FLATKEYVALUE leaves
        if let Some(storages) = pending.get(STORAGE_FLATKEYVALUE) {
            for (key, value) in storages {
                if let ParsedTrieKey::StorageLeaf {
                    address_hash,
                    slot_hash,
                } = parse_trie_key(key, STORAGE_FLATKEYVALUE)
                {
                    // Decode the RLP-encoded storage value to [u8; 32]
                    let storage_value = decode_storage_value_from_rlp(value);
                    state_trie
                        .storage_trie_by_hash(&address_hash)
                        .set_by_hash(&slot_hash, storage_value);
                    flushed += 1;
                }
            }
        }

        // Intermediate nodes (TRIE_NODES tables) are intentionally discarded.
        // ethrex-db recomputes the trie structure when computing the root hash.

        // Drop the trie lock before persisting
        drop(state_trie);

        // Persist checkpoint to disk (using block 0, hash zero as placeholder during snap sync)
        blockchain
            .persist_state_trie_checkpoint(0, H256::zero())
            .map_err(|e| {
                StoreError::Custom(format!("Failed to persist state trie checkpoint: {}", e))
            })?;

        tracing::info!("Flushed {} trie entries to ethrex-db", flushed);

        Ok(flushed)
    }
}

impl StorageBackend for EthrexDbBackend {
    fn clear_table(&self, table: &'static str) -> Result<(), StoreError> {
        if is_ethrex_db_table(table) {
            // Clear pending writes for this table
            let mut pending = self.pending_trie_writes.write().map_err(|_| {
                StoreError::Custom("Failed to acquire write lock on pending trie writes".into())
            })?;
            pending.remove(table);
            // Note: We don't clear ethrex_db state here as it's managed via finalization
            Ok(())
        } else {
            self.auxiliary.clear_table(table)
        }
    }

    fn begin_read(&self) -> Result<Box<dyn StorageReadView + '_>, StoreError> {
        let aux_read = self.auxiliary.begin_read()?;
        let pending = self.pending_trie_writes.read().map_err(|_| {
            StoreError::Custom("Failed to acquire read lock on pending trie writes".into())
        })?;

        Ok(Box::new(EthrexDbReadView {
            blockchain: self.blockchain.clone(),
            auxiliary: aux_read,
            pending_snapshot: pending.clone(),
        }))
    }

    fn begin_write(&self) -> Result<Box<dyn StorageWriteBatch + 'static>, StoreError> {
        let aux_write = self.auxiliary.begin_write()?;

        Ok(Box::new(EthrexDbWriteBatch {
            pending_trie_writes: self.pending_trie_writes.clone(),
            trie_batch: HashMap::new(),
            auxiliary: aux_write,
        }))
    }

    fn begin_locked(
        &self,
        table_name: &'static str,
    ) -> Result<Box<dyn StorageLockedView + 'static>, StoreError> {
        if is_ethrex_db_table(table_name) {
            // For trie tables, create a locked view that reads from pending writes + blockchain
            let pending = self.pending_trie_writes.read().map_err(|_| {
                StoreError::Custom("Failed to acquire read lock on pending trie writes".into())
            })?;

            Ok(Box::new(EthrexDbLockedView {
                blockchain: self.blockchain.clone(),
                table_name,
                pending_snapshot: pending.get(table_name).cloned().unwrap_or_default(),
            }))
        } else {
            self.auxiliary.begin_locked(table_name)
        }
    }

    fn create_checkpoint(&self, path: &Path) -> Result<(), StoreError> {
        // Create checkpoint for auxiliary RocksDB
        let aux_checkpoint_path = path.join("auxiliary");
        self.auxiliary.create_checkpoint(&aux_checkpoint_path)?;

        // Note: ethrex_db has its own snapshot mechanism via PagedDb::create_snapshot()
        // For now, we rely on finalization for state durability
        Ok(())
    }

    fn flush_pending_writes(&self) -> Result<usize, StoreError> {
        self.flush_pending_trie_writes()
    }
}

/// Read view for the hybrid backend.
pub struct EthrexDbReadView<'a> {
    /// Reference to the blockchain for state reads.
    blockchain: Arc<RwLock<Blockchain>>,
    /// Auxiliary read view for non-trie data.
    auxiliary: Box<dyn StorageReadView + 'a>,
    /// Snapshot of pending trie writes at the time of view creation.
    pending_snapshot: HashMap<&'static str, HashMap<Vec<u8>, Vec<u8>>>,
}

impl<'a> StorageReadView for EthrexDbReadView<'a> {
    fn get(&self, table: &'static str, key: &[u8]) -> Result<Option<Vec<u8>>, StoreError> {
        if is_ethrex_db_table(table) {
            // First check pending writes
            if let Some(table_data) = self.pending_snapshot.get(table) {
                if let Some(value) = table_data.get(key) {
                    return Ok(Some(value.clone()));
                }
            }

            // Try to parse the key and query ethrex_db for leaf data
            match parse_trie_key(key, table) {
                ParsedTrieKey::AccountLeaf { address_hash } => {
                    // Query ethrex_db for account data using the hashed address
                    let blockchain = self.blockchain.read().map_err(|_| {
                        StoreError::Custom("Failed to acquire read lock on blockchain".into())
                    })?;

                    if let Some(account) = blockchain.get_finalized_account_by_hash(&address_hash) {
                        // Encode account data for ethrex's trie format
                        // ethrex uses RLP encoding for account state
                        let encoded = encode_account_for_trie(&account);
                        return Ok(Some(encoded));
                    }
                    Ok(None)
                }
                ParsedTrieKey::StorageLeaf {
                    address_hash,
                    slot_hash,
                } => {
                    // Query ethrex_db for storage value
                    let blockchain = self.blockchain.read().map_err(|_| {
                        StoreError::Custom("Failed to acquire read lock on blockchain".into())
                    })?;

                    if let Some(value) =
                        blockchain.get_finalized_storage_by_hash(&address_hash, &slot_hash)
                    {
                        // Storage values are encoded as RLP(U256)
                        let encoded = encode_storage_value_for_trie(&value);
                        return Ok(Some(encoded));
                    }
                    Ok(None)
                }
                ParsedTrieKey::IntermediateNode => {
                    // Intermediate nodes are not stored in ethrex_db
                    // They exist only in the pending writes or need to be reconstructed
                    Ok(None)
                }
                ParsedTrieKey::Invalid => {
                    // Unknown key format, return None
                    Ok(None)
                }
            }
        } else {
            self.auxiliary.get(table, key)
        }
    }

    fn prefix_iterator(
        &self,
        table: &'static str,
        prefix: &[u8],
    ) -> Result<Box<dyn Iterator<Item = PrefixResult> + '_>, StoreError> {
        if is_ethrex_db_table(table) {
            // Return iterator over pending writes matching prefix
            let results: Vec<PrefixResult> = self
                .pending_snapshot
                .get(table)
                .map(|table_data| {
                    table_data
                        .iter()
                        .filter(|(k, _)| k.starts_with(prefix))
                        .map(|(k, v)| {
                            Ok((k.clone().into_boxed_slice(), v.clone().into_boxed_slice()))
                        })
                        .collect()
                })
                .unwrap_or_default();

            Ok(Box::new(results.into_iter()))
        } else {
            self.auxiliary.prefix_iterator(table, prefix)
        }
    }
}

/// Write batch for the hybrid backend.
pub struct EthrexDbWriteBatch {
    /// Reference to the shared pending trie writes.
    pending_trie_writes: Arc<RwLock<HashMap<&'static str, HashMap<Vec<u8>, Vec<u8>>>>>,
    /// Local batch of trie writes to be merged on commit.
    trie_batch: HashMap<&'static str, Vec<(Vec<u8>, Vec<u8>)>>,
    /// Auxiliary write batch for non-trie data.
    auxiliary: Box<dyn StorageWriteBatch + 'static>,
}

impl StorageWriteBatch for EthrexDbWriteBatch {
    fn put(&mut self, table: &'static str, key: &[u8], value: &[u8]) -> Result<(), StoreError> {
        if is_ethrex_db_table(table) {
            self.trie_batch
                .entry(table)
                .or_default()
                .push((key.to_vec(), value.to_vec()));
            Ok(())
        } else {
            self.auxiliary.put(table, key, value)
        }
    }

    fn put_batch(
        &mut self,
        table: &'static str,
        batch: Vec<(Vec<u8>, Vec<u8>)>,
    ) -> Result<(), StoreError> {
        if is_ethrex_db_table(table) {
            self.trie_batch.entry(table).or_default().extend(batch);
            Ok(())
        } else {
            self.auxiliary.put_batch(table, batch)
        }
    }

    fn delete(&mut self, table: &'static str, key: &[u8]) -> Result<(), StoreError> {
        if is_ethrex_db_table(table) {
            // For deletes, we could store a tombstone or handle differently
            // For now, just remove from pending writes if present
            let mut pending = self.pending_trie_writes.write().map_err(|_| {
                StoreError::Custom("Failed to acquire write lock on pending trie writes".into())
            })?;
            if let Some(table_data) = pending.get_mut(table) {
                table_data.remove(key);
            }
            Ok(())
        } else {
            self.auxiliary.delete(table, key)
        }
    }

    fn commit(&mut self) -> Result<(), StoreError> {
        // Commit auxiliary writes to RocksDB
        self.auxiliary.commit()?;

        // Merge trie batch into pending writes
        if !self.trie_batch.is_empty() {
            let mut pending = self.pending_trie_writes.write().map_err(|_| {
                StoreError::Custom("Failed to acquire write lock on pending trie writes".into())
            })?;

            for (table, entries) in self.trie_batch.drain() {
                let table_data = pending.entry(table).or_default();
                for (key, value) in entries {
                    table_data.insert(key, value);
                }
            }
        }

        Ok(())
    }
}

/// Locked view for trie tables in the hybrid backend.
pub struct EthrexDbLockedView {
    /// Reference to the blockchain for state reads.
    blockchain: Arc<RwLock<Blockchain>>,
    /// The table this view is locked to.
    table_name: &'static str,
    /// Snapshot of pending writes for this table.
    pending_snapshot: HashMap<Vec<u8>, Vec<u8>>,
}

impl StorageLockedView for EthrexDbLockedView {
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, StoreError> {
        // First check pending writes
        if let Some(value) = self.pending_snapshot.get(key) {
            return Ok(Some(value.clone()));
        }

        // Try to parse the key and query ethrex_db for leaf data
        match parse_trie_key(key, self.table_name) {
            ParsedTrieKey::AccountLeaf { address_hash } => {
                let blockchain = self.blockchain.read().map_err(|_| {
                    StoreError::Custom("Failed to acquire read lock on blockchain".into())
                })?;

                if let Some(account) = blockchain.get_finalized_account_by_hash(&address_hash) {
                    let encoded = encode_account_for_trie(&account);
                    return Ok(Some(encoded));
                }
                Ok(None)
            }
            ParsedTrieKey::StorageLeaf {
                address_hash,
                slot_hash,
            } => {
                let blockchain = self.blockchain.read().map_err(|_| {
                    StoreError::Custom("Failed to acquire read lock on blockchain".into())
                })?;

                if let Some(value) =
                    blockchain.get_finalized_storage_by_hash(&address_hash, &slot_hash)
                {
                    let encoded = encode_storage_value_for_trie(&value);
                    return Ok(Some(encoded));
                }
                Ok(None)
            }
            ParsedTrieKey::IntermediateNode | ParsedTrieKey::Invalid => {
                // Intermediate nodes are not stored in ethrex_db
                Ok(None)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_hybrid_backend_creation() {
        let temp_dir = TempDir::new().unwrap();
        let backend = EthrexDbBackend::open(temp_dir.path()).unwrap();

        // Verify storage locations were created
        assert!(temp_dir.path().join("state.db").exists());
        assert!(temp_dir.path().join("auxiliary").exists());

        // Verify we can access both components
        let _blockchain = backend.blockchain();
        let _auxiliary = backend.auxiliary();
    }

    #[test]
    fn test_table_routing() {
        assert!(is_ethrex_db_table(ACCOUNT_TRIE_NODES));
        assert!(is_ethrex_db_table(STORAGE_TRIE_NODES));
        assert!(is_ethrex_db_table(ACCOUNT_FLATKEYVALUE));
        assert!(is_ethrex_db_table(STORAGE_FLATKEYVALUE));
        assert!(!is_ethrex_db_table("headers"));
        assert!(!is_ethrex_db_table("bodies"));
    }

    #[test]
    fn test_write_and_read_auxiliary() {
        let temp_dir = TempDir::new().unwrap();
        let backend = EthrexDbBackend::open(temp_dir.path()).unwrap();

        // Write to auxiliary table (headers)
        {
            let mut tx = backend.begin_write().unwrap();
            tx.put("headers", b"key1", b"value1").unwrap();
            tx.commit().unwrap();
        }

        // Read back
        {
            let tx = backend.begin_read().unwrap();
            let value = tx.get("headers", b"key1").unwrap();
            assert_eq!(value, Some(b"value1".to_vec()));
        }
    }

    #[test]
    fn test_write_and_read_trie_pending() {
        let temp_dir = TempDir::new().unwrap();
        let backend = EthrexDbBackend::open(temp_dir.path()).unwrap();

        // Write to trie table (goes to pending writes)
        {
            let mut tx = backend.begin_write().unwrap();
            tx.put(ACCOUNT_TRIE_NODES, b"trie_key", b"trie_value")
                .unwrap();
            tx.commit().unwrap();
        }

        // Read back from pending
        {
            let tx = backend.begin_read().unwrap();
            let value = tx.get(ACCOUNT_TRIE_NODES, b"trie_key").unwrap();
            assert_eq!(value, Some(b"trie_value".to_vec()));
        }
    }

    // =========================================================================
    // Path Translation Tests
    // =========================================================================

    #[test]
    fn test_nibbles_to_hash() {
        // Test with exactly 64 nibbles (produces 32-byte hash)
        let mut nibbles = [0u8; 64];
        for i in 0..64 {
            nibbles[i] = (i % 16) as u8;
        }
        let hash = nibbles_to_hash(&nibbles).unwrap();
        // First two nibbles (0, 1) should become byte 0x01
        assert_eq!(hash[0], 0x01);
        // Nibbles (2, 3) should become byte 0x23
        assert_eq!(hash[1], 0x23);
        // Last two nibbles (14, 15) % 16 = (14, 15) should become 0xef
        assert_eq!(hash[31], 0xef);

        // Test with wrong length (not 64 nibbles)
        let short_nibbles = [0u8; 32];
        assert!(nibbles_to_hash(&short_nibbles).is_none());

        // Test with invalid nibble values (>= 16)
        let mut invalid_nibbles = [0u8; 64];
        invalid_nibbles[0] = 16; // Invalid nibble
        assert!(nibbles_to_hash(&invalid_nibbles).is_none());
    }

    #[test]
    fn test_parse_account_leaf_key() {
        // Create a valid account leaf key: 64 nibbles of address hash + terminator (16)
        let mut key = vec![0u8; 65];
        // Fill with a recognizable pattern: 0x0102...1f20
        for i in 0..64 {
            key[i] = (i % 16) as u8;
        }
        key[64] = 16; // terminator

        let parsed = parse_trie_key(&key, ACCOUNT_FLATKEYVALUE);

        match parsed {
            ParsedTrieKey::AccountLeaf { address_hash } => {
                // Verify the hash was correctly converted from nibbles
                // First two nibbles (0, 1) should become byte 0x01
                assert_eq!(address_hash[0], 0x01);
                assert_eq!(address_hash[1], 0x23);
            }
            _ => panic!("Expected AccountLeaf, got {:?}", parsed),
        }
    }

    #[test]
    fn test_parse_storage_leaf_key() {
        // Create a valid storage leaf key:
        // 64 nibbles (address hash) + separator (17) + 64 nibbles (slot hash) + 2 terminators
        let mut key = vec![0u8; 131];
        // Address hash nibbles
        for i in 0..64 {
            key[i] = (i % 16) as u8;
        }
        // Separator
        key[64] = 17;
        // Slot hash nibbles
        for i in 65..129 {
            key[i] = ((i - 65) % 16) as u8;
        }
        // Terminators
        key[129] = 16;
        key[130] = 0;

        let parsed = parse_trie_key(&key, STORAGE_FLATKEYVALUE);

        match parsed {
            ParsedTrieKey::StorageLeaf {
                address_hash,
                slot_hash,
            } => {
                // Verify both hashes were correctly converted
                assert_eq!(address_hash[0], 0x01);
                assert_eq!(slot_hash[0], 0x01);
            }
            _ => panic!("Expected StorageLeaf, got {:?}", parsed),
        }
    }

    #[test]
    fn test_parse_intermediate_node() {
        // Intermediate nodes have keys shorter than leaf keys
        let short_key = vec![0x01, 0x02, 0x03, 0x04];

        let parsed = parse_trie_key(&short_key, ACCOUNT_FLATKEYVALUE);
        assert_eq!(parsed, ParsedTrieKey::IntermediateNode);

        let parsed = parse_trie_key(&short_key, STORAGE_FLATKEYVALUE);
        assert_eq!(parsed, ParsedTrieKey::IntermediateNode);

        // Trie node tables always return IntermediateNode
        let parsed = parse_trie_key(&short_key, ACCOUNT_TRIE_NODES);
        assert_eq!(parsed, ParsedTrieKey::IntermediateNode);
    }

    #[test]
    fn test_trim_leading_zeros() {
        assert_eq!(trim_leading_zeros(&[0, 0, 0, 1, 2, 3]), &[1, 2, 3]);
        assert_eq!(trim_leading_zeros(&[1, 2, 3]), &[1, 2, 3]);
        assert_eq!(trim_leading_zeros(&[0, 0, 0]), &[] as &[u8]);
        assert_eq!(trim_leading_zeros(&[]), &[] as &[u8]);
    }

    #[test]
    fn test_encode_length() {
        assert_eq!(encode_length(0), vec![] as Vec<u8>);
        assert_eq!(encode_length(1), vec![1]);
        assert_eq!(encode_length(255), vec![255]);
        assert_eq!(encode_length(256), vec![1, 0]);
        assert_eq!(encode_length(65535), vec![255, 255]);
    }
}
