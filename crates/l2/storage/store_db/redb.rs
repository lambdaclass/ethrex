use std::{borrow::Borrow, panic::RefUnwindSafe, sync::Arc};

use ethrex_common::types::BlockNumber;
use ethrex_storage::error::StoreError;
use redb::{AccessGuard, Database, Key, TableDefinition, Value};

use crate::storage::api::StoreEngineL2;

const BATCHES_BY_BLOCK_NUMBER_TABLE: TableDefinition<BlockNumber, u64> =
    TableDefinition::new("BatchesByBlockNumbers");

#[derive(Debug)]
pub struct RedBStoreL2 {
    db: Arc<Database>,
}

impl RefUnwindSafe for RedBStoreL2 {}
impl RedBStoreL2 {
    pub fn new() -> Result<Self, StoreError> {
        Ok(Self {
            db: Arc::new(init_db()?),
        })
    }

    // Helper method to write into a redb table
    async fn write<'k, 'v, 'a, K, V>(
        &self,
        table: TableDefinition<'a, K, V>,
        key: K::SelfType<'k>,
        value: V::SelfType<'v>,
    ) -> Result<(), StoreError>
    where
        K: Key + Send + 'static,
        V: Value + Send + 'static,
        K::SelfType<'k>: Send,
        V::SelfType<'v>: Send,
        'a: 'static,
        'k: 'static,
        'v: 'static,
    {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            let write_txn = db.begin_write()?;
            write_txn.open_table(table)?.insert(key, value)?;
            write_txn.commit()?;

            Ok(())
        })
        .await
        .map_err(|e| StoreError::Custom(format!("task panicked: {e}")))?
    }
    // Helper method to read from a redb table
    fn read<'k, 'a, K, V>(
        &self,
        table: TableDefinition<'a, K, V>,
        key: impl Borrow<K::SelfType<'k>>,
    ) -> Result<Option<AccessGuard<'static, V>>, StoreError>
    where
        K: Key + 'static,
        V: Value,
    {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(table)?;
        let result = table.get(key)?;

        Ok(result)
    }
}

pub fn init_db() -> Result<Database, StoreError> {
    let db = Database::create("ethrex_l2.redb")?;

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
