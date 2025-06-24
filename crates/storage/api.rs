use bytes::Bytes;
use ethereum_types::{H256, U256};
use ethrex_common::types::{
    AccountState, Block, BlockBody, BlockHash, BlockHeader, BlockNumber, ChainConfig, Index,
    Receipt, Transaction, payload::PayloadBundle,
};
use std::collections::HashMap;
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

    async fn apply_storage_trie_changes(
        &self,
        changeset: HashMap<H256, Vec<(NodeHash, Vec<u8>)>>,
    ) -> Result<(), StoreError>;

    /// Add a batch of blocks in a single transaction.
    /// This will store -> BlockHeader, BlockBody, BlockTransactions, BlockNumber.
    async fn add_blocks(&self, blocks: Vec<Block>) -> Result<(), StoreError>;

    /// Sets the blocks as part of the canonical chain
    async fn mark_chain_as_canonical(&self, blocks: &[Block]) -> Result<(), StoreError>;

    /// Add block header
    async fn add_block_header(
        &self,
        block_hash: BlockHash,
        block_header: BlockHeader,
    ) -> Result<(), StoreError>;

    /// Add a batch of block headers
    async fn add_block_headers(
        &self,
        block_hashes: Vec<BlockHash>,
        block_headers: Vec<BlockHeader>,
    ) -> Result<(), StoreError>;

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

    /// Obtain receipt for a canonical block represented by the block number.
    async fn get_receipt(
        &self,
        block_number: BlockNumber,
        index: Index,
    ) -> Result<Option<Receipt>, StoreError>;

    /// Add account code
    async fn add_account_code(&self, code_hash: H256, code: Bytes) -> Result<(), StoreError>;

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
        Ok(index
            .try_into()
            .ok()
            .and_then(|index: usize| block_body.transactions.get(index).cloned()))
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

    /// Returns the stored chain configuration
    fn get_chain_config(&self) -> Result<ChainConfig, StoreError>;

    /// Update earliest block number
    async fn update_earliest_block_number(
        &self,
        block_number: BlockNumber,
    ) -> Result<(), StoreError>;

    /// Obtain earliest block number
    async fn get_earliest_block_number(&self) -> Result<Option<BlockNumber>, StoreError>;

    /// Update finalized block number
    async fn update_finalized_block_number(
        &self,
        block_number: BlockNumber,
    ) -> Result<(), StoreError>;

    /// Obtain finalized block number
    async fn get_finalized_block_number(&self) -> Result<Option<BlockNumber>, StoreError>;

    /// Update safe block number
    async fn update_safe_block_number(&self, block_number: BlockNumber) -> Result<(), StoreError>;

    /// Obtain safe block number
    async fn get_safe_block_number(&self) -> Result<Option<BlockNumber>, StoreError>;

    /// Update latest block number
    async fn update_latest_block_number(&self, block_number: BlockNumber)
    -> Result<(), StoreError>;

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

    /// Set the canonical block hash for a given block number.
    async fn set_canonical_block(
        &self,
        number: BlockNumber,
        hash: BlockHash,
    ) -> Result<(), StoreError>;

    /// Unsets canonical block for a block number.
    async fn unset_canonical_block(&self, number: BlockNumber) -> Result<(), StoreError>;

    async fn add_payload(&self, payload_id: u64, block: Block) -> Result<(), StoreError>;

    async fn get_payload(&self, payload_id: u64) -> Result<Option<PayloadBundle>, StoreError>;

    async fn update_payload(
        &self,
        payload_id: u64,
        payload: PayloadBundle,
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

    /// Sets storage trie paths in need of healing, grouped by hashed address
    /// This will overwite previously stored paths for the received storages but will not remove other storage's paths
    async fn set_storage_heal_paths(
        &self,
        accounts: Vec<(H256, Vec<Nibbles>)>,
    ) -> Result<(), StoreError>;

    /// Gets the storage trie paths in need of healing, grouped by hashed address
    /// Gets paths from at most `limit` storage tries and removes them from the store
    #[allow(clippy::type_complexity)]
    async fn take_storage_heal_paths(
        &self,
        limit: usize,
    ) -> Result<Vec<(H256, Vec<Nibbles>)>, StoreError>;

    /// Sets the state trie paths in need of healing
    async fn set_state_heal_paths(&self, paths: Vec<Nibbles>) -> Result<(), StoreError>;

    /// Gets the state trie paths in need of healing
    async fn get_state_heal_paths(&self) -> Result<Option<Vec<Nibbles>>, StoreError>;

    /// Clears all checkpoint data created during the last snap sync
    async fn clear_snap_state(&self) -> Result<(), StoreError>;

    /// Write an account batch into the current state snapshot
    async fn write_snapshot_account_batch(
        &self,
        account_hashes: Vec<H256>,
        account_states: Vec<AccountState>,
    ) -> Result<(), StoreError>;

    /// Write a storage batch into the current storage snapshot
    async fn write_snapshot_storage_batch(
        &self,
        account_hash: H256,
        storage_keys: Vec<H256>,
        storage_values: Vec<U256>,
    ) -> Result<(), StoreError>;

    /// Write multiple storage batches belonging to different accounts into the current storage snapshot
    async fn write_snapshot_storage_batches(
        &self,
        account_hashes: Vec<H256>,
        storage_keys: Vec<Vec<H256>>,
        storage_values: Vec<Vec<U256>>,
    ) -> Result<(), StoreError>;

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

    /// Clears the state and storage snapshots
    async fn clear_snapshot(&self) -> Result<(), StoreError>;

    /// Reads the next `MAX_SNAPSHOT_READS` accounts from the state snapshot as from the `start` hash
    fn read_account_snapshot(&self, start: H256) -> Result<Vec<(H256, AccountState)>, StoreError>;

    /// Reads the next `MAX_SNAPSHOT_READS` elements from the storage snapshot as from the `start` storage key
    async fn read_storage_snapshot(
        &self,
        start: H256,
        account_hash: H256,
    ) -> Result<Vec<(H256, U256)>, StoreError>;

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
}
