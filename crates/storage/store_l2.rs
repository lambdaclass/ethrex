use std::sync::Arc;

use crate::store_db_l2::in_memory::Store as InMemoryStore;
#[cfg(feature = "libmdbx")]
use crate::store_db_l2::libmdbx::LibmdbxStoreL2;
use crate::store_db_l2::redb::RedBStoreL2;
use crate::{api_l2::StoreEngineL2, error::StoreError};
use ethrex_common::types::BlockNumber;
use tracing::info;

#[derive(Debug, Clone)]
pub struct Store {
    engine: Arc<dyn StoreEngineL2>,
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
                engine: Arc::new(LibmdbxStoreL2::new_l2(path)?),
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

    pub fn get_batch_number_for_block(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<u64>, StoreError> {
        self.engine.get_batch_number_for_block(block_number)
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
}
