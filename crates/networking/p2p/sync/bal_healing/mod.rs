//! BAL-replay state healing for snap/2.
//!
//! Fork gate: activate only when `chain_config.is_amsterdam_activated(header.timestamp)`
//! — the same fork that gates EIP-7928 BAL production (Task 0.4 contract).

mod apply;

pub use apply::apply_bal;

use ethrex_common::{H256, constants::EMPTY_BLOCK_ACCESS_LIST_HASH, types::BlockHeader};
use ethrex_storage::Store;
use tracing::{debug, info, warn};

use crate::{
    peer_handler::PeerHandler,
    peer_table::PeerTableServerProtocol as _,
    snap::{
        constants::{BAL_MAX_RETRIES_PER_BLOCK, BAL_REQUEST_BATCH_SIZE},
        request_block_access_lists,
    },
    sync::{SyncError, healing::heal_state_trie_wrap},
};

/// Advance the local state from `start_block` up to the block whose hash is
/// `target_block_hash` by fetching and replaying BALs block-by-block.
///
/// Algorithm (Task 6.8):
/// 1. Load all block headers from `start_block.number+1` to `target_block_hash`.
/// 2. Batch their hashes (`BAL_REQUEST_BATCH_SIZE = 64`) and request BALs.
/// 3. For each BAL:
///    a. Verify hash against `header.block_access_list_hash`.
///    b. Apply via `apply_bal` and verify per-block state root.
///    c. Persist the BAL (heal path does not go through `store_block`).
/// 4. Return the final state root when all blocks have been replayed.
///
/// On BAL hash/state-root mismatch or peer failure, the peer is penalised
/// and the block is re-requested from a different peer (up to
/// `BAL_MAX_RETRIES_PER_BLOCK` retries before `record_critical_failure`).
/// If all snap/2 peers are exhausted, falls back to snap/1 trie-node healing.
///
/// Task 7.4 (reorg check): before applying each BAL, verify
/// `header_{i}.parent_hash == hash(header_{i-1})`.
pub async fn advance_state_via_bals(
    store: &Store,
    peers: &PeerHandler,
    start_block: BlockHeader,
    target_block_hash: H256,
) -> Result<H256, SyncError> {
    // Step 1: load headers from start+1 to target.
    let headers = load_headers_range(store, start_block.number + 1, target_block_hash).await?;
    if headers.is_empty() {
        info!("advance_state_via_bals: no headers to replay");
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

        // Retry loop for each batch.
        let mut batch_filled = vec![false; batch_headers.len()];
        let mut retry_counts: Vec<u32> = vec![0; batch_headers.len()];

        while batch_filled.iter().any(|f| !f) {
            // Collect hashes that still need fetching.
            let pending_hashes: Vec<H256> = batch_hashes
                .iter()
                .enumerate()
                .filter(|(idx, _)| !batch_filled[*idx])
                .map(|(_, h)| *h)
                .collect();
            let pending_indices: Vec<usize> = (0..batch_hashes.len())
                .filter(|idx| !batch_filled[*idx])
                .collect();

            match request_block_access_lists(peers, &pending_hashes).await {
                Err(e) => {
                    warn!("advance_state_via_bals: failed to get snap/2 peer: {e}");
                    // No snap/2 peers; fall back to snap/1 healing for remaining blocks.
                    fallback_to_snap1_healing(store, peers, current_root, &headers[i..]).await?;
                    return Ok(current_root);
                }
                // BLOCKER 2: use the peer_id returned by request_block_access_lists so
                // record_failure always targets the peer that actually sent the bad response.
                Ok((response_bals, peer_id)) => {
                    // Strict in-order application invariant (MAJOR 3): BAL[i] is applied
                    // only after BAL[i-1] has been fully committed. pending_indices is
                    // already sorted ascending, so we process items in order and skip any
                    // slot whose predecessor is not yet filled.
                    for (bal_opt, &batch_idx) in response_bals.iter().zip(pending_indices.iter()) {
                        let header = &batch_headers[batch_idx];
                        let block_hash = batch_hashes[batch_idx];

                        let Some(bal) = bal_opt else {
                            // Peer returned None for this slot.
                            retry_counts[batch_idx] += 1;
                            if retry_counts[batch_idx] >= BAL_MAX_RETRIES_PER_BLOCK {
                                peers.peer_table.record_critical_failure(peer_id)?;
                            } else {
                                peers.peer_table.record_failure(peer_id)?;
                            }
                            continue;
                        };

                        // Task 6.10: validate_ordering failure → treat as malicious.
                        if let Err(e) = bal.validate_ordering() {
                            warn!(
                                "advance_state_via_bals: BAL ordering invalid for {block_hash:?}: {e}"
                            );
                            peers.peer_table.record_failure(peer_id)?;
                            retry_counts[batch_idx] += 1;
                            if retry_counts[batch_idx] >= BAL_MAX_RETRIES_PER_BLOCK {
                                peers.peer_table.record_critical_failure(peer_id)?;
                            }
                            continue;
                        }

                        // Step 3a: verify BAL hash.
                        let expected_bal_hash = header
                            .block_access_list_hash
                            .unwrap_or(*EMPTY_BLOCK_ACCESS_LIST_HASH);
                        let actual_bal_hash = bal.compute_hash();
                        if actual_bal_hash != expected_bal_hash {
                            warn!(
                                "advance_state_via_bals: BAL hash mismatch for {block_hash:?}: expected {expected_bal_hash:?}, got {actual_bal_hash:?}"
                            );
                            peers.peer_table.record_failure(peer_id)?;
                            retry_counts[batch_idx] += 1;
                            if retry_counts[batch_idx] >= BAL_MAX_RETRIES_PER_BLOCK {
                                peers.peer_table.record_critical_failure(peer_id)?;
                            }
                            continue;
                        }

                        // Task 7.4: reorg check — verify parent linkage before applying.
                        // For batch_idx == 0 in the first batch, parent is `start_block`.
                        // For subsequent slots: parent is the previous header.
                        let expected_parent = if i == 0 && batch_idx == 0 {
                            parent_hash
                        } else if batch_idx > 0 {
                            batch_headers[batch_idx - 1].hash()
                        } else {
                            // First slot of a non-first batch: parent is last of previous batch.
                            headers[i - 1].hash()
                        };
                        if header.parent_hash != expected_parent {
                            warn!(
                                "advance_state_via_bals: reorg detected at block {}: parent {:?} != expected {:?}",
                                header.number, header.parent_hash, expected_parent
                            );
                            // Abort and re-sync from a fresh pivot.
                            return Err(SyncError::StateRootMismatch(
                                header.parent_hash,
                                expected_parent,
                            ));
                        }

                        // Step 3b: apply and verify state root.
                        // Strict ordering: only apply batch_idx if all prior slots are filled,
                        // so current_root is always the locally-committed root of the
                        // immediately preceding block (not a header-stored value).
                        let all_prior_filled = (0..batch_idx).all(|k| batch_filled[k]);
                        if !all_prior_filled {
                            // Cannot apply out of order; will be retried next round.
                            continue;
                        }

                        // Use current_root (tracked from apply_bal return values, not from
                        // header state_root fields) as the parent for the next apply.
                        match apply_bal(store, current_root, bal, header) {
                            Ok(new_root) => {
                                current_root = new_root;
                                parent_hash = block_hash;

                                // Step 3c: persist BAL into store (heal path bypasses store_block).
                                // Required even though Phase 0 wires persistence into normal
                                // block import: the heal path applies state diffs directly
                                // and never goes through `store_block`, so it must persist
                                // BALs itself to be able to serve them later.
                                // (closes v1 minor m4)
                                if let Err(e) = store.store_block_access_list(block_hash, bal) {
                                    warn!(
                                        "advance_state_via_bals: failed to persist BAL for {block_hash:?}: {e}"
                                    );
                                }

                                batch_filled[batch_idx] = true;
                                debug!(
                                    "advance_state_via_bals: applied BAL for block {} ({block_hash:?}), new root: {new_root:?}",
                                    header.number
                                );
                            }
                            Err(SyncError::StateRootMismatch(expected, got)) => {
                                warn!(
                                    "advance_state_via_bals: state root mismatch for block {}: expected {expected:?}, got {got:?}",
                                    header.number
                                );
                                peers.peer_table.record_failure(peer_id)?;
                                retry_counts[batch_idx] += 1;
                                if retry_counts[batch_idx] >= BAL_MAX_RETRIES_PER_BLOCK {
                                    peers.peer_table.record_critical_failure(peer_id)?;
                                }
                            }
                            Err(e) => return Err(e),
                        }
                    }
                }
            }

            // Check if any slot has exhausted retries — fall back to snap/1.
            let any_exhausted = retry_counts
                .iter()
                .enumerate()
                .any(|(idx, &count)| !batch_filled[idx] && count >= BAL_MAX_RETRIES_PER_BLOCK);
            if any_exhausted {
                warn!(
                    "advance_state_via_bals: exhausted retries for batch at block {}; falling back to snap/1 healing",
                    i
                );
                fallback_to_snap1_healing(store, peers, current_root, &headers[i..]).await?;
                return Ok(current_root);
            }
        }

        i += BAL_REQUEST_BATCH_SIZE;
    }

    info!(
        "advance_state_via_bals: all {} blocks replayed",
        headers.len()
    );
    Ok(current_root)
}

/// Load headers from `start_number` up to (and including) the block with hash `target_hash`.
async fn load_headers_range(
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
            .ok_or(SyncError::MissingHeaderForBal(H256::zero()))?;
        let header = store
            .get_block_header_by_hash(hash)?
            .ok_or(SyncError::MissingHeaderForBal(hash))?;
        headers.push(header);
    }
    Ok(headers)
}

/// Fall back to snap/1 trie-node healing for the given headers (Task 6.9).
///
/// Called when all available snap/2 peers have been exhausted after retries.
async fn fallback_to_snap1_healing(
    store: &Store,
    peers: &PeerHandler,
    state_root: H256,
    _remaining_headers: &[BlockHeader],
) -> Result<(), SyncError> {
    warn!("advance_state_via_bals: falling back to snap/1 trie-node healing");
    // Invoke the existing snap/1 state healing for the remaining state gap.
    // `heal_state_trie_wrap` returns false when the pivot becomes stale, but in
    // the BAL fallback context we call it once and accept partial healing —
    // the caller's outer staleness loop will retry if needed.
    let mut dummy_leafs: u64 = 0;
    let mut dummy_storage = crate::sync::AccountStorageRoots::default();
    let mut dummy_collector = crate::sync::code_collector::CodeHashCollector::new(
        std::path::PathBuf::from("/tmp/bal_fallback_code_hashes"),
    );
    // Use a far-future staleness timestamp so healing runs to completion.
    let staleness_ts = u64::MAX;
    heal_state_trie_wrap(
        state_root,
        store.clone(),
        peers,
        staleness_ts,
        &mut dummy_leafs,
        &mut dummy_storage,
        &mut dummy_collector,
    )
    .await?;
    Ok(())
}
