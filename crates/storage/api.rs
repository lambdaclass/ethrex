use bytes::Bytes;
use ethereum_types::H256;
use ethrex_common::types::{
    Block, BlockBody, BlockHash, BlockHeader, BlockNumber, ChainConfig, Index, Receipt, Transaction,
};
use std::{fmt::Debug, panic::RefUnwindSafe};

use crate::UpdateBatch;
use crate::{error::StoreError, store::STATE_TRIE_SEGMENTS};
use ethrex_trie::{Nibbles, NodeHash, Trie};

// We need async_trait because the stabilized feature lacks support for object safety
// (i.e. dyn StoreEngine)
#[async_trait::async_trait]
pub trait StoreEngine: Debug + Send + Sync + RefUnwindSafe {
    /// Store changes in a batch from a vec of blocks
    async fn apply_updates(&self, update_batch: UpdateBatch) -> Result<(), StoreError>;

    /// Add a batch of blocks in a single transaction.
    /// This will store -> BlockHeader, BlockBody, BlockTransactions, BlockNumber.
    async fn add_blocks(&self, blocks: Vec<Block>) -> Result<(), StoreError>;

    /// Add block header
    async fn add_block_header(
        &self,
        block_hash: BlockHash,
        block_header: BlockHeader,
    ) -> Result<(), StoreError>;

    /// Add a batch of block headers
    async fn add_block_headers(&self, block_headers: Vec<BlockHeader>) -> Result<(), StoreError>;

    /// Obtain canonical block header
    fn get_block_header(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<BlockHeader>, StoreError>;

    /// Add block body
    async fn add_block_body(
        &self,
        block_hash: BlockHash,
        block_body: BlockBody,
    ) -> Result<(), StoreError>;

    /// Obtain canonical block body
    async fn get_block_body(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<BlockBody>, StoreError>;

    /// Remove canonical block
    async fn remove_block(&self, block_number: BlockNumber) -> Result<(), StoreError>;

    /// Obtain canonical block bodies in from..=to
    async fn get_block_bodies(
        &self,
        from: BlockNumber,
        to: BlockNumber,
    ) -> Result<Vec<BlockBody>, StoreError>;

    /// Obtain block bodies from a list of hashes
    async fn get_block_bodies_by_hash(
        &self,
        hashes: Vec<BlockHash>,
    ) -> Result<Vec<BlockBody>, StoreError>;

    /// Obtain any block body using the hash
    async fn get_block_body_by_hash(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockBody>, StoreError>;

    fn get_block_header_by_hash(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockHeader>, StoreError>;

    async fn add_pending_block(&self, block: Block) -> Result<(), StoreError>;
    async fn get_pending_block(&self, block_hash: BlockHash) -> Result<Option<Block>, StoreError>;

    /// Add block number for a given hash
    async fn add_block_number(
        &self,
        block_hash: BlockHash,
        block_number: BlockNumber,
    ) -> Result<(), StoreError>;

    /// Obtain block number for a given hash
    async fn get_block_number(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockNumber>, StoreError>;

    /// Store transaction location (block number and index of the transaction within the block)
    async fn add_transaction_location(
        &self,
        transaction_hash: H256,
        block_number: BlockNumber,
        block_hash: BlockHash,
        index: Index,
    ) -> Result<(), StoreError>;

    /// Store transaction locations in batch (one db transaction for all)
    async fn add_transaction_locations(
        &self,
        locations: Vec<(H256, BlockNumber, BlockHash, Index)>,
    ) -> Result<(), StoreError>;

    /// Obtain transaction location (block hash and index)
    async fn get_transaction_location(
        &self,
        transaction_hash: H256,
    ) -> Result<Option<(BlockNumber, BlockHash, Index)>, StoreError>;

    /// Add receipt
    async fn add_receipt(
        &self,
        block_hash: BlockHash,
        index: Index,
        receipt: Receipt,
    ) -> Result<(), StoreError>;

    /// Add receipts
    async fn add_receipts(
        &self,
        block_hash: BlockHash,
        receipts: Vec<Receipt>,
    ) -> Result<(), StoreError>;

    /// Obtain receipt by block hash and index
    async fn get_receipt(
        &self,
        block_hash: BlockHash,
        index: Index,
    ) -> Result<Option<Receipt>, StoreError>;

    /// Add account code
    async fn add_account_code(&self, code_hash: H256, code: Bytes) -> Result<(), StoreError>;

    /// Clears all checkpoint data created during the last snap sync
    async fn clear_snap_state(&self) -> Result<(), StoreError>;

    /// Obtain account code via code hash
    fn get_account_code(&self, code_hash: H256) -> Result<Option<Bytes>, StoreError>;

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
    ) -> Result<Option<BlockHash>, StoreError>;

    /// Stores the chain configuration values, should only be called once after reading the genesis file
    /// Ignores previously stored values if present
    async fn set_chain_config(&self, chain_config: &ChainConfig) -> Result<(), StoreError>;

    /// Update earliest block number
    async fn update_earliest_block_number(
        &self,
        block_number: BlockNumber,
    ) -> Result<(), StoreError>;

    /// Obtain earliest block number
    async fn get_earliest_block_number(&self) -> Result<Option<BlockNumber>, StoreError>;

    /// Obtain finalized block number
    async fn get_finalized_block_number(&self) -> Result<Option<BlockNumber>, StoreError>;

    /// Obtain safe block number
    async fn get_safe_block_number(&self) -> Result<Option<BlockNumber>, StoreError>;

    /// Obtain latest block number
    async fn get_latest_block_number(&self) -> Result<Option<BlockNumber>, StoreError>;

    /// Update pending block number
    async fn update_pending_block_number(
        &self,
        block_number: BlockNumber,
    ) -> Result<(), StoreError>;

    /// Obtain pending block number
    async fn get_pending_block_number(&self) -> Result<Option<BlockNumber>, StoreError>;

    /// Obtain a storage trie from the given address and storage_root
    /// Doesn't check if the account is stored
    /// Used for internal store operations
    fn open_storage_trie(
        &self,
        hashed_address: H256,
        storage_root: H256,
    ) -> Result<Trie, StoreError>;

    /// Obtain a state trie from the given state root
    /// Doesn't check if the state root is valid
    /// Used for internal store operations
    fn open_state_trie(&self, state_root: H256) -> Result<Trie, StoreError>;

    /// Obtain a state trie locked for reads from the given state root
    /// Doesn't check if the state root is valid
    /// Used for internal store operations
    fn open_locked_state_trie(&self, state_root: H256) -> Result<Trie, StoreError> {
        self.open_state_trie(state_root)
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
    ) -> Result<(), StoreError>;

    fn get_receipts_for_block(&self, block_hash: &BlockHash) -> Result<Vec<Receipt>, StoreError>;

    // Snap State methods

    /// Sets the hash of the last header downloaded during a snap sync
    async fn set_header_download_checkpoint(&self, block_hash: BlockHash)
    -> Result<(), StoreError>;

    /// Gets the hash of the last header downloaded during a snap sync
    async fn get_header_download_checkpoint(&self) -> Result<Option<BlockHash>, StoreError>;

    /// Sets the last key fetched from the state trie being fetched during snap sync
    async fn set_state_trie_key_checkpoint(
        &self,
        last_keys: [H256; STATE_TRIE_SEGMENTS],
    ) -> Result<(), StoreError>;

    /// Gets the last key fetched from the state trie being fetched during snap sync
    async fn get_state_trie_key_checkpoint(
        &self,
    ) -> Result<Option<[H256; STATE_TRIE_SEGMENTS]>, StoreError>;

    /// Sets the state trie paths in need of healing
    async fn set_state_heal_paths(&self, paths: Vec<(Nibbles, H256)>) -> Result<(), StoreError>;

    /// Gets the state trie paths in need of healing
    async fn get_state_heal_paths(&self) -> Result<Option<Vec<(Nibbles, H256)>>, StoreError>;

    /// Set the latest root of the rebuilt state trie and the last downloaded hashes from each segment
    async fn set_state_trie_rebuild_checkpoint(
        &self,
        checkpoint: (H256, [H256; STATE_TRIE_SEGMENTS]),
    ) -> Result<(), StoreError>;

    /// Get the latest root of the rebuilt state trie and the last downloaded hashes from each segment
    async fn get_state_trie_rebuild_checkpoint(
        &self,
    ) -> Result<Option<(H256, [H256; STATE_TRIE_SEGMENTS])>, StoreError>;

    /// Get the accont hashes and roots of the storage tries awaiting rebuild
    async fn set_storage_trie_rebuild_pending(
        &self,
        pending: Vec<(H256, H256)>,
    ) -> Result<(), StoreError>;

    /// Get the accont hashes and roots of the storage tries awaiting rebuild
    async fn get_storage_trie_rebuild_pending(
        &self,
    ) -> Result<Option<Vec<(H256, H256)>>, StoreError>;

    /// The `forkchoice_update` and `new_payload` methods require the `latest_valid_hash`
    /// when processing an invalid payload. To provide this, we must track invalid chains.
    ///
    /// We only store the last known valid head upon encountering a bad block,
    /// rather than tracking every subsequent invalid block.
    async fn set_latest_valid_ancestor(
        &self,
        bad_block: BlockHash,
        latest_valid: BlockHash,
    ) -> Result<(), StoreError>;

    /// Returns the latest valid ancestor hash for a given invalid block hash.
    /// Used to provide `latest_valid_hash` in the Engine API when processing invalid payloads.
    async fn get_latest_valid_ancestor(
        &self,
        block: BlockHash,
    ) -> Result<Option<BlockHash>, StoreError>;

    /// Obtain block number for a given hash
    fn get_block_number_sync(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockNumber>, StoreError>;

    /// Get the canonical block hash for a given block number.
    fn get_canonical_block_hash_sync(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<BlockHash>, StoreError>;

    async fn write_storage_trie_nodes_batch(
        &self,
        storage_trie_nodes: Vec<(H256, Vec<(NodeHash, Vec<u8>)>)>,
    ) -> Result<(), StoreError>;

    async fn write_account_code_batch(
        &self,
        account_codes: Vec<(H256, Bytes)>,
    ) -> Result<(), StoreError>;
}
