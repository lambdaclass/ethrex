//! Full sync implementation
//!
//! This module contains the logic for full synchronization mode where all blocks
//! are fetched via p2p eth requests and executed to rebuild the state.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ethrex_blockchain::{
    BatchBlockProcessingFailure, Blockchain,
    error::{ChainError, InvalidBlockError},
};
use ethrex_common::{
    H256,
    types::{Block, block_access_list::BlockAccessList},
};
use ethrex_storage::Store;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::peer_handler::{BlockRequestOrder, PeerHandler};
use crate::snap::constants::MAX_HEADER_FETCH_ATTEMPTS;

use super::{EXECUTE_BATCH_SIZE, SyncError};

/// Forkchoice heads older than this (in seconds) trigger a "consensus is behind"
/// warning during sync. A synced consensus client always advertises a head only
/// a few seconds old, so a large age means the consensus client itself is lagging
/// chain head and is the sync bottleneck.
const STALE_FORKCHOICE_HEAD_SECS: u64 = 1800;

/// Distance (in blocks) below which the node is considered to be following head.
/// Below this we suppress the per-cycle sync-target logging to avoid noise on an
/// already-synced node, which runs a sync cycle on every slot.
const FOLLOW_DISTANCE: u64 = 8;

/// Render a duration in seconds as a compact human string, e.g. "13d 4h".
fn humanize_secs(secs: u64) -> String {
    if secs < 60 {
        return "< 1m".to_string();
    }
    let days = secs / 86_400;
    let hours = (secs % 86_400) / 3_600;
    let mins = (secs % 3_600) / 60;
    if days > 0 {
        format!("{days}d {hours}h")
    } else if hours > 0 {
        format!("{hours}h {mins}m")
    } else {
        format!("{mins}m")
    }
}

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

    // The consensus-provided forkchoice head, captured before `sync_head` is rewound
    // over the pending blocks above. Used for sync-target diagnostics so we report the
    // actual head rather than the rewound ancestor we end up requesting headers from.
    let fcu_head = pending_blocks
        .last()
        .map(|block| (block.header.number, block.header.timestamp));

    // Request all block headers between the sync head and our local chain
    // We will begin from the sync head so that we download the latest state first, ensuring we follow the correct chain
    // This step is not parallelized
    let mut start_block_number;
    let mut end_block_number = 0;
    let mut headers = vec![];
    let mut single_batch = true;

    let mut attempts = 0;

    // Tracks whether this cycle started meaningfully behind the consensus-provided
    // head, so we can log progress and a final "caught up" message without spamming
    // a synced node. Set on the first batch of headers we fetch.
    let mut started_behind = false;
    let mut sync_target_logged = false;

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

        // On the first batch, report the distance to the consensus-provided head and
        // warn if that head is stale (a strong signal the consensus client is behind).
        if !sync_target_logged {
            sync_target_logged = true;
            let (target, target_ts) =
                fcu_head.unwrap_or((first_header.number, first_header.timestamp));
            let local_head = store.get_latest_block_number().await?;
            let behind = target.saturating_sub(local_head);
            if behind > FOLLOW_DISTANCE {
                started_behind = true;
                info!(
                    "Sync target from consensus forkchoice: block {target} ({behind} blocks ahead of local head {local_head})"
                );
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                let age = now.saturating_sub(target_ts);
                if age > STALE_FORKCHOICE_HEAD_SECS {
                    warn!(
                        "Consensus forkchoice head (block {target}) is {} old. This can happen while the consensus client is still catching up to chain head; \
                         if so, execution will only advance as fast as it does. If sync seems slow, it may be worth checking the consensus client's sync status.",
                        humanize_secs(age)
                    );
                }
            }
        }

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

            // Download block bodies in parallel from multiple peers
            let bodies = match download_peers.request_block_bodies_parallel(&batch_headers).await {
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

            // Download block bodies in parallel from multiple peers
            let bodies = match download_peers.request_block_bodies_parallel(&batch_headers).await {
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
            peers,
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
            peers,
        )
        .await?;
    }

    // If this cycle started behind, announce that we've caught up to the head the
    // consensus client gave us, so the operator can tell idle-waiting from a hang.
    if started_behind {
        let local_head = store.get_latest_block_number().await?;
        info!(
            "Reached consensus-provided head at block {local_head}. Waiting for the next forkchoice update from the consensus client."
        );
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
    peers: &mut PeerHandler,
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
    let chain_config = store.get_chain_config();
    let bals: Vec<Option<BlockAccessList>> = {
        // Only the final batch goes through `run_blocks_pipeline`, which is the
        // path that actually consumes BALs. Non-final batches use
        // `blockchain.add_blocks_in_batch()` which doesn't accept BALs, so
        // fetching them for those batches just wastes a network round-trip.
        let any_amsterdam = final_batch
            && blocks
                .iter()
                .any(|b| chain_config.is_amsterdam_activated(b.header.timestamp));
        if any_amsterdam {
            match peers.request_block_access_lists(&blocks_hashes).await {
                Ok(Some(bals)) if bals.len() == blocks.len() => bals,
                _ => {
                    debug!("[SYNCING] BAL fetch unavailable or failed, proceeding without BALs");
                    vec![None; blocks.len()]
                }
            }
        } else {
            vec![None; blocks.len()]
        }
    };
    // Run the batch
    if let Err((err, batch_failure)) =
        add_blocks(blockchain.clone(), blocks, bals, final_batch, cancel_token).await
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
/// If sync_head_found is false, they will be executed in a single batch,
/// falling back to one-by-one pipeline execution if the batch fails with
/// a post-execution error (works around batch-mode state corruption bugs).
async fn add_blocks(
    blockchain: Arc<Blockchain>,
    blocks: Vec<Block>,
    bals: Vec<Option<BlockAccessList>>,
    sync_head_found: bool,
    cancel_token: CancellationToken,
) -> Result<(), (ChainError, Option<BatchBlockProcessingFailure>)> {
    // If we found the sync head, run the blocks sequentially to store all the blocks's state
    if sync_head_found {
        return run_blocks_pipeline(blockchain, blocks, bals).await;
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
            run_blocks_pipeline(blockchain, blocks, bals).await
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
    bals: Vec<Option<BlockAccessList>>,
) -> Result<(), (ChainError, Option<BatchBlockProcessingFailure>)> {
    tokio::task::spawn_blocking(move || {
        let mut last_valid_hash = H256::default();
        for (block, bal) in blocks.into_iter().zip(bals.into_iter()) {
            let block_hash = block.hash();
            blockchain
                .add_block_pipeline(block, bal.as_ref())
                .map_err(|e| {
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
