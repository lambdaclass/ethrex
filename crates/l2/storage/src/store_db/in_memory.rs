use std::{
    collections::HashMap,
    fmt::Debug,
    ops::Range,
    sync::{Arc, Mutex, MutexGuard},
};

use crate::error::RollupStoreError;
use ethrex_common::{
    H256,
    types::{AccountUpdate, Blob, BlockNumber, batch::Batch},
};
use ethrex_l2_common::prover::{BatchProof, ProverType};

use crate::api::StoreEngineRollup;

#[derive(Default, Clone)]
pub struct Store(Arc<Mutex<StoreInner>>);

#[derive(Default, Debug)]
struct StoreInner {
    /// Map of batches by block numbers
    batches_by_block: HashMap<BlockNumber, u64>,
    /// Map of message hashes by batch numbers
    message_hashes_by_batch: HashMap<u64, Vec<H256>>,
    /// Map of batch number to block numbers
    block_numbers_by_batch: HashMap<u64, Vec<BlockNumber>>,
    /// Map of batch number to deposit logs hash
    privileged_transactions_hashes: HashMap<u64, H256>,
    /// Map of batch number to state root
    state_roots: HashMap<u64, H256>,
    /// Map of batch number to blob
    blobs: HashMap<u64, Vec<Blob>>,
    /// Lastest sent batch proof
    lastest_sent_batch_proof: u64,
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
    /// Privileged transactions included in the batch being built
    precommit_privileged: Option<Range<u64>>,
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

    async fn get_message_hashes_by_batch(
        &self,
        batch_number: u64,
    ) -> Result<Option<Vec<H256>>, RollupStoreError> {
        Ok(self
            .inner()?
            .message_hashes_by_batch
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

    async fn get_privileged_transactions_hash_by_batch_number(
        &self,
        batch_number: u64,
    ) -> Result<Option<H256>, RollupStoreError> {
        Ok(self
            .inner()?
            .privileged_transactions_hashes
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
        let mut values = self.inner()?.operations_counts;
        values[0] += transaction_inc;
        values[1] += privileged_tx_inc;
        values[2] += messages_inc;
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

    async fn get_lastest_sent_batch_proof(&self) -> Result<u64, RollupStoreError> {
        Ok(self.inner()?.lastest_sent_batch_proof)
    }

    async fn set_lastest_sent_batch_proof(
        &self,
        batch_number: u64,
    ) -> Result<(), RollupStoreError> {
        self.inner()?.lastest_sent_batch_proof = batch_number;
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

    async fn revert_to_batch(&self, batch_number: u64) -> Result<(), RollupStoreError> {
        let mut store = self.inner()?;
        store
            .batches_by_block
            .retain(|_, batch| *batch <= batch_number);
        store
            .message_hashes_by_batch
            .retain(|batch, _| *batch <= batch_number);
        store
            .block_numbers_by_batch
            .retain(|batch, _| *batch <= batch_number);
        store
            .privileged_transactions_hashes
            .retain(|batch, _| *batch <= batch_number);
        store.state_roots.retain(|batch, _| *batch <= batch_number);
        store.blobs.retain(|batch, _| *batch <= batch_number);
        store.precommit_privileged = None;
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
            .message_hashes_by_batch
            .insert(batch.number, batch.message_hashes);

        inner
            .privileged_transactions_hashes
            .insert(batch.number, batch.privileged_transactions_hash);

        inner.blobs.insert(batch.number, batch.blobs_bundle.blobs);

        inner.state_roots.insert(batch.number, batch.state_root);

        if let Some(commit_tx) = batch.commit_tx {
            inner.commit_txs.insert(batch.number, commit_tx);
        }
        if let Some(verify_tx) = batch.verify_tx {
            inner.verify_txs.insert(batch.number, verify_tx);
        }
        inner.precommit_privileged = None;
        Ok(())
    }

    async fn precommit_privileged(&self) -> Result<Option<Range<u64>>, RollupStoreError> {
        Ok(self.inner()?.precommit_privileged.clone())
    }

    async fn update_precommit_privileged(
        &self,
        range: Option<Range<u64>>,
    ) -> Result<(), RollupStoreError> {
        self.inner()?.precommit_privileged = range;
        Ok(())
    }
}

impl Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("In Memory L2 Store").finish()
    }
}
