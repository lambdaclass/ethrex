// Storage API for L2

use std::fmt::Debug;

use ethrex_common::{
    H256,
    types::{
        AccountUpdate, Blob, BlockNumber, balance_diff::BalanceDiff, batch::Batch,
        fee_config::FeeConfig,
    },
};
use ethrex_l2_common::prover::{BatchProof, ProverInputData, ProverType};

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

    /// Gets the L1 message hashes by a given batch number.
    async fn get_l1_out_message_hashes_by_batch(
        &self,
        batch_number: u64,
    ) -> Result<Option<Vec<H256>>, RollupStoreError>;

    /// Returns the block numbers by a given batch_number
    async fn get_block_numbers_by_batch(
        &self,
        batch_number: u64,
    ) -> Result<Option<Vec<BlockNumber>>, RollupStoreError>;

    /// Returns the balance diffs by a given batch_number
    async fn get_balance_diffs_by_batch(
        &self,
        batch_number: u64,
    ) -> Result<Option<Vec<BalanceDiff>>, RollupStoreError>;

    async fn get_l1_in_messages_rolling_hash_by_batch_number(
        &self,
        batch_number: u64,
    ) -> Result<Option<H256>, RollupStoreError>;

    async fn get_l2_in_message_rolling_hashes_by_batch(
        &self,
        batch_number: u64,
    ) -> Result<Option<Vec<(u64, H256)>>, RollupStoreError>;

    async fn get_non_privileged_transactions_by_batch(
        &self,
        batch_number: u64,
    ) -> Result<Option<u64>, RollupStoreError>;

    async fn get_state_root_by_batch_number(
        &self,
        batch_number: u64,
    ) -> Result<Option<H256>, RollupStoreError>;

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

    async fn seal_batch(&self, batch: Batch) -> Result<(), RollupStoreError>;

    async fn seal_batch_with_prover_input(
        &self,
        batch: Batch,
        prover_version: &str,
        prover_input_data: ProverInputData,
    ) -> Result<(), RollupStoreError>;

    async fn get_last_batch_number(&self) -> Result<Option<u64>, RollupStoreError>;

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
        signature: ethereum_types::Signature,
    ) -> Result<(), RollupStoreError>;

    /// Retrieves the sequencer signature for a given block hash.
    async fn get_signature_by_block(
        &self,
        block_hash: H256,
    ) -> Result<Option<ethereum_types::Signature>, RollupStoreError>;

    /// Stores the sequencer signature for a given batch number.
    async fn store_signature_by_batch(
        &self,
        batch_number: u64,
        signature: ethereum_types::Signature,
    ) -> Result<(), RollupStoreError>;

    /// Retrieves the sequencer signature for a given batch number.
    async fn get_signature_by_batch(
        &self,
        batch_number: u64,
    ) -> Result<Option<ethereum_types::Signature>, RollupStoreError>;

    async fn get_latest_sent_batch_proof(&self) -> Result<u64, RollupStoreError>;

    async fn set_latest_sent_batch_proof(&self, batch_number: u64) -> Result<(), RollupStoreError>;

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

    async fn delete_proof_by_batch_and_type(
        &self,
        batch_number: u64,
        proof_type: ProverType,
    ) -> Result<(), RollupStoreError>;

    async fn revert_to_batch(&self, batch_number: u64) -> Result<(), RollupStoreError>;

    async fn store_prover_input_by_batch_and_version(
        &self,
        batch_number: u64,
        prover_version: &str,
        prover_input: ProverInputData,
    ) -> Result<(), RollupStoreError>;

    async fn get_prover_input_by_batch_and_version(
        &self,
        batch_number: u64,
        prover_version: &str,
    ) -> Result<Option<ProverInputData>, RollupStoreError>;

    async fn store_fee_config_by_block(
        &self,
        block_number: BlockNumber,
        fee_config: FeeConfig,
    ) -> Result<(), RollupStoreError>;

    async fn get_fee_config_by_block(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<FeeConfig>, RollupStoreError>;
}
