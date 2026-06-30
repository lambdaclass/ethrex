//! BAL-replay state healing for snap/2 (EIP-8189).
//!
//! Fork gate: activate only when the pivot is post-Amsterdam
//! (i.e. `pivot_header.block_access_list_hash.is_some()`).

mod apply;

pub use apply::apply_bal;

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
    sync::{SyncDiagnostics, SyncError},
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
/// 1. Load all block headers from `start_block.number+1` to `target_block_hash`.
/// 2. Batch their hashes (`BAL_REQUEST_BATCH_SIZE = 64`) and request BALs via snap/2.
/// 3. For each BAL:
///    a. Verify hash against `header.block_access_list_hash` (§68).
///    b. Apply via `apply_bal` and verify per-block state root.
///    c. Persist the BAL into the store.
/// 4. Return the final state root when all blocks have been replayed.
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
    // Step 1: load headers from start+1 to target.
    let headers = load_headers_range(store, start_block.number + 1, target_block_hash).await?;
    if headers.is_empty() {
        info!("advance_state_via_bals: no headers to replay, returning start root");
        return Ok(start_block.state_root);
    }

    let mut current_root = start_block.state_root;
    let mut parent_hash = start_block.hash();

    // Step 2: process in batches.
    let mut i = 0;
    while i < headers.len() {
        let batch_end = (i + BAL_REQUEST_BATCH_SIZE).min(headers.len());
        let batch_headers = &headers[i..batch_end];
        let batch_hashes: Vec<H256> = batch_headers.iter().map(|h| h.hash()).collect();

        let mut batch_filled = vec![false; batch_headers.len()];
        let mut retry_counts: Vec<u32> = vec![0; batch_headers.len()];

        while batch_filled.iter().any(|f| !f) {
            let pending_hashes: Vec<H256> = batch_hashes
                .iter()
                .enumerate()
                .filter(|(idx, _)| !batch_filled[*idx])
                .map(|(_, h)| *h)
                .collect();
            let pending_indices: Vec<usize> = (0..batch_hashes.len())
                .filter(|idx| !batch_filled[*idx])
                .collect();

            {
                let mut diag = diagnostics.write().await;
                diag.snap2_bal_requests_sent += 1;
            }

            match peers.request_snap2_bals(&pending_hashes).await {
                Err(e) => {
                    warn!("advance_state_via_bals: failed to get snap/2 peer: {e}");
                    {
                        let mut diag = diagnostics.write().await;
                        diag.snap2_peer_failures += 1;
                    }
                    // Return partial progress; caller falls back to snap/1 healing.
                    return Ok(current_root);
                }
                Ok(None) => {
                    warn!(
                        "advance_state_via_bals: no snap/2 peer available; returning partial root for snap/1 fallback"
                    );
                    {
                        let mut diag = diagnostics.write().await;
                        diag.snap2_peer_failures += 1;
                    }
                    return Ok(current_root);
                }
                Ok(Some((response_bals, peer_id))) => {
                    for (bal_opt, &batch_idx) in response_bals.iter().zip(pending_indices.iter()) {
                        let header = &batch_headers[batch_idx];
                        let block_hash = batch_hashes[batch_idx];

                        let Some(bal) = bal_opt else {
                            retry_counts[batch_idx] += 1;
                            {
                                let mut diag = diagnostics.write().await;
                                diag.snap2_validation_failures += 1;
                            }
                            if retry_counts[batch_idx] >= BAL_MAX_RETRIES_PER_BLOCK {
                                let _ = peers.peer_table.record_critical_failure(peer_id);
                            } else {
                                let _ = peers.peer_table.record_failure(peer_id);
                            }
                            continue;
                        };

                        // Strict in-batch ordering: defer apply until every prior
                        // slot has been filled. Without this, an out-of-order
                        // response could apply BAL[2] against a state that hasn't
                        // yet had BAL[1] applied — producing the wrong root.
                        let all_prior_filled = (0..batch_idx).all(|k| batch_filled[k]);
                        if !all_prior_filled {
                            continue;
                        }

                        let expected_parent = if i == 0 && batch_idx == 0 {
                            parent_hash
                        } else if batch_idx > 0 {
                            batch_headers[batch_idx - 1].hash()
                        } else {
                            headers[i - 1].hash()
                        };

                        match try_apply_bal_block(store, header, bal, current_root, expected_parent)
                        {
                            Ok(new_root) => {
                                current_root = new_root;
                                parent_hash = block_hash;
                                batch_filled[batch_idx] = true;
                                {
                                    let mut diag = diagnostics.write().await;
                                    diag.snap2_blocks_replayed += 1;
                                }
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
                                // retry from a different peer.
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
                                retry_counts[batch_idx] += 1;
                                if retry_counts[batch_idx] >= BAL_MAX_RETRIES_PER_BLOCK {
                                    let _ = peers.peer_table.record_critical_failure(peer_id);
                                } else {
                                    let _ = peers.peer_table.record_failure(peer_id);
                                }
                            }
                        }
                    }
                }
            }

            // If any slot has exhausted retries, return partial progress and let
            // the caller fall back to snap/1 healing for the remainder.
            let any_exhausted = retry_counts
                .iter()
                .enumerate()
                .any(|(idx, &count)| !batch_filled[idx] && count >= BAL_MAX_RETRIES_PER_BLOCK);
            if any_exhausted {
                warn!(
                    "advance_state_via_bals: exhausted retries for batch at block index {}; returning partial root for snap/1 fallback",
                    i
                );
                return Ok(current_root);
            }
        }

        i += BAL_REQUEST_BATCH_SIZE;
    }

    info!(
        "advance_state_via_bals: all {} blocks replayed, final root: {:?}",
        headers.len(),
        current_root
    );
    Ok(current_root)
}

/// Load headers from `start_number` up to (and including) the block with hash `target_hash`.
pub(super) async fn load_headers_range(
    store: &Store,
    start_number: u64,
    target_hash: H256,
) -> Result<Vec<BlockHeader>, SyncError> {
    let target_header = store
        .get_block_header_by_hash(target_hash)?
        .ok_or(SyncError::MissingHeaderForBal(target_hash))?;

    let end_number = target_header.number;
    if start_number > end_number {
        return Ok(vec![]);
    }

    let mut headers = Vec::with_capacity((end_number - start_number + 1) as usize);
    for number in start_number..=end_number {
        let hash = store
            .get_canonical_block_hash(number)
            .await?
            .ok_or(SyncError::MissingCanonicalBlock(number))?;
        let header = store
            .get_block_header_by_hash(hash)?
            .ok_or(SyncError::MissingHeaderForBal(hash))?;
        headers.push(header);
    }
    Ok(headers)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethrex_common::H256;
    use ethrex_common::types::BlockHeader;
    use ethrex_storage::{EngineType, Store};

    /// `PeerHandler` requires an `RLPxInitiator` actor to construct; that makes
    /// it impractical to directly unit-test `advance_state_via_bals` here. The
    /// orchestration is covered by the deferred E2E test (M4 — Phase 3). What
    /// we test instead are the deterministic inputs to that orchestration:
    /// `load_headers_range`, which feeds every downstream apply / validate
    /// step, and the `MissingHeaderForBal` short-circuit that the function
    /// produces before any peer interaction.

    async fn store_canonical_header(store: &Store, header: BlockHeader) -> H256 {
        let number = header.number;
        let hash = header.hash();
        store
            .add_block_header(hash, header)
            .await
            .expect("add_block_header");
        // Set the canonical hash at this number so `get_canonical_block_hash`
        // resolves during `load_headers_range`. `forkchoice_update` takes the
        // list of (number, hash) pairs that should become canonical.
        store
            .forkchoice_update(vec![(number, hash)], number, hash, None, None)
            .await
            .expect("forkchoice_update");
        hash
    }

    fn header_with(number: u64, parent_hash: H256) -> BlockHeader {
        BlockHeader {
            number,
            parent_hash,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn load_headers_range_empty_when_start_after_target() {
        let store = Store::new("memory", EngineType::InMemory).expect("in-memory store");
        let target_hash = store_canonical_header(&store, header_with(5, H256::zero())).await;
        // start_number > target.number ⇒ empty.
        let headers = load_headers_range(&store, 10, target_hash)
            .await
            .expect("load_headers_range");
        assert!(headers.is_empty());
    }

    #[tokio::test]
    async fn load_headers_range_missing_target_returns_error() {
        let store = Store::new("memory", EngineType::InMemory).expect("in-memory store");
        let unknown = H256::from([0xCCu8; 32]);
        let err = load_headers_range(&store, 0, unknown)
            .await
            .expect_err("must error on missing target header");
        assert!(matches!(err, SyncError::MissingHeaderForBal(h) if h == unknown));
    }

    #[tokio::test]
    async fn load_headers_range_returns_canonical_chain_in_order() {
        let store = Store::new("memory", EngineType::InMemory).expect("in-memory store");
        // Build a 4-block canonical chain anchored at zero.
        let mut last_hash = H256::zero();
        for n in 1u64..=4 {
            last_hash = store_canonical_header(&store, header_with(n, last_hash)).await;
        }
        let headers = load_headers_range(&store, 2, last_hash)
            .await
            .expect("load_headers_range");
        assert_eq!(headers.len(), 3);
        assert_eq!(headers[0].number, 2);
        assert_eq!(headers[1].number, 3);
        assert_eq!(headers[2].number, 4);
    }
}
