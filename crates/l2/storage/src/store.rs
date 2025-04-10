use std::sync::Arc;

use crate::api::StoreEngineL2;
use crate::store_db::in_memory::Store as InMemoryStore;
#[cfg(feature = "libmdbx")]
use crate::store_db::libmdbx::Store as LibmdbxStoreL2;
#[cfg(feature = "redb")]
use crate::store_db::redb::RedBStoreL2;
use ethrex_common::{types::BlockNumber, H256};
use ethrex_storage::error::StoreError;
use tracing::info;

#[derive(Debug, Clone)]
pub struct Store {
    engine: Arc<dyn StoreEngineL2>,
}

impl Default for Store {
    fn default() -> Self {
        Self {
            engine: Arc::new(InMemoryStore::new()),
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineType {
    InMemory,
    #[cfg(feature = "libmdbx")]
    Libmdbx,
    #[cfg(feature = "redb")]
    RedB,
}

impl Store {
    pub fn new(path: &str, engine_type: EngineType) -> Result<Self, StoreError> {
        info!("Starting l2 storage engine ({engine_type:?})");
        let store = match engine_type {
            #[cfg(feature = "libmdbx")]
            EngineType::Libmdbx => Self {
                engine: Arc::new(LibmdbxStoreL2::new(path)?),
            },
            EngineType::InMemory => Self {
                engine: Arc::new(InMemoryStore::new()),
            },
            #[cfg(feature = "redb")]
            EngineType::RedB => Self {
                engine: Arc::new(RedBStoreL2::new()?),
            },
        };
        info!("Started l2 store engine");
        Ok(store)
    }

    pub async fn get_batch_number_for_block(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<u64>, StoreError> {
        self.engine.get_batch_number_for_block(block_number).await
    }
    pub async fn store_batch_number_for_block(
        &self,
        block_number: BlockNumber,
        batch_number: u64,
    ) -> Result<(), StoreError> {
        self.engine
            .store_batch_number_for_block(block_number, batch_number)
            .await
    }

    pub async fn get_withdrawal_hashes_for_batch(
        &self,
        batch_number: u64,
    ) -> Result<Option<Vec<H256>>, StoreError> {
        self.engine
            .get_withdrawal_hashes_for_batch(batch_number)
            .await
    }

    pub async fn store_withdrawal_hashes_for_batch(
        &self,
        batch_number: u64,
        withdrawal_hashes: Vec<H256>,
    ) -> Result<(), StoreError> {
        self.engine
            .store_withdrawal_hashes_for_batch(batch_number, withdrawal_hashes)
            .await
    }
}
