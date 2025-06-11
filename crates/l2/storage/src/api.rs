// Storage API for L2

use std::{fmt::Debug, panic::RefUnwindSafe};

use ethrex_common::{types::BlockNumber, H256};
use ethrex_storage::error::StoreError;

// We need async_trait because the stabilized feature lacks support for object safety
// (i.e. dyn StoreEngine)
#[async_trait::async_trait]
pub trait StoreEngineRollup: Debug + Send + Sync + RefUnwindSafe {
    /// Returns the batch number by a given block number.
    async fn get_batch_number_by_block(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<u64>, StoreError>;

    /// Stores the batch number by a given block number.
    async fn store_batch_number_by_block(
        &self,
        block_number: BlockNumber,
        batch_number: u64,
    ) -> Result<(), StoreError>;

    /// Gets the withdrawal hashes by a given batch number.
    async fn get_withdrawal_hashes_by_batch(
        &self,
        batch_number: u64,
    ) -> Result<Option<Vec<H256>>, StoreError>;

    /// Stores the withdrawal hashes by a given batch number.
    async fn store_withdrawal_hashes_by_batch(
        &self,
        batch_number: u64,
        withdrawal_hashes: Vec<H256>,
    ) -> Result<(), StoreError>;

    /// Stores the block numbers by a given batch_number
    async fn store_block_numbers_by_batch(
        &self,
        batch_number: u64,
        block_numbers: Vec<BlockNumber>,
    ) -> Result<(), StoreError>;

    /// Returns the block numbers by a given batch_number
    async fn get_block_numbers_by_batch(
        &self,
        batch_number: u64,
    ) -> Result<Option<Vec<BlockNumber>>, StoreError>;

    async fn update_operations_count(
        &self,
        transaction_inc: u64,
        deposits_inc: u64,
        withdrawals_inc: u64,
    ) -> Result<(), StoreError>;

    async fn get_operations_count(&self) -> Result<[u64; 3], StoreError>;

    /// Returns whether the batch with the given number is present.
    async fn contains_batch(&self, batch_number: &u64) -> Result<bool, StoreError>;

    async fn store_signature_by_block(
        &self,
        block_hash: H256,
        signature: [u8; 68],
    ) -> Result<(), StoreError>;

    async fn get_signature_by_block(
        &self,
        block_hash: H256,
    ) -> Result<Option<[u8; 68]>, StoreError>;

    async fn store_signature_by_batch(
        &self,
        batch_number: u64,
        signature: [u8; 68],
    ) -> Result<(), StoreError>;

    async fn get_signature_by_batch(
        &self,
        batch_number: u64,
    ) -> Result<Option<[u8; 68]>, StoreError>;
}
