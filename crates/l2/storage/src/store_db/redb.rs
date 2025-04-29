use std::{panic::RefUnwindSafe, sync::Arc};

use ethrex_common::{types::BlockNumber, H256};
use ethrex_rlp::encode::RLPEncode;
use ethrex_storage::error::StoreError;
use redb::{AccessGuard, Database, Key, TableDefinition, Value};

use crate::{
    api::StoreEngineRollup,
    rlp::{BlockNumbersRLP, WithdrawalHashesRLP},
};

const BATCHES_BY_BLOCK_NUMBER_TABLE: TableDefinition<BlockNumber, u64> =
    TableDefinition::new("BatchesByBlockNumbers");

const WITHDRAWALS_BY_BATCH: TableDefinition<u64, WithdrawalHashesRLP> =
    TableDefinition::new("WithdrawalHashesByBatch");

const BLOCK_NUMBERS_BY_BATCH: TableDefinition<u64, BlockNumbersRLP> =
    TableDefinition::new("BlockNumbersByBatch");

#[derive(Debug)]
pub struct RedBStoreRollup {
    db: Arc<Database>,
}

impl RefUnwindSafe for RedBStoreRollup {}
impl RedBStoreRollup {
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
    async fn read<'k, 'a, K, V>(
        &self,
        table: TableDefinition<'a, K, V>,
        key: K::SelfType<'k>,
    ) -> Result<Option<AccessGuard<'static, V>>, StoreError>
    where
        K: Key + Send + 'static,
        V: Value + Send + 'static,
        K::SelfType<'k>: Send,
        'a: 'static,
        'k: 'static,
    {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            let read_txn = db.begin_read()?;
            let table = read_txn.open_table(table)?;
            let result = table.get(key)?;
            Ok(result)
        })
        .await
        .map_err(|e| StoreError::Custom(format!("task panicked: {e}")))?
    }
}

pub fn init_db() -> Result<Database, StoreError> {
    let db = Database::create("ethrex_l2.redb")?;

    let table_creation_txn = db.begin_write()?;

    table_creation_txn.open_table(BATCHES_BY_BLOCK_NUMBER_TABLE)?;
    table_creation_txn.open_table(WITHDRAWALS_BY_BATCH)?;
    table_creation_txn.commit()?;

    Ok(db)
}

#[async_trait::async_trait]
impl StoreEngineRollup for RedBStoreRollup {
    async fn get_batch_number_by_block(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<u64>, StoreError> {
        Ok(self
            .read(BATCHES_BY_BLOCK_NUMBER_TABLE, block_number)
            .await?
            .map(|b| b.value()))
    }

    async fn store_batch_number_by_block(
        &self,
        block_number: BlockNumber,
        batch_number: u64,
    ) -> Result<(), StoreError> {
        self.write(BATCHES_BY_BLOCK_NUMBER_TABLE, block_number, batch_number)
            .await
    }

    async fn get_withdrawal_hashes_by_batch(
        &self,
        batch_number: u64,
    ) -> Result<Option<Vec<H256>>, StoreError> {
        Ok(self
            .read(WITHDRAWALS_BY_BATCH, batch_number)
            .await?
            .map(|w| w.value().to()))
    }

    async fn store_withdrawal_hashes_by_batch(
        &self,
        batch_number: u64,
        withdrawals: Vec<H256>,
    ) -> Result<(), StoreError> {
        self.write(
            WITHDRAWALS_BY_BATCH,
            batch_number,
            <Vec<H256> as Into<WithdrawalHashesRLP>>::into(withdrawals),
        )
        .await
    }

    async fn store_block_numbers_by_batch(
        &self,
        batch_number: u64,
        block_numbers: Vec<BlockNumber>,
    ) -> Result<(), StoreError> {
        self.write(
            BLOCK_NUMBERS_BY_BATCH,
            batch_number,
            BlockNumbersRLP::from_bytes(block_numbers.encode_to_vec()),
        )
        .await
    }

    async fn get_block_numbers_by_batch(
        &self,
        batch_number: u64,
    ) -> Result<Option<Vec<BlockNumber>>, StoreError> {
        Ok(self
            .read(BLOCK_NUMBERS_BY_BATCH, batch_number)
            .await?
            .map(|rlp| rlp.value().to()))
    }

    async fn contains_batch(&self, batch_number: &u64) -> Result<bool, StoreError> {
        let exists = self
            .read(BLOCK_NUMBERS_BY_BATCH, *batch_number)
            .await?
            .is_some();
        Ok(exists)
    }
}
