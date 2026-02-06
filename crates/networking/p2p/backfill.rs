//! Background backfill of historical block bodies and receipts.
//!
//! After snap sync completes, this module downloads block bodies and receipts
//! from the pivot block backward, enabling full historical data availability.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use ethrex_common::types::validate_block_body;
use ethrex_common::types::{BlockBody, BlockHash, BlockHeader, Receipt};
use ethrex_common::validation::validate_receipts_root;
use ethrex_storage::Store;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use crate::peer_handler::PeerHandler;

/// Batch size for backfill requests
const BACKFILL_BATCH_SIZE: usize = 64;

/// Delay between batches in milliseconds (rate limiting)
const BACKFILL_RATE_LIMIT_MS: u64 = 100;

/// Retry delay on failure
const BACKFILL_RETRY_DELAY_SECS: u64 = 10;

/// Minimum block number for backfill (merge block for EIP-4444 compliance)
/// Pre-merge blocks are no longer served via P2P as of May 2025
pub const MERGE_BLOCK_NUMBER: u64 = 15_537_393;

/// Error type for backfill operations
#[derive(Debug, thiserror::Error)]
pub enum BackfillError {
    #[error("Storage error: {0}")]
    Store(#[from] ethrex_storage::error::StoreError),

    #[error("RLP decode error: {0}")]
    RlpDecode(#[from] ethrex_rlp::error::RLPDecodeError),

    #[error("Peer handler error")]
    PeerHandler,

    #[error("No peers available for backfill")]
    NoPeers,

    #[error("Block validation failed: {0}")]
    ValidationFailed(String),

    #[error("Missing canonical hash for block {0}")]
    MissingCanonicalHash(u64),

    #[error("Missing header for hash {0:?}")]
    MissingHeader(BlockHash),

    #[error("Bodies/receipts count mismatch")]
    CountMismatch,
}

/// Backfill manager for downloading historical block bodies and receipts.
#[derive(Debug)]
pub struct BackfillManager {
    store: Store,
    peers: PeerHandler,
    cancel_token: CancellationToken,
    spawn_attempted: AtomicBool,
}

impl BackfillManager {
    /// Creates a new backfill manager.
    pub fn new(store: Store, peers: PeerHandler, cancel_token: CancellationToken) -> Self {
        Self {
            store,
            peers,
            cancel_token,
            spawn_attempted: AtomicBool::new(false),
        }
    }

    /// Spawns the backfill background task.
    ///
    /// Uses compare_exchange to ensure only one task is spawned.
    /// Returns true if task was spawned, false if already running.
    pub fn spawn(self: Arc<Self>) -> bool {
        // Atomic check-and-set to prevent duplicate spawns
        if self
            .spawn_attempted
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            debug!("Backfill task already spawned");
            return false;
        }

        tokio::spawn(async move {
            if self.cancel_token.is_cancelled() {
                debug!("Backfill task cancelled before start");
                return;
            }
            if let Err(e) = self.run_backfill().await {
                error!("Backfill task failed: {e}");
            }
        });

        true
    }

    /// Main backfill loop.
    async fn run_backfill(&self) -> Result<(), BackfillError> {
        // Get the starting point (earliest block we have headers for but no body)
        let start_block = match self.find_backfill_start().await? {
            Some(block) => block,
            None => {
                info!("No backfill needed - all blocks have bodies");
                return Ok(());
            }
        };

        // Target is merge block (EIP-4444 compliance) or 0 for testnets
        let target_block = MERGE_BLOCK_NUMBER.min(start_block);

        info!(
            "Starting body/receipt backfill from block {} to {}",
            start_block, target_block
        );

        let mut current_block = start_block;

        while current_block > target_block && !self.cancel_token.is_cancelled() {
            let batch_end = current_block;
            let batch_start = current_block
                .saturating_sub(BACKFILL_BATCH_SIZE as u64 - 1)
                .max(target_block);

            match self.backfill_batch(batch_start, batch_end).await {
                Ok(blocks_processed) => {
                    if blocks_processed > 0 {
                        // Save checkpoint at the last block we just completed
                        self.store.set_body_backfill_checkpoint(batch_start).await?;

                        current_block = batch_start.saturating_sub(1);

                        debug!(
                            "Backfilled {} blocks, progress: {} -> {}",
                            blocks_processed, batch_end, batch_start
                        );
                    }

                    // Rate limiting
                    sleep(Duration::from_millis(BACKFILL_RATE_LIMIT_MS)).await;
                }
                Err(e) => {
                    warn!(
                        "Backfill batch failed: {e}, retrying in {}s",
                        BACKFILL_RETRY_DELAY_SECS
                    );
                    sleep(Duration::from_secs(BACKFILL_RETRY_DELAY_SECS)).await;
                }
            }
        }

        info!("Body/receipt backfill complete");
        Ok(())
    }

    /// Finds the starting block for backfill (highest block with header but no body).
    async fn find_backfill_start(&self) -> Result<Option<u64>, BackfillError> {
        // Check for saved checkpoint first
        if let Some(checkpoint) = self.store.get_body_backfill_checkpoint().await? {
            return Ok(Some(checkpoint));
        }

        // Otherwise, find the highest block with header but no body
        let latest = self.store.get_latest_block_number().await?;

        for block_num in (MERGE_BLOCK_NUMBER..=latest).rev() {
            let hash = match self.store.get_canonical_block_hash(block_num).await? {
                Some(h) => h,
                None => continue,
            };

            // Check if we have the body
            if self.store.get_block_body_by_hash(hash).await?.is_none() {
                return Ok(Some(block_num));
            }
        }

        Ok(None)
    }

    /// Backfills a batch of blocks.
    async fn backfill_batch(&self, start: u64, end: u64) -> Result<usize, BackfillError> {
        if start > end {
            return Ok(0);
        }

        // Collect block hashes for the range
        let mut block_hashes = Vec::with_capacity((end - start + 1) as usize);
        let mut headers = Vec::with_capacity((end - start + 1) as usize);

        for block_num in start..=end {
            let hash = self
                .store
                .get_canonical_block_hash(block_num)
                .await?
                .ok_or(BackfillError::MissingCanonicalHash(block_num))?;
            let header = self
                .store
                .get_block_header_by_hash(hash)?
                .ok_or(BackfillError::MissingHeader(hash))?;
            block_hashes.push(hash);
            headers.push(header);
        }

        // Request bodies from peers
        let bodies = self.fetch_bodies(&headers).await?;

        // Request receipts from peers
        let receipts = self.fetch_receipts(&block_hashes).await?;

        if bodies.len() != headers.len() || receipts.len() != headers.len() {
            return Err(BackfillError::CountMismatch);
        }

        // Validate and store
        for ((header, body), block_receipts) in headers.iter().zip(bodies).zip(receipts) {
            // Validate block body
            validate_block_body(header, &body)
                .map_err(|e| BackfillError::ValidationFailed(format!("body: {e}")))?;

            // Validate receipts root
            validate_receipts_root(header, &block_receipts)
                .map_err(|e| BackfillError::ValidationFailed(format!("receipts: {e}")))?;

            // Store body
            let block_hash = header.compute_block_hash();
            self.store.add_block_body(block_hash, body).await?;

            // Store receipts
            self.store.add_receipts(block_hash, block_receipts).await?;
        }

        Ok(headers.len())
    }

    /// Fetches block bodies from peers.
    async fn fetch_bodies(&self, headers: &[BlockHeader]) -> Result<Vec<BlockBody>, BackfillError> {
        // Use peer handler to request bodies
        let mut peers = self.peers.clone();
        peers
            .request_block_bodies(headers)
            .await
            .map_err(|_| BackfillError::PeerHandler)?
            .ok_or(BackfillError::NoPeers)
    }

    /// Fetches receipts from peers.
    async fn fetch_receipts(
        &self,
        hashes: &[BlockHash],
    ) -> Result<Vec<Vec<Receipt>>, BackfillError> {
        // Use peer handler to request receipts
        let mut peers = self.peers.clone();
        peers
            .request_receipts(hashes)
            .await
            .map_err(|_| BackfillError::PeerHandler)?
            .ok_or(BackfillError::NoPeers)
    }
}
