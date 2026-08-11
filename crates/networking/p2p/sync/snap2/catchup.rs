//! Rolling the snap/2 flat state forward when the pivot moves.
//!
//! From devp2p `caps/snap.md`, "Synchronization algorithm":
//!
//! > 3. As the chain advances from `P` to `P+K`, fetch BALs for `P+1..P+K` via
//! >    `GetBlockAccessLists`, verify each against the `block_access_list_hash`
//! >    of its header, and apply the resulting state diff to the partial flat
//! >    state. `P+K` is then the target for any remaining range requests.
//! > 4. Repeat step 3 if the pivot advances again during catch-up.
//!
//! Every pivot move runs this before the download resumes against the new root,
//! so the flat state is never more than one pivot behind the range requests.

use std::sync::Arc;

use ethrex_common::{H256, constants::EMPTY_BLOCK_ACCESS_LIST_HASH, types::BlockHeader};
use ethrex_crypto::NativeCrypto;
use ethrex_storage::Store;
use tracing::{debug, info, warn};

use crate::{
    peer_handler::PeerHandler,
    peer_table::PeerTableServerProtocol as _,
    snap::constants::{BAL_MAX_RETRIES_PER_BLOCK, BAL_REQUEST_BATCH_SIZE},
    sync::{
        SyncDiagnostics, SyncError,
        bal_healing::load_headers_range,
        snap2::{DownloadCursor, FlatState, apply_bal_flat},
    },
};

/// Furthest the flat state may be rolled forward before the access lists it
/// needs are assumed gone.
///
/// [EIP-7928] requires the execution layer to retain access lists for at least
/// the weak subjectivity period, 3533 epochs, so that is the largest gap a peer
/// is obliged to be able to serve. Past it a catch-up would stall partway
/// through, and `caps/snap.md` has the node discard its partial state and
/// resync instead.
///
/// [EIP-7928]: https://eips.ethereum.org/EIPS/eip-7928
pub const MAX_CATCH_UP_BLOCKS: u64 = 3533 * 32;

/// Whether rolling the flat state from `from` to `to` would span more blocks
/// than peers are required to retain access lists for.
pub fn catch_up_exceeds_retention(from: &BlockHeader, to: &BlockHeader) -> bool {
    to.number.saturating_sub(from.number) > MAX_CATCH_UP_BLOCKS
}

/// Apply the access lists of every block in `(from, to]` to the flat state.
///
/// Blocks are applied in strict order. `caps/snap.md`: "BALs **must** be
/// applied in strict block order against the correct fork, with each BAL hash
/// verified before application. A wrong-fork or out-of-order BAL produces an
/// invalid state root, detected at the final root check."
///
/// Unlike the snap/1 replay this does not check a state root per block: there
/// is no complete trie to hash mid-download, and the flat state deliberately
/// holds leaves from several roots. The single check is the reconstruction's.
pub async fn catch_up(
    store: &Store,
    peers: &mut PeerHandler,
    flat: &FlatState,
    cursor: &DownloadCursor,
    from: &BlockHeader,
    to: &BlockHeader,
    diagnostics: &Arc<tokio::sync::RwLock<SyncDiagnostics>>,
) -> Result<(), SyncError> {
    if to.number <= from.number {
        return Ok(());
    }
    let headers = load_headers_range(store, from.number + 1, to.hash()).await?;
    info!(
        from = from.number,
        to = to.number,
        blocks = headers.len(),
        "snap/2 catch-up: rolling flat state forward"
    );

    let mut parent_hash = from.hash();
    for batch in headers.chunks(BAL_REQUEST_BATCH_SIZE) {
        let mut applied = 0usize;
        let mut attempts = 0u32;

        while applied < batch.len() {
            let pending: Vec<H256> = batch[applied..]
                .iter()
                .map(|header| header.hash())
                .collect();

            diagnostics.write().await.snap2_bal_requests_sent += 1;

            let Some((response, peer_id)) = peers.request_snap2_bals(&pending).await? else {
                diagnostics.write().await.snap2_peer_failures += 1;
                return Err(SyncError::Snap2CatchUpStalled(
                    batch[applied].number,
                    "no peer could serve block access lists".to_string(),
                ));
            };

            // A response may be truncated from the tail but must cover at least
            // the first slot, or nothing progresses.
            if response.is_empty() {
                attempts += 1;
                let _ = if attempts >= BAL_MAX_RETRIES_PER_BLOCK {
                    peers.peer_table.record_critical_failure(peer_id)
                } else {
                    peers.peer_table.record_failure(peer_id)
                };
                diagnostics.write().await.snap2_peer_failures += 1;
                if attempts >= BAL_MAX_RETRIES_PER_BLOCK {
                    return Err(SyncError::Snap2CatchUpStalled(
                        batch[applied].number,
                        "peers returned no entries".to_string(),
                    ));
                }
                continue;
            }

            for bal in response {
                let header = &batch[applied];

                let Some(bal) = bal else {
                    // The peer does not hold this one. Stop the run here and
                    // re-request the rest; another peer may have it.
                    diagnostics.write().await.snap2_bals_unavailable += 1;
                    break;
                };

                if header.parent_hash != parent_hash {
                    return Err(SyncError::ChainReorgDetected {
                        expected_parent: parent_hash,
                        actual_parent: header.parent_hash,
                    });
                }
                bal.validate_ordering().map_err(|err| {
                    SyncError::Snap2CatchUpStalled(header.number, format!("bad ordering: {err}"))
                })?;
                let expected = header
                    .block_access_list_hash
                    .unwrap_or(*EMPTY_BLOCK_ACCESS_LIST_HASH);
                let actual = bal.compute_hash(&NativeCrypto);
                if actual != expected {
                    let _ = peers.peer_table.record_critical_failure(peer_id);
                    diagnostics.write().await.snap2_validation_failures += 1;
                    return Err(SyncError::Snap2CatchUpStalled(
                        header.number,
                        format!("access list hash {actual:?} does not match header {expected:?}"),
                    ));
                }

                let stats = apply_bal_flat(flat, cursor, store, &bal)?;
                debug!(
                    block = header.number,
                    accounts = stats.accounts_written,
                    slots = stats.slots_written,
                    skipped_accounts = stats.accounts_skipped,
                    skipped_slots = stats.slots_skipped,
                    "snap/2 catch-up applied an access list"
                );

                // Serving this list onward costs nothing and a peer that has
                // just synced is the one best placed to help the next.
                if let Err(err) = store.store_block_access_list(header.hash(), &bal) {
                    warn!(block = header.number, "failed to persist a BAL: {err}");
                }

                parent_hash = header.hash();
                applied += 1;
                attempts = 0;
                diagnostics.write().await.snap2_blocks_replayed += 1;
            }

            let _ = peers.peer_table.record_success(peer_id);
        }
    }

    Ok(())
}
