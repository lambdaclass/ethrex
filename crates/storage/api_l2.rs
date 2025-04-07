// Storage API for L2

use std::{fmt::Debug, panic::RefUnwindSafe};

use ethrex_common::types::BlockNumber;

use crate::error::StoreError;
// We need async_trait because the stabilized feature lacks support for object safety
// (i.e. dyn StoreEngine)
#[async_trait::async_trait]
pub trait StoreEngineL2: Debug + Send + Sync + RefUnwindSafe {
    /// Returns the batch number for a given block number.
    fn get_batch_number_for_block(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<u64>, StoreError>;

    /// Stores the batch number for a given block number.
    async fn store_batch_number_for_block(
        &self,
        block_number: BlockNumber,
        batch_number: u64,
    ) -> Result<(), StoreError>;
}
