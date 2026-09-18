//! BAL-replay state healing for snap/2 (EIP-8189).
//!
//! Fork gate: activate only when the pivot is post-Amsterdam
//! (i.e. `pivot_header.block_access_list_hash.is_some()`).

mod apply;

pub use apply::apply_bal;
pub(crate) use apply::store_code_sync;

use std::sync::Arc;

use ethrex_common::{
    H256,
    constants::EMPTY_BLOCK_ACCESS_LIST_HASH,
    types::{BlockHeader, block_access_list::BlockAccessList},
};
use ethrex_crypto::NativeCrypto;
use ethrex_storage::Store;
use tracing::{debug, info, warn};

use crate::{
    peer_handler::PeerHandler,
    peer_table::PeerTableServerProtocol as _,
    snap::constants::{BAL_MAX_RETRIES_PER_BLOCK, BAL_REQUEST_BATCH_SIZE},
    sync::{SyncDiagnostics, SyncError, snap2::gap_headers},
};

/// Reason a single-block BAL apply could not produce a valid post-state.
///
/// Pure-function output of [`try_apply_bal_block`]. The driver decides whether
/// each variant is retryable (e.g. fetch from another peer) or fatal
/// (e.g. chain reorg detected).
#[derive(Debug, thiserror::Error)]
pub enum ApplyBalError {
    #[error("BAL ordering invalid: {0}")]
    BadOrdering(String),
    #[error("BAL hash mismatch: expected {expected:?}, got {actual:?}")]
    BadHash { expected: H256, actual: H256 },
    #[error("parent hash mismatch: expected {expected_parent:?}, actual {actual_parent:?}")]
    BadParent {
        expected_parent: H256,
        actual_parent: H256,
    },
    #[error("state root mismatch after apply: expected {expected:?}, got {got:?}")]
    BadStateRoot { expected: H256, got: H256 },
    #[error("internal error during BAL apply: {0}")]
    Internal(Box<SyncError>),
}

/// Validate and apply a single block's BAL against the parent state.
///
/// Pure: no peer I/O, no diagnostics, no retry. Performs the EIP-8189
/// validation checks (ordering, hash, parent linkage, post-state root)
/// in the order the driver needs them and returns the new state root
/// on success. The BAL is also persisted to `store` so this node can
/// serve it onward — the heal path never goes through `Blockchain::store_block`.
pub fn try_apply_bal_block(
    store: &Store,
    header: &BlockHeader,
    bal: &BlockAccessList,
    parent_state_root: H256,
    expected_parent_hash: H256,
) -> Result<H256, ApplyBalError> {
    bal.validate_ordering()
        .map_err(ApplyBalError::BadOrdering)?;

    let expected_bal_hash = header
        .block_access_list_hash
        .unwrap_or(*EMPTY_BLOCK_ACCESS_LIST_HASH);
    let actual_bal_hash = bal.compute_hash(&NativeCrypto);
    if actual_bal_hash != expected_bal_hash {
        return Err(ApplyBalError::BadHash {
            expected: expected_bal_hash,
            actual: actual_bal_hash,
        });
    }

    if header.parent_hash != expected_parent_hash {
        return Err(ApplyBalError::BadParent {
            expected_parent: expected_parent_hash,
            actual_parent: header.parent_hash,
        });
    }

    match apply_bal(store, parent_state_root, bal, header) {
        Ok(new_root) => {
            if let Err(e) = store.store_block_access_list(header.hash(), bal) {
                warn!(
                    "try_apply_bal_block: failed to persist BAL for {:?}: {e}",
                    header.hash()
                );
            }
            Ok(new_root)
        }
        Err(SyncError::StateRootMismatch(expected, got)) => {
            Err(ApplyBalError::BadStateRoot { expected, got })
        }
        Err(other) => Err(ApplyBalError::Internal(Box::new(other))),
    }
}

/// Advance local state from `start_block` up to the block whose hash is
/// `target_block_hash` by fetching and replaying BALs block-by-block.
///
/// Algorithm (EIP-8189 §"Synchronization Algorithm"):
/// 1. Resolve the headers in `(start_block, target]` by walking parent hashes
///    back from the target, which also detects a reorg past `start_block`.
/// 2. Batch their hashes (`BAL_REQUEST_BATCH_SIZE`) and request BALs via snap/2.
/// 3. Apply each returned BAL in order with [`try_apply_bal_block`], which
///    verifies the hash against the header and the resulting state root, and
///    persists the BAL.
/// 4. Return the final state root when all blocks have been replayed.
///
/// Responses are consumed strictly in order. A peer may truncate from the tail
/// or mark an entry unavailable; either way the run stops there and the rest is
/// re-requested, possibly from another peer. Re-fetching a few entries a
/// previous response already carried is cheaper than keeping them aside.
///
/// Returns the post-replay state root. On degraded paths (no snap/2 peer, peer
/// request error, exhausted per-block retries) returns the partial root reached
/// so far — the caller compares against the target and falls back to snap/1
/// trie healing for the remainder. Fatal conditions (chain reorg detected,
/// internal store errors) propagate via `Err`.
pub async fn advance_state_via_bals(
    store: &Store,
    peers: &mut PeerHandler,
    start_block: BlockHeader,
    target_block_hash: H256,
    diagnostics: &Arc<tokio::sync::RwLock<SyncDiagnostics>>,
) -> Result<H256, SyncError> {
    let target = store
        .get_block_header_by_hash(target_block_hash)?
        .ok_or(SyncError::MissingHeaderForBal(target_block_hash))?;
    if target.number <= start_block.number {
        info!("advance_state_via_bals: no headers to replay, returning start root");
        return Ok(start_block.state_root);
    }
    let headers = gap_headers(store, &start_block, &target)?;

    let mut current_root = start_block.state_root;
    let mut parent_hash = start_block.hash();

    for batch in headers.chunks(BAL_REQUEST_BATCH_SIZE) {
        let mut applied = 0usize;
        // Failures since the last block that applied. Every failure concerns the
        // first pending block — the run stops at the first bad or missing entry —
        // so this is that block's retry count.
        let mut attempts = 0u32;

        while applied < batch.len() {
            let pending: Vec<H256> = batch[applied..]
                .iter()
                .map(|header| header.hash())
                .collect();
            diagnostics.write().await.snap2_bal_requests_sent += 1;

            let (response, peer_id) = match peers.request_snap2_bals(&pending).await {
                Ok(Some(served)) => served,
                Ok(None) => {
                    warn!(
                        "advance_state_via_bals: no snap/2 peer available; returning partial root for snap/1 fallback"
                    );
                    diagnostics.write().await.snap2_peer_failures += 1;
                    return Ok(current_root);
                }
                Err(e) => {
                    warn!("advance_state_via_bals: failed to get snap/2 peer: {e}");
                    diagnostics.write().await.snap2_peer_failures += 1;
                    return Ok(current_root);
                }
            };

            // EIP-8189 permits truncating a response from the tail, but one
            // covering no entry at all makes no progress. Charge it to the first
            // pending block so the retry budget stays finite.
            if response.is_empty() {
                warn!(
                    "advance_state_via_bals: peer {peer_id} returned no entries for {} pending blocks",
                    pending.len()
                );
                diagnostics.write().await.snap2_peer_failures += 1;
                attempts += 1;
                if attempts >= BAL_MAX_RETRIES_PER_BLOCK {
                    let _ = peers.peer_table.record_critical_failure(peer_id);
                    return exhausted(batch[applied].number, current_root);
                }
                let _ = peers.peer_table.record_failure(peer_id);
                continue;
            }

            for bal in response {
                let header = &batch[applied];
                let block_hash = header.hash();

                let Some(bal) = bal else {
                    // The peer does not hold this one. Stop the run here and
                    // re-request the rest; another peer may have it.
                    diagnostics.write().await.snap2_bals_unavailable += 1;
                    attempts += 1;
                    if attempts >= BAL_MAX_RETRIES_PER_BLOCK {
                        let _ = peers.peer_table.record_critical_failure(peer_id);
                        return exhausted(header.number, current_root);
                    }
                    let _ = peers.peer_table.record_failure(peer_id);
                    break;
                };

                match try_apply_bal_block(store, header, &bal, current_root, parent_hash) {
                    Ok(new_root) => {
                        current_root = new_root;
                        parent_hash = block_hash;
                        applied += 1;
                        attempts = 0;
                        diagnostics.write().await.snap2_blocks_replayed += 1;
                        debug!(
                            "advance_state_via_bals: applied BAL for block {} ({block_hash:?}), new root: {new_root:?}",
                            header.number
                        );
                        let _ = peers.peer_table.record_success(peer_id);
                    }
                    Err(ApplyBalError::BadParent {
                        expected_parent,
                        actual_parent,
                    }) => {
                        // `gap_headers` already verified every link, so this cannot
                        // fire for a header it returned; the mapping is kept so a
                        // future caller with unchecked headers still gets the
                        // recoverable error.
                        warn!(
                            "advance_state_via_bals: reorg detected at block {}: parent {actual_parent:?} != expected {expected_parent:?}",
                            header.number
                        );
                        return Err(SyncError::ChainReorgDetected {
                            expected_parent,
                            actual_parent,
                        });
                    }
                    Err(ApplyBalError::Internal(e)) => return Err(*e),
                    Err(err) => {
                        // BadOrdering | BadHash | BadStateRoot — peer-attributable,
                        // retry from a different peer. Stop the run; the rest is
                        // re-requested.
                        warn!(
                            "advance_state_via_bals: validation failed for block {} ({block_hash:?}): {err}",
                            header.number
                        );
                        {
                            let mut diag = diagnostics.write().await;
                            diag.snap2_validation_failures += 1;
                            if matches!(err, ApplyBalError::BadStateRoot { .. }) {
                                diag.snap2_peer_failures += 1;
                            }
                        }
                        attempts += 1;
                        if attempts >= BAL_MAX_RETRIES_PER_BLOCK {
                            let _ = peers.peer_table.record_critical_failure(peer_id);
                            return exhausted(header.number, current_root);
                        }
                        let _ = peers.peer_table.record_failure(peer_id);
                        break;
                    }
                }
            }
        }
    }

    info!(
        "advance_state_via_bals: all {} blocks replayed, final root: {:?}",
        headers.len(),
        current_root
    );
    Ok(current_root)
}

/// The retry budget for one block ran out: report it and hand the partial root
/// back so the caller can heal the remainder with snap/1.
fn exhausted(block_number: u64, current_root: H256) -> Result<H256, SyncError> {
    warn!(
        "advance_state_via_bals: exhausted retries at block {block_number}; returning partial root for snap/1 fallback"
    );
    Ok(current_root)
}
