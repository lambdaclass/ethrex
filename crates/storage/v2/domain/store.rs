use crate::rlp::{AccountCodeRLP, BlockBodyRLP, BlockHashRLP, BlockHeaderRLP};
use crate::v2::backend::{StorageBackend, StorageError};
use crate::v2::schema::{DBTable, SchemaRegistry};
use crate::{UpdateBatch, api::StoreEngine, store::STATE_TRIE_SEGMENTS, utils::ChainDataIndex};

use bytes::Bytes;
use ethereum_types::{H256, U256};
use ethrex_common::types::{
    Block, BlockBody, BlockHash, BlockHeader, BlockNumber, ChainConfig, Index, Receipt, Transaction,
};
use ethrex_trie::{Nibbles, NodeHash, Trie};

use std::{fmt::Debug, sync::Arc};

/// Domain store that implements StoreEngine using the new layered architecture
///
/// This is the single implementation that replaces all the duplicated logic
/// in rocksdb.rs, libmdbx.rs, and in_memory.rs
#[derive(Debug)]
pub struct DomainStore {
    schema: SchemaRegistry,
}

impl DomainStore {
    /// Create a new DomainStore with the given storage backend
    pub async fn new(backend: Arc<dyn StorageBackend>) -> Result<Self, StorageError> {
        let schema = SchemaRegistry::new(backend)
            .await
            .map_err(|e| StorageError::Custom(format!("Failed to initialize schema: {:?}", e)))?;

        Ok(Self { schema })
    }
}

impl DomainStore {
    /// Store changes in a batch from a vec of blocks
    async fn apply_updates(&self, update_batch: UpdateBatch) -> Result<(), StorageError> {
        todo!()
    }

    /// Add a batch of blocks in a single transaction.
    /// This will store -> BlockHeader, BlockBody, BlockTransactions, BlockNumber.
    async fn add_blocks(&self, blocks: Vec<Block>) -> Result<(), StorageError> {
        todo!()
    }

    /// Add block header
    async fn add_block_header(
        &self,
        block_hash: BlockHash,
        block_header: BlockHeader,
    ) -> Result<(), StorageError> {
        let hash_key = BlockHashRLP::from(block_hash).bytes().clone();
        let header_value = BlockHeaderRLP::from(block_header).bytes().clone();
        self.schema
            .put(DBTable::Headers, &hash_key, &header_value)
            .await
    }

    /// Add a batch of block headers
    async fn add_block_headers(&self, block_headers: Vec<BlockHeader>) -> Result<(), StorageError> {
        todo!()
    }

    /// Obtain canonical block header
    fn get_block_header(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<BlockHeader>, StorageError> {
        todo!()
    }

    /// Add block body
    async fn add_block_body(
        &self,
        block_hash: BlockHash,
        block_body: BlockBody,
    ) -> Result<(), StorageError> {
        let hash_key = BlockHashRLP::from(block_hash).bytes().clone();
        let body_value = BlockBodyRLP::from(block_body).bytes().clone();
        self.schema
            .put(DBTable::Bodies, &hash_key, &body_value)
            .await
    }

    /// Obtain canonical block body
    async fn get_block_body(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<BlockBody>, StorageError> {
        todo!()
    }

    /// Remove canonical block
    async fn remove_block(&self, block_number: BlockNumber) -> Result<(), StorageError> {
        todo!()
    }

    /// Obtain canonical block bodies in from..=to
    async fn get_block_bodies(
        &self,
        from: BlockNumber,
        to: BlockNumber,
    ) -> Result<Vec<BlockBody>, StorageError> {
        todo!()
    }

    /// Obtain block bodies from a list of hashes
    async fn get_block_bodies_by_hash(
        &self,
        hashes: Vec<BlockHash>,
    ) -> Result<Vec<BlockBody>, StorageError> {
        todo!()
    }

    /// Obtain any block body using the hash
    async fn get_block_body_by_hash(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockBody>, StorageError> {
        todo!()
    }

    fn get_block_header_by_hash(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockHeader>, StorageError> {
        todo!()
    }

    async fn add_pending_block(&self, block: Block) -> Result<(), StorageError> {
        todo!()
    }
    async fn get_pending_block(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<Block>, StorageError> {
        todo!()
    }

    /// Add block number for a given hash
    async fn add_block_number(
        &self,
        block_hash: BlockHash,
        block_number: BlockNumber,
    ) -> Result<(), StorageError> {
        todo!()
    }

    /// Obtain block number for a given hash
    async fn get_block_number(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockNumber>, StorageError> {
        todo!()
    }

    /// Store transaction location (block number and index of the transaction within the block)
    async fn add_transaction_location(
        &self,
        transaction_hash: H256,
        block_number: BlockNumber,
        block_hash: BlockHash,
        index: Index,
    ) -> Result<(), StorageError> {
        todo!()
    }

    /// Store transaction locations in batch (one db transaction for all)
    async fn add_transaction_locations(
        &self,
        locations: Vec<(H256, BlockNumber, BlockHash, Index)>,
    ) -> Result<(), StorageError> {
        todo!()
    }

    /// Obtain transaction location (block hash and index)
    async fn get_transaction_location(
        &self,
        transaction_hash: H256,
    ) -> Result<Option<(BlockNumber, BlockHash, Index)>, StorageError> {
        todo!()
    }

    /// Add receipt
    async fn add_receipt(
        &self,
        block_hash: BlockHash,
        index: Index,
        receipt: Receipt,
    ) -> Result<(), StorageError> {
        todo!()
    }

    /// Add receipts
    async fn add_receipts(
        &self,
        block_hash: BlockHash,
        receipts: Vec<Receipt>,
    ) -> Result<(), StorageError> {
        todo!()
    }

    /// Obtain receipt by block hash and index
    async fn get_receipt(
        &self,
        block_hash: BlockHash,
        index: Index,
    ) -> Result<Option<Receipt>, StorageError> {
        todo!()
    }

    /// Add account code
    async fn add_account_code(&self, code_hash: H256, code: Bytes) -> Result<(), StorageError> {
        let hash_key = code_hash.as_bytes().to_vec();
        let code_value = AccountCodeRLP::from(code).bytes().clone();
        self.schema
            .put(DBTable::AccountCodes, &hash_key, &code_value)
            .await
    }

    /// Clears all checkpoint data created during the last snap sync
    async fn clear_snap_state(&self) -> Result<(), StorageError> {
        todo!()
    }

    /// Obtain account code via code hash
    fn get_account_code(&self, code_hash: H256) -> Result<Option<Bytes>, StorageError> {
        todo!()
    }

    async fn get_transaction_by_hash(
        &self,
        transaction_hash: H256,
    ) -> Result<Option<Transaction>, StorageError> {
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
    ) -> Result<Option<Transaction>, StorageError> {
        todo!()
    }

    async fn get_block_by_hash(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<Block>, StorageError> {
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
    ) -> Result<Option<Block>, StorageError> {
        let Some(block_hash) = self.get_canonical_block_hash(block_number).await? else {
            return Ok(None);
        };
        self.get_block_by_hash(block_hash).await
    }

    // Get the canonical block hash for a given block number.
    async fn get_canonical_block_hash(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<BlockHash>, StorageError> {
        todo!()
    }

    /// Stores the chain configuration values, should only be called once after reading the genesis file
    /// Ignores previously stored values if present
    async fn set_chain_config(&self, chain_config: &ChainConfig) -> Result<(), StorageError> {
        todo!()
    }

    /// Update earliest block number
    async fn update_earliest_block_number(
        &self,
        block_number: BlockNumber,
    ) -> Result<(), StorageError> {
        todo!()
    }

    /// Obtain earliest block number
    async fn get_earliest_block_number(&self) -> Result<Option<BlockNumber>, StorageError> {
        todo!()
    }

    /// Obtain finalized block number
    async fn get_finalized_block_number(&self) -> Result<Option<BlockNumber>, StorageError> {
        todo!()
    }

    /// Obtain safe block number
    async fn get_safe_block_number(&self) -> Result<Option<BlockNumber>, StorageError> {
        todo!()
    }

    /// Obtain latest block number
    async fn get_latest_block_number(&self) -> Result<Option<BlockNumber>, StorageError> {
        todo!()
    }

    /// Update pending block number
    async fn update_pending_block_number(
        &self,
        block_number: BlockNumber,
    ) -> Result<(), StorageError> {
        todo!()
    }

    /// Obtain pending block number
    async fn get_pending_block_number(&self) -> Result<Option<BlockNumber>, StorageError> {
        todo!()
    }

    /// Obtain a storage trie from the given address and storage_root
    /// Doesn't check if the account is stored
    /// Used for internal store operations
    fn open_storage_trie(
        &self,
        hashed_address: H256,
        storage_root: H256,
    ) -> Result<Trie, StorageError> {
        todo!()
    }

    /// Obtain a state trie from the given state root
    /// Doesn't check if the state root is valid
    /// Used for internal store operations
    fn open_state_trie(&self, state_root: H256) -> Result<Trie, StorageError> {
        todo!()
    }

    /// Obtain a state trie locked for reads from the given state root
    /// Doesn't check if the state root is valid
    /// Used for internal store operations
    fn open_locked_state_trie(&self, state_root: H256) -> Result<Trie, StorageError> {
        self.open_state_trie(state_root)
    }

    /// Obtain a read-locked storage trie from the given address and storage_root
    /// Doesn't check if the account is stored
    /// Used for internal store operations
    fn open_locked_storage_trie(
        &self,
        hashed_address: H256,
        storage_root: H256,
    ) -> Result<Trie, StorageError> {
        self.open_storage_trie(hashed_address, storage_root)
    }

    async fn forkchoice_update(
        &self,
        new_canonical_blocks: Option<Vec<(BlockNumber, BlockHash)>>,
        head_number: BlockNumber,
        head_hash: BlockHash,
        safe: Option<BlockNumber>,
        finalized: Option<BlockNumber>,
    ) -> Result<(), StorageError> {
        todo!()
    }

    fn get_receipts_for_block(&self, block_hash: &BlockHash) -> Result<Vec<Receipt>, StorageError> {
        todo!()
    }

    // Snap State methods

    /// Sets the hash of the last header downloaded during a snap sync
    async fn set_header_download_checkpoint(
        &self,
        block_hash: BlockHash,
    ) -> Result<(), StorageError> {
        todo!()
    }

    /// Gets the hash of the last header downloaded during a snap sync
    async fn get_header_download_checkpoint(&self) -> Result<Option<BlockHash>, StorageError> {
        todo!()
    }

    /// Sets the last key fetched from the state trie being fetched during snap sync
    async fn set_state_trie_key_checkpoint(
        &self,
        last_keys: [H256; STATE_TRIE_SEGMENTS],
    ) -> Result<(), StorageError> {
        todo!()
    }

    /// Gets the last key fetched from the state trie being fetched during snap sync
    async fn get_state_trie_key_checkpoint(
        &self,
    ) -> Result<Option<[H256; STATE_TRIE_SEGMENTS]>, StorageError> {
        todo!()
    }

    /// Sets the state trie paths in need of healing
    async fn set_state_heal_paths(&self, paths: Vec<(Nibbles, H256)>) -> Result<(), StorageError> {
        todo!()
    }

    /// Gets the state trie paths in need of healing
    async fn get_state_heal_paths(&self) -> Result<Option<Vec<(Nibbles, H256)>>, StorageError> {
        todo!()
    }

    /// Write a storage batch into the current storage snapshot
    async fn write_snapshot_storage_batch(
        &self,
        account_hash: H256,
        storage_keys: Vec<H256>,
        storage_values: Vec<U256>,
    ) -> Result<(), StorageError> {
        todo!()
    }

    /// Write multiple storage batches belonging to different accounts into the current storage snapshot
    async fn write_snapshot_storage_batches(
        &self,
        account_hashes: Vec<H256>,
        storage_keys: Vec<Vec<H256>>,
        storage_values: Vec<Vec<U256>>,
    ) -> Result<(), StorageError> {
        todo!()
    }

    /// Set the latest root of the rebuilt state trie and the last downloaded hashes from each segment
    async fn set_state_trie_rebuild_checkpoint(
        &self,
        checkpoint: (H256, [H256; STATE_TRIE_SEGMENTS]),
    ) -> Result<(), StorageError> {
        todo!()
    }

    /// Get the latest root of the rebuilt state trie and the last downloaded hashes from each segment
    async fn get_state_trie_rebuild_checkpoint(
        &self,
    ) -> Result<Option<(H256, [H256; STATE_TRIE_SEGMENTS])>, StorageError> {
        todo!()
    }

    /// Get the accont hashes and roots of the storage tries awaiting rebuild
    async fn set_storage_trie_rebuild_pending(
        &self,
        pending: Vec<(H256, H256)>,
    ) -> Result<(), StorageError> {
        todo!()
    }

    /// Get the accont hashes and roots of the storage tries awaiting rebuild
    async fn get_storage_trie_rebuild_pending(
        &self,
    ) -> Result<Option<Vec<(H256, H256)>>, StorageError> {
        todo!()
    }

    /// Reads the next `MAX_SNAPSHOT_READS` elements from the storage snapshot as from the `start` storage key
    async fn read_storage_snapshot(
        &self,
        start: H256,
        account_hash: H256,
    ) -> Result<Vec<(H256, U256)>, StorageError> {
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
    ) -> Result<(), StorageError> {
        todo!()
    }

    /// Returns the latest valid ancestor hash for a given invalid block hash.
    /// Used to provide `latest_valid_hash` in the Engine API when processing invalid payloads.
    async fn get_latest_valid_ancestor(
        &self,
        block: BlockHash,
    ) -> Result<Option<BlockHash>, StorageError> {
        todo!()
    }

    /// Obtain block number for a given hash
    fn get_block_number_sync(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockNumber>, StorageError> {
        todo!()
    }

    /// Get the canonical block hash for a given block number.
    async fn get_canonical_block_hash_sync(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<BlockHash>, StorageError> {
        let number_key = block_number.to_le_bytes().to_vec();
        match self
            .schema
            .get(DBTable::CanonicalHashes, &number_key)
            .await?
        {
            Some(bytes) => {
                let rlp = BlockHashRLP::from_bytes(bytes)
                    .to()
                    .map_err(StorageError::from)?;
                Ok(Some(rlp))
            }
            None => Ok(None),
        }
    }

    async fn write_storage_trie_nodes_batch(
        &self,
        storage_trie_nodes: Vec<(H256, Vec<(NodeHash, Vec<u8>)>)>,
    ) -> Result<(), StorageError> {
        todo!()
    }

    async fn write_account_code_batch(
        &self,
        account_codes: Vec<(H256, Bytes)>,
    ) -> Result<(), StorageError> {
        todo!()
    }
}

impl DomainStore {
    // Helper methods for key generation
    fn chain_data_key(index: ChainDataIndex) -> Vec<u8> {
        vec![index as u8]
    }
}
