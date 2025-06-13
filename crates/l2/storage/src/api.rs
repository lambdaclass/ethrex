// Storage API for L2

use std::{fmt::Debug, panic::RefUnwindSafe};

use ethrex_common::{
    types::{AccountUpdate, Blob, BlockNumber},
};
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

    async fn store_deposit_logs_hash_by_batch_number(
        &self,
        batch_number: u64,
        deposit_logs_hash: H256,
    ) -> Result<(), StoreError>;

    async fn get_deposit_logs_hash_by_batch_number(
        &self,
        batch_number: u64,
    ) -> Result<Option<H256>, StoreError>;

    async fn store_state_root_by_batch_number(
        &self,
        batch_number: u64,
        state_root: H256,
    ) -> Result<(), StoreError>;

    async fn get_state_root_by_batch_number(
        &self,
        batch_number: u64,
    ) -> Result<Option<H256>, StoreError>;

    async fn store_blob_bundle_by_batch_number(
        &self,
        batch_number: u64,
        state_diff: Vec<Blob>,
    ) -> Result<(), StoreError>;

    async fn get_blob_bundle_by_batch_number(
        &self,
        batch_number: u64,
    ) -> Result<Option<Vec<Blob>>, StoreError>;

    async fn update_operations_count(
        &self,
        transaction_inc: u64,
        deposits_inc: u64,
        withdrawals_inc: u64,
    ) -> Result<(), StoreError>;

    async fn get_operations_count(&self) -> Result<[u64; 3], StoreError>;

    /// Returns whether the batch with the given number is present.
    async fn contains_batch(&self, batch_number: &u64) -> Result<bool, StoreError>;

    async fn get_lastest_sent_batch_proof(&self) -> Result<u64, StoreError>;

    async fn set_lastest_sent_batch_proof(&self, batch_number: u64) -> Result<(), StoreError>;

    async fn get_account_updates_by_block_number(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<Vec<AccountUpdate>>, StoreError>;

    async fn store_account_updates_by_block_number(
        &self,
        block_number: BlockNumber,
        account_updates: Vec<AccountUpdate>,
    ) -> Result<(), StoreError>;
}
