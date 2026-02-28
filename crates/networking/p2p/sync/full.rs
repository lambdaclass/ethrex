//! Full sync implementation
//!
//! This module contains the logic for full synchronization mode where all blocks
//! are fetched via p2p eth requests and executed to rebuild the state.

use std::cmp::min;
use std::sync::Arc;
use std::time::Duration;

use ethrex_blockchain::{BatchBlockProcessingFailure, Blockchain, error::ChainError};
use ethrex_common::{H256, types::Block};
use ethrex_storage::Store;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::peer_handler::{BlockRequestOrder, PeerHandler};
use crate::snap::constants::{MAX_BLOCK_BODIES_TO_REQUEST, MAX_HEADER_FETCH_ATTEMPTS};

use super::{EXECUTE_BATCH_SIZE, SyncError};

/// Performs full sync cycle - fetches and executes all blocks between current head and sync head
///
/// # Returns
///
/// Returns an error if the sync fails at any given step and aborts all active processes
pub async fn sync_cycle_full(
    peers: &mut PeerHandler,
    blockchain: Arc<Blockchain>,
    cancel_token: CancellationToken,
    mut sync_head: H256,
    store: Store,
) -> Result<(), SyncError> {
    info!("Syncing to sync_head {:?}", sync_head);

    // Check if the sync_head is a pending block, if so, gather all pending blocks belonging to its chain
    let mut pending_blocks = vec![];
    while let Some(block) = store.get_pending_block(sync_head).await? {
        if store.is_canonical_sync(block.hash())? {
            // Ignore canonical blocks still in pending
            break;
        }
        sync_head = block.header.parent_hash;
        pending_blocks.insert(0, block);
    }

    // Request all block headers between the sync head and our local chain
    // We will begin from the sync head so that we download the latest state first, ensuring we follow the correct chain
    // This step is not parallelized
    let mut start_block_number;
    let mut end_block_number = 0;
    let mut headers = vec![];
    let mut single_batch = true;

    let mut attempts = 0;

    // Request and store all block headers from the advertised sync head
    loop {
        let Some(mut block_headers) = peers
            .request_block_headers_from_hash(sync_head, BlockRequestOrder::NewToOld)
            .await?
        else {
            if attempts > MAX_HEADER_FETCH_ATTEMPTS {
                warn!("Sync failed to find target block header, aborting");
                return Ok(());
            }
            attempts += 1;
            tokio::time::sleep(Duration::from_millis(1.1_f64.powf(attempts as f64) as u64)).await;
            continue;
        };
        debug!("Sync Log 9: Received {} block headers", block_headers.len());

        let first_header = block_headers.first().ok_or(SyncError::NoBlocks)?;
        let last_header = block_headers.last().ok_or(SyncError::NoBlocks)?;

        info!(
            "Received {} block headers| First Number: {} Last Number: {}",
            block_headers.len(),
            first_header.number,
            last_header.number,
        );
        end_block_number = end_block_number.max(first_header.number);
        start_block_number = last_header.number;

        sync_head = last_header.parent_hash;
        if store.is_canonical_sync(sync_head)? || sync_head.is_zero() {
            // Incoming chain merged with current chain
            // Filter out already canonical blocks from batch
            let mut first_canon_block = block_headers.len();
            for (index, header) in block_headers.iter().enumerate() {
                if store.is_canonical_sync(header.hash())? {
                    first_canon_block = index;
                    break;
                }
            }
            block_headers.drain(first_canon_block..block_headers.len());
            if let Some(last_header) = block_headers.last() {
                start_block_number = last_header.number;
            }
            // If the fullsync consists of a single batch of headers we can just keep them in memory instead of writing them to Store
            if single_batch {
                headers = block_headers.into_iter().rev().collect();
            } else {
                store.add_fullsync_batch(block_headers).await?;
            }
            break;
        }
        store.add_fullsync_batch(block_headers).await?;
        single_batch = false;
    }
    end_block_number += 1;
    start_block_number = start_block_number.max(1);

    // Download block bodies and execute full blocks in batches
    for start in (start_block_number..end_block_number).step_by(*EXECUTE_BATCH_SIZE) {
        let batch_size = EXECUTE_BATCH_SIZE.min((end_block_number - start) as usize);
        let final_batch = end_block_number == start + batch_size as u64;
        // Retrieve batch from DB
        if !single_batch {
            headers = store
                .read_fullsync_batch(start, batch_size as u64)
                .await?
                .into_iter()
                .map(|opt| opt.ok_or(SyncError::MissingFullsyncBatch))
                .collect::<Result<Vec<_>, SyncError>>()?;
        }
        let mut blocks = Vec::new();
        // Request block bodies
        // Download block bodies
        while !headers.is_empty() {
            let header_batch = &headers[..min(MAX_BLOCK_BODIES_TO_REQUEST, headers.len())];
            let bodies = peers
                .request_block_bodies(header_batch)
                .await?
                .ok_or(SyncError::BodiesNotFound)?;
            debug!("Obtained: {} block bodies", bodies.len());
            let block_batch = headers
                .drain(..bodies.len())
                .zip(bodies)
                .map(|(header, body)| Block { header, body });
            blocks.extend(block_batch);
        }
        if !blocks.is_empty() {
            // Execute blocks
            info!(
                "Executing {} blocks for full sync. First block hash: {:#?} Last block hash: {:#?}",
                blocks.len(),
                blocks.first().ok_or(SyncError::NoBlocks)?.hash(),
                blocks.last().ok_or(SyncError::NoBlocks)?.hash()
            );
            add_blocks_in_batch(
                blockchain.clone(),
                cancel_token.clone(),
                blocks,
                final_batch,
                store.clone(),
            )
            .await?;
        }
    }

    // Execute pending blocks
    if !pending_blocks.is_empty() {
        info!(
            "Executing {} blocks for full sync. First block hash: {:#?} Last block hash: {:#?}",
            pending_blocks.len(),
            pending_blocks.first().ok_or(SyncError::NoBlocks)?.hash(),
            pending_blocks.last().ok_or(SyncError::NoBlocks)?.hash()
        );
        add_blocks_in_batch(
            blockchain.clone(),
            cancel_token.clone(),
            pending_blocks,
            true,
            store.clone(),
        )
        .await?;
    }

    store.clear_fullsync_headers().await?;
    Ok(())
}

async fn add_blocks_in_batch(
    blockchain: Arc<Blockchain>,
    cancel_token: CancellationToken,
    blocks: Vec<Block>,
    final_batch: bool,
    store: Store,
) -> Result<(), SyncError> {
    let execution_start = Instant::now();
    // Copy some values for later
    let blocks_len = blocks.len();
    let numbers_and_hashes = blocks
        .iter()
        .map(|b| (b.header.number, b.hash()))
        .collect::<Vec<_>>();
    let (last_block_number, last_block_hash) = numbers_and_hashes
        .last()
        .cloned()
        .ok_or(SyncError::InvalidRangeReceived)?;
    let (first_block_number, first_block_hash) = numbers_and_hashes
        .first()
        .cloned()
        .ok_or(SyncError::InvalidRangeReceived)?;

    let blocks_hashes = blocks.iter().map(|block| block.hash()).collect::<Vec<_>>();
    // Run the batch
    if let Err((err, batch_failure)) =
        add_blocks(blockchain.clone(), blocks, final_batch, cancel_token).await
    {
        if let Some(batch_failure) = batch_failure {
            warn!("Failed to add block during FullSync: {err}");
            // Since running the batch failed we set the failing block and its descendants
            // with having an invalid ancestor on the following cases.
            if let ChainError::InvalidBlock(_) = err {
                let mut block_hashes_with_invalid_ancestor: Vec<H256> = vec![];
                if let Some(index) = blocks_hashes
                    .iter()
                    .position(|x| x == &batch_failure.failed_block_hash)
                {
                    block_hashes_with_invalid_ancestor = blocks_hashes[index..].to_vec();
                }

                for hash in block_hashes_with_invalid_ancestor {
                    store
                        .set_latest_valid_ancestor(hash, batch_failure.last_valid_hash)
                        .await?;
                }
            }
        }
        return Err(err.into());
    }

    store
        .forkchoice_update(
            numbers_and_hashes,
            last_block_number,
            last_block_hash,
            None,
            None,
        )
        .await?;

    let execution_time: f64 = execution_start.elapsed().as_millis() as f64 / 1000.0;
    let blocks_per_second = blocks_len as f64 / execution_time;

    info!(
        "[SYNCING] Executed & stored {} blocks in {:.3} seconds.\n\
        Started at block with hash {} (number {}).\n\
        Finished at block with hash {} (number {}).\n\
        Blocks per second: {:.3}",
        blocks_len,
        execution_time,
        first_block_hash,
        first_block_number,
        last_block_hash,
        last_block_number,
        blocks_per_second
    );
    Ok(())
}

/// Executes the given blocks and stores them
/// If sync_head_found is true, they will be executed one by one
/// If sync_head_found is false, they will be executed in a single batch
async fn add_blocks(
    blockchain: Arc<Blockchain>,
    blocks: Vec<Block>,
    sync_head_found: bool,
    cancel_token: CancellationToken,
) -> Result<(), (ChainError, Option<BatchBlockProcessingFailure>)> {
    // If we found the sync head, run the blocks sequentially to store all the blocks's state
    if sync_head_found {
        tokio::task::spawn_blocking(move || {
            let mut last_valid_hash = H256::default();
            for block in blocks {
                let block_hash = block.hash();
                blockchain.add_block_pipeline(block, None).map_err(|e| {
                    (
                        e,
                        Some(BatchBlockProcessingFailure {
                            last_valid_hash,
                            failed_block_hash: block_hash,
                        }),
                    )
                })?;
                last_valid_hash = block_hash;
            }
            Ok(())
        })
        .await
        .map_err(|e| (ChainError::Custom(e.to_string()), None))?
    } else {
        blockchain.add_blocks_in_batch(blocks, cancel_token).await
    }
}
