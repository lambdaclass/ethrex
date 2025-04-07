use std::path::Path;

use ethrex_common::types::BlockNumber;
use libmdbx::{orm::Database, table_info, DatabaseOptions, Mode, PageSize, ReadWriteOptions};

pub use crate::store_db::libmdbx::Store as LibmdbxStoreL2;
use crate::{api_l2::StoreEngineL2, error::StoreError, store_db::libmdbx::DB_PAGE_SIZE};

impl LibmdbxStoreL2 {
    pub fn new_l2(path: &str) -> Result<Self, StoreError> {
        Ok(Self {
            db: Arc::new(init_db_l2(Some(path))),
        })
    }
}

/// Initializes a new database with the provided path. If the path is `None`, the database
/// will be temporary.
pub fn init_db(path: Option<impl AsRef<Path>>) -> Database {
    let tables = [table_info!(BatchByBlockNumber)].into_iter().collect();
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
    Database::create_with_options(path, options, &tables).unwrap()
}

#[async_trait::async_trait]
impl StoreEngineL2 for LibmdbxStoreL2 {
    fn get_batch_number_for_block(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<u64>, StoreError> {
        Ok(Some(0))
    }

    async fn store_batch_number_for_block(
        &self,
        block_number: BlockNumber,
        batch_number: u64,
    ) -> Result<(), StoreError> {
        Ok(())
    }
}
