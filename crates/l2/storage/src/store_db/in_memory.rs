use std::{
    collections::HashMap,
    fmt::Debug,
    sync::{Arc, Mutex, MutexGuard},
};

use crate::error::RollupStoreError;
use ethrex_common::{
    H256,
    types::{
        AccountUpdate, Blob, BlockNumber, balance_diff::BalanceDiff, batch::Batch,
        fee_config::FeeConfig,
    },
};
use ethrex_l2_common::prover::{BatchProof, ProverInputData, ProverType};

use crate::api::StoreEngineRollup;

#[derive(Default, Clone)]
pub struct Store(Arc<Mutex<StoreInner>>);

#[derive(Default, Debug)]
struct StoreInner {
    /// Map of batches by block numbers
    batches_by_block: HashMap<BlockNumber, u64>,
    /// Map of l1 message hashes by batch numbers
    l1_out_message_hashes_by_batch: HashMap<u64, Vec<H256>>,
    /// Map of balance diffs by batch numbers
    balance_diffs_by_batch: HashMap<u64, Vec<BalanceDiff>>,
    /// Map of batch number to block numbers
    block_numbers_by_batch: HashMap<u64, Vec<BlockNumber>>,
    /// Map of batch number to deposit logs hash
    l1_in_messages_rolling_hashes: HashMap<u64, H256>,
    /// Map of batch number to L2 in message rolling hashes
    l2_in_message_rolling_hashes: HashMap<u64, Vec<(u64, H256)>>,
    /// Map of batch number to non-privileged transactions count
    non_privileged_transactions_by_batch: HashMap<u64, u64>,
    /// Map of batch number to state root
    state_roots: HashMap<u64, H256>,
    /// Map of batch number to blob
    blobs: HashMap<u64, Vec<Blob>>,
    /// latest sent batch proof
    latest_sent_batch_proof: u64,
    /// Metrics for transaction, deposits and messages count
    operations_counts: [u64; 3],
    /// Map of signatures from the sequencer by block hashes
    signatures_by_block: HashMap<H256, ethereum_types::Signature>,
    /// Map of signatures from the sequencer by batch numbers
    signatures_by_batch: HashMap<u64, ethereum_types::Signature>,
    /// Map of block number to account updates
    account_updates_by_block_number: HashMap<BlockNumber, Vec<AccountUpdate>>,
    /// Map of (ProverType, batch_number) to batch proof data
    batch_proofs: HashMap<(ProverType, u64), BatchProof>,
    /// Map of batch number to commit transaction hash
    commit_txs: HashMap<u64, H256>,
    /// Map of batch number to verify transaction hash
    verify_txs: HashMap<u64, H256>,
    /// Map of (batch_number, prover_version) to serialized prover input data
    batch_prover_input: HashMap<(u64, String), Vec<u8>>,
    /// Map of block number to FeeConfig
    fee_config_by_block: HashMap<BlockNumber, FeeConfig>,
}

impl Store {
    pub fn new() -> Self {
        Self::default()
    }
    fn inner(&self) -> Result<MutexGuard<'_, StoreInner>, RollupStoreError> {
        self.0
            .lock()
            .map_err(|_| RollupStoreError::Custom("Failed to lock the store".to_string()))
    }
}

#[async_trait::async_trait]
impl StoreEngineRollup for Store {
    async fn get_batch_number_by_block(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<u64>, RollupStoreError> {
        Ok(self.inner()?.batches_by_block.get(&block_number).copied())
    }

    async fn get_l1_out_message_hashes_by_batch(
        &self,
        batch_number: u64,
    ) -> Result<Option<Vec<H256>>, RollupStoreError> {
        Ok(self
            .inner()?
            .l1_out_message_hashes_by_batch
            .get(&batch_number)
            .cloned())
    }

    async fn get_balance_diffs_by_batch(
        &self,
        batch_number: u64,
    ) -> Result<Option<Vec<BalanceDiff>>, RollupStoreError> {
        Ok(self
            .inner()?
            .balance_diffs_by_batch
            .get(&batch_number)
            .cloned())
    }

    /// Returns the block numbers for a given batch_number
    async fn get_block_numbers_by_batch(
        &self,
        batch_number: u64,
    ) -> Result<Option<Vec<BlockNumber>>, RollupStoreError> {
        let block_numbers = self
            .inner()?
            .block_numbers_by_batch
            .get(&batch_number)
            .cloned();
        Ok(block_numbers)
    }

    async fn get_l1_in_messages_rolling_hash_by_batch_number(
        &self,
        batch_number: u64,
    ) -> Result<Option<H256>, RollupStoreError> {
        Ok(self
            .inner()?
            .l1_in_messages_rolling_hashes
            .get(&batch_number)
            .cloned())
    }

    async fn get_l2_in_message_rolling_hashes_by_batch(
        &self,
        batch_number: u64,
    ) -> Result<Option<Vec<(u64, H256)>>, RollupStoreError> {
        Ok(self
            .inner()?
            .l2_in_message_rolling_hashes
            .get(&batch_number)
            .cloned())
    }

    async fn get_state_root_by_batch_number(
        &self,
        batch_number: u64,
    ) -> Result<Option<H256>, RollupStoreError> {
        Ok(self.inner()?.state_roots.get(&batch_number).cloned())
    }

    async fn get_blob_bundle_by_batch_number(
        &self,
        batch_number: u64,
    ) -> Result<Option<Vec<Blob>>, RollupStoreError> {
        Ok(self.inner()?.blobs.get(&batch_number).cloned())
    }

    async fn get_commit_tx_by_batch(
        &self,
        batch_number: u64,
    ) -> Result<Option<H256>, RollupStoreError> {
        Ok(self.inner()?.commit_txs.get(&batch_number).cloned())
    }

    async fn store_commit_tx_by_batch(
        &self,
        batch_number: u64,
        commit_tx: H256,
    ) -> Result<(), RollupStoreError> {
        self.inner()?.commit_txs.insert(batch_number, commit_tx);
        Ok(())
    }

    async fn get_verify_tx_by_batch(
        &self,
        batch_number: u64,
    ) -> Result<Option<H256>, RollupStoreError> {
        Ok(self.inner()?.verify_txs.get(&batch_number).cloned())
    }

    async fn store_verify_tx_by_batch(
        &self,
        batch_number: u64,
        verify_tx: H256,
    ) -> Result<(), RollupStoreError> {
        self.inner()?.verify_txs.insert(batch_number, verify_tx);
        Ok(())
    }

    async fn contains_batch(&self, batch_number: &u64) -> Result<bool, RollupStoreError> {
        Ok(self
            .inner()?
            .block_numbers_by_batch
            .contains_key(batch_number))
    }

    async fn update_operations_count(
        &self,
        transaction_inc: u64,
        privileged_tx_inc: u64,
        messages_inc: u64,
    ) -> Result<(), RollupStoreError> {
        let mut inner = self.inner()?;
        inner.operations_counts[0] += transaction_inc;
        inner.operations_counts[1] += privileged_tx_inc;
        inner.operations_counts[2] += messages_inc;
        Ok(())
    }

    async fn get_operations_count(&self) -> Result<[u64; 3], RollupStoreError> {
        Ok(self.inner()?.operations_counts)
    }

    async fn store_signature_by_block(
        &self,
        block_hash: H256,
        signature: ethereum_types::Signature,
    ) -> Result<(), RollupStoreError> {
        self.inner()?
            .signatures_by_block
            .insert(block_hash, signature);
        Ok(())
    }

    async fn get_signature_by_block(
        &self,
        block_hash: H256,
    ) -> Result<Option<ethereum_types::Signature>, RollupStoreError> {
        Ok(self.inner()?.signatures_by_block.get(&block_hash).cloned())
    }

    async fn store_signature_by_batch(
        &self,
        batch_number: u64,
        signature: ethereum_types::Signature,
    ) -> Result<(), RollupStoreError> {
        self.inner()?
            .signatures_by_batch
            .insert(batch_number, signature);
        Ok(())
    }

    async fn get_signature_by_batch(
        &self,
        batch_number: u64,
    ) -> Result<Option<ethereum_types::Signature>, RollupStoreError> {
        Ok(self
            .inner()?
            .signatures_by_batch
            .get(&batch_number)
            .cloned())
    }

    async fn get_latest_sent_batch_proof(&self) -> Result<u64, RollupStoreError> {
        Ok(self.inner()?.latest_sent_batch_proof)
    }

    async fn set_latest_sent_batch_proof(&self, batch_number: u64) -> Result<(), RollupStoreError> {
        self.inner()?.latest_sent_batch_proof = batch_number;
        Ok(())
    }

    async fn get_account_updates_by_block_number(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<Vec<AccountUpdate>>, RollupStoreError> {
        Ok(self
            .inner()?
            .account_updates_by_block_number
            .get(&block_number)
            .cloned())
    }

    async fn store_account_updates_by_block_number(
        &self,
        block_number: BlockNumber,
        account_updates: Vec<AccountUpdate>,
    ) -> Result<(), RollupStoreError> {
        self.inner()?
            .account_updates_by_block_number
            .insert(block_number, account_updates);
        Ok(())
    }

    async fn store_proof_by_batch_and_type(
        &self,
        batch_number: u64,
        proof_type: ProverType,
        proof: BatchProof,
    ) -> Result<(), RollupStoreError> {
        self.inner()?
            .batch_proofs
            .insert((proof_type, batch_number), proof);
        Ok(())
    }

    async fn get_proof_by_batch_and_type(
        &self,
        batch_number: u64,
        proof_type: ProverType,
    ) -> Result<Option<BatchProof>, RollupStoreError> {
        Ok(self
            .inner()?
            .batch_proofs
            .get(&(proof_type, batch_number))
            .cloned())
    }

    async fn get_non_privileged_transactions_by_batch(
        &self,
        batch_number: u64,
    ) -> Result<Option<u64>, RollupStoreError> {
        Ok(self
            .inner()?
            .non_privileged_transactions_by_batch
            .get(&batch_number)
            .cloned())
    }

    async fn revert_to_batch(&self, batch_number: u64) -> Result<(), RollupStoreError> {
        let mut store = self.inner()?;
        store
            .batches_by_block
            .retain(|_, batch| *batch <= batch_number);
        store
            .l1_out_message_hashes_by_batch
            .retain(|batch, _| *batch <= batch_number);
        store
            .block_numbers_by_batch
            .retain(|batch, _| *batch <= batch_number);
        store
            .l1_in_messages_rolling_hashes
            .retain(|batch, _| *batch <= batch_number);
        store.state_roots.retain(|batch, _| *batch <= batch_number);
        store.blobs.retain(|batch, _| *batch <= batch_number);
        store
            .batch_prover_input
            .retain(|(batch, _), _| *batch <= batch_number);
        Ok(())
    }

    async fn seal_batch(&self, batch: Batch) -> Result<(), RollupStoreError> {
        let mut inner = self.inner()?;
        let blocks: Vec<u64> = (batch.first_block..=batch.last_block).collect();

        for block_number in blocks.iter() {
            inner.batches_by_block.insert(*block_number, batch.number);
        }

        inner.block_numbers_by_batch.insert(batch.number, blocks);

        inner
            .l1_out_message_hashes_by_batch
            .insert(batch.number, batch.l1_out_message_hashes);

        inner
            .l1_in_messages_rolling_hashes
            .insert(batch.number, batch.l1_in_messages_rolling_hash);

        inner
            .non_privileged_transactions_by_batch
            .insert(batch.number, batch.non_privileged_transactions);

        inner
            .balance_diffs_by_batch
            .insert(batch.number, batch.balance_diffs);

        inner
            .l2_in_message_rolling_hashes
            .insert(batch.number, batch.l2_in_message_rolling_hashes);

        inner.blobs.insert(batch.number, batch.blobs_bundle.blobs);

        inner.state_roots.insert(batch.number, batch.state_root);

        if let Some(commit_tx) = batch.commit_tx {
            inner.commit_txs.insert(batch.number, commit_tx);
        }
        if let Some(verify_tx) = batch.verify_tx {
            inner.verify_txs.insert(batch.number, verify_tx);
        }
        Ok(())
    }

    async fn seal_batch_with_prover_input(
        &self,
        batch: Batch,
        prover_version: &str,
        prover_input_data: ProverInputData,
    ) -> Result<(), RollupStoreError> {
        let batch_number = batch.number;

        // There is no problem in performing these two operations not atomically
        // as in the in-memory store restarts will lose all data anyway.
        self.seal_batch(batch).await?;
        self.store_prover_input_by_batch_and_version(
            batch_number,
            prover_version,
            prover_input_data,
        )
        .await
    }

    async fn delete_proof_by_batch_and_type(
        &self,
        batch_number: u64,
        proof_type: ProverType,
    ) -> Result<(), RollupStoreError> {
        let mut inner = self.inner()?;
        inner.batch_proofs.remove(&(proof_type, batch_number));
        Ok(())
    }

    async fn get_last_batch_number(&self) -> Result<Option<u64>, RollupStoreError> {
        Ok(self.inner()?.state_roots.keys().max().cloned())
    }

    async fn store_prover_input_by_batch_and_version(
        &self,
        batch_number: u64,
        prover_version: &str,
        prover_input: ProverInputData,
    ) -> Result<(), RollupStoreError> {
        let witness_bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&prover_input)
            .map_err(|e| RollupStoreError::Custom(format!("Failed to serialize witness: {}", e)))?
            .to_vec();

        self.inner()?
            .batch_prover_input
            .insert((batch_number, prover_version.to_string()), witness_bytes);

        Ok(())
    }

    async fn get_prover_input_by_batch_and_version(
        &self,
        batch_number: u64,
        prover_version: &str,
    ) -> Result<Option<ProverInputData>, RollupStoreError> {
        let Some(witness_bytes) = self
            .inner()?
            .batch_prover_input
            .get(&(batch_number, prover_version.to_string()))
            .cloned()
        else {
            return Ok(None);
        };

        let prover_input = rkyv::from_bytes::<ProverInputData, rkyv::rancor::Error>(&witness_bytes)
            .map_err(|e| {
                RollupStoreError::Custom(format!(
                    "Failed to deserialize prover input for batch {batch_number} and version {prover_version}: {e}",
                ))
            })?;

        Ok(Some(prover_input))
    }

    async fn store_fee_config_by_block(
        &self,
        block_number: BlockNumber,
        fee_config: FeeConfig,
    ) -> Result<(), RollupStoreError> {
        self.inner()?
            .fee_config_by_block
            .insert(block_number, fee_config);
        Ok(())
    }

    async fn get_fee_config_by_block(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<FeeConfig>, RollupStoreError> {
        Ok(self
            .inner()?
            .fee_config_by_block
            .get(&block_number)
            .cloned())
    }
}

impl Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("In Memory L2 Store").finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn update_operations_count_accumulates_values() {
        let store = Store::new();

        store
            .update_operations_count(1, 2, 3)
            .await
            .expect("first update should succeed");

        store
            .update_operations_count(4, 5, 6)
            .await
            .expect("second update should succeed");

        let counts = store
            .get_operations_count()
            .await
            .expect("get_operations_count should succeed");

        assert_eq!(counts[0], 5);
        assert_eq!(counts[1], 7);
        assert_eq!(counts[2], 9);
    }
}
