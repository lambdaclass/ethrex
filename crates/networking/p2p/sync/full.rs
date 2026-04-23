//! Full sync implementation
//!
//! This module contains the logic for full synchronization mode where all blocks
//! are fetched via p2p eth requests and executed to rebuild the state.

use std::sync::Arc;
use std::time::Duration;

use ethrex_blockchain::{
    BatchBlockProcessingFailure, Blockchain,
    error::{ChainError, InvalidBlockError},
};
use ethrex_common::{
    H256,
    types::{Block, BlockHeader},
};
use ethrex_storage::Store;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::peer_handler::{BlockRequestOrder, PeerHandler};
use crate::snap::constants::MAX_HEADER_FETCH_ATTEMPTS;

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
    let mut start_block_number = 0u64;
    let mut end_block_number = 0u64;
    let mut headers = vec![];
    let mut single_batch = true;

    // BSC (fast block times): the sync_head hash from peer Status is always stale.
    // Skip hash-based lookup entirely and go straight to forward sync by block number.
    let chain_id = store.get_chain_config().chain_id;
    let is_bsc = chain_id == 56 || chain_id == 97;

    if is_bsc {
        // BSC: loop forward sync in batches until caught up or stalled.
        // Each iteration fetches 128 headers, downloads bodies, and executes.
        // This avoids waiting for a new peer sync_head between batches.
        loop {
            let latest = store.get_latest_block_number().await.unwrap_or(0);
            if latest == 0 {
                warn!("BSC forward sync: no local blocks, aborting");
                return Ok(());
            }
            headers.clear();
            start_block_number = 0;
            end_block_number = 0;
            if !request_forward_headers(
                peers,
                &store,
                latest,
                &mut start_block_number,
                &mut end_block_number,
                &mut headers,
            )
            .await?
            {
                info!("BSC forward sync: no new headers available, cycle done");
                return Ok(());
            }
            end_block_number += 1;
            start_block_number = start_block_number.max(1);

            // Download bodies in parallel from multiple peers, then execute.
            let batch_size = headers.len();
            let bodies = peers.request_block_bodies_parallel(&headers).await?;
            if bodies.len() != headers.len() {
                return Err(SyncError::BodiesNotFound);
            }
            let blocks: Vec<Block> = headers
                .drain(..)
                .zip(bodies)
                .map(|(header, body)| Block { header, body })
                .collect();
            if blocks.is_empty() {
                return Ok(());
            }
            info!(
                "BSC: executing {} blocks ({}..{})",
                blocks.len(),
                blocks.first().map(|b| b.header.number).unwrap_or(0),
                blocks.last().map(|b| b.header.number).unwrap_or(0),
            );
            add_blocks_in_batch(
                blockchain.clone(),
                cancel_token.clone(),
                blocks,
                true,
                store.clone(),
            )
            .await?;

            if batch_size < *EXECUTE_BATCH_SIZE {
                // Got fewer headers than a full batch — we're caught up
                return Ok(());
            }
            // Otherwise loop to fetch next batch
        }
    } else {
        let mut attempts = 0;

        // Request and store all block headers from the advertised sync head
        loop {
            let Some(mut block_headers) = peers
                .request_block_headers_from_hash(sync_head, BlockRequestOrder::NewToOld)
                .await?
            else {
                if attempts >= MAX_HEADER_FETCH_ATTEMPTS {
                    warn!(
                        "Sync failed to find target block header after {attempts} attempts, aborting to wait for a newer sync head"
                    );
                    return Ok(());
                }
                attempts += 1;
                warn!(
                    "Failed to fetch headers for sync head (attempt {attempts}/{MAX_HEADER_FETCH_ATTEMPTS}), retrying in 2s"
                );
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            };
            debug!("Sync Log 9: Received {} block headers", block_headers.len());
            // Reset failure counter on success so it tracks consecutive failures
            attempts = 0;

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
    } // end else (non-BSC hash-based download)
    end_block_number += 1;
    start_block_number = start_block_number.max(1);

    // Pipeline: download block bodies in a background task while the main loop executes.
    // This overlaps network I/O with block execution for better throughput.
    let (body_tx, mut body_rx) =
        tokio::sync::mpsc::channel::<Result<(Vec<Block>, bool), SyncError>>(2);

    // Clone resources for the background download task
    let mut download_peers = peers.clone();
    let download_store = store.clone();

    let download_task = tokio::spawn(async move {
        // If single_batch, we already have headers in memory — send them as the one and only batch.
        if single_batch {
            let final_batch = true;
            let mut batch_headers = headers;
            let bodies = match download_peers
                .request_block_bodies_parallel(&batch_headers)
                .await
            {
                Ok(bodies) => bodies,
                Err(e) => {
                    let _ = body_tx.send(Err(e.into())).await;
                    return;
                }
            };
            if bodies.len() != batch_headers.len() {
                let _ = body_tx.send(Err(SyncError::BodiesNotFound)).await;
                return;
            }
            debug!("Obtained: {} block bodies in parallel", bodies.len());
            let blocks: Vec<Block> = batch_headers
                .drain(..)
                .zip(bodies)
                .map(|(header, body)| Block { header, body })
                .collect();
            if !blocks.is_empty() {
                let _ = body_tx.send(Ok((blocks, final_batch))).await;
            }
            return;
        }

        // Multi-batch path: iterate through all batches, download bodies, and send them.
        for start in (start_block_number..end_block_number).step_by(*EXECUTE_BATCH_SIZE) {
            let batch_size = EXECUTE_BATCH_SIZE.min((end_block_number - start) as usize);
            let final_batch = end_block_number == start + batch_size as u64;

            let batch_headers = match download_store
                .read_fullsync_batch(start, batch_size as u64)
                .await
            {
                Ok(h) => h,
                Err(e) => {
                    let _ = body_tx.send(Err(e.into())).await;
                    return;
                }
            };
            let mut batch_headers: Vec<_> = match batch_headers
                .into_iter()
                .map(|opt| opt.ok_or(SyncError::MissingFullsyncBatch))
                .collect()
            {
                Ok(h) => h,
                Err(e) => {
                    let _ = body_tx.send(Err(e)).await;
                    return;
                }
            };

            // Download block bodies in parallel from multiple peers.
            let bodies = match download_peers
                .request_block_bodies_parallel(&batch_headers)
                .await
            {
                Ok(bodies) => bodies,
                Err(e) => {
                    let _ = body_tx.send(Err(e.into())).await;
                    return;
                }
            };
            if bodies.len() != batch_headers.len() {
                let _ = body_tx.send(Err(SyncError::BodiesNotFound)).await;
                return;
            }
            debug!("Obtained: {} block bodies in parallel", bodies.len());
            let blocks: Vec<Block> = batch_headers
                .drain(..)
                .zip(bodies)
                .map(|(header, body)| Block { header, body })
                .collect();
            if !blocks.is_empty() && body_tx.send(Ok((blocks, final_batch))).await.is_err() {
                // Receiver dropped (execution loop stopped), stop downloading
                return;
            }
        }
    });

    // Main loop: receive downloaded batches and execute them
    while let Some(result) = body_rx.recv().await {
        let (blocks, final_batch) = result?;
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

    // Ensure the download task completes and propagate any panics
    download_task.await?;

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

/// Max request failures before giving up on forward sync.
const MAX_FORWARD_SYNC_FAILURES: u64 = 200;

/// Max time to wait for peers.
const FORWARD_SYNC_PEER_WAIT: Duration = Duration::from_secs(300);

/// Requests block headers forward from `latest + 1` by block number.
async fn request_forward_headers(
    peers: &mut PeerHandler,
    store: &Store,
    latest: u64,
    start_block_number: &mut u64,
    end_block_number: &mut u64,
    headers: &mut Vec<BlockHeader>,
) -> Result<bool, SyncError> {
    let mut failures = 0u64;
    let wait_start = Instant::now();

    loop {
        let peer_count = peers.count_total_peers().await.unwrap_or(0);
        if peer_count == 0 {
            if wait_start.elapsed() > FORWARD_SYNC_PEER_WAIT {
                warn!(
                    "Forward sync: no peers after {:?}, giving up",
                    FORWARD_SYNC_PEER_WAIT
                );
                return Ok(false);
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
            continue;
        }

        let forward_start = store.get_latest_block_number().await.unwrap_or(latest) + 1;
        info!(
            forward_start,
            peer_count, failures, "Forward sync: requesting headers by number"
        );
        // Headers are small (~500B each); 1024 fits well under the response
        // limit. Using EXECUTE_BATCH_SIZE lets the outer loop keep iterating
        // (via `batch_size >= EXECUTE_BATCH_SIZE`) until we drain what a peer
        // can serve, instead of exiting after 128 blocks and waiting for the
        // sync bridge to re-trigger on the next BlockRangeUpdate.
        match peers
            .request_block_headers_from_number(
                forward_start,
                *EXECUTE_BATCH_SIZE as u64,
                BlockRequestOrder::OldToNew,
            )
            .await?
        {
            Some(forward_headers) if !forward_headers.is_empty() => {
                let first = forward_headers.first().ok_or(SyncError::NoBlocks)?;
                let last = forward_headers.last().ok_or(SyncError::NoBlocks)?;
                info!(
                    "Forward sync: received {} headers from block {} to {}",
                    forward_headers.len(),
                    first.number,
                    last.number,
                );
                *start_block_number = first.number;
                *end_block_number = last.number;
                *headers = forward_headers;
                return Ok(true);
            }
            _ => {
                failures += 1;
                if failures >= MAX_FORWARD_SYNC_FAILURES {
                    warn!(failures, "Forward sync: too many failures, giving up");
                    return Ok(false);
                }
                warn!(failures, "Forward sync: request failed, retrying...");
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    }
}

/// Executes the given blocks and stores them
/// If sync_head_found is true, they will be executed one by one
/// If sync_head_found is false, they will be executed in a single batch,
/// falling back to one-by-one pipeline execution if the batch fails with
/// a post-execution error (works around batch-mode state corruption bugs).
async fn add_blocks(
    blockchain: Arc<Blockchain>,
    blocks: Vec<Block>,
    sync_head_found: bool,
    cancel_token: CancellationToken,
) -> Result<(), (ChainError, Option<BatchBlockProcessingFailure>)> {
    // If we found the sync head, run the blocks sequentially to store all the blocks's state
    if sync_head_found {
        return run_blocks_pipeline(blockchain, blocks).await;
    }

    // Try batch execution first (faster).
    // We clone blocks because add_blocks_in_batch takes ownership but we need
    // them for the fallback. The clone cost is negligible (~1-5ms) vs batch
    // execution time (median ~29s on hoodi).
    match blockchain
        .add_blocks_in_batch(blocks.clone(), cancel_token)
        .await
    {
        Ok(()) => Ok(()),
        Err((ChainError::InvalidBlock(ref err), ref batch_failure))
            if is_post_execution_error(err) =>
        {
            // Batch execution can produce incorrect results due to cross-block
            // state cache pollution (e.g. `mark_modified` setting `exists = true`
            // leaking across block boundaries). Fall back to single-block pipeline
            // execution which uses fresh state per block.
            let failed_block_info = batch_failure
                .as_ref()
                .and_then(|f| {
                    blocks
                        .iter()
                        .find(|b| b.hash() == f.failed_block_hash)
                        .map(|b| format!("block {} ({})", b.header.number, f.failed_block_hash))
                })
                .unwrap_or_else(|| "unknown block".to_string());
            warn!(
                "Batch execution failed at {failed_block_info} with: {err}. \
                 Retrying batch with per-block pipeline execution."
            );
            run_blocks_pipeline(blockchain, blocks).await
        }
        Err(e) => Err(e),
    }
}

/// Returns true for errors that arise from EVM execution and could differ
/// between batch mode (shared VM state) and single-block pipeline mode.
/// Pre-execution validation errors (header, body, structural) would fail
/// identically in both modes, so retrying them is pointless.
fn is_post_execution_error(err: &InvalidBlockError) -> bool {
    matches!(
        err,
        InvalidBlockError::GasUsedMismatch(_, _)
            | InvalidBlockError::StateRootMismatch
            | InvalidBlockError::ReceiptsRootMismatch
            | InvalidBlockError::RequestsHashMismatch
            | InvalidBlockError::BlockAccessListHashMismatch
            | InvalidBlockError::BlobGasUsedMismatch
    )
}

async fn run_blocks_pipeline(
    blockchain: Arc<Blockchain>,
    blocks: Vec<Block>,
) -> Result<(), (ChainError, Option<BatchBlockProcessingFailure>)> {
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
}
