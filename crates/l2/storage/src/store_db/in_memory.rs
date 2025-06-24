use std::{
    collections::HashMap,
    fmt::Debug,
    sync::{Arc, Mutex, MutexGuard},
};

use crate::error::RollupStoreError;
use ethrex_common::{
    H256,
    types::{AccountUpdate, Blob, BlockNumber},
};

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
    deposit_logs_hashes: HashMap<u64, H256>,
    /// Map of batch number to state root
    state_roots: HashMap<u64, H256>,
    /// Map of batch number to blob
    blobs: HashMap<u64, Vec<Blob>>,
    /// Lastest sent batch proof
    lastest_sent_batch_proof: u64,
    /// Metrics for transaction, deposits and messages count
    operations_counts: [u64; 3],
    /// Map of block number to account updates
    account_updates_by_block_number: HashMap<BlockNumber, Vec<AccountUpdate>>,
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

    async fn store_batch_number_by_block(
        &self,
        block_number: BlockNumber,
        batch_number: u64,
    ) -> Result<(), RollupStoreError> {
        self.inner()?
            .batches_by_block
            .insert(block_number, batch_number);
        Ok(())
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

    async fn store_message_hashes_by_batch(
        &self,
        batch_number: u64,
        messages: Vec<H256>,
    ) -> Result<(), RollupStoreError> {
        self.inner()?
            .message_hashes_by_batch
            .insert(batch_number, messages);
        Ok(())
    }

    /// Returns the block numbers for a given batch_number
    async fn store_block_numbers_by_batch(
        &self,
        batch_number: u64,
        block_numbers: Vec<BlockNumber>,
    ) -> Result<(), RollupStoreError> {
        self.inner()?
            .block_numbers_by_batch
            .insert(batch_number, block_numbers);
        Ok(())
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

    async fn store_deposit_logs_hash_by_batch_number(
        &self,
        batch_number: u64,
        deposit_logs_hash: H256,
    ) -> Result<(), RollupStoreError> {
        self.inner()?
            .deposit_logs_hashes
            .insert(batch_number, deposit_logs_hash);
        Ok(())
    }

    async fn get_deposit_logs_hash_by_batch_number(
        &self,
        batch_number: u64,
    ) -> Result<Option<H256>, RollupStoreError> {
        Ok(self
            .inner()?
            .deposit_logs_hashes
            .get(&batch_number)
            .cloned())
    }

    async fn store_state_root_by_batch_number(
        &self,
        batch_number: u64,
        state_root: H256,
    ) -> Result<(), RollupStoreError> {
        self.inner()?.state_roots.insert(batch_number, state_root);
        Ok(())
    }

    async fn get_state_root_by_batch_number(
        &self,
        batch_number: u64,
    ) -> Result<Option<H256>, RollupStoreError> {
        Ok(self.inner()?.state_roots.get(&batch_number).cloned())
    }

    async fn store_blob_bundle_by_batch_number(
        &self,
        batch_number: u64,
        state_diff: Vec<Blob>,
    ) -> Result<(), RollupStoreError> {
        self.inner()?.blobs.insert(batch_number, state_diff);
        Ok(())
    }

    async fn get_blob_bundle_by_batch_number(
        &self,
        batch_number: u64,
    ) -> Result<Option<Vec<Blob>>, RollupStoreError> {
        Ok(self.inner()?.blobs.get(&batch_number).cloned())
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
        deposits_inc: u64,
        messages_inc: u64,
    ) -> Result<(), RollupStoreError> {
        let mut values = self.inner()?.operations_counts;
        values[0] += transaction_inc;
        values[1] += deposits_inc;
        values[2] += messages_inc;
        Ok(())
    }

    async fn get_operations_count(&self) -> Result<[u64; 3], RollupStoreError> {
        Ok(self.inner()?.operations_counts)
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
            .deposit_logs_hashes
            .retain(|batch, _| *batch <= batch_number);
        store.state_roots.retain(|batch, _| *batch <= batch_number);
        store.blobs.retain(|batch, _| *batch <= batch_number);
        Ok(())
    }
}

impl Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("In Memory L2 Store").finish()
    }
}
