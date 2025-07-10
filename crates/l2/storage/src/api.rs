// Storage API for L2

use std::fmt::Debug;

use ethrex_common::{
    H256,
    types::{AccountUpdate, Blob, BlockNumber},
};
use ethrex_l2_common::prover::{BatchProof, ProverType};

use crate::error::RollupStoreError;

// We need async_trait because the stabilized feature lacks support for object safety
// (i.e. dyn StoreEngine)
#[async_trait::async_trait]
pub trait StoreEngineRollup: Debug + Send + Sync {
    /// Returns the batch number by a given block number.
    async fn get_batch_number_by_block(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<u64>, RollupStoreError>;

    /// Stores the batch number by a given block number.
    async fn store_batch_number_by_block(
        &self,
        block_number: BlockNumber,
        batch_number: u64,
    ) -> Result<(), RollupStoreError>;

    /// Gets the message hashes by a given batch number.
    async fn get_message_hashes_by_batch(
        &self,
        batch_number: u64,
    ) -> Result<Option<Vec<H256>>, RollupStoreError>;

    /// Stores the message hashes by a given batch number.
    async fn store_message_hashes_by_batch(
        &self,
        batch_number: u64,
        message_hashes: Vec<H256>,
    ) -> Result<(), RollupStoreError>;

    /// Stores the block numbers by a given batch_number
    async fn store_block_numbers_by_batch(
        &self,
        batch_number: u64,
        block_numbers: Vec<BlockNumber>,
    ) -> Result<(), RollupStoreError>;

    /// Returns the block numbers by a given batch_number
    async fn get_block_numbers_by_batch(
        &self,
        batch_number: u64,
    ) -> Result<Option<Vec<BlockNumber>>, RollupStoreError>;

    async fn store_privileged_transactions_hash_by_batch_number(
        &self,
        batch_number: u64,
        privileged_transactions_hash: H256,
    ) -> Result<(), RollupStoreError>;

    async fn get_privileged_transactions_hash_by_batch_number(
        &self,
        batch_number: u64,
    ) -> Result<Option<H256>, RollupStoreError>;

    async fn store_state_root_by_batch_number(
        &self,
        batch_number: u64,
        state_root: H256,
    ) -> Result<(), RollupStoreError>;

    async fn get_state_root_by_batch_number(
        &self,
        batch_number: u64,
    ) -> Result<Option<H256>, RollupStoreError>;

    async fn store_blob_bundle_by_batch_number(
        &self,
        batch_number: u64,
        state_diff: Vec<Blob>,
    ) -> Result<(), RollupStoreError>;

    async fn get_blob_bundle_by_batch_number(
        &self,
        batch_number: u64,
    ) -> Result<Option<Vec<Blob>>, RollupStoreError>;

    async fn get_commit_tx_by_batch(
        &self,
        batch_number: u64,
    ) -> Result<Option<H256>, RollupStoreError>;

    async fn store_commit_tx_by_batch(
        &self,
        batch_number: u64,
        commit_tx: H256,
    ) -> Result<(), RollupStoreError>;

    async fn get_verify_tx_by_batch(
        &self,
        batch_number: u64,
    ) -> Result<Option<H256>, RollupStoreError>;

    async fn store_verify_tx_by_batch(
        &self,
        batch_number: u64,
        verify_tx: H256,
    ) -> Result<(), RollupStoreError>;

    async fn update_operations_count(
        &self,
        transaction_inc: u64,
        privileged_tx_inc: u64,
        messages_inc: u64,
    ) -> Result<(), RollupStoreError>;

    async fn get_operations_count(&self) -> Result<[u64; 3], RollupStoreError>;

    /// Returns whether the batch with the given number is present.
    async fn contains_batch(&self, batch_number: &u64) -> Result<bool, RollupStoreError>;

    /// Stores the sequencer signature for a given block hash.
    async fn store_signature_by_block(
        &self,
        block_hash: H256,
        signature: [u8; 68],
    ) -> Result<(), RollupStoreError>;

    /// Retrieves the sequencer signature for a given block hash.
    async fn get_signature_by_block(
        &self,
        block_hash: H256,
    ) -> Result<Option<[u8; 68]>, RollupStoreError>;

    /// Stores the sequencer signature for a given batch number.
    async fn store_signature_by_batch(
        &self,
        batch_number: u64,
        signature: [u8; 68],
    ) -> Result<(), RollupStoreError>;

    /// Retrieves the sequencer signature for a given batch number.
    async fn get_signature_by_batch(
        &self,
        batch_number: u64,
    ) -> Result<Option<[u8; 68]>, RollupStoreError>;

    async fn get_lastest_sent_batch_proof(&self) -> Result<u64, RollupStoreError>;

    async fn set_lastest_sent_batch_proof(&self, batch_number: u64)
    -> Result<(), RollupStoreError>;

    async fn get_account_updates_by_block_number(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<Vec<AccountUpdate>>, RollupStoreError>;

    async fn store_account_updates_by_block_number(
        &self,
        block_number: BlockNumber,
        account_updates: Vec<AccountUpdate>,
    ) -> Result<(), RollupStoreError>;

    async fn store_proof_by_batch_and_type(
        &self,
        batch_number: u64,
        proof_type: ProverType,
        proof: BatchProof,
    ) -> Result<(), RollupStoreError>;

    async fn get_proof_by_batch_and_type(
        &self,
        batch_number: u64,
        proof_type: ProverType,
    ) -> Result<Option<BatchProof>, RollupStoreError>;

    async fn revert_to_batch(&self, batch_number: u64) -> Result<(), RollupStoreError>;

    /// Get latest batch number
    async fn get_latest_batch_number(&self) -> Result<u64, RollupStoreError>;

    /// Set latest batch number
    async fn set_latest_batch_number(&self, batch_number: u64) -> Result<(), RollupStoreError>;
}
