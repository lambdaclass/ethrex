use std::{
    collections::HashMap,
    fmt::Debug,
    sync::{Arc, Mutex, MutexGuard},
};

use ethrex_common::{types::BlockNumber, H256};
use ethrex_storage::error::StoreError;

use crate::api::StoreEngineRollup;

#[derive(Default, Clone)]
pub struct Store(Arc<Mutex<StoreInner>>);

#[derive(Default, Debug)]
struct StoreInner {
    /// Map of batches by block numbers
    batches_by_block: HashMap<BlockNumber, u64>,
    /// Map of withdrawals hashes by batch numbers
    withdrawal_hashes_by_batch: HashMap<u64, Vec<H256>>,
    /// Map of batch number to block numbers
    block_numbers_by_batch: HashMap<u64, Vec<BlockNumber>>,
    /// Metrics for transaction, deposits and withdrawals count
    operations_counts: [u64; 3],
    /// Map of signatures from the sequencer by block hashes
    signatures_by_block: HashMap<H256, [u8; 68]>,
    /// Map of signatures from the sequencer by batch numbers
    signatures_by_batch: HashMap<u64, [u8; 68]>,
}

impl Store {
    pub fn new() -> Self {
        Self::default()
    }
    fn inner(&self) -> Result<MutexGuard<'_, StoreInner>, StoreError> {
        self.0
            .lock()
            .map_err(|_| StoreError::Custom("Failed to lock the store".to_string()))
    }
}

#[async_trait::async_trait]
impl StoreEngineRollup for Store {
    async fn get_batch_number_by_block(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<u64>, StoreError> {
        Ok(self.inner()?.batches_by_block.get(&block_number).copied())
    }

    async fn store_batch_number_by_block(
        &self,
        block_number: BlockNumber,
        batch_number: u64,
    ) -> Result<(), StoreError> {
        self.inner()?
            .batches_by_block
            .insert(block_number, batch_number);
        Ok(())
    }

    async fn get_withdrawal_hashes_by_batch(
        &self,
        batch_number: u64,
    ) -> Result<Option<Vec<H256>>, StoreError> {
        Ok(self
            .inner()?
            .withdrawal_hashes_by_batch
            .get(&batch_number)
            .cloned())
    }

    async fn store_withdrawal_hashes_by_batch(
        &self,
        batch_number: u64,
        withdrawals: Vec<H256>,
    ) -> Result<(), StoreError> {
        self.inner()?
            .withdrawal_hashes_by_batch
            .insert(batch_number, withdrawals);
        Ok(())
    }

    /// Returns the block numbers for a given batch_number
    async fn store_block_numbers_by_batch(
        &self,
        batch_number: u64,
        block_numbers: Vec<BlockNumber>,
    ) -> Result<(), StoreError> {
        self.inner()?
            .block_numbers_by_batch
            .insert(batch_number, block_numbers);
        Ok(())
    }

    /// Returns the block numbers for a given batch_number
    async fn get_block_numbers_by_batch(
        &self,
        batch_number: u64,
    ) -> Result<Option<Vec<BlockNumber>>, StoreError> {
        let block_numbers = self
            .inner()?
            .block_numbers_by_batch
            .get(&batch_number)
            .cloned();
        Ok(block_numbers)
    }

    async fn contains_batch(&self, batch_number: &u64) -> Result<bool, StoreError> {
        Ok(self
            .inner()?
            .block_numbers_by_batch
            .contains_key(batch_number))
    }

    async fn update_operations_count(
        &self,
        transaction_inc: u64,
        deposits_inc: u64,
        withdrawals_inc: u64,
    ) -> Result<(), StoreError> {
        let mut values = self.inner()?.operations_counts;
        values[0] += transaction_inc;
        values[1] += deposits_inc;
        values[2] += withdrawals_inc;
        Ok(())
    }

    async fn get_operations_count(&self) -> Result<[u64; 3], StoreError> {
        Ok(self.inner()?.operations_counts)
    }

    async fn store_signature_by_block(
        &self,
        block_hash: H256,
        signature: [u8; 68],
    ) -> Result<(), StoreError> {
        self.inner()?
            .signatures_by_block
            .insert(block_hash, signature);
        Ok(())
    }

    async fn get_signature_by_block(
        &self,
        block_hash: H256,
    ) -> Result<Option<[u8; 68]>, StoreError> {
        Ok(self.inner()?.signatures_by_block.get(&block_hash).cloned())
    }

    async fn store_signature_by_batch(
        &self,
        batch_number: u64,
        signature: [u8; 68],
    ) -> Result<(), StoreError> {
        self.inner()?
            .signatures_by_batch
            .insert(batch_number, signature);
        Ok(())
    }

    async fn get_signature_by_batch(
        &self,
        batch_number: u64,
    ) -> Result<Option<[u8; 68]>, StoreError> {
        Ok(self
            .inner()?
            .signatures_by_batch
            .get(&batch_number)
            .cloned())
    }
}

impl Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("In Memory L2 Store").finish()
    }
}
