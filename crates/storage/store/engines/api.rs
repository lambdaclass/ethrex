use bytes::Bytes;
use ethereum_types::{H256, U256};
use ethrex_core::types::{
    AccountState, BlobsBundle, Block, BlockBody, BlockHash, BlockHeader, BlockNumber, ChainConfig,
    Index, Receipt, Transaction,
};
use std::{fmt::Debug, panic::RefUnwindSafe};

use crate::{error::StoreError, STATE_TRIE_SEGMENTS};
use ethrex_trie::{Nibbles, Trie};

pub trait StoreEngine: Debug + Send + Sync + RefUnwindSafe {
    /// Add block header
    fn add_block_header(
        &self,
        block_hash: BlockHash,
        block_header: BlockHeader,
    ) -> Result<(), StoreError>;

    /// Add a batch of block headers
    fn add_block_headers(
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
    fn add_block_body(
        &self,
        block_hash: BlockHash,
        block_body: BlockBody,
    ) -> Result<(), StoreError>;

    /// Obtain canonical block body
    fn get_block_body(&self, block_number: BlockNumber) -> Result<Option<BlockBody>, StoreError>;

    /// Obtain any block body using the hash
    fn get_block_body_by_hash(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockBody>, StoreError>;

    fn get_block_header_by_hash(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockHeader>, StoreError>;

    fn add_pending_block(&self, block: Block) -> Result<(), StoreError>;
    fn get_pending_block(&self, block_hash: BlockHash) -> Result<Option<Block>, StoreError>;

    /// Add block number for a given hash
    fn add_block_number(
        &self,
        block_hash: BlockHash,
        block_number: BlockNumber,
    ) -> Result<(), StoreError>;

    /// Obtain block number for a given hash
    fn get_block_number(&self, block_hash: BlockHash) -> Result<Option<BlockNumber>, StoreError>;

    // TODO (#307): Remove TotalDifficulty.
    /// Add block total difficulty
    fn add_block_total_difficulty(
        &self,
        block_hash: BlockHash,
        block_total_difficulty: U256,
    ) -> Result<(), StoreError>;

    // TODO (#307): Remove TotalDifficulty.
    /// Obtain block total difficulty
    fn get_block_total_difficulty(&self, block_hash: BlockHash)
        -> Result<Option<U256>, StoreError>;

    /// Store transaction location (block number and index of the transaction within the block)
    fn add_transaction_location(
        &self,
        transaction_hash: H256,
        block_number: BlockNumber,
        block_hash: BlockHash,
        index: Index,
    ) -> Result<(), StoreError>;

    /// Store transaction locations in batch (one db transaction for all)
    fn add_transaction_locations(
        &self,
        locations: Vec<(H256, BlockNumber, BlockHash, Index)>,
    ) -> Result<(), StoreError>;

    /// Obtain transaction location (block hash and index)
    fn get_transaction_location(
        &self,
        transaction_hash: H256,
    ) -> Result<Option<(BlockNumber, BlockHash, Index)>, StoreError>;

    /// Add receipt
    fn add_receipt(
        &self,
        block_hash: BlockHash,
        index: Index,
        receipt: Receipt,
    ) -> Result<(), StoreError>;

    /// Add receipt
    fn add_receipts(&self, block_hash: BlockHash, receipts: Vec<Receipt>)
        -> Result<(), StoreError>;

    /// Obtain receipt for a canonical block represented by the block number.
    fn get_receipt(
        &self,
        block_number: BlockNumber,
        index: Index,
    ) -> Result<Option<Receipt>, StoreError>;

    /// Add account code
    fn add_account_code(&self, code_hash: H256, code: Bytes) -> Result<(), StoreError>;

    /// Obtain account code via code hash
    fn get_account_code(&self, code_hash: H256) -> Result<Option<Bytes>, StoreError>;

    fn get_transaction_by_hash(
        &self,
        transaction_hash: H256,
    ) -> Result<Option<Transaction>, StoreError> {
        let (_block_number, block_hash, index) =
            match self.get_transaction_location(transaction_hash)? {
                Some(location) => location,
                None => return Ok(None),
            };
        self.get_transaction_by_location(block_hash, index)
    }

    fn get_transaction_by_location(
        &self,
        block_hash: H256,
        index: u64,
    ) -> Result<Option<Transaction>, StoreError> {
        let block_body = match self.get_block_body_by_hash(block_hash)? {
            Some(body) => body,
            None => return Ok(None),
        };
        Ok(index
            .try_into()
            .ok()
            .and_then(|index: usize| block_body.transactions.get(index).cloned()))
    }

    fn get_block_by_hash(&self, block_hash: BlockHash) -> Result<Option<Block>, StoreError> {
        let header = match self.get_block_header_by_hash(block_hash)? {
            Some(header) => header,
            None => return Ok(None),
        };
        let body = match self.get_block_body_by_hash(block_hash)? {
            Some(body) => body,
            None => return Ok(None),
        };
        Ok(Some(Block::new(header, body)))
    }

    // Get the canonical block hash for a given block number.
    fn get_canonical_block_hash(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<BlockHash>, StoreError>;

    /// Stores the chain configuration values, should only be called once after reading the genesis file
    /// Ignores previously stored values if present
    fn set_chain_config(&self, chain_config: &ChainConfig) -> Result<(), StoreError>;

    /// Returns the stored chain configuration
    fn get_chain_config(&self) -> Result<ChainConfig, StoreError>;

    /// Update earliest block number
    fn update_earliest_block_number(&self, block_number: BlockNumber) -> Result<(), StoreError>;

    /// Obtain earliest block number
    fn get_earliest_block_number(&self) -> Result<Option<BlockNumber>, StoreError>;

    /// Update finalized block number
    fn update_finalized_block_number(&self, block_number: BlockNumber) -> Result<(), StoreError>;

    /// Obtain finalized block number
    fn get_finalized_block_number(&self) -> Result<Option<BlockNumber>, StoreError>;

    /// Update safe block number
    fn update_safe_block_number(&self, block_number: BlockNumber) -> Result<(), StoreError>;

    /// Obtain safe block number
    fn get_safe_block_number(&self) -> Result<Option<BlockNumber>, StoreError>;

    /// Update latest block number
    fn update_latest_block_number(&self, block_number: BlockNumber) -> Result<(), StoreError>;

    /// Obtain latest block number
    fn get_latest_block_number(&self) -> Result<Option<BlockNumber>, StoreError>;

    // TODO (#307): Remove TotalDifficulty.
    /// Update latest total difficulty
    fn update_latest_total_difficulty(
        &self,
        latest_total_difficulty: U256,
    ) -> Result<(), StoreError>;

    // TODO (#307): Remove TotalDifficulty.
    /// Obtain latest total difficulty
    fn get_latest_total_difficulty(&self) -> Result<Option<U256>, StoreError>;

    /// Update pending block number
    fn update_pending_block_number(&self, block_number: BlockNumber) -> Result<(), StoreError>;

    /// Obtain pending block number
    fn get_pending_block_number(&self) -> Result<Option<BlockNumber>, StoreError>;

    /// Obtain a storage trie from the given address and storage_root
    /// Doesn't check if the account is stored
    /// Used for internal store operations
    fn open_storage_trie(&self, hashed_address: H256, storage_root: H256) -> Trie;

    /// Obtain a state trie from the given state root
    /// Doesn't check if the state root is valid
    /// Used for internal store operations
    fn open_state_trie(&self, state_root: H256) -> Trie;

    /// Set the canonical block hash for a given block number.
    fn set_canonical_block(&self, number: BlockNumber, hash: BlockHash) -> Result<(), StoreError>;

    /// Unsets canonical block for a block number.
    fn unset_canonical_block(&self, number: BlockNumber) -> Result<(), StoreError>;

    fn add_payload(&self, payload_id: u64, block: Block) -> Result<(), StoreError>;

    fn get_payload(
        &self,
        payload_id: u64,
    ) -> Result<Option<(Block, U256, BlobsBundle, bool)>, StoreError>;

    fn update_payload(
        &self,
        payload_id: u64,
        block: Block,
        block_value: U256,
        blobs_bundle: BlobsBundle,
        completed: bool,
    ) -> Result<(), StoreError>;

    fn get_receipts_for_block(&self, block_hash: &BlockHash) -> Result<Vec<Receipt>, StoreError>;

    // Snap State methods

    /// Sets the hash of the last header downloaded during a snap sync
    fn set_header_download_checkpoint(&self, block_hash: BlockHash) -> Result<(), StoreError>;

    /// Gets the hash of the last header downloaded during a snap sync
    fn get_header_download_checkpoint(&self) -> Result<Option<BlockHash>, StoreError>;

    /// Sets the last key fetched from the state trie being fetched during snap sync
    fn set_state_trie_key_checkpoint(
        &self,
        last_keys: [H256; STATE_TRIE_SEGMENTS],
    ) -> Result<(), StoreError>;

    /// Gets the last key fetched from the state trie being fetched during snap sync
    fn get_state_trie_key_checkpoint(
        &self,
    ) -> Result<Option<[H256; STATE_TRIE_SEGMENTS]>, StoreError>;

    /// Sets the storage trie paths in need of healing, grouped by hashed address
    fn set_storage_heal_paths(&self, accounts: Vec<(H256, Vec<Nibbles>)>)
        -> Result<(), StoreError>;

    /// Gets the storage trie paths in need of healing, grouped by hashed address
    #[allow(clippy::type_complexity)]
    fn get_storage_heal_paths(&self) -> Result<Option<Vec<(H256, Vec<Nibbles>)>>, StoreError>;

    /// Sets the state trie paths in need of healing
    fn set_state_heal_paths(&self, paths: Vec<Nibbles>) -> Result<(), StoreError>;

    /// Gets the state trie paths in need of healing
    fn get_state_heal_paths(&self) -> Result<Option<Vec<Nibbles>>, StoreError>;

    /// Clears all checkpoint data created during the last snap sync
    fn clear_snap_state(&self) -> Result<(), StoreError>;

    fn is_synced(&self) -> Result<bool, StoreError>;

    fn update_sync_status(&self, status: bool) -> Result<(), StoreError>;

    /// Write an account batch into the current state snapshot
    fn write_snapshot_account_batch(
        &self,
        account_hashes: Vec<H256>,
        account_states: Vec<AccountState>,
    ) -> Result<(), StoreError>;

    // Write a storage batch into the current storage snapshot
    fn write_snapshot_storage_batch(
        &self,
        account_hash: H256,
        storage_keys: Vec<H256>,
        storage_values: Vec<U256>,
    ) -> Result<(), StoreError>;

    /// Rebuilds state trie from a snapshot, returns the resulting trie's root
    /// and the addresses of the storages whose root doesn't match the one in the account state
    fn rebuild_state_from_snapshot(&self) -> Result<(H256, Vec<H256>), StoreError>;

    fn rebuild_state_trie_segment(
        &self,
        start: H256,
        end: H256,
    ) -> Result<(H256, Vec<H256>, H256), StoreError>;

    // NO PUEDO TENER ASYNC ACA!!!!
}
