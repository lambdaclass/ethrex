use crate::rlpx::error::PeerConnectionError;
use ethrex_blockchain::Blockchain;
use ethrex_blockchain::error::ChainError;
use ethrex_blockchain::fork_choice::apply_fork_choice;
use ethrex_common::types::Block;
use ethrex_common::types::fee_config::FeeConfig;
use ethrex_storage::{DB_COMMIT_THRESHOLD, Store};
use ethrex_storage_rollup::StoreRollup;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use tokio::sync::Notify;
use tracing::{error, info};

#[derive(Debug)]
struct QueuedBlock {
    block: Arc<Block>,
    fee_config: FeeConfig,
}

/// Imports the blocks L2 peers broadcast, for the whole node: one at a time, in order, from the
/// local head on.
///
/// Connections hand the blocks they receive over to this importer instead of importing them
/// themselves, so:
/// - imports never overlap: the blockchain's merkleization pool only fits one block at a time,
///   and overlapping imports deadlock on it;
/// - a block delivered by several connections is queued and imported once;
/// - connections never wait on an import.
#[derive(Debug, Clone, Default)]
pub struct L2BlockImporter {
    queue: Arc<Mutex<BTreeMap<u64, QueuedBlock>>>,
    block_queued: Arc<Notify>,
}

impl L2BlockImporter {
    /// Creates the importer and starts importing queued blocks in the background.
    pub fn spawn(storage: Store, blockchain: Arc<Blockchain>, store_rollup: StoreRollup) -> Self {
        let importer = Self::default();
        tokio::spawn(importer.clone().run(storage, blockchain, store_rollup));
        importer
    }

    /// Queues a block for import. Returns `false` if a block with its number is already queued.
    pub fn submit(&self, block: Arc<Block>, fee_config: FeeConfig) -> bool {
        let number = block.header.number;
        let mut queue = self.queue();
        if queue.contains_key(&number) {
            return false;
        }
        queue.insert(number, QueuedBlock { block, fee_config });
        drop(queue);
        self.block_queued.notify_one();
        true
    }

    pub fn is_queued(&self, number: u64) -> bool {
        self.queue().contains_key(&number)
    }

    async fn run(self, storage: Store, blockchain: Arc<Blockchain>, store_rollup: StoreRollup) {
        loop {
            self.block_queued.notified().await;
            if let Err(err) = self
                .import_ready_blocks(&storage, &blockchain, &store_rollup)
                .await
            {
                error!(error=%err, "Failed to import blocks received from L2 peers");
            }
        }
    }

    /// Imports, in order, the queued blocks that extend the local chain, and drops the ones at
    /// or below its head. Each import runs on a blocking thread, off the async runtime.
    pub async fn import_ready_blocks(
        &self,
        storage: &Store,
        blockchain: &Arc<Blockchain>,
        store_rollup: &StoreRollup,
    ) -> Result<(), PeerConnectionError> {
        loop {
            let next_block = storage.get_latest_block_number()? + 1;
            let Some(QueuedBlock { block, fee_config }) = self.take(next_block) else {
                return Ok(());
            };
            let block_hash = block.hash();
            let block_number = block.header.number;
            let block = Arc::unwrap_or_clone(block);
            let importer = blockchain.clone();
            // Bounded, not the plain pipeline: commit trie layers every DB_COMMIT_THRESHOLD
            // blocks instead of on the canonical safe-commit gate. That gate fires on every
            // block here, because the fork-choice call below marks each imported block head,
            // safe AND finalized — so the unbounded form costs a trie commit, and therefore an
            // fsync, per block. On a follower catching up that is the whole bottleneck: a
            // staging node measured 90% disk utilisation at 35 blocks/s while writing 2.5 MB/s
            // to a device that does 3.1 GB/s. Saturated by flush operations, not by volume.
            //
            // Its precondition — "a single canonical chain with no competing forks" — is exactly
            // a single-sequencer validium. Blocks arrive in order from the lead sequencer and
            // there is no fork choice to make, which the head==safe==finalized call below already
            // concedes. Every other bulk-import path in the tree (startup regeneration, RLP
            // import, L1 full sync, add_blocks_in_batch) already uses this entry point; the L2
            // importer was the one that did not.
            tokio::task::spawn_blocking(move || {
                importer.add_block_pipeline_bounded(block, None, DB_COMMIT_THRESHOLD)
            })
            .await
            .map_err(|e| {
                PeerConnectionError::InternalError(format!("Block import task failed: {e}"))
            })?
            .inspect_err(|e| {
                error!(
                    error=%e,
                    block_number,
                    ?block_hash,
                    "Error adding new block",
                );
            })?;

            apply_fork_choice(storage, block_hash, block_hash, block_hash, None)
                .await
                .map_err(|e| {
                    PeerConnectionError::BlockchainError(ChainError::Custom(format!(
                        "Error adding new block {} with hash {:?}, error: {e}",
                        block_number, block_hash
                    )))
                })?;

            store_rollup
                .store_fee_config_by_block(block_number, fee_config)
                .await?;
            info!(
                "Added new block {} with hash {:?}",
                block_number, block_hash
            );
        }
    }

    /// Removes and returns block `number`, dropping every queued block below it.
    ///
    /// Pops stale entries one at a time instead of using `split_off`, which would rebuild the
    /// map on every import: almost every queued block is above `number`.
    fn take(&self, number: u64) -> Option<QueuedBlock> {
        let mut queue = self.queue();
        while queue
            .first_key_value()
            .is_some_and(|(&first, _)| first < number)
        {
            queue.pop_first();
        }
        queue.remove(&number)
    }

    fn queue(&self) -> MutexGuard<'_, BTreeMap<u64, QueuedBlock>> {
        // The lock is only held for map operations, which leave it consistent even if one panics.
        self.queue.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
