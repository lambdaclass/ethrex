use ethrex_common::types::BlockNumber;
use libmdbx::{table, table_info};

pub use crate::store_db::libmdbx::Store as LibmdbxStoreL2;
use crate::{api_l2::StoreEngineL2, error::StoreError};

impl LibmdbxStoreL2 {
    pub fn new_l2(path: &str) -> Result<Self, StoreError> {
        let tables = [table_info!(BatchesByBlockNumber)].into_iter().collect();
        Self::new_with_tables(path, tables)
    }
}

#[async_trait::async_trait]
impl StoreEngineL2 for LibmdbxStoreL2 {
    fn get_batch_number_for_block(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<u64>, StoreError> {
        self.read::<BatchesByBlockNumber>(block_number)
    }

    async fn store_batch_number_for_block(
        &self,
        block_number: BlockNumber,
        batch_number: u64,
    ) -> Result<(), StoreError> {
        self.write::<BatchesByBlockNumber>(block_number, batch_number)
            .await
    }
}

table!(
    /// Batch number by block number
    ( BatchesByBlockNumber ) BlockNumber => u64
);
