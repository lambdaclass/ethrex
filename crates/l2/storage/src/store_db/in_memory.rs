use std::{
    collections::HashMap,
    fmt::Debug,
    sync::{Arc, Mutex, MutexGuard},
};

use ethrex_common::types::BlockNumber;
use ethrex_storage::error::StoreError;

use crate::api::StoreEngineL2;

#[derive(Default, Clone)]
pub struct Store(Arc<Mutex<StoreInner>>);

#[derive(Default, Debug)]
struct StoreInner {
    /// Map of block number to batch number
    batches_by_block: HashMap<BlockNumber, u64>,
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
impl StoreEngineL2 for Store {
    async fn get_batch_number_for_block(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<u64>, StoreError> {
        Ok(self.inner()?.batches_by_block.get(&block_number).copied())
    }

    async fn store_batch_number_for_block(
        &self,
        block_number: BlockNumber,
        batch_number: u64,
    ) -> Result<(), StoreError> {
        self.inner()?
            .batches_by_block
            .insert(block_number, batch_number);
        Ok(())
    }
}

impl Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("In Memory L2 Store").finish()
    }
}
