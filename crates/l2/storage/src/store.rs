use std::sync::Arc;

use crate::api::StoreEngineRollup;
use crate::error::RollupStoreError;
use crate::store_db::in_memory::Store as InMemoryStore;
#[cfg(feature = "libmdbx")]
use crate::store_db::libmdbx::Store as LibmdbxStoreRollup;
#[cfg(feature = "redb")]
use crate::store_db::redb::RedBStoreRollup;
#[cfg(feature = "sql")]
use crate::store_db::sql::SQLStore;
use ethrex_common::{
    H256,
    types::{AccountUpdate, Blob, BlobsBundle, BlockNumber, batch::Batch},
};
use tracing::info;

#[derive(Debug, Clone)]
pub struct Store {
    engine: Arc<dyn StoreEngineRollup>,
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
    #[cfg(feature = "sql")]
    SQL,
}

impl Store {
    pub fn new(_path: &str, engine_type: EngineType) -> Result<Self, RollupStoreError> {
        info!("Starting l2 storage engine ({engine_type:?})");
        let store = match engine_type {
            #[cfg(feature = "libmdbx")]
            EngineType::Libmdbx => Self {
                engine: Arc::new(LibmdbxStoreRollup::new(_path)?),
            },
            EngineType::InMemory => Self {
                engine: Arc::new(InMemoryStore::new()),
            },
            #[cfg(feature = "redb")]
            EngineType::RedB => Self {
                engine: Arc::new(RedBStoreRollup::new()?),
            },
            #[cfg(feature = "sql")]
            EngineType::SQL => Self {
                engine: Arc::new(SQLStore::new(_path)?),
            },
        };
        info!("Started l2 store engine");
        Ok(store)
    }

    pub async fn init(&self) -> Result<(), RollupStoreError> {
        // Stores batch 0 with block 0
        self.seal_batch(Batch {
            number: 0,
            first_block: 0,
            last_block: 0,
            state_root: H256::zero(),
            deposit_logs_hash: H256::zero(),
            message_hashes: Vec::new(),
            blobs_bundle: BlobsBundle::empty(),
        })
        .await?;
        // Sets the lastest sent batch proof to 0
        self.set_lastest_sent_batch_proof(0).await
    }

    /// Stores the block numbers by a given batch_number
    pub async fn store_block_numbers_by_batch(
        &self,
        batch_number: u64,
        block_numbers: Vec<BlockNumber>,
    ) -> Result<(), RollupStoreError> {
        self.engine
            .store_block_numbers_by_batch(batch_number, block_numbers)
            .await
    }

    /// Returns the block numbers by a given batch_number
    pub async fn get_block_numbers_by_batch(
        &self,
        batch_number: u64,
    ) -> Result<Option<Vec<BlockNumber>>, RollupStoreError> {
        self.engine.get_block_numbers_by_batch(batch_number).await
    }

    pub async fn get_batch_number_by_block(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<u64>, RollupStoreError> {
        self.engine.get_batch_number_by_block(block_number).await
    }
    pub async fn store_batch_number_by_block(
        &self,
        block_number: BlockNumber,
        batch_number: u64,
    ) -> Result<(), RollupStoreError> {
        self.engine
            .store_batch_number_by_block(block_number, batch_number)
            .await
    }

    pub async fn get_message_hashes_by_batch(
        &self,
        batch_number: u64,
    ) -> Result<Option<Vec<H256>>, RollupStoreError> {
        self.engine.get_message_hashes_by_batch(batch_number).await
    }

    pub async fn store_message_hashes_by_batch(
        &self,
        batch_number: u64,
        message_hashes: Vec<H256>,
    ) -> Result<(), RollupStoreError> {
        self.engine
            .store_message_hashes_by_batch(batch_number, message_hashes)
            .await
    }

    pub async fn get_deposit_logs_hash_by_batch(
        &self,
        batch_number: u64,
    ) -> Result<Option<H256>, RollupStoreError> {
        self.engine
            .get_deposit_logs_hash_by_batch_number(batch_number)
            .await
    }

    pub async fn store_deposit_logs_hash_by_batch(
        &self,
        batch_number: u64,
        deposit_logs_hash: H256,
    ) -> Result<(), RollupStoreError> {
        self.engine
            .store_deposit_logs_hash_by_batch_number(batch_number, deposit_logs_hash)
            .await
    }

    pub async fn get_state_root_by_batch(
        &self,
        batch_number: u64,
    ) -> Result<Option<H256>, RollupStoreError> {
        self.engine
            .get_state_root_by_batch_number(batch_number)
            .await
    }

    pub async fn store_state_root_by_batch(
        &self,
        batch_number: u64,
        state_root: H256,
    ) -> Result<(), RollupStoreError> {
        self.engine
            .store_state_root_by_batch_number(batch_number, state_root)
            .await
    }

    pub async fn get_blobs_by_batch(
        &self,
        batch_number: u64,
    ) -> Result<Option<Vec<Blob>>, RollupStoreError> {
        self.engine
            .get_blob_bundle_by_batch_number(batch_number)
            .await
    }

    pub async fn store_blobs_by_batch(
        &self,
        batch_number: u64,
        blobs: Vec<Blob>,
    ) -> Result<(), RollupStoreError> {
        self.engine
            .store_blob_bundle_by_batch_number(batch_number, blobs)
            .await
    }

    pub async fn get_batch(&self, batch_number: u64) -> Result<Option<Batch>, RollupStoreError> {
        let Some(blocks) = self.get_block_numbers_by_batch(batch_number).await? else {
            return Ok(None);
        };

        let first_block = *blocks.first().ok_or(RollupStoreError::Custom(
            "Failed while trying to retrieve the first block of a known batch. This is a bug."
                .to_owned(),
        ))?;
        let last_block = *blocks.last().ok_or(RollupStoreError::Custom(
            "Failed while trying to retrieve the last block of a known batch. This is a bug."
                .to_owned(),
        ))?;

        let state_root =
            self.get_state_root_by_batch(batch_number)
                .await?
                .ok_or(RollupStoreError::Custom(
                "Failed while trying to retrieve the state root of a known batch. This is a bug."
                    .to_owned(),
            ))?;
        let blobs_bundle = BlobsBundle::create_from_blobs(
            &self
                .get_blobs_by_batch(batch_number)
                .await?
                .ok_or(RollupStoreError::Custom(
                    "Failed while trying to retrieve the blobs of a known batch. This is a bug."
                        .to_owned(),
                ))?,
        ).map_err(|e| {
            RollupStoreError::Custom(format!("Failed to create blobs bundle from blob while getting batch from database: {e}. This is a bug"))
        })?;
        let message_hashes = self
            .get_message_hashes_by_batch(batch_number)
            .await?
            .ok_or(RollupStoreError::Custom(
            "Failed while trying to retrieve the message hashes of a known batch. This is a bug."
                .to_owned(),
        ))?;
        let deposit_logs_hash = self
            .get_deposit_logs_hash_by_batch(batch_number)
            .await?.ok_or(RollupStoreError::Custom(
            "Failed while trying to retrieve the deposit logs hash of a known batch. This is a bug."
                .to_owned(),
        ))?;

        Ok(Some(Batch {
            number: batch_number,
            first_block,
            last_block,
            state_root,
            blobs_bundle,
            message_hashes,
            deposit_logs_hash,
        }))
    }

    pub async fn seal_batch(&self, batch: Batch) -> Result<(), RollupStoreError> {
        let blocks: Vec<u64> = (batch.first_block..=batch.last_block).collect();

        for block_number in blocks.iter() {
            self.store_batch_number_by_block(*block_number, batch.number)
                .await?;
        }
        self.store_block_numbers_by_batch(batch.number, blocks)
            .await?;
        self.store_message_hashes_by_batch(batch.number, batch.message_hashes)
            .await?;
        self.store_deposit_logs_hash_by_batch(batch.number, batch.deposit_logs_hash)
            .await?;
        self.store_blobs_by_batch(batch.number, batch.blobs_bundle.blobs)
            .await?;
        self.store_state_root_by_batch(batch.number, batch.state_root)
            .await?;
        Ok(())
    }

    pub async fn update_operations_count(
        &self,
        transaction_inc: u64,
        deposits_inc: u64,
        messages_inc: u64,
    ) -> Result<(), RollupStoreError> {
        self.engine
            .update_operations_count(transaction_inc, deposits_inc, messages_inc)
            .await
    }

    pub async fn get_operations_count(&self) -> Result<[u64; 3], RollupStoreError> {
        self.engine.get_operations_count().await
    }

    /// Returns whether the batch with the given number is present.
    pub async fn contains_batch(&self, batch_number: &u64) -> Result<bool, RollupStoreError> {
        self.engine.contains_batch(batch_number).await
    }

    /// Returns the lastest sent batch proof
    pub async fn get_lastest_sent_batch_proof(&self) -> Result<u64, RollupStoreError> {
        self.engine.get_lastest_sent_batch_proof().await
    }

    /// Sets the lastest sent batch proof
    pub async fn set_lastest_sent_batch_proof(
        &self,
        batch_number: u64,
    ) -> Result<(), RollupStoreError> {
        self.engine.set_lastest_sent_batch_proof(batch_number).await
    }

    /// Returns the account updates yielded from executing a block
    pub async fn get_account_updates_by_block_number(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<Vec<AccountUpdate>>, RollupStoreError> {
        self.engine
            .get_account_updates_by_block_number(block_number)
            .await
    }

    /// Stores the account updates yielded from executing a block
    pub async fn store_account_updates_by_block_number(
        &self,
        block_number: BlockNumber,
        account_updates: Vec<AccountUpdate>,
    ) -> Result<(), RollupStoreError> {
        self.engine
            .store_account_updates_by_block_number(block_number, account_updates)
            .await
    }

    /// Reverts to a previous batch, discarding operations in them
    pub async fn revert_to_batch(&self, batch_number: u64) -> Result<(), RollupStoreError> {
        self.engine.revert_to_batch(batch_number).await
    }
}
