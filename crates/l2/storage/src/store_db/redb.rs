use std::{panic::RefUnwindSafe, sync::Arc};

use crate::error::RollupStoreError;
use ethrex_common::{
    H256,
    types::{AccountUpdate, Blob, BlockNumber},
};
use ethrex_rlp::encode::RLPEncode;
use redb::{AccessGuard, Database, Key, ReadableTable, TableDefinition, Value, WriteTransaction};

use crate::{
    api::StoreEngineRollup,
    rlp::{BlockNumbersRLP, MessageHashesRLP, OperationsCountRLP, Rlp},
};

const BATCHES_BY_BLOCK_NUMBER_TABLE: TableDefinition<BlockNumber, u64> =
    TableDefinition::new("BatchesByBlockNumbers");

const MESSAGES_BY_BATCH: TableDefinition<u64, MessageHashesRLP> =
    TableDefinition::new("MesageHashesByBatch");

const BLOCK_NUMBERS_BY_BATCH: TableDefinition<u64, BlockNumbersRLP> =
    TableDefinition::new("BlockNumbersByBatch");

const OPERATIONS_COUNTS: TableDefinition<u64, OperationsCountRLP> =
    TableDefinition::new("OperationsCount");

const BLOB_BUNDLES: TableDefinition<u64, Rlp<Vec<Blob>>> = TableDefinition::new("BlobBundles");

const STATE_ROOTS: TableDefinition<u64, Rlp<H256>> = TableDefinition::new("StateRoots");

const DEPOSIT_LOGS_HASHES: TableDefinition<u64, Rlp<H256>> =
    TableDefinition::new("DepositLogsHashes");

const LAST_SENT_BATCH_PROOF: TableDefinition<u64, u64> = TableDefinition::new("LastSentBatchProof");

const ACCOUNT_UPDATES_BY_BLOCK_NUMBER: TableDefinition<BlockNumber, Vec<u8>> =
    TableDefinition::new("AccountUpdatesByBlockNumber");

#[derive(Debug)]
pub struct RedBStoreRollup {
    db: Arc<Database>,
}

impl RefUnwindSafe for RedBStoreRollup {}
impl RedBStoreRollup {
    pub fn new() -> Result<Self, RollupStoreError> {
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
    ) -> Result<(), RollupStoreError>
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
            let write_txn = db.begin_write().map_err(Box::new)?;
            write_txn.open_table(table)?.insert(key, value)?;
            write_txn.commit()?;

            Ok(())
        })
        .await
        .map_err(|e| RollupStoreError::Custom(format!("task panicked: {e}")))?
    }
    // Helper method to read from a redb table
    async fn read<'k, 'a, K, V>(
        &self,
        table: TableDefinition<'a, K, V>,
        key: K::SelfType<'k>,
    ) -> Result<Option<AccessGuard<'static, V>>, RollupStoreError>
    where
        K: Key + Send + 'static,
        V: Value + Send + 'static,
        K::SelfType<'k>: Send,
        'a: 'static,
        'k: 'static,
    {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            let read_txn = db.begin_read().map_err(Box::new)?;
            let table = read_txn.open_table(table)?;
            let result = table.get(key)?;
            Ok(result)
        })
        .await
        .map_err(|e| RollupStoreError::Custom(format!("task panicked: {e}")))?
    }
}

pub fn init_db() -> Result<Database, RollupStoreError> {
    let db = Database::create("ethrex_l2.redb")?;

    let table_creation_txn = db.begin_write().map_err(Box::new)?;

    table_creation_txn.open_table(BATCHES_BY_BLOCK_NUMBER_TABLE)?;
    table_creation_txn.open_table(MESSAGES_BY_BATCH)?;
    table_creation_txn.open_table(OPERATIONS_COUNTS)?;
    table_creation_txn.open_table(BLOB_BUNDLES)?;
    table_creation_txn.open_table(STATE_ROOTS)?;
    table_creation_txn.open_table(DEPOSIT_LOGS_HASHES)?;
    table_creation_txn.open_table(BLOCK_NUMBERS_BY_BATCH)?;
    table_creation_txn.open_table(LAST_SENT_BATCH_PROOF)?;
    table_creation_txn.open_table(ACCOUNT_UPDATES_BY_BLOCK_NUMBER)?;
    table_creation_txn.commit()?;

    Ok(db)
}

#[async_trait::async_trait]
impl StoreEngineRollup for RedBStoreRollup {
    async fn get_batch_number_by_block(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<u64>, RollupStoreError> {
        Ok(self
            .read(BATCHES_BY_BLOCK_NUMBER_TABLE, block_number)
            .await?
            .map(|b| b.value()))
    }

    async fn store_batch_number_by_block(
        &self,
        block_number: BlockNumber,
        batch_number: u64,
    ) -> Result<(), RollupStoreError> {
        self.write(BATCHES_BY_BLOCK_NUMBER_TABLE, block_number, batch_number)
            .await
    }

    async fn get_message_hashes_by_batch(
        &self,
        batch_number: u64,
    ) -> Result<Option<Vec<H256>>, RollupStoreError> {
        Ok(self
            .read(MESSAGES_BY_BATCH, batch_number)
            .await?
            .map(|w| w.value().to()))
    }

    async fn store_message_hashes_by_batch(
        &self,
        batch_number: u64,
        messages: Vec<H256>,
    ) -> Result<(), RollupStoreError> {
        self.write(
            MESSAGES_BY_BATCH,
            batch_number,
            <Vec<H256> as Into<MessageHashesRLP>>::into(messages),
        )
        .await
    }

    async fn store_block_numbers_by_batch(
        &self,
        batch_number: u64,
        block_numbers: Vec<BlockNumber>,
    ) -> Result<(), RollupStoreError> {
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
    ) -> Result<Option<Vec<BlockNumber>>, RollupStoreError> {
        Ok(self
            .read(BLOCK_NUMBERS_BY_BATCH, batch_number)
            .await?
            .map(|rlp| rlp.value().to()))
    }

    async fn contains_batch(&self, batch_number: &u64) -> Result<bool, RollupStoreError> {
        let exists = self
            .read(BLOCK_NUMBERS_BY_BATCH, *batch_number)
            .await?
            .is_some();
        Ok(exists)
    }

    async fn store_deposit_logs_hash_by_batch_number(
        &self,
        batch_number: u64,
        deposit_logs_hash: H256,
    ) -> Result<(), RollupStoreError> {
        self.write(DEPOSIT_LOGS_HASHES, batch_number, deposit_logs_hash.into())
            .await
    }

    async fn get_deposit_logs_hash_by_batch_number(
        &self,
        batch_number: u64,
    ) -> Result<Option<H256>, RollupStoreError> {
        Ok(self
            .read(DEPOSIT_LOGS_HASHES, batch_number)
            .await?
            .map(|rlp| rlp.value().to()))
    }

    async fn store_state_root_by_batch_number(
        &self,
        batch_number: u64,
        state_root: H256,
    ) -> Result<(), RollupStoreError> {
        self.write(STATE_ROOTS, batch_number, state_root.into())
            .await
    }

    async fn get_state_root_by_batch_number(
        &self,
        batch_number: u64,
    ) -> Result<Option<H256>, RollupStoreError> {
        Ok(self
            .read(STATE_ROOTS, batch_number)
            .await?
            .map(|rlp| rlp.value().to()))
    }

    async fn store_blob_bundle_by_batch_number(
        &self,
        batch_number: u64,
        state_diff: Vec<Blob>,
    ) -> Result<(), RollupStoreError> {
        self.write(BLOB_BUNDLES, batch_number, state_diff.into())
            .await
    }

    async fn get_blob_bundle_by_batch_number(
        &self,
        batch_number: u64,
    ) -> Result<Option<Vec<Blob>>, RollupStoreError> {
        Ok(self
            .read(BLOB_BUNDLES, batch_number)
            .await?
            .map(|rlp| rlp.value().to()))
    }

    async fn update_operations_count(
        &self,
        transaction_inc: u64,
        deposits_inc: u64,
        messages_inc: u64,
    ) -> Result<(), RollupStoreError> {
        let (transaction_count, messages_count, deposits_count) = {
            let current_operations = self.get_operations_count().await?;
            (
                current_operations[0] + transaction_inc,
                current_operations[1] + messages_inc,
                current_operations[2] + deposits_inc,
            )
        };

        self.write(
            OPERATIONS_COUNTS,
            0,
            OperationsCountRLP::from_bytes(
                vec![transaction_count, messages_count, deposits_count].encode_to_vec(),
            ),
        )
        .await
    }

    async fn get_operations_count(&self) -> Result<[u64; 3], RollupStoreError> {
        let operations = self
            .read(OPERATIONS_COUNTS, 0)
            .await?
            .map(|rlp| rlp.value().to());
        match operations {
            Some(mut operations) => Ok([
                operations.pop().unwrap_or_default(),
                operations.pop().unwrap_or_default(),
                operations.pop().unwrap_or_default(),
            ]),
            _ => Ok([0, 0, 0]),
        }
    }

    async fn get_lastest_sent_batch_proof(&self) -> Result<u64, RollupStoreError> {
        Ok(self
            .read(LAST_SENT_BATCH_PROOF, 0)
            .await?
            .map(|b| b.value())
            .unwrap_or(0))
    }

    async fn set_lastest_sent_batch_proof(
        &self,
        batch_number: u64,
    ) -> Result<(), RollupStoreError> {
        self.write(LAST_SENT_BATCH_PROOF, 0, batch_number).await
    }

    async fn get_account_updates_by_block_number(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<Vec<AccountUpdate>>, RollupStoreError> {
        self.read(ACCOUNT_UPDATES_BY_BLOCK_NUMBER, block_number)
            .await?
            .map(|s| bincode::deserialize(&s.value()))
            .transpose()
            .map_err(RollupStoreError::from)
    }

    async fn store_account_updates_by_block_number(
        &self,
        block_number: BlockNumber,
        account_updates: Vec<AccountUpdate>,
    ) -> Result<(), RollupStoreError> {
        let serialized = bincode::serialize(&account_updates)?;
        self.write(ACCOUNT_UPDATES_BY_BLOCK_NUMBER, block_number, serialized)
            .await
    }

    async fn revert_to_batch(&self, batch_number: u64) -> Result<(), RollupStoreError> {
        let Some(kept_blocks) = self.get_block_numbers_by_batch(batch_number).await? else {
            return Ok(());
        };
        let last_kept_block = *kept_blocks.iter().max().unwrap_or(&0);
        let txn = self.db.begin_write().map_err(Box::new)?;
        delete_starting_at(&txn, BATCHES_BY_BLOCK_NUMBER_TABLE, last_kept_block + 1)?;
        delete_starting_at(&txn, MESSAGES_BY_BATCH, batch_number + 1)?;
        delete_starting_at(&txn, BLOCK_NUMBERS_BY_BATCH, batch_number + 1)?;
        delete_starting_at(&txn, DEPOSIT_LOGS_HASHES, batch_number + 1)?;
        delete_starting_at(&txn, STATE_ROOTS, batch_number + 1)?;
        delete_starting_at(&txn, BLOB_BUNDLES, batch_number + 1)?;
        txn.commit()?;
        Ok(())
    }
}

/// Deletes keys above key, assuming they are contiguous
fn delete_starting_at<V: redb::Value>(
    txn: &WriteTransaction,
    table: TableDefinition<u64, V>,
    mut key: u64,
) -> Result<(), RollupStoreError> {
    let mut table = txn.open_table(table)?;
    while table.get(key)?.is_some() {
        table.remove(key)?;
        key += 1;
    }
    Ok(())
}
