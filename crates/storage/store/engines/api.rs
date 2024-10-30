use bytes::Bytes;
use ethereum_rust_core::types::{
    BlobsBundle, Block, BlockBody, BlockHash, BlockHeader, BlockNumber, ChainConfig, Index,
    MempoolTransaction, Receipt, Transaction,
};
use ethereum_types::{Address, H256, U256};
use std::{collections::HashMap, fmt::Debug, panic::RefUnwindSafe};

use crate::error::StoreError;
use ethereum_rust_trie::Trie;

pub trait StoreEngine: Debug + Send + Sync + RefUnwindSafe {
    /// Add block header
    fn add_block_header(
        &self,
        block_hash: BlockHash,
        block_header: BlockHeader,
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

    /// Obtain transaction location (block hash and index)
    fn get_transaction_location(
        &self,
        transaction_hash: H256,
    ) -> Result<Option<(BlockNumber, BlockHash, Index)>, StoreError>;

    /// Add transaction to the pool
    fn add_transaction_to_pool(
        &self,
        hash: H256,
        transaction: MempoolTransaction,
    ) -> Result<(), StoreError>;

    /// Get a transaction from the pool
    fn get_transaction_from_pool(
        &self,
        hash: H256,
    ) -> Result<Option<MempoolTransaction>, StoreError>;

    /// Store blobs bundle into the pool table by its blob transaction's hash
    fn add_blobs_bundle_to_pool(
        &self,
        tx_hash: H256,
        blobs_bundle: BlobsBundle,
    ) -> Result<(), StoreError>;

    /// Get a blobs bundle from pool table given its blob transaction's hash
    fn get_blobs_bundle_from_pool(&self, tx_hash: H256) -> Result<Option<BlobsBundle>, StoreError>;

    /// Remove a transaction from the pool
    fn remove_transaction_from_pool(&self, hash: H256) -> Result<(), StoreError>;

    /// Applies the filter and returns a set of suitable transactions from the mempool.
    /// These transactions will be grouped by sender and sorted by nonce
    fn filter_pool_transactions(
        &self,
        filter: &dyn Fn(&Transaction) -> bool,
    ) -> Result<HashMap<Address, Vec<MempoolTransaction>>, StoreError>;

    /// Add receipt
    fn add_receipt(
        &self,
        block_hash: BlockHash,
        index: Index,
        receipt: Receipt,
    ) -> Result<(), StoreError>;

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
        Ok(Some(Block { header, body }))
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

    // Update earliest block number
    fn update_earliest_block_number(&self, block_number: BlockNumber) -> Result<(), StoreError>;

    // Obtain earliest block number
    fn get_earliest_block_number(&self) -> Result<Option<BlockNumber>, StoreError>;

    // Update finalized block number
    fn update_finalized_block_number(&self, block_number: BlockNumber) -> Result<(), StoreError>;

    // Obtain finalized block number
    fn get_finalized_block_number(&self) -> Result<Option<BlockNumber>, StoreError>;

    // Update safe block number
    fn update_safe_block_number(&self, block_number: BlockNumber) -> Result<(), StoreError>;

    // Obtain safe block number
    fn get_safe_block_number(&self) -> Result<Option<BlockNumber>, StoreError>;

    // Update latest block number
    fn update_latest_block_number(&self, block_number: BlockNumber) -> Result<(), StoreError>;

    // Obtain latest block number
    fn get_latest_block_number(&self) -> Result<Option<BlockNumber>, StoreError>;

    // TODO (#307): Remove TotalDifficulty.
    // Update latest total difficulty
    fn update_latest_total_difficulty(
        &self,
        latest_total_difficulty: U256,
    ) -> Result<(), StoreError>;

    // TODO (#307): Remove TotalDifficulty.
    // Obtain latest total difficulty
    fn get_latest_total_difficulty(&self) -> Result<Option<U256>, StoreError>;

    // Update pending block number
    fn update_pending_block_number(&self, block_number: BlockNumber) -> Result<(), StoreError>;

    // Obtain pending block number
    fn get_pending_block_number(&self) -> Result<Option<BlockNumber>, StoreError>;

    // Obtain a storage trie from the given address and storage_root
    // Doesn't check if the account is stored
    // Used for internal store operations
    fn open_storage_trie(&self, address: Address, storage_root: H256) -> Trie;

    // Obtain a state trie from the given state root
    // Doesn't check if the state root is valid
    // Used for internal store operations
    fn open_state_trie(&self, state_root: H256) -> Trie;

    // Set the canonical block hash for a given block number.
    fn set_canonical_block(&self, number: BlockNumber, hash: BlockHash) -> Result<(), StoreError>;

    // Unsets canonical block for a block number.
    fn unset_canonical_block(&self, number: BlockNumber) -> Result<(), StoreError>;

    fn add_payload(&self, payload_id: u64, block: Block) -> Result<(), StoreError>;

    fn get_payload(&self, payload_id: u64) -> Result<Option<Block>, StoreError>;
}
