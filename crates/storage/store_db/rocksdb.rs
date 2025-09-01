use bytes::Bytes;
use ethrex_common::{
    H256, U256,
    types::{
        Block, BlockBody, BlockHash, BlockHeader, BlockNumber, ChainConfig, Index, Receipt,
        Transaction, payload::PayloadBundle,
    },
};
use ethrex_trie::{Nibbles, NodeHash, Trie};
use rocksdb::{ColumnFamily, ColumnFamilyDescriptor, DB, Options, WriteBatch};
use std::sync::Arc;

use crate::{
    STATE_TRIE_SEGMENTS, UpdateBatch,
    api::StoreEngine,
    error::StoreError,
    rlp::{BlockBodyRLP, BlockHashRLP, BlockHeaderRLP},
};
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};
use std::fmt::Debug;

// Column Family names - matching libmdbx tables
const CF_CANONICAL_BLOCK_HASHES: &str = "canonical_block_hashes";
const CF_BLOCK_NUMBERS: &str = "block_numbers";
const CF_HEADERS: &str = "headers";
const CF_BODIES: &str = "bodies";
const CF_ACCOUNT_CODES: &str = "account_codes";
const CF_RECEIPTS: &str = "receipts";
const CF_TRANSACTION_LOCATIONS: &str = "transaction_locations";
const CF_CHAIN_DATA: &str = "chain_data";
const CF_SNAP_STATE: &str = "snap_state";
const CF_STATE_TRIE_NODES: &str = "state_trie_nodes";
const CF_STORAGE_TRIES_NODES: &str = "storage_tries_nodes";
const CF_PAYLOADS: &str = "payloads";
const CF_PENDING_BLOCKS: &str = "pending_blocks";
const CF_STORAGE_SNAPSHOT: &str = "storage_snapshot";
const CF_INVALID_ANCESTORS: &str = "invalid_ancestors";

#[derive(Debug)]
pub struct Store {
    db: Arc<DB>,
}

impl Store {
    pub fn new(path: &str) -> Result<Self, StoreError> {
        let mut db_options = Options::default();
        db_options.create_if_missing(true);
        db_options.create_missing_column_families(true);

        // Performance configurations similar to libmdbx setup
        db_options.set_max_open_files(1000);
        db_options.set_use_fsync(false);
        db_options.set_bytes_per_sync(1048576);
        db_options.set_disable_auto_compactions(false);

        // Column families matching libmdbx tables
        let column_families = vec![
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
            CF_PAYLOADS,
            CF_PENDING_BLOCKS,
            CF_STORAGE_SNAPSHOT,
            CF_INVALID_ANCESTORS,
        ];

        let mut cf_descriptors = Vec::new();
        for cf_name in column_families {
            let mut cf_opts = Options::default();
            cf_opts.set_compression_type(rocksdb::DBCompressionType::Lz4);
            cf_descriptors.push(ColumnFamilyDescriptor::new(cf_name, cf_opts));
        }

        let db = DB::open_cf_descriptors(&db_options, path, cf_descriptors)
            .map_err(|e| StoreError::Custom(format!("Failed to open RocksDB: {}", e)))?;

        Ok(Self { db: Arc::new(db) })
    }

    // Helper method to get column family handle
    fn cf_handle(&self, cf_name: &str) -> Result<&ColumnFamily, StoreError> {
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
            db.put_cf(cf, key, value)
                .map_err(|e| StoreError::Custom(format!("RocksDB write error: {}", e)))
        })
        .await
        .map_err(|e| StoreError::Custom(format!("Task panicked: {}", e)))?
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
            db.get_cf(cf, key)
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
            .get_cf(cf, key)
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
                batch.put_cf(cf, key, value);
            }

            db.write(batch)
                .map_err(|e| StoreError::Custom(format!("RocksDB batch write error: {}", e)))
        })
        .await
        .map_err(|e| StoreError::Custom(format!("Task panicked: {}", e)))?
    }

    // Helper method to add canonical block hash (like libmdbx does implicitly)
    async fn add_canonical_block_hash(
        &self,
        block_number: BlockNumber,
        block_hash: BlockHash,
    ) -> Result<(), StoreError> {
        let number_key = block_number.encode_to_vec();
        let hash_value = BlockHashRLP::from(block_hash).bytes().clone();
        self.write_async(CF_CANONICAL_BLOCK_HASHES, number_key, hash_value)
            .await
    }
}

#[async_trait::async_trait]
impl StoreEngine for Store {
    async fn apply_updates(&self, update_batch: UpdateBatch) -> Result<(), StoreError> {
        todo!()
    }

    /// Add a batch of blocks in a single transaction.
    /// This will store -> BlockHeader, BlockBody, BlockTransactions, BlockNumber.
    async fn add_blocks(&self, blocks: Vec<Block>) -> Result<(), StoreError> {
        todo!()
    }

    /// Add block header
    async fn add_block_header(
        &self,
        block_hash: BlockHash,
        block_header: BlockHeader,
    ) -> Result<(), StoreError> {
        let hash_key = BlockHashRLP::from(block_hash).bytes().clone();
        let header_value = BlockHeaderRLP::from(block_header).bytes().clone();
        self.write_async(CF_HEADERS, hash_key, header_value).await
    }

    /// Add a batch of block headers
    async fn add_block_headers(&self, block_headers: Vec<BlockHeader>) -> Result<(), StoreError> {
        let mut batch_ops = Vec::new();

        for header in block_headers {
            let block_hash = header.hash();
            let hash_key = BlockHashRLP::from(block_hash).bytes().clone();
            let header_value = BlockHeaderRLP::from(header.clone()).bytes().clone();

            batch_ops.push((CF_HEADERS.to_string(), hash_key, header_value));

            // Also add the block number mapping
            let number_key = header.number.encode_to_vec();
            batch_ops.push((
                CF_BLOCK_NUMBERS.to_string(),
                BlockHashRLP::from(block_hash).bytes().clone(),
                number_key,
            ));
        }

        self.write_batch_async(batch_ops).await
    }

    /// Obtain canonical block header
    fn get_block_header(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<BlockHeader>, StoreError> {
        // First get the canonical hash for this block number
        let Some(block_hash) = self.get_canonical_block_hash_sync(block_number)? else {
            return Ok(None);
        };

        // Then get the header using the hash
        self.get_block_header_by_hash(block_hash)
    }

    /// Add block body
    async fn add_block_body(
        &self,
        block_hash: BlockHash,
        block_body: BlockBody,
    ) -> Result<(), StoreError> {
        let hash_key = BlockHashRLP::from(block_hash).bytes().clone();
        let body_value = BlockBodyRLP::from(block_body).bytes().clone();
        self.write_async(CF_BODIES, hash_key, body_value).await
    }

    /// Obtain canonical block body
    async fn get_block_body(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<BlockBody>, StoreError> {
        // First get the canonical hash for this block number
        let Some(block_hash) = self.get_canonical_block_hash_sync(block_number)? else {
            return Ok(None);
        };

        // Then get the body using the hash
        self.get_block_body_by_hash(block_hash).await
    }

    /// Remove canonical block
    async fn remove_block(&self, block_number: BlockNumber) -> Result<(), StoreError> {
        todo!()
    }

    /// Obtain canonical block bodies in from..=to
    async fn get_block_bodies(
        &self,
        from: BlockNumber,
        to: BlockNumber,
    ) -> Result<Vec<BlockBody>, StoreError> {
        todo!()
    }

    /// Obtain block bodies from a list of hashes
    async fn get_block_bodies_by_hash(
        &self,
        hashes: Vec<BlockHash>,
    ) -> Result<Vec<BlockBody>, StoreError> {
        todo!()
    }

    /// Obtain any block body using the hash
    async fn get_block_body_by_hash(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockBody>, StoreError> {
        let hash_key = BlockHashRLP::from(block_hash).bytes().clone();

        match self.read_async(CF_BODIES, hash_key).await? {
            Some(body_bytes) => {
                let body_rlp = BlockBodyRLP::from_bytes(body_bytes);
                body_rlp.to().map(Some).map_err(StoreError::from)
            }
            None => Ok(None),
        }
    }

    fn get_block_header_by_hash(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockHeader>, StoreError> {
        let hash_key = BlockHashRLP::from(block_hash).bytes().clone();

        match self.read_sync(CF_HEADERS, hash_key)? {
            Some(header_bytes) => {
                let header_rlp = BlockHeaderRLP::from_bytes(header_bytes);
                header_rlp.to().map(Some).map_err(StoreError::from)
            }
            None => Ok(None),
        }
    }

    async fn add_pending_block(&self, block: Block) -> Result<(), StoreError> {
        todo!()
    }
    async fn get_pending_block(&self, block_hash: BlockHash) -> Result<Option<Block>, StoreError> {
        todo!()
    }

    /// Add block number for a given hash
    async fn add_block_number(
        &self,
        block_hash: BlockHash,
        block_number: BlockNumber,
    ) -> Result<(), StoreError> {
        let hash_key = BlockHashRLP::from(block_hash).bytes().clone();
        let number_value = block_number.encode_to_vec();
        self.write_async(CF_BLOCK_NUMBERS, hash_key, number_value)
            .await
    }

    /// Obtain block number for a given hash
    async fn get_block_number(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockNumber>, StoreError> {
        let hash_key = BlockHashRLP::from(block_hash).bytes().clone();

        match self.read_async(CF_BLOCK_NUMBERS, hash_key).await? {
            Some(number_bytes) => BlockNumber::decode(number_bytes.as_slice())
                .map(Some)
                .map_err(StoreError::from),
            None => Ok(None),
        }
    }

    /// Store transaction location (block number and index of the transaction within the block)
    async fn add_transaction_location(
        &self,
        transaction_hash: H256,
        block_number: BlockNumber,
        block_hash: BlockHash,
        index: Index,
    ) -> Result<(), StoreError> {
        todo!()
    }

    /// Store transaction locations in batch (one db transaction for all)
    async fn add_transaction_locations(
        &self,
        locations: Vec<(H256, BlockNumber, BlockHash, Index)>,
    ) -> Result<(), StoreError> {
        todo!()
    }

    /// Obtain transaction location (block hash and index)
    async fn get_transaction_location(
        &self,
        transaction_hash: H256,
    ) -> Result<Option<(BlockNumber, BlockHash, Index)>, StoreError> {
        todo!()
    }

    /// Add receipt
    async fn add_receipt(
        &self,
        block_hash: BlockHash,
        index: Index,
        receipt: Receipt,
    ) -> Result<(), StoreError> {
        todo!()
    }

    /// Add receipts
    async fn add_receipts(
        &self,
        block_hash: BlockHash,
        receipts: Vec<Receipt>,
    ) -> Result<(), StoreError> {
        todo!()
    }

    /// Obtain receipt for a canonical block represented by the block number.
    async fn get_receipt(
        &self,
        block_number: BlockNumber,
        index: Index,
    ) -> Result<Option<Receipt>, StoreError> {
        todo!()
    }

    /// Add account code
    async fn add_account_code(&self, code_hash: H256, code: Bytes) -> Result<(), StoreError> {
        todo!()
    }

    /// Clears all checkpoint data created during the last snap sync
    async fn clear_snap_state(&self) -> Result<(), StoreError> {
        todo!()
    }

    /// Obtain account code via code hash
    fn get_account_code(&self, code_hash: H256) -> Result<Option<Bytes>, StoreError> {
        todo!()
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

    async fn get_block_by_number(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<Block>, StoreError> {
        let Some(block_hash) = self.get_canonical_block_hash(block_number).await? else {
            return Ok(None);
        };
        self.get_block_by_hash(block_hash).await
    }

    // Get the canonical block hash for a given block number.
    async fn get_canonical_block_hash(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<BlockHash>, StoreError> {
        let number_key = block_number.encode_to_vec();

        match self
            .read_async(CF_CANONICAL_BLOCK_HASHES, number_key)
            .await?
        {
            Some(hash_bytes) => {
                let hash_rlp = BlockHashRLP::from_bytes(hash_bytes);
                hash_rlp.to().map(Some).map_err(StoreError::from)
            }
            None => Ok(None),
        }
    }

    /// Stores the chain configuration values, should only be called once after reading the genesis file
    /// Ignores previously stored values if present
    async fn set_chain_config(&self, chain_config: &ChainConfig) -> Result<(), StoreError> {
        todo!()
    }

    /// Update earliest block number
    async fn update_earliest_block_number(
        &self,
        block_number: BlockNumber,
    ) -> Result<(), StoreError> {
        todo!()
    }

    /// Obtain earliest block number
    async fn get_earliest_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        todo!()
    }

    /// Obtain finalized block number
    async fn get_finalized_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        todo!()
    }

    /// Obtain safe block number
    async fn get_safe_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        todo!()
    }

    /// Obtain latest block number
    async fn get_latest_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        todo!()
    }

    /// Update pending block number
    async fn update_pending_block_number(
        &self,
        block_number: BlockNumber,
    ) -> Result<(), StoreError> {
        todo!()
    }

    /// Obtain pending block number
    async fn get_pending_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        todo!()
    }

    /// Obtain a storage trie from the given address and storage_root
    /// Doesn't check if the account is stored
    /// Used for internal store operations
    fn open_storage_trie(
        &self,
        hashed_address: H256,
        storage_root: H256,
    ) -> Result<Trie, StoreError> {
        todo!()
    }

    /// Obtain a state trie from the given state root
    /// Doesn't check if the state root is valid
    /// Used for internal store operations
    fn open_state_trie(&self, state_root: H256) -> Result<Trie, StoreError> {
        todo!()
    }

    /// Obtain a state trie locked for reads from the given state root
    /// Doesn't check if the state root is valid
    /// Used for internal store operations
    fn open_locked_state_trie(&self, state_root: H256) -> Result<Trie, StoreError> {
        todo!()
    }

    /// Obtain a read-locked storage trie from the given address and storage_root
    /// Doesn't check if the account is stored
    /// Used for internal store operations
    fn open_locked_storage_trie(
        &self,
        hashed_address: H256,
        storage_root: H256,
    ) -> Result<Trie, StoreError> {
        self.open_storage_trie(hashed_address, storage_root)
    }

    async fn forkchoice_update(
        &self,
        new_canonical_blocks: Option<Vec<(BlockNumber, BlockHash)>>,
        head_number: BlockNumber,
        head_hash: BlockHash,
        safe: Option<BlockNumber>,
        finalized: Option<BlockNumber>,
    ) -> Result<(), StoreError> {
        todo!()
    }

    async fn add_payload(&self, payload_id: u64, block: Block) -> Result<(), StoreError> {
        todo!()
    }

    async fn get_payload(&self, payload_id: u64) -> Result<Option<PayloadBundle>, StoreError> {
        todo!()
    }

    async fn update_payload(
        &self,
        payload_id: u64,
        payload: PayloadBundle,
    ) -> Result<(), StoreError> {
        todo!()
    }

    fn get_receipts_for_block(&self, block_hash: &BlockHash) -> Result<Vec<Receipt>, StoreError> {
        todo!()
    }

    // Snap State methods

    /// Sets the hash of the last header downloaded during a snap sync
    async fn set_header_download_checkpoint(
        &self,
        block_hash: BlockHash,
    ) -> Result<(), StoreError> {
        todo!()
    }

    /// Gets the hash of the last header downloaded during a snap sync
    async fn get_header_download_checkpoint(&self) -> Result<Option<BlockHash>, StoreError> {
        todo!()
    }

    /// Sets the last key fetched from the state trie being fetched during snap sync
    async fn set_state_trie_key_checkpoint(
        &self,
        last_keys: [H256; STATE_TRIE_SEGMENTS],
    ) -> Result<(), StoreError> {
        todo!()
    }

    /// Gets the last key fetched from the state trie being fetched during snap sync
    async fn get_state_trie_key_checkpoint(
        &self,
    ) -> Result<Option<[H256; STATE_TRIE_SEGMENTS]>, StoreError> {
        todo!()
    }

    /// Sets the state trie paths in need of healing
    async fn set_state_heal_paths(&self, paths: Vec<(Nibbles, H256)>) -> Result<(), StoreError> {
        todo!()
    }

    /// Gets the state trie paths in need of healing
    async fn get_state_heal_paths(&self) -> Result<Option<Vec<(Nibbles, H256)>>, StoreError> {
        todo!()
    }

    /// Write a storage batch into the current storage snapshot
    async fn write_snapshot_storage_batch(
        &self,
        account_hash: H256,
        storage_keys: Vec<H256>,
        storage_values: Vec<U256>,
    ) -> Result<(), StoreError> {
        todo!()
    }

    /// Write multiple storage batches belonging to different accounts into the current storage snapshot
    async fn write_snapshot_storage_batches(
        &self,
        account_hashes: Vec<H256>,
        storage_keys: Vec<Vec<H256>>,
        storage_values: Vec<Vec<U256>>,
    ) -> Result<(), StoreError> {
        todo!()
    }

    /// Set the latest root of the rebuilt state trie and the last downloaded hashes from each segment
    async fn set_state_trie_rebuild_checkpoint(
        &self,
        checkpoint: (H256, [H256; STATE_TRIE_SEGMENTS]),
    ) -> Result<(), StoreError> {
        todo!()
    }

    /// Get the latest root of the rebuilt state trie and the last downloaded hashes from each segment
    async fn get_state_trie_rebuild_checkpoint(
        &self,
    ) -> Result<Option<(H256, [H256; STATE_TRIE_SEGMENTS])>, StoreError> {
        todo!()
    }

    /// Get the accont hashes and roots of the storage tries awaiting rebuild
    async fn set_storage_trie_rebuild_pending(
        &self,
        pending: Vec<(H256, H256)>,
    ) -> Result<(), StoreError> {
        todo!()
    }

    /// Get the accont hashes and roots of the storage tries awaiting rebuild
    async fn get_storage_trie_rebuild_pending(
        &self,
    ) -> Result<Option<Vec<(H256, H256)>>, StoreError> {
        todo!()
    }

    /// Reads the next `MAX_SNAPSHOT_READS` elements from the storage snapshot as from the `start` storage key
    async fn read_storage_snapshot(
        &self,
        start: H256,
        account_hash: H256,
    ) -> Result<Vec<(H256, U256)>, StoreError> {
        todo!()
    }

    /// The `forkchoice_update` and `new_payload` methods require the `latest_valid_hash`
    /// when processing an invalid payload. To provide this, we must track invalid chains.
    ///
    /// We only store the last known valid head upon encountering a bad block,
    /// rather than tracking every subsequent invalid block.
    async fn set_latest_valid_ancestor(
        &self,
        bad_block: BlockHash,
        latest_valid: BlockHash,
    ) -> Result<(), StoreError> {
        todo!()
    }

    /// Returns the latest valid ancestor hash for a given invalid block hash.
    /// Used to provide `latest_valid_hash` in the Engine API when processing invalid payloads.
    async fn get_latest_valid_ancestor(
        &self,
        block: BlockHash,
    ) -> Result<Option<BlockHash>, StoreError> {
        todo!()
    }

    /// Obtain block number for a given hash
    fn get_block_number_sync(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockNumber>, StoreError> {
        let hash_key = BlockHashRLP::from(block_hash).bytes().clone();

        match self.read_sync(CF_BLOCK_NUMBERS, hash_key)? {
            Some(number_bytes) => BlockNumber::decode(number_bytes.as_slice())
                .map(Some)
                .map_err(StoreError::from),
            None => Ok(None),
        }
    }

    /// Get the canonical block hash for a given block number.
    fn get_canonical_block_hash_sync(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<BlockHash>, StoreError> {
        let number_key = block_number.encode_to_vec();

        match self.read_sync(CF_CANONICAL_BLOCK_HASHES, number_key)? {
            Some(hash_bytes) => {
                let hash_rlp = BlockHashRLP::from_bytes(hash_bytes);
                hash_rlp.to().map(Some).map_err(StoreError::from)
            }
            None => Ok(None),
        }
    }

    async fn write_storage_trie_nodes_batch(
        &self,
        storage_trie_nodes: Vec<(H256, Vec<(NodeHash, Vec<u8>)>)>,
    ) -> Result<(), StoreError> {
        todo!()
    }

    async fn write_account_code_batch(
        &self,
        account_codes: Vec<(H256, Bytes)>,
    ) -> Result<(), StoreError> {
        todo!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethrex_common::types::BlockHeader;
    use ethrex_common::{Address, H256, U256};
    use tempdir::TempDir;

    fn create_test_header() -> BlockHeader {
        BlockHeader {
            hash: Default::default(),
            parent_hash: H256::random(),
            ommers_hash: H256::random(),
            coinbase: Address::random(),
            state_root: H256::random(),
            transactions_root: H256::random(),
            receipts_root: H256::random(),
            logs_bloom: Default::default(),
            difficulty: U256::from(1000),
            number: 1,
            gas_limit: 8000000,
            gas_used: 5000000,
            timestamp: 1234567890,
            extra_data: vec![].into(),
            prev_randao: H256::random(),
            nonce: 42,
            base_fee_per_gas: Some(1000),
            withdrawals_root: None,
            blob_gas_used: None,
            excess_blob_gas: None,
            parent_beacon_block_root: None,
            requests_hash: None,
        }
    }

    #[tokio::test]
    async fn test_store_creation() {
        let temp_dir = TempDir::new("rocksdb_test").unwrap();
        let db_path = temp_dir.path().join("test_db");

        let store = Store::new(db_path.to_str().unwrap()).unwrap();
        // Just test that we can create the store successfully
        assert!(store.db.live_files().is_ok());
    }

    #[tokio::test]
    async fn test_header_round_trip() {
        let temp_dir = TempDir::new("rocksdb_test").unwrap();
        let db_path = temp_dir.path().join("test_db");
        let store = Store::new(db_path.to_str().unwrap()).unwrap();

        let header = create_test_header();
        let block_hash = header.hash();

        // Store the header
        store
            .add_block_header(block_hash, header.clone())
            .await
            .unwrap();

        // Retrieve the header
        let retrieved_header = store.get_block_header_by_hash(block_hash).unwrap().unwrap();

        // Verify they match
        assert_eq!(header.parent_hash, retrieved_header.parent_hash);
        assert_eq!(header.number, retrieved_header.number);
        assert_eq!(header.difficulty, retrieved_header.difficulty);
        assert_eq!(header.gas_limit, retrieved_header.gas_limit);
    }

    #[tokio::test]
    async fn test_canonical_chain_lookup() {
        let temp_dir = TempDir::new("rocksdb_test").unwrap();
        let db_path = temp_dir.path().join("test_db");
        let store = Store::new(db_path.to_str().unwrap()).unwrap();

        let header = create_test_header();
        let block_hash = header.hash();
        let block_number = header.number;

        // Store the header
        store
            .add_block_header(block_hash, header.clone())
            .await
            .unwrap();

        // Store the canonical mapping and block number mapping
        store
            .add_canonical_block_hash(block_number, block_hash)
            .await
            .unwrap();
        store
            .add_block_number(block_hash, block_number)
            .await
            .unwrap();

        // Test canonical hash lookup
        let canonical_hash = store
            .get_canonical_block_hash(block_number)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(canonical_hash, block_hash);

        // Test sync version
        let canonical_hash_sync = store
            .get_canonical_block_hash_sync(block_number)
            .unwrap()
            .unwrap();
        assert_eq!(canonical_hash_sync, block_hash);

        // Test block number lookup
        let retrieved_number = store.get_block_number(block_hash).await.unwrap().unwrap();
        assert_eq!(retrieved_number, block_number);

        // Test sync version
        let retrieved_number_sync = store.get_block_number_sync(block_hash).unwrap().unwrap();
        assert_eq!(retrieved_number_sync, block_number);

        // Test get_block_header with canonical lookup
        let canonical_header = store.get_block_header(block_number).unwrap().unwrap();
        assert_eq!(canonical_header.parent_hash, header.parent_hash);
        assert_eq!(canonical_header.number, header.number);
    }

    #[tokio::test]
    async fn test_nonexistent_data() {
        let temp_dir = TempDir::new("rocksdb_test").unwrap();
        let db_path = temp_dir.path().join("test_db");
        let store = Store::new(db_path.to_str().unwrap()).unwrap();

        let random_hash = H256::random();
        let random_number = 999999;

        // Test that nonexistent data returns None
        assert!(
            store
                .get_block_header_by_hash(random_hash)
                .unwrap()
                .is_none()
        );
        assert!(store.get_block_header(random_number).unwrap().is_none());
        assert!(
            store
                .get_canonical_block_hash(random_number)
                .await
                .unwrap()
                .is_none()
        );
        assert!(store.get_block_number(random_hash).await.unwrap().is_none());
    }
}
