use ethrex_common::types::BlockNumber;
use redb::{Database, TableDefinition};

pub use crate::store_db::redb::RedBStore as RedBStoreL2;
use crate::{api_l2::StoreEngineL2, error::StoreError};

const BATCHES_BY_BLOCK_NUMBER_TABLE: TableDefinition<BlockNumber, u64> =
    TableDefinition::new("BatchesByBlockNumbers");

impl RedBStoreL2 {
    pub fn new_l2(path: &str) -> Result<Self, StoreError> {
        Self::new_with_db(init_db(path)?)
    }
}

pub fn init_db(path: &str) -> Result<Database, StoreError> {
    let db = Database::create(path)?;
    let table_creation_txn = db.begin_write()?;
    table_creation_txn.open_table(BATCHES_BY_BLOCK_NUMBER_TABLE)?;
    table_creation_txn.commit()?;
    Ok(db)
}

#[async_trait::async_trait]
impl StoreEngineL2 for RedBStoreL2 {
    fn get_batch_number_for_block(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<u64>, StoreError> {
        Ok(self
            .read(BATCHES_BY_BLOCK_NUMBER_TABLE, block_number)?
            .map(|b| b.value()))
    }

    async fn store_batch_number_for_block(
        &self,
        block_number: BlockNumber,
        batch_number: u64,
    ) -> Result<(), StoreError> {
        self.write(BATCHES_BY_BLOCK_NUMBER_TABLE, block_number, batch_number)
            .await
    }
}
