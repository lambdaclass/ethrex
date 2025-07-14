use std::{panic::RefUnwindSafe, sync::Arc};

use crate::error::RollupStoreError;
use ethrex_common::{
    H256,
    types::{AccountUpdate, Blob, BlockNumber, batch::Batch},
};
use ethrex_l2_common::prover::{BatchProof, ProverType};
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

const PRIVILEGED_TRANSACTIONS_HASHES: TableDefinition<u64, Rlp<H256>> =
    TableDefinition::new("PrivilegedTransactionHashes");

const LAST_SENT_BATCH_PROOF: TableDefinition<u64, u64> = TableDefinition::new("LastSentBatchProof");

const ACCOUNT_UPDATES_BY_BLOCK_NUMBER: TableDefinition<BlockNumber, Vec<u8>> =
    TableDefinition::new("AccountUpdatesByBlockNumber");

const BATCH_PROOF_BY_BATCH_AND_TYPE: TableDefinition<(u64, u32), Vec<u8>> =
    TableDefinition::new("BatchProofByBatchAndType");

const COMMIT_TX_BY_BATCH: TableDefinition<u64, Rlp<H256>> = TableDefinition::new("CommitTxByBatch");

const VERIFY_TX_BY_BATCH: TableDefinition<u64, Rlp<H256>> = TableDefinition::new("VerifyTxByBatch");

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
    table_creation_txn.open_table(PRIVILEGED_TRANSACTIONS_HASHES)?;
    table_creation_txn.open_table(BLOCK_NUMBERS_BY_BATCH)?;
    table_creation_txn.open_table(LAST_SENT_BATCH_PROOF)?;
    table_creation_txn.open_table(ACCOUNT_UPDATES_BY_BLOCK_NUMBER)?;
    table_creation_txn.open_table(BATCH_PROOF_BY_BATCH_AND_TYPE)?;
    table_creation_txn.open_table(COMMIT_TX_BY_BATCH)?;
    table_creation_txn.open_table(VERIFY_TX_BY_BATCH)?;
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

    async fn get_message_hashes_by_batch(
        &self,
        batch_number: u64,
    ) -> Result<Option<Vec<H256>>, RollupStoreError> {
        Ok(self
            .read(MESSAGES_BY_BATCH, batch_number)
            .await?
            .map(|w| w.value().to()))
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

    async fn get_privileged_transactions_hash_by_batch_number(
        &self,
        batch_number: u64,
    ) -> Result<Option<H256>, RollupStoreError> {
        Ok(self
            .read(PRIVILEGED_TRANSACTIONS_HASHES, batch_number)
            .await?
            .map(|rlp| rlp.value().to()))
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

    async fn get_blob_bundle_by_batch_number(
        &self,
        batch_number: u64,
    ) -> Result<Option<Vec<Blob>>, RollupStoreError> {
        Ok(self
            .read(BLOB_BUNDLES, batch_number)
            .await?
            .map(|rlp| rlp.value().to()))
    }

    async fn get_commit_tx_by_batch(
        &self,
        batch_number: u64,
    ) -> Result<Option<H256>, RollupStoreError> {
        Ok(self
            .read(COMMIT_TX_BY_BATCH, batch_number)
            .await?
            .map(|rlp| rlp.value().to()))
    }

    async fn store_commit_tx_by_batch(
        &self,
        batch_number: u64,
        commit_tx: H256,
    ) -> Result<(), RollupStoreError> {
        self.write(COMMIT_TX_BY_BATCH, batch_number, commit_tx.into())
            .await
    }

    async fn get_verify_tx_by_batch(
        &self,
        batch_number: u64,
    ) -> Result<Option<H256>, RollupStoreError> {
        Ok(self
            .read(VERIFY_TX_BY_BATCH, batch_number)
            .await?
            .map(|rlp| rlp.value().to()))
    }

    async fn store_verify_tx_by_batch(
        &self,
        batch_number: u64,
        verify_tx: H256,
    ) -> Result<(), RollupStoreError> {
        self.write(VERIFY_TX_BY_BATCH, batch_number, verify_tx.into())
            .await
    }

    async fn update_operations_count(
        &self,
        transaction_inc: u64,
        privileged_tx_inc: u64,
        messages_inc: u64,
    ) -> Result<(), RollupStoreError> {
        let (transaction_count, messages_count, privileged_tx_count) = {
            let current_operations = self.get_operations_count().await?;
            (
                current_operations[0] + transaction_inc,
                current_operations[1] + messages_inc,
                current_operations[2] + privileged_tx_inc,
            )
        };

        self.write(
            OPERATIONS_COUNTS,
            0,
            OperationsCountRLP::from_bytes(
                vec![transaction_count, messages_count, privileged_tx_count].encode_to_vec(),
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
    async fn store_proof_by_batch_and_type(
        &self,
        batch_number: u64,
        proof_type: ProverType,
        proof: BatchProof,
    ) -> Result<(), RollupStoreError> {
        let serialized = bincode::serialize(&proof)?;
        self.write(
            BATCH_PROOF_BY_BATCH_AND_TYPE,
            (batch_number, proof_type.into()),
            serialized,
        )
        .await
    }

    async fn get_proof_by_batch_and_type(
        &self,
        batch_number: u64,
        proof_type: ProverType,
    ) -> Result<Option<BatchProof>, RollupStoreError> {
        self.read(
            BATCH_PROOF_BY_BATCH_AND_TYPE,
            (batch_number, proof_type.into()),
        )
        .await?
        .map(|s| bincode::deserialize(&s.value()))
        .transpose()
        .map_err(RollupStoreError::from)
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
        delete_starting_at(&txn, PRIVILEGED_TRANSACTIONS_HASHES, batch_number + 1)?;
        delete_starting_at(&txn, STATE_ROOTS, batch_number + 1)?;
        delete_starting_at(&txn, BLOB_BUNDLES, batch_number + 1)?;
        txn.commit()?;
        Ok(())
    }

    async fn seal_batch(&self, batch: Batch) -> Result<(), RollupStoreError> {
        let blocks: Vec<u64> = (batch.first_block..=batch.last_block).collect();
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            let transaction = db.begin_write().map_err(Box::new)?;

            {
                let mut table = transaction.open_table(BATCHES_BY_BLOCK_NUMBER_TABLE)?;
                for block in blocks.iter() {
                    table.insert(*block, batch.number)?;
                }
            }

            transaction.open_table(BLOCK_NUMBERS_BY_BATCH)?.insert(
                batch.number,
                BlockNumbersRLP::from_bytes(blocks.encode_to_vec()),
            )?;

            transaction.open_table(MESSAGES_BY_BATCH)?.insert(
                batch.number,
                <Vec<H256> as Into<MessageHashesRLP>>::into(batch.message_hashes),
            )?;

            transaction
                .open_table(PRIVILEGED_TRANSACTIONS_HASHES)?
                .insert(
                    batch.number,
                    <H256 as Into<Rlp<H256>>>::into(batch.privileged_transactions_hash),
                )?;

            transaction.open_table(BLOB_BUNDLES)?.insert(
                batch.number,
                <Vec<Blob> as Into<Rlp<Vec<Blob>>>>::into(batch.blobs_bundle.blobs),
            )?;

            transaction.open_table(STATE_ROOTS)?.insert(
                batch.number,
                <H256 as Into<Rlp<H256>>>::into(batch.state_root),
            )?;

            if let Some(commit_tx) = batch.commit_tx {
                transaction
                    .open_table(COMMIT_TX_BY_BATCH)?
                    .insert(batch.number, <H256 as Into<Rlp<H256>>>::into(commit_tx))?;
            }

            if let Some(verify_tx) = batch.verify_tx {
                transaction
                    .open_table(VERIFY_TX_BY_BATCH)?
                    .insert(batch.number, <H256 as Into<Rlp<H256>>>::into(verify_tx))?;
            }

            transaction.commit()?;
            Ok(())
        })
        .await
        .map_err(|e| RollupStoreError::Custom(format!("task panicked: {e}")))?
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
