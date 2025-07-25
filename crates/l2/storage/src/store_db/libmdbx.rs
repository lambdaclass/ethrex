use std::{
    fmt::{Debug, Formatter},
    path::Path,
    sync::Arc,
};

use crate::error::RollupStoreError;
use ethrex_common::{
    H256,
    types::{AccountUpdate, Blob, BlockNumber, batch::Batch},
};
use ethrex_l2_common::prover::{BatchProof, ProverType};
use ethrex_rlp::encode::RLPEncode;
use libmdbx::{
    DatabaseOptions, Mode, PageSize, RW, ReadWriteOptions,
    orm::{Database, Table, Transaction},
    table, table_info,
};

use crate::{
    api::StoreEngineRollup,
    rlp::{BlockNumbersRLP, MessageHashesRLP, OperationsCountRLP, Rlp},
};

pub struct Store {
    db: Arc<Database>,
}
impl Store {
    pub fn new(path: &str) -> Result<Self, RollupStoreError> {
        Ok(Self {
            db: Arc::new(init_db(Some(path))?),
        })
    }

    // Helper method to write into a libmdbx table
    async fn write<T: Table>(&self, key: T::Key, value: T::Value) -> Result<(), RollupStoreError> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            let txn = db
                .begin_readwrite()
                .map_err(RollupStoreError::LibmdbxError)?;
            txn.upsert::<T>(key, value)
                .map_err(RollupStoreError::LibmdbxError)?;
            txn.commit().map_err(RollupStoreError::LibmdbxError)
        })
        .await
        .map_err(|e| RollupStoreError::Custom(format!("task panicked: {e}")))?
    }

    // Helper method to read from a libmdbx table
    async fn read<T: Table>(&self, key: T::Key) -> Result<Option<T::Value>, RollupStoreError> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            let txn = db.begin_read().map_err(RollupStoreError::LibmdbxError)?;
            txn.get::<T>(key).map_err(RollupStoreError::LibmdbxError)
        })
        .await
        .map_err(|e| RollupStoreError::Custom(format!("task panicked: {e}")))?
    }
}

/// default page size recommended by libmdbx
///
/// - See here: https://github.com/erthink/libmdbx/tree/master?tab=readme-ov-file#limitations
/// - and here: https://libmdbx.dqdkfa.ru/structmdbx_1_1env_1_1geometry.html#a45048bf2de9120d01dae2151c060d459
const DB_PAGE_SIZE: usize = 4096;

/// Initializes a new database with the provided path. If the path is `None`, the database
/// will be temporary.
pub fn init_db(path: Option<impl AsRef<Path>>) -> Result<Database, RollupStoreError> {
    let tables = [
        table_info!(BatchesByBlockNumber),
        table_info!(MessageHashesByBatch),
        table_info!(BlockNumbersByBatch),
        table_info!(OperationsCount),
        table_info!(SignatureByBlockHash),
        table_info!(SignatureByBatch),
        table_info!(BlobsBundles),
        table_info!(StateRoots),
        table_info!(PrivilegedTransactionsHash),
        table_info!(LastSentBatchProof),
        table_info!(AccountUpdatesByBlockNumber),
        table_info!(BatchProofs),
        table_info!(CommitTxByBatch),
        table_info!(VerifyTxByBatch),
    ]
    .into_iter()
    .collect();
    let path = path.map(|p| p.as_ref().to_path_buf());
    let options = DatabaseOptions {
        page_size: Some(PageSize::Set(DB_PAGE_SIZE)),
        mode: Mode::ReadWrite(ReadWriteOptions {
            // Set max DB size to 1TB
            max_size: Some(1024_isize.pow(4)),
            ..Default::default()
        }),
        ..Default::default()
    };
    Database::create_with_options(path, options, &tables).map_err(RollupStoreError::LibmdbxError)
}

impl Debug for Store {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Libmdbx L2 Store").finish()
    }
}

#[async_trait::async_trait]
impl StoreEngineRollup for Store {
    async fn get_batch_number_by_block(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<u64>, RollupStoreError> {
        self.read::<BatchesByBlockNumber>(block_number).await
    }

    async fn get_message_hashes_by_batch(
        &self,
        batch_number: u64,
    ) -> Result<Option<Vec<H256>>, RollupStoreError> {
        Ok(self
            .read::<MessageHashesByBatch>(batch_number)
            .await?
            .map(|w| w.to()))
    }

    async fn get_block_numbers_by_batch(
        &self,
        batch_number: u64,
    ) -> Result<Option<Vec<BlockNumber>>, RollupStoreError> {
        Ok(self
            .read::<BlockNumbersByBatch>(batch_number)
            .await?
            .map(|numbers| numbers.to()))
    }

    async fn get_privileged_transactions_hash_by_batch_number(
        &self,
        batch_number: u64,
    ) -> Result<Option<H256>, RollupStoreError> {
        Ok(self
            .read::<PrivilegedTransactionsHash>(batch_number)
            .await?
            .map(|hash| hash.to()))
    }

    async fn get_state_root_by_batch_number(
        &self,
        batch_number: u64,
    ) -> Result<Option<H256>, RollupStoreError> {
        Ok(self
            .read::<StateRoots>(batch_number)
            .await?
            .map(|hash| hash.to()))
    }

    async fn get_blob_bundle_by_batch_number(
        &self,
        batch_number: u64,
    ) -> Result<Option<Vec<Blob>>, RollupStoreError> {
        Ok(self
            .read::<BlobsBundles>(batch_number)
            .await?
            .map(|blobs| blobs.to()))
    }

    async fn get_commit_tx_by_batch(
        &self,
        batch_number: u64,
    ) -> Result<Option<H256>, RollupStoreError> {
        Ok(self
            .read::<CommitTxByBatch>(batch_number)
            .await?
            .map(|tx| tx.to()))
    }

    async fn store_commit_tx_by_batch(
        &self,
        batch_number: u64,
        commit_tx: H256,
    ) -> Result<(), RollupStoreError> {
        self.write::<CommitTxByBatch>(batch_number, commit_tx.into())
            .await
    }

    async fn get_verify_tx_by_batch(
        &self,
        batch_number: u64,
    ) -> Result<Option<H256>, RollupStoreError> {
        Ok(self
            .read::<VerifyTxByBatch>(batch_number)
            .await?
            .map(|tx| tx.to()))
    }

    async fn store_verify_tx_by_batch(
        &self,
        batch_number: u64,
        verify_tx: H256,
    ) -> Result<(), RollupStoreError> {
        self.write::<VerifyTxByBatch>(batch_number, verify_tx.into())
            .await
    }

    async fn contains_batch(&self, batch_number: &u64) -> Result<bool, RollupStoreError> {
        let exists = self
            .read::<BlockNumbersByBatch>(*batch_number)
            .await?
            .is_some();
        Ok(exists)
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
                current_operations[1] + privileged_tx_inc,
                current_operations[2] + messages_inc,
            )
        };

        self.write::<OperationsCount>(
            0,
            OperationsCountRLP::from_bytes(
                vec![transaction_count, messages_count, privileged_tx_count].encode_to_vec(),
            ),
        )
        .await
    }

    async fn get_operations_count(&self) -> Result<[u64; 3], RollupStoreError> {
        let operations = self
            .read::<OperationsCount>(0)
            .await?
            .map(|operations| operations.to());
        match operations {
            Some(mut operations) => Ok([
                operations.remove(0),
                operations.remove(0),
                operations.remove(0),
            ]),
            _ => Ok([0, 0, 0]),
        }
    }

    async fn store_signature_by_block(
        &self,
        block_hash: H256,
        signature: ethereum_types::Signature,
    ) -> Result<(), RollupStoreError> {
        let key = block_hash.as_fixed_bytes();
        let signature = signature.to_fixed_bytes();
        self.write::<SignatureByBlockHash>(*key, signature).await
    }

    async fn get_signature_by_block(
        &self,
        block_hash: H256,
    ) -> Result<Option<ethereum_types::Signature>, RollupStoreError> {
        let key = block_hash.as_fixed_bytes();
        Ok(self
            .read::<SignatureByBlockHash>(*key)
            .await?
            .map(|s| ethereum_types::Signature::from_slice(&s)))
    }

    async fn store_signature_by_batch(
        &self,
        batch_number: u64,
        signature: ethereum_types::Signature,
    ) -> Result<(), RollupStoreError> {
        let signature = signature.to_fixed_bytes();
        self.write::<SignatureByBatch>(batch_number, signature)
            .await
    }

    async fn get_signature_by_batch(
        &self,
        batch_number: u64,
    ) -> Result<Option<ethereum_types::Signature>, RollupStoreError> {
        Ok(self
            .read::<SignatureByBatch>(batch_number)
            .await?
            .map(|s| ethereum_types::Signature::from_slice(&s)))
    }

    async fn get_lastest_sent_batch_proof(&self) -> Result<u64, RollupStoreError> {
        self.read::<LastSentBatchProof>(0)
            .await
            .map(|v| v.unwrap_or(0))
    }

    async fn set_lastest_sent_batch_proof(
        &self,
        batch_number: u64,
    ) -> Result<(), RollupStoreError> {
        self.write::<LastSentBatchProof>(0, batch_number).await
    }

    async fn get_account_updates_by_block_number(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<Vec<AccountUpdate>>, RollupStoreError> {
        self.read::<AccountUpdatesByBlockNumber>(block_number)
            .await?
            .map(|s| bincode::deserialize(&s))
            .transpose()
            .map_err(RollupStoreError::from)
    }

    async fn store_account_updates_by_block_number(
        &self,
        block_number: BlockNumber,
        account_updates: Vec<AccountUpdate>,
    ) -> Result<(), RollupStoreError> {
        let serialized = bincode::serialize(&account_updates)?;
        self.write::<AccountUpdatesByBlockNumber>(block_number, serialized)
            .await
    }
    async fn store_proof_by_batch_and_type(
        &self,
        batch_number: u64,
        proof_type: ProverType,
        proof: BatchProof,
    ) -> Result<(), RollupStoreError> {
        let serialized = bincode::serialize(&proof)?;
        self.write::<BatchProofs>((batch_number, proof_type.into()), serialized)
            .await
    }

    async fn get_proof_by_batch_and_type(
        &self,
        batch_number: u64,
        proof_type: ProverType,
    ) -> Result<Option<BatchProof>, RollupStoreError> {
        self.read::<BatchProofs>((batch_number, proof_type.into()))
            .await?
            .map(|s| bincode::deserialize(&s))
            .transpose()
            .map_err(RollupStoreError::from)
    }

    async fn revert_to_batch(&self, batch_number: u64) -> Result<(), RollupStoreError> {
        let Some(kept_blocks) = self.get_block_numbers_by_batch(batch_number).await? else {
            return Ok(());
        };
        let last_kept_block = *kept_blocks.iter().max().unwrap_or(&0);
        let txn = self
            .db
            .begin_readwrite()
            .map_err(RollupStoreError::LibmdbxError)?;
        delete_starting_at::<BatchesByBlockNumber>(&txn, last_kept_block + 1)?;
        delete_starting_at::<MessageHashesByBatch>(&txn, batch_number + 1)?;
        delete_starting_at::<BlockNumbersByBatch>(&txn, batch_number + 1)?;
        delete_starting_at::<PrivilegedTransactionsHash>(&txn, batch_number + 1)?;
        delete_starting_at::<StateRoots>(&txn, batch_number + 1)?;
        delete_starting_at::<BlobsBundles>(&txn, batch_number + 1)?;
        txn.commit().map_err(RollupStoreError::LibmdbxError)?;
        Ok(())
    }

    async fn seal_batch(&self, batch: Batch) -> Result<(), RollupStoreError> {
        let blocks: Vec<u64> = (batch.first_block..=batch.last_block).collect();
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            let transaction = db
                .begin_readwrite()
                .map_err(RollupStoreError::LibmdbxError)?;

            for block in blocks.iter() {
                transaction
                    .upsert::<BatchesByBlockNumber>(*block, batch.number)
                    .map_err(RollupStoreError::LibmdbxError)?;
            }

            transaction
                .upsert::<BlockNumbersByBatch>(
                    batch.number,
                    BlockNumbersRLP::from_bytes(blocks.encode_to_vec()),
                )
                .map_err(RollupStoreError::LibmdbxError)?;

            transaction
                .upsert::<MessageHashesByBatch>(batch.number, batch.message_hashes.into())
                .map_err(RollupStoreError::LibmdbxError)?;

            transaction
                .upsert::<PrivilegedTransactionsHash>(
                    batch.number,
                    Rlp::from_bytes(batch.privileged_transactions_hash.encode_to_vec()),
                )
                .map_err(RollupStoreError::LibmdbxError)?;

            transaction
                .upsert::<BlobsBundles>(batch.number, batch.blobs_bundle.blobs.into())
                .map_err(RollupStoreError::LibmdbxError)?;

            transaction
                .upsert::<StateRoots>(
                    batch.number,
                    Rlp::from_bytes(batch.state_root.encode_to_vec()),
                )
                .map_err(RollupStoreError::LibmdbxError)?;

            if let Some(commit_tx) = batch.commit_tx {
                transaction
                    .upsert::<CommitTxByBatch>(batch.number, commit_tx.into())
                    .map_err(RollupStoreError::LibmdbxError)?;
            }

            if let Some(verify_tx) = batch.verify_tx {
                transaction
                    .upsert::<VerifyTxByBatch>(batch.number, verify_tx.into())
                    .map_err(RollupStoreError::LibmdbxError)?;
            }

            transaction.commit().map_err(RollupStoreError::LibmdbxError)
        })
        .await
        .map_err(|e| RollupStoreError::Custom(format!("task panicked: {e}")))?
    }
}

/// Deletes keys above key, assuming they are contiguous
fn delete_starting_at<T: Table<Key = u64>>(
    txn: &Transaction<RW>,
    mut key: u64,
) -> Result<(), RollupStoreError> {
    while let Some(val) = txn.get::<T>(key).map_err(RollupStoreError::LibmdbxError)? {
        txn.delete::<T>(key, Some(val))
            .map_err(RollupStoreError::LibmdbxError)?;
        key += 1;
    }
    Ok(())
}

table!(
    /// Batch number by block number
    ( BatchesByBlockNumber ) BlockNumber => u64
);

table!(
    /// messages by batch number
    ( MessageHashesByBatch ) u64 => MessageHashesRLP
);

table!(
    /// Block numbers by batch number
    ( BlockNumbersByBatch ) u64 => BlockNumbersRLP
);

table!(
    /// Transaction, privileged transaction, messages count
    ( OperationsCount ) u64 => OperationsCountRLP
);

table!(
    /// Signature by block hash
    ( SignatureByBlockHash ) [u8; 32] => [u8; 65]
);

table!(
    /// Signature by batch number
    ( SignatureByBatch ) u64 => [u8; 65]
);

table!(
    /// Blobs bundles by batch number
    ( BlobsBundles ) u64 => Rlp<Vec<Blob>>
);

table!(
    /// State roots by batch number
    ( StateRoots ) u64 => Rlp<H256>
);

table!(
    /// Privileged transactions hash by batch number
    ( PrivilegedTransactionsHash ) u64 => Rlp<H256>
);

table!(
    /// Last sent batch proof
    ( LastSentBatchProof ) u64 => u64
);

table!(
    /// List of serialized account updates by block number
    ( AccountUpdatesByBlockNumber ) BlockNumber => Vec<u8>
);

table!(
    /// Stores batch proofs, keyed by (BatchNumber, ProverType as u8).
    /// Value is the bincode-encoded BatchProof data.
    (BatchProofs) (u64, u32) => Vec<u8>
);

table!(
    /// Commit transaction by batch number
    ( CommitTxByBatch ) u64 => Rlp<H256>
);

table!(
    /// Verify transaction by batch number
    ( VerifyTxByBatch ) u64 => Rlp<H256>
);
