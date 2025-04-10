use std::{
    fmt::{Debug, Formatter},
    path::Path,
    sync::Arc,
};

use ethrex_common::{types::BlockNumber, H256};
use ethrex_storage::error::StoreError;
use libmdbx::{
    orm::{Database, Table},
    table, table_info, DatabaseOptions, Mode, PageSize, ReadWriteOptions,
};

use crate::{api::StoreEngineL2, rlp::WithdrawalHashesRLP};

pub struct Store {
    db: Arc<Database>,
}
impl Store {
    pub fn new(path: &str) -> Result<Self, StoreError> {
        Ok(Self {
            db: Arc::new(init_db(Some(path))?),
        })
    }

    // Helper method to write into a libmdbx table
    async fn write<T: Table>(&self, key: T::Key, value: T::Value) -> Result<(), StoreError> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            let txn = db.begin_readwrite().map_err(StoreError::LibmdbxError)?;
            txn.upsert::<T>(key, value)
                .map_err(StoreError::LibmdbxError)?;
            txn.commit().map_err(StoreError::LibmdbxError)
        })
        .await
        .map_err(|e| StoreError::Custom(format!("task panicked: {e}")))?
    }

    // Helper method to read from a libmdbx table
    async fn read<T: Table>(&self, key: T::Key) -> Result<Option<T::Value>, StoreError> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            let txn = db.begin_read().map_err(StoreError::LibmdbxError)?;
            txn.get::<T>(key).map_err(StoreError::LibmdbxError)
        })
        .await
        .map_err(|e| StoreError::Custom(format!("task panicked: {e}")))?
    }
}

/// default page size recommended by libmdbx
///
/// - See here: https://github.com/erthink/libmdbx/tree/master?tab=readme-ov-file#limitations
/// - and here: https://libmdbx.dqdkfa.ru/structmdbx_1_1env_1_1geometry.html#a45048bf2de9120d01dae2151c060d459
const DB_PAGE_SIZE: usize = 4096;

/// Initializes a new database with the provided path. If the path is `None`, the database
/// will be temporary.
pub fn init_db(path: Option<impl AsRef<Path>>) -> Result<Database, StoreError> {
    let tables = [
        table_info!(BatchesByBlockNumber),
        table_info!(WithdrawalHashesByBatch),
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
    Database::create_with_options(path, options, &tables).map_err(StoreError::LibmdbxError)
}

impl Debug for Store {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Libmdbx L2 Store").finish()
    }
}

#[async_trait::async_trait]
impl StoreEngineL2 for Store {
    async fn get_batch_number_for_block(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<u64>, StoreError> {
        self.read::<BatchesByBlockNumber>(block_number).await
    }

    async fn store_batch_number_for_block(
        &self,
        block_number: BlockNumber,
        batch_number: u64,
    ) -> Result<(), StoreError> {
        self.write::<BatchesByBlockNumber>(block_number, batch_number)
            .await
    }

    async fn get_withdrawal_hashes_for_batch(
        &self,
        batch_number: u64,
    ) -> Result<Option<Vec<H256>>, StoreError> {
        Ok(self
            .read::<WithdrawalHashesByBatch>(batch_number)
            .await?
            .map(|w| w.to()))
    }

    async fn store_withdrawal_hashes_for_batch(
        &self,
        batch_number: u64,
        withdrawals: Vec<H256>,
    ) -> Result<(), StoreError> {
        self.write::<WithdrawalHashesByBatch>(batch_number, withdrawals.into())
            .await
    }
}

table!(
    /// Batch number by block number
    ( BatchesByBlockNumber ) BlockNumber => u64
);

table!(
    /// Withdrawals by batch number
    ( WithdrawalHashesByBatch ) u64 => WithdrawalHashesRLP
);
