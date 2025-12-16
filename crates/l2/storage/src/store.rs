use std::{path::Path, sync::Arc};

use crate::api::StoreEngineRollup;
use crate::error::RollupStoreError;
use crate::store_db::in_memory::Store as InMemoryStore;
#[cfg(feature = "sql")]
use crate::store_db::sql::SQLStore;
use ethrex_common::{
    H256,
    types::{
        AccountUpdate, Blob, BlobsBundle, BlockNumber, Fork, balance_diff::BalanceDiff,
        batch::Batch, fee_config::FeeConfig,
    },
};
use ethrex_l2_common::prover::{BatchProof, ProverInputData, ProverType};
use tracing::info;

#[derive(Debug, Clone)]
pub struct Store {
    engine: Arc<dyn StoreEngineRollup>,
}

impl Default for Store {
    fn default() -> Self {
        Self {
            engine: Arc::new(InMemoryStore::new()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineType {
    InMemory,
    #[cfg(feature = "sql")]
    SQL,
}

impl Store {
    pub fn new(_path: &Path, engine_type: EngineType) -> Result<Self, RollupStoreError> {
        info!("Starting l2 storage engine ({engine_type:?})");
        let store = match engine_type {
            EngineType::InMemory => Self {
                engine: Arc::new(InMemoryStore::new()),
            },
            #[cfg(feature = "sql")]
            EngineType::SQL => Self {
                engine: Arc::new(SQLStore::new(_path)?),
            },
        };
        info!("Started l2 store engine");
        Ok(store)
    }

    pub async fn init(&self) -> Result<(), RollupStoreError> {
        // Stores batch 0 with block 0
        self.seal_batch(Batch {
            number: 0,
            first_block: 0,
            last_block: 0,
            state_root: H256::zero(),
            l1_in_messages_rolling_hash: H256::zero(),
            l2_in_message_rolling_hashes: Vec::new(),
            l1_out_message_hashes: Vec::new(),
            non_privileged_transactions: 0,
            balance_diffs: Vec::new(),
            blobs_bundle: BlobsBundle::empty(),
            commit_tx: None,
            verify_tx: None,
        })
        .await?;
        // Sets the latest sent batch proof to 0
        if self.get_latest_sent_batch_proof().await.is_err() {
            // If not set, we initialize it to 0
            self.set_latest_sent_batch_proof(0).await?;
        };
        Ok(())
    }

    /// Returns the block numbers by a given batch_number
    pub async fn get_block_numbers_by_batch(
        &self,
        batch_number: u64,
    ) -> Result<Option<Vec<BlockNumber>>, RollupStoreError> {
        self.engine.get_block_numbers_by_batch(batch_number).await
    }

    pub async fn get_batch_number_by_block(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<u64>, RollupStoreError> {
        self.engine.get_batch_number_by_block(block_number).await
    }

    pub async fn get_l1_out_message_hashes_by_batch(
        &self,
        batch_number: u64,
    ) -> Result<Option<Vec<H256>>, RollupStoreError> {
        self.engine
            .get_l1_out_message_hashes_by_batch(batch_number)
            .await
    }

    pub async fn get_balance_diffs_by_batch(
        &self,
        batch_number: u64,
    ) -> Result<Option<Vec<BalanceDiff>>, RollupStoreError> {
        self.engine.get_balance_diffs_by_batch(batch_number).await
    }

    pub async fn get_l1_in_messages_rolling_hash_by_batch_number(
        &self,
        batch_number: u64,
    ) -> Result<Option<H256>, RollupStoreError> {
        self.engine
            .get_l1_in_messages_rolling_hash_by_batch_number(batch_number)
            .await
    }

    pub async fn get_l2_in_message_rolling_hashes_by_batch(
        &self,
        batch_number: u64,
    ) -> Result<Option<Vec<(u64, H256)>>, RollupStoreError> {
        self.engine
            .get_l2_in_message_rolling_hashes_by_batch(batch_number)
            .await
    }

    pub async fn get_state_root_by_batch(
        &self,
        batch_number: u64,
    ) -> Result<Option<H256>, RollupStoreError> {
        self.engine
            .get_state_root_by_batch_number(batch_number)
            .await
    }

    pub async fn get_blobs_by_batch(
        &self,
        batch_number: u64,
    ) -> Result<Option<Vec<Blob>>, RollupStoreError> {
        self.engine
            .get_blob_bundle_by_batch_number(batch_number)
            .await
    }

    pub async fn get_commit_tx_by_batch(
        &self,
        batch_number: u64,
    ) -> Result<Option<H256>, RollupStoreError> {
        self.engine.get_commit_tx_by_batch(batch_number).await
    }

    pub async fn store_commit_tx_by_batch(
        &self,
        batch_number: u64,
        commit_tx: H256,
    ) -> Result<(), RollupStoreError> {
        self.engine
            .store_commit_tx_by_batch(batch_number, commit_tx)
            .await
    }

    pub async fn get_verify_tx_by_batch(
        &self,
        batch_number: u64,
    ) -> Result<Option<H256>, RollupStoreError> {
        self.engine.get_verify_tx_by_batch(batch_number).await
    }

    pub async fn store_verify_tx_by_batch(
        &self,
        batch_number: u64,
        verify_tx: H256,
    ) -> Result<(), RollupStoreError> {
        self.engine
            .store_verify_tx_by_batch(batch_number, verify_tx)
            .await
    }

    pub async fn get_batch_number(&self) -> Result<Option<u64>, RollupStoreError> {
        self.engine.get_last_batch_number().await
    }

    pub async fn get_batch(
        &self,
        batch_number: u64,
        fork: Fork,
    ) -> Result<Option<Batch>, RollupStoreError> {
        let Some(blocks) = self.get_block_numbers_by_batch(batch_number).await? else {
            return Ok(None);
        };

        let first_block = *blocks.first().ok_or(RollupStoreError::Custom(
            "Failed while trying to retrieve the first block of a known batch. This is a bug."
                .to_owned(),
        ))?;
        let last_block = *blocks.last().ok_or(RollupStoreError::Custom(
            "Failed while trying to retrieve the last block of a known batch. This is a bug."
                .to_owned(),
        ))?;

        let state_root =
            self.get_state_root_by_batch(batch_number)
                .await?
                .ok_or(RollupStoreError::Custom(
                "Failed while trying to retrieve the state root of a known batch. This is a bug."
                    .to_owned(),
            ))?;

        let blobs_bundle = BlobsBundle::create_from_blobs(
            // Currently validium mode doesn't generate blobs, so no one will be stored
            // TODO: If/When that behaviour change, this should throw error on None
            &self
                .get_blobs_by_batch(batch_number)
                .await?
                .unwrap_or_default(),
                if fork <= Fork::Prague { None } else { Some(1) },
        ).map_err(|e| {
            RollupStoreError::Custom(format!("Failed to create blobs bundle from blob while getting batch from database: {e}. This is a bug"))
        })?;

        let l1_out_message_hashes = self
            .get_l1_out_message_hashes_by_batch(batch_number)
            .await?
            .unwrap_or_default();

        let balance_diffs = self
            .get_balance_diffs_by_batch(batch_number)
            .await?
            .unwrap_or_default();

        let l1_in_messages_rolling_hash = self
            .get_l1_in_messages_rolling_hash_by_batch_number(batch_number)
            .await?.ok_or(RollupStoreError::Custom(
            "Failed while trying to retrieve the deposit logs hash of a known batch. This is a bug."
                .to_owned(),
        ))?;

        let non_privileged_transactions = self
            .engine
            .get_non_privileged_transactions_by_batch(batch_number)
            .await?
            .ok_or(RollupStoreError::Custom(
            "Failed while trying to retrieve the non-privileged transactions count of a known batch. This is a bug."
                .to_owned(),
        ))?;

        let l2_in_message_rolling_hashes = self
            .get_l2_in_message_rolling_hashes_by_batch(batch_number)
            .await?
            .ok_or(RollupStoreError::Custom(
            "Failed while trying to retrieve the L2 in messages rolling hashes of a known batch. This is a bug."
                .to_owned(),
        ))?;

        let commit_tx = self.get_commit_tx_by_batch(batch_number).await?;

        let verify_tx = self.get_verify_tx_by_batch(batch_number).await?;

        Ok(Some(Batch {
            number: batch_number,
            first_block,
            last_block,
            state_root,
            blobs_bundle,
            l1_out_message_hashes,
            l1_in_messages_rolling_hash,
            l2_in_message_rolling_hashes,
            non_privileged_transactions,
            balance_diffs,
            commit_tx,
            verify_tx,
        }))
    }

    pub async fn seal_batch(&self, batch: Batch) -> Result<(), RollupStoreError> {
        self.engine.seal_batch(batch).await
    }

    /// Seals a batch along with its prover input data in one atomic operation.
    pub async fn seal_batch_with_prover_input(
        &self,
        batch: Batch,
        prover_version: &str,
        prover_input: ProverInputData,
    ) -> Result<(), RollupStoreError> {
        self.engine
            .seal_batch_with_prover_input(batch, prover_version, prover_input)
            .await
    }

    pub async fn update_operations_count(
        &self,
        transaction_inc: u64,
        privileged_tx_inc: u64,
        messages_inc: u64,
    ) -> Result<(), RollupStoreError> {
        self.engine
            .update_operations_count(transaction_inc, privileged_tx_inc, messages_inc)
            .await
    }

    pub async fn get_operations_count(&self) -> Result<[u64; 3], RollupStoreError> {
        self.engine.get_operations_count().await
    }

    /// Returns whether the batch with the given number is present.
    pub async fn contains_batch(&self, batch_number: &u64) -> Result<bool, RollupStoreError> {
        self.engine.contains_batch(batch_number).await
    }

    /// Stores the sequencer signature for a given block hash.
    /// When the lead sequencer sends a block by P2P, it signs the message and it is validated
    /// If we want to gossip or broadcast the message, we need to store the signature for later use
    pub async fn store_signature_by_block(
        &self,
        block_hash: H256,
        signature: ethereum_types::Signature,
    ) -> Result<(), RollupStoreError> {
        self.engine
            .store_signature_by_block(block_hash, signature)
            .await
    }

    /// Returns the sequencer signature for a given block hash.
    /// We want to retrieve the validated signature to broadcast or gossip the block to the peers
    /// So they can also validate the message
    pub async fn get_signature_by_block(
        &self,
        block_hash: H256,
    ) -> Result<Option<ethereum_types::Signature>, RollupStoreError> {
        self.engine.get_signature_by_block(block_hash).await
    }

    /// Stores the sequencer signature for a given batch number.
    /// When the lead sequencer sends a batch by P2P, it
    /// should also sign it, this will map a batch number
    /// to the batch's signature.
    pub async fn store_signature_by_batch(
        &self,
        batch_number: u64,
        signature: ethereum_types::Signature,
    ) -> Result<(), RollupStoreError> {
        self.engine
            .store_signature_by_batch(batch_number, signature)
            .await
    }

    /// Returns the sequencer signature for a given batch number.
    /// This is used mostly in P2P to avoid signing an
    /// already known batch.
    pub async fn get_signature_by_batch(
        &self,
        batch_number: u64,
    ) -> Result<Option<ethereum_types::Signature>, RollupStoreError> {
        self.engine.get_signature_by_batch(batch_number).await
    }

    /// Returns the latest sent batch proof
    pub async fn get_latest_sent_batch_proof(&self) -> Result<u64, RollupStoreError> {
        self.engine.get_latest_sent_batch_proof().await
    }

    /// Sets the latest sent batch proof
    pub async fn set_latest_sent_batch_proof(
        &self,
        batch_number: u64,
    ) -> Result<(), RollupStoreError> {
        self.engine.set_latest_sent_batch_proof(batch_number).await
    }

    /// Returns the account updates yielded from executing a block
    pub async fn get_account_updates_by_block_number(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<Vec<AccountUpdate>>, RollupStoreError> {
        self.engine
            .get_account_updates_by_block_number(block_number)
            .await
    }

    /// Stores the account updates yielded from executing a block
    pub async fn store_account_updates_by_block_number(
        &self,
        block_number: BlockNumber,
        account_updates: Vec<AccountUpdate>,
    ) -> Result<(), RollupStoreError> {
        self.engine
            .store_account_updates_by_block_number(block_number, account_updates)
            .await
    }

    pub async fn store_proof_by_batch_and_type(
        &self,
        batch_number: u64,
        proof_type: ProverType,
        proof: BatchProof,
    ) -> Result<(), RollupStoreError> {
        self.engine
            .store_proof_by_batch_and_type(batch_number, proof_type, proof)
            .await
    }

    pub async fn get_proof_by_batch_and_type(
        &self,
        batch_number: u64,
        proof_type: ProverType,
    ) -> Result<Option<BatchProof>, RollupStoreError> {
        self.engine
            .get_proof_by_batch_and_type(batch_number, proof_type)
            .await
    }

    /// Reverts to a previous batch, discarding operations in them
    pub async fn revert_to_batch(&self, batch_number: u64) -> Result<(), RollupStoreError> {
        self.engine.revert_to_batch(batch_number).await
    }

    pub async fn delete_proof_by_batch_and_type(
        &self,
        batch_number: u64,
        proof_type: ProverType,
    ) -> Result<(), RollupStoreError> {
        self.engine
            .delete_proof_by_batch_and_type(batch_number, proof_type)
            .await
    }

    pub async fn store_prover_input_by_batch_and_version(
        &self,
        batch_number: u64,
        prover_version: &str,
        prover_input: ProverInputData,
    ) -> Result<(), RollupStoreError> {
        self.engine
            .store_prover_input_by_batch_and_version(batch_number, prover_version, prover_input)
            .await
    }

    pub async fn get_prover_input_by_batch_and_version(
        &self,
        batch_number: u64,
        prover_version: &str,
    ) -> Result<Option<ProverInputData>, RollupStoreError> {
        self.engine
            .get_prover_input_by_batch_and_version(batch_number, prover_version)
            .await
    }

    pub async fn store_fee_config_by_block(
        &self,
        block_number: BlockNumber,
        fee_config: FeeConfig,
    ) -> Result<(), RollupStoreError> {
        self.engine
            .store_fee_config_by_block(block_number, fee_config)
            .await
    }
    pub async fn get_fee_config_by_block(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<FeeConfig>, RollupStoreError> {
        self.engine.get_fee_config_by_block(block_number).await
    }
}
