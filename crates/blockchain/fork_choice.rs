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

/// Maximum number of canonical blocks ethrex can revert in a single forkchoice update.
///
/// This is an implementation cap, not a spec policy. ethrex's state-history retention
/// keeps the last ~128 blocks of state diffs, so reorgs deeper than this cannot be
/// undone regardless of finalization status — the data simply isn't there.
///
/// The spec (execution-apis PR 786, "engine: Restrict no-reorg to the prefix of known
/// finalized") only forbids reorging past the finalized prefix. The finalized check is
/// applied first; this cap is a secondary guard for the implementation limit.
///
/// Reference values across ELs (devnet branches, 2026-04-30):
/// - besu (main): 90_000 — effectively unlimited
/// - erigon (glamsterdam-devnet-0): 96, env-configurable via `MAX_REORG_DEPTH`
/// - geth / nethermind / reth: no engine-API rejection; trust the CL's fork choice
pub const REORG_DEPTH_LIMIT: u64 = 128;

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
    let head_is_canonical = is_canonical(store, head.number, head_hash).await?;

    // execution-apis PR 786: the no-reorg skip is only allowed when there is a known
    // finalized block and the head references a VALID ancestor of it. Skipping for
    // unfinalized canonical ancestors is no longer permitted - those must trigger a reorg.
    //
    // Also require that head's state is actually reachable from disk. After enough
    // commits past head, the head's state root is no longer present in the trie
    // (the disk root has moved forward). Treating that FCU as a no-op would let the
    // CL move on and then fail downstream during `engine_getPayload` with a confusing
    // "state root missing" error. Falling through here lets the regular path detect
    // the missing state via the `has_state_root` check below and route the FCU into
    // the deep-reorg apply path, which installs the overlay that makes head's state
    // readable again.
    if let Some(stored_finalized) = store.get_finalized_block_number().await?
        && head.number <= stored_finalized
        && head_is_canonical
        && store.has_state_root(head.state_root)?
    {
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

    // execution-apis PR 786 point 6: -38006 TooDeepReorg is returned when the reorg
    // depth exceeds the limitation specific to the client software. ethrex's limit
    // is its state-history retention: we keep the last REORG_DEPTH_LIMIT blocks of
    // state diffs, so reorgs deeper than that cannot be unwound. We do not reject
    // reorgs that would cross the finalized prefix — the spec's only requirement on
    // finalized is point 2 (skip-when-ancestor-of-finalized, handled above) and
    // point 5 (-38002 for disconnected safe/finalized). The CL is authoritative on
    // fork choice and an EL must honor what the CL sends if it physically can.
    //
    // The shared canonical ancestor is `head` itself when head is canonical (the
    // FCU truncates the canonical chain), or one below the lowest sidechain block
    // in `new_canonical_blocks` otherwise.
    let canonical_link_height = if head_is_canonical {
        head.number
    } else {
        new_canonical_blocks
            .last()
            .map(|(n, _)| *n)
            .unwrap_or(head.number)
            .saturating_sub(1)
    };
    let reorg_depth = latest.saturating_sub(canonical_link_height);
    if reorg_depth > REORG_DEPTH_LIMIT {
        return Err(InvalidForkChoice::TooDeepReorg {
            reorg_depth,
            limit: REORG_DEPTH_LIMIT,
        });
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
// Deep-reorg apply path (issue #6685, PR 3 orchestration).
// ===========================================================================

/// Wrapper around [`apply_fork_choice`] that handles the deep-reorg case:
/// when the head's state is not reachable from the on-disk trie (the link
/// block's state has been flushed past the layer-cache edge), build an
/// in-memory overlay from the journal, replay the side chain against it,
/// and atomically reconcile on the first new-chain commit.
///
/// For shallow reorgs and no-op cases the call falls through to
/// `apply_fork_choice` and behaves identically. The 128-block
/// [`REORG_DEPTH_LIMIT`] cap is left in place; PR 4 lifts it.
pub async fn apply_fork_choice_with_deep_reorg(
    blockchain: &Blockchain,
    head_hash: H256,
    safe_hash: H256,
    finalized_hash: H256,
) -> Result<BlockHeader, InvalidForkChoice> {
    // Short-circuit when a previous deep-reorg apply is still in flight. The CL
    // retries on SYNCING; once the in-progress reorg completes, the next FCU
    // is processed normally.
    if blockchain.is_reorg_in_progress() {
        return Err(InvalidForkChoice::Syncing);
    }

    let store = blockchain.store();
    match apply_fork_choice(store, head_hash, safe_hash, finalized_hash).await {
        Ok(header) => Ok(header),
        Err(InvalidForkChoice::StateNotReachable) => {
            info!(%head_hash, "head state not reachable from disk; attempting deep-reorg apply");
            reorg_apply_deep(blockchain, head_hash, safe_hash, finalized_hash).await
        }
        Err(e) => Err(e),
    }
}

/// Drives the deep-reorg apply pass:
///
/// 1. Walk back through `HEADERS` to find the pivot ; the deepest block on
///    the OLD canonical chain that is also an ancestor of the new head.
/// 2. Look up the cache edge `D` from `STATE_HISTORY` (the highest journal
///    entry's block number).
/// 3. Build the OLD canonical chain's hash chain in `[pivot+1, D]` so the
///    overlay constructor can verify each journal entry's `block_hash`.
/// 4. Install the overlay; layer cache is reset and reads cascade through it.
/// 5. Execute the side-chain blocks `[pivot+1 .. head]` (inclusive of head)
///    in chain order via `Blockchain::add_block`. The first such block's
///    commit triggers the Section 9 reconciliation that folds overlay +
///    layer_T into a single atomic disk write.
/// 6. Update `CANONICAL_BLOCK_HASHES` via `forkchoice_update`.
async fn reorg_apply_deep(
    blockchain: &Blockchain,
    head_hash: H256,
    safe_hash: H256,
    finalized_hash: H256,
) -> Result<BlockHeader, InvalidForkChoice> {
    // Mark the reorg in progress for the duration of this call. The guard
    // clears the flag on every exit path (success, error, panic via
    // unwinding). Concurrent FCUs see the flag set and short-circuit to
    // SYNCING (see `apply_fork_choice_with_deep_reorg`).
    let _reorg_guard = blockchain.enter_reorg();

    let store = blockchain.store();

    let head = store
        .get_block_header_by_hash(head_hash)?
        .ok_or(InvalidForkChoice::Syncing)?;

    // Branch is head's non-canonical ancestors, in descending order. The
    // deepest entry's `(number-1)` is the pivot. Head itself is NOT in the
    // branch and must be appended to the replay list below.
    let new_canonical_blocks = find_link_with_canonical_chain(store, &head)
        .await?
        .ok_or(InvalidForkChoice::UnlinkedHead)?;

    let pivot_number = match new_canonical_blocks.last() {
        Some((n, _)) => n.saturating_sub(1),
        None => head.number.saturating_sub(1),
    };

    // Overlay range is `[pivot+1, edge]` where `edge` is the highest committed
    // block (= highest journal entry).
    let edge = store
        .highest_state_history_block_number()?
        .ok_or(InvalidForkChoice::StateNotReachable)?;
    let to_block = pivot_number.saturating_add(1);
    if edge < to_block {
        // Pivot is above the cache edge ; `apply_fork_choice` should have
        // succeeded as a shallow reorg. Bail.
        warn!(
            edge,
            to_block, "deep-reorg path entered but pivot is above cache edge"
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

    // Install the overlay. On failure the existing cache stays intact.
    store
        .install_overlay_for_reorg(edge, to_block, |n| canonical_hashes.get(&n).copied())
        .map_err(|e| {
            error!(error = %e, "deep-reorg: overlay install failed");
            InvalidForkChoice::StateNotReachable
        })?;

    // From this point on, any error must reset the layer cache to a fresh
    // empty state; a half-installed overlay + partial new-chain layers would
    // leak into subsequent FCU evaluations. The guard fires `abort_reorg`
    // on drop unless `disarm()` is called below after success.
    let mut abort_guard = AbortReorgGuard::new(store);

    // Execute side-chain blocks in chain order (oldest first), including head.
    // `find_link_with_canonical_chain` returns the branch in descending order
    // and EXCLUDES head; we reverse the branch and append head so reorg replay
    // covers `[pivot+1 .. head]`.
    //
    // When `new_canonical_blocks` is empty, head was already canonical and only
    // its state was unreachable from disk (commits had moved past it). Installing
    // the overlay is the entire fix; there is no side-chain to replay and head
    // itself is the pivot, so re-executing it would just trip `ParentStateNotFound`
    // (its parent state is what the overlay just made readable).
    let replay_iter: Vec<(BlockNumber, H256)> = if new_canonical_blocks.is_empty() {
        Vec::new()
    } else {
        new_canonical_blocks
            .iter()
            .rev()
            .copied()
            .chain(std::iter::once((head.number, head_hash)))
            .collect()
    };

    for (number, block_hash) in replay_iter {
        let block = match store.get_block_by_hash(block_hash).await? {
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

    // forkchoice_update succeeded; new chain is canonical. Disarm the abort
    // guard so the cache (now correct) is preserved on return.
    abort_guard.disarm();

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

/// RAII guard that calls [`Store::abort_reorg`] on drop, resetting the layer
/// cache to a fresh empty state, unless [`disarm`](Self::disarm) is called
/// first.
///
/// Armed by [`reorg_apply_deep`] immediately after `install_overlay_for_reorg`
/// succeeds, so any subsequent failure (side-chain execution error, missing
/// block body, fork-choice update error, panic via unwinding) leaves the
/// store recoverable for the next FCU.
struct AbortReorgGuard<'a> {
    store: &'a Store,
    armed: bool,
}

impl<'a> AbortReorgGuard<'a> {
    fn new(store: &'a Store) -> Self {
        Self { store, armed: true }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for AbortReorgGuard<'_> {
    fn drop(&mut self) {
        if self.armed
            && let Err(e) = self.store.abort_reorg()
        {
            error!(error = %e, "AbortReorgGuard: abort_reorg failed during cleanup");
        }
    }
}

/// Maps a [`ChainError`] from a side-chain block execution to the closest
/// [`InvalidForkChoice`] variant. Side-chain execution failures during deep
/// reorg almost always indicate the new chain is invalid; we collapse them
/// to `StateNotReachable` (engine API responds `SYNCING`). A follow-up could
/// walk back to emit `InvalidAncestor` with the last-valid-hash.
fn map_chain_error_for_fcu(_: ChainError) -> InvalidForkChoice {
    InvalidForkChoice::StateNotReachable
}
