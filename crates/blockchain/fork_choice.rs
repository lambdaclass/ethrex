use ethrex_common::{
    H256,
    types::{BlockHash, BlockHeader, BlockNumber},
};
use ethrex_metrics::metrics;
use ethrex_storage::{Store, error::StoreError};
use std::collections::HashMap;
use tracing::{error, info, warn};

use crate::{
    Blockchain,
    error::{self, ChainError, InvalidForkChoice},
    is_canonical,
};

/// Applies new fork choice data to the current blockchain. It performs validity checks:
/// - The finalized, safe and head hashes must correspond to already saved blocks.
/// - The saved blocks should be in the correct order (finalized <= safe <= head).
/// - They must be connected.
///
/// After the validity checks, the canonical chain is updated so that all head's ancestors
/// and itself are made canonical.
///
/// If the fork choice state is applied correctly, the head block header is returned.
pub async fn apply_fork_choice(
    store: &Store,
    head_hash: H256,
    safe_hash: H256,
    finalized_hash: H256,
) -> Result<BlockHeader, InvalidForkChoice> {
    if head_hash.is_zero() {
        return Err(InvalidForkChoice::InvalidHeadHash);
    }

    let finalized_res = if !finalized_hash.is_zero() {
        store.get_block_header_by_hash(finalized_hash)?
    } else {
        None
    };

    let safe_res = if !safe_hash.is_zero() {
        store.get_block_header_by_hash(safe_hash)?
    } else {
        None
    };

    let head_res = store.get_block_header_by_hash(head_hash)?;

    if !safe_hash.is_zero() {
        check_order(&safe_res, &head_res)?;
    }

    if !finalized_hash.is_zero() && !safe_hash.is_zero() {
        check_order(&finalized_res, &safe_res)?;
    }

    let Some(head) = head_res else {
        return Err(InvalidForkChoice::Syncing);
    };

    let latest = store.get_latest_block_number().await?;

    // If the head block is an already present head ancestor, skip the update.
    if is_canonical(store, head.number, head_hash).await? && head.number < latest {
        return Err(InvalidForkChoice::NewHeadAlreadyCanonical);
    }

    // Find blocks that will be part of the new canonical chain.
    let Some(new_canonical_blocks) = find_link_with_canonical_chain(store, &head).await? else {
        return Err(InvalidForkChoice::UnlinkedHead);
    };

    let (link_block_number, link_block_hash) = match new_canonical_blocks.last() {
        Some((number, hash)) => (*number, *hash),
        None => (head.number, head_hash),
    };

    // Check that finalized and safe blocks are part of the new canonical chain.
    if let Some(ref finalized) = finalized_res
        && !((is_canonical(store, finalized.number, finalized_hash).await?
            && finalized.number <= link_block_number)
            || (finalized.number == head.number && finalized_hash == head_hash)
            || new_canonical_blocks.contains(&(finalized.number, finalized_hash)))
    {
        return Err(InvalidForkChoice::Disconnected(
            error::ForkChoiceElement::Head,
            error::ForkChoiceElement::Finalized,
        ));
    }

    if let Some(ref safe) = safe_res
        && !((is_canonical(store, safe.number, safe_hash).await?
            && safe.number <= link_block_number)
            || (safe.number == head.number && safe_hash == head_hash)
            || new_canonical_blocks.contains(&(safe.number, safe_hash)))
    {
        return Err(InvalidForkChoice::Disconnected(
            error::ForkChoiceElement::Head,
            error::ForkChoiceElement::Safe,
        ));
    }

    let Some(link_header) = store.get_block_header_by_hash(link_block_hash)? else {
        // Probably unreachable, but we return this error just in case.
        error!("Link block not found although it was just retrieved from the DB");
        return Err(InvalidForkChoice::UnlinkedHead);
    };

    // If the state can't be constructed from the DB, we ignore it and log a warning.
    // TODO(#5564): handle arbitrary reorgs
    if !store.has_state_root(link_header.state_root)? {
        warn!(
            link_block=%link_block_hash,
            link_number=%link_header.number,
            head_number=%head.number,
            "FCU head state not reachable from DB state. Ignoring fork choice update. This is expected if the consensus client is currently syncing. Otherwise, if consensus is synced and this is a consistent message it can be fixed by removing the DB and re-syncing the execution client."
        );
        return Err(InvalidForkChoice::StateNotReachable);
    }

    // Finished all validations.

    store
        .forkchoice_update(
            new_canonical_blocks,
            head.number,
            head_hash,
            safe_res.map(|h| h.number),
            finalized_res.map(|h| h.number),
        )
        .await?;

    metrics!(
        use ethrex_metrics::blocks::METRICS_BLOCKS;

        METRICS_BLOCKS.set_head_height(head.number);
    );

    Ok(head)
}

// Checks that block 1 is prior to block 2 and that if the second is present, the first one is too.
fn check_order(
    block_1: &Option<BlockHeader>,
    block_2: &Option<BlockHeader>,
) -> Result<(), InvalidForkChoice> {
    // We don't need to perform the check if the hashes are null
    match (block_1, block_2) {
        (None, Some(_)) => Err(InvalidForkChoice::ElementNotFound(
            error::ForkChoiceElement::Finalized,
        )),
        (Some(b1), Some(b2)) => {
            if b1.number > b2.number {
                Err(InvalidForkChoice::Unordered)
            } else {
                Ok(())
            }
        }
        _ => Err(InvalidForkChoice::Syncing),
    }
}

// Find branch of the blockchain connecting a block with the canonical chain. Returns the
// number-hash pairs representing all blocks in that brunch. If genesis is reached and the link
// hasn't been found, an error is returned.
//
// Return values:
// - Err(StoreError): a db-related error happened.
// - Ok(None): The block is not connected to the canonical chain.
// - Ok(Some([])): the block is already canonical.
// - Ok(Some(branch)): the "branch" is a sequence of blocks that connects the ancestor and the
//   descendant.
async fn find_link_with_canonical_chain(
    store: &Store,
    block_header: &BlockHeader,
) -> Result<Option<Vec<(BlockNumber, BlockHash)>>, StoreError> {
    let mut block_number = block_header.number;
    let block_hash = block_header.hash();
    let mut branch = Vec::new();

    if is_canonical(store, block_number, block_hash).await? {
        return Ok(Some(branch));
    }

    let genesis_number = store.get_earliest_block_number().await?;
    let mut header = block_header.clone();

    while block_number > genesis_number {
        block_number -= 1;
        let parent_hash = header.parent_hash;

        // Check that the parent exists.
        let parent_header = match store.get_block_header_by_hash(parent_hash) {
            Ok(Some(header)) => header,
            Ok(None) => return Ok(None),
            Err(error) => return Err(error),
        };

        if is_canonical(store, block_number, parent_hash).await? {
            return Ok(Some(branch));
        } else {
            branch.push((block_number, parent_hash));
        }

        header = parent_header;
    }

    Ok(None)
}

// ===========================================================================
// Deep-reorg apply path (Section 8 orchestration).
// ===========================================================================

/// Wrapper around [`apply_fork_choice`] that handles the deep-reorg case:
/// when the head's state is not directly reachable from the on-disk trie,
/// build an in-memory overlay from the journal, replay the side chain
/// against it, and reconcile on the first new-chain commit.
///
/// Falls through to the simpler `apply_fork_choice` for shallow reorgs and
/// no-op cases.
///
/// Pre-condition: the journal contains entries down to the deepest required
/// pivot. If finalization has pruned past the pivot, the call returns
/// [`InvalidForkChoice::StateNotReachable`] (the engine API responds with
/// `SYNCING`, matching today's behavior pre-deep-reorg).
pub async fn apply_fork_choice_with_deep_reorg(
    blockchain: &Blockchain,
    head_hash: H256,
    safe_hash: H256,
    finalized_hash: H256,
) -> Result<BlockHeader, InvalidForkChoice> {
    // Section 11 — short-circuit when a previous deep-reorg apply is still in
    // flight. The CL retries on SYNCING; once the in-progress reorg completes,
    // the next FCU is processed normally. Reth's pattern at
    // `crates/engine/tree/src/tree/mod.rs:1173-1178`.
    if blockchain.is_reorg_in_progress() {
        return Err(InvalidForkChoice::Syncing);
    }

    let store = blockchain.store();
    match apply_fork_choice(store, head_hash, safe_hash, finalized_hash).await {
        Ok(header) => Ok(header),
        Err(InvalidForkChoice::StateNotReachable) => {
            info!(%head_hash, "head state not directly reachable; attempting deep-reorg apply");
            reorg_apply_deep(blockchain, head_hash, safe_hash, finalized_hash).await
        }
        Err(e) => Err(e),
    }
}

/// Drives the deep-reorg apply pass:
///
/// 1. Walk back through `HEADERS` to find the pivot — the deepest block on
///    the OLD canonical chain that is also an ancestor of the new head.
/// 2. Look up the cache edge `D` from `STATE_HISTORY` (the highest journal
///    entry's block number).
/// 3. Build the OLD canonical chain's hash chain in `[pivot+1, D]` so the
///    overlay constructor can verify each journal entry's `block_hash`.
/// 4. Install the overlay (storage primitive — Section 8.3-8.5).
/// 5. Execute the side-chain blocks `[pivot+1 .. new_head]` in chain order
///    via `Blockchain::add_block`. The first such block's commit triggers
///    the Section 9 reconciliation that folds overlay + layer_T into a
///    single atomic disk write.
/// 6. Update `CANONICAL_BLOCK_HASHES` via `forkchoice_update`.
async fn reorg_apply_deep(
    blockchain: &Blockchain,
    head_hash: H256,
    safe_hash: H256,
    finalized_hash: H256,
) -> Result<BlockHeader, InvalidForkChoice> {
    // Mark the reorg in progress for the duration of this call. The guard
    // clears the flag on every exit path (success, early return, panic via
    // unwinding). Concurrent FCUs from the engine API will see the flag set
    // and short-circuit to SYNCING (see `apply_fork_choice_with_deep_reorg`).
    let _reorg_guard = blockchain.enter_reorg();

    let store = blockchain.store();

    let head = store
        .get_block_header_by_hash(head_hash)?
        .ok_or(InvalidForkChoice::Syncing)?;

    // Branch is the side-fork chain in DESCENDING order (new_head's parent
    // first, then deeper). The deepest entry's `(number-1)` is the pivot.
    let new_canonical_blocks = find_link_with_canonical_chain(store, &head)
        .await?
        .ok_or(InvalidForkChoice::UnlinkedHead)?;

    // Pivot = parent of the deepest side-fork entry, or head's direct parent
    // if the branch is empty (head's parent is canonical, no real reorg).
    let pivot_number = match new_canonical_blocks.last() {
        Some((n, _)) => n.saturating_sub(1),
        None => head.number.saturating_sub(1),
    };

    // The overlay's range is `[pivot+1, edge]` where edge is the highest
    // committed block (= highest journal entry).
    let edge = store
        .highest_state_history_block_number()?
        .ok_or(InvalidForkChoice::StateNotReachable)?;
    let to_block = pivot_number.saturating_add(1);
    if edge < to_block {
        // The pivot is above the cache edge — apply_fork_choice should have
        // handled this as a shallow reorg. If we reach here, something is
        // off; punt.
        warn!(
            edge, to_block,
            "deep-reorg path entered but pivot is above cache edge"
        );
        return Err(InvalidForkChoice::StateNotReachable);
    }

    // Pre-build the OLD canonical chain's hash lookup for journal verification.
    // This must reflect the chain BEFORE we update CANONICAL_BLOCK_HASHES below.
    let mut canonical_hashes: HashMap<BlockNumber, H256> = HashMap::new();
    for n in to_block..=edge {
        if let Some(hash) = store.get_canonical_block_hash_sync(n)? {
            canonical_hashes.insert(n, hash);
        }
    }

    // Install overlay. Errors abort cleanly; the existing cache stays intact.
    store
        .install_overlay_for_reorg(edge, to_block, |n| canonical_hashes.get(&n).copied())
        .map_err(|e| {
            error!(error = %e, "deep-reorg: overlay install failed");
            InvalidForkChoice::StateNotReachable
        })?;

    // Execute the side-chain blocks in CHAIN order (oldest first). The
    // existing `add_block` path handles execution + storage; layer cache
    // reads cascade through the freshly-installed overlay.
    for (number, block_hash) in new_canonical_blocks.iter().rev() {
        let block = match store.get_block_by_hash(*block_hash).await? {
            Some(b) => b,
            None => {
                warn!(%number, %block_hash, "deep-reorg: side-chain block body missing");
                return Err(InvalidForkChoice::UnlinkedHead);
            }
        };
        if let Err(e) = blockchain.add_block(block) {
            error!(%number, %block_hash, error = %e, "deep-reorg: side-chain block execution failed");
            return Err(map_chain_error_for_fcu(e));
        }
    }

    // Resolve safe / finalized for the canonical-hash update.
    let safe_res = if !safe_hash.is_zero() {
        store.get_block_header_by_hash(safe_hash)?
    } else {
        None
    };
    let finalized_res = if !finalized_hash.is_zero() {
        store.get_block_header_by_hash(finalized_hash)?
    } else {
        None
    };

    store
        .forkchoice_update(
            new_canonical_blocks,
            head.number,
            head_hash,
            safe_res.map(|h| h.number),
            finalized_res.map(|h| h.number),
        )
        .await?;

    metrics!(
        use ethrex_metrics::blocks::METRICS_BLOCKS;
        METRICS_BLOCKS.set_head_height(head.number);
    );

    info!(
        head_number = head.number,
        pivot_number,
        side_chain_len = head.number.saturating_sub(pivot_number),
        "deep-reorg apply succeeded"
    );

    Ok(head)
}

/// Maps a `ChainError` from a side-chain block execution into the
/// closest-fitting [`InvalidForkChoice`] variant. Most chain errors during
/// side-chain replay indicate the new chain is invalid, so we collapse them
/// to a generic `StateNotReachable` (engine API responds `SYNCING`) — a more
/// specific `InvalidAncestor` could be emitted in a follow-up that walks
/// back to find the exact bad block.
fn map_chain_error_for_fcu(_: ChainError) -> InvalidForkChoice {
    InvalidForkChoice::StateNotReachable
}
