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

/// Computes the maximum reorg depth ethrex will accept for a given fork-choice update.
///
/// The ceiling is finality-bounded: the CL is authoritative, and the EL only rejects
/// when it physically cannot unwind. Three cases are handled:
///
/// 1. **Finalized block known** (`finalized_hash` is non-zero and resolves): ceiling is
///    `latest - finalized_number`, capped by the operator override if set.
/// 2. **No finalized block, journal non-empty** (pre-merge or fresh node): ceiling is
///    `latest - (lowest_journal_block - 1)`, capped by the operator override if set.
///    The `-1` accounts for the entry at `lowest_journal_block` itself being the
///    reverse-diff used to step PAST that block; the deepest reachable pivot is
///    therefore one below the journal floor.
/// 3. **No finalized block, journal empty**: ceiling is the layer-cache retention
///    (`Store::reorg_retention`, capped at `latest`), i.e. how deep the node can reorg
///    straight from its in-memory diff-layers. With nothing finalized there is no lower
///    bound to protect, so any reorg the node can physically serve is allowed; the
///    operator override caps it when set. (Pre-PR-4 this was the fixed 128 cap.)
///
/// An operator override of `Some(0)` disables deep reorgs entirely regardless of
/// the finality state (pre-PR-4 behaviour, under full operator control).
///
/// Reference values across ELs (devnet branches, 2026-04-30):
/// - besu (main): 90_000 — effectively unlimited
/// - erigon (glamsterdam-devnet-0): 96, env-configurable via `MAX_REORG_DEPTH`
/// - geth / nethermind / reth: no engine-API rejection; trust the CL's fork choice
async fn compute_reorg_ceiling(
    store: &Store,
    finalized_hash: &H256,
    latest: u64,
    max_reorg_depth: Option<u64>,
) -> Result<u64, InvalidForkChoice> {
    // Operator cap of 0 means "no deep reorgs at all".
    if max_reorg_depth == Some(0) {
        return Ok(0);
    }

    if !finalized_hash.is_zero() {
        // Case 1: finalized block is known AND locally synced.
        // If the CL sends a finalized hash the EL hasn't synced yet (plausible after
        // snap sync or a recent restart), fall through to the journal/operator path
        // rather than treating finalized_number as 0 (which would let any depth pass).
        if let Some(h) = store.get_block_header_by_hash(*finalized_hash)? {
            // Floor at 1: a depth-1 operation is always a canonical extend or a
            // single-block fork switch whose parent state is live, never a
            // rewrite of finalized history, so it must never be rejected as
            // "too deep". Without the floor, `latest - finalized` collapses to 0
            // whenever the finalized hash IS the head (e.g. the L2 sequencer
            // calls `apply_fork_choice(head, head, head, ..)` after producing a
            // block), which would reject every canonical extension with
            // `TooDeepReorg { reorg_depth: 1, limit: 0 }`.
            let finality_ceiling = latest.saturating_sub(h.number).max(1);
            return Ok(match max_reorg_depth {
                Some(d) => finality_ceiling.min(d),
                None => finality_ceiling,
            });
        }
    }

    // Case 2 / 3: no finalized block.
    match store.lowest_state_history_block_number()? {
        Some(lowest) => {
            // Case 2: journal is non-empty; use journal extent as physical ceiling.
            //
            // Each entry at block N is a reverse-diff unwinding N -> N-1, so the
            // deepest reachable pivot is `lowest - 1` (we use the entry at `lowest`
            // to step from state-after-`lowest` to state-after-(`lowest`-1)). Max
            // physical depth is therefore `latest - (lowest - 1)`, matching case 1
            // when the journal floor lines up with finality (`lowest = finalized + 1`).
            //
            // `lowest.saturating_sub(1)` is safe: journal entries are written by
            // commit, genesis is never committed, so `lowest >= 1` whenever the
            // outer `Some(lowest)` branch fires. The saturating form is defensive
            // against a future change that could relax that invariant.
            let journal_ceiling = latest.saturating_sub(lowest.saturating_sub(1));
            Ok(match max_reorg_depth {
                Some(d) => journal_ceiling.min(d),
                None => journal_ceiling,
            })
        }
        None => {
            // Case 3: no finalized block and the journal is empty. This is a fresh or
            // short chain that has not reached the commit threshold, so every block
            // since genesis is still an uncommitted in-memory diff-layer and the node
            // can serve a reorg to any of them. The physical ceiling is the layer-cache
            // retention (the pre-PR-4 fixed cap), bounded by `latest`. Returning 0 here
            // wrongly rejected legal shallow reorgs on unfinalized chains (the Hive
            // engine re-org tests). The operator override still caps it when set.
            let physical = latest.min(store.reorg_retention()?);
            Ok(match max_reorg_depth {
                Some(d) => physical.min(d),
                None => physical,
            })
        }
    }
}

/// Applies new fork choice data to the current blockchain. It performs validity checks:
/// - The finalized, safe and head hashes must correspond to already saved blocks.
/// - The saved blocks should be in the correct order (finalized <= safe <= head).
/// - They must be connected.
///
/// After the validity checks, the canonical chain is updated so that all head's ancestors
/// and itself are made canonical.
///
/// If the fork choice state is applied correctly, the head block header is returned.
///
/// `max_reorg_depth` is the operator-configured cap (`BlockchainOptions::max_reorg_depth`).
/// Pass `None` for the finality-bounded default. See [`compute_reorg_ceiling`] for the
/// three-case logic.
pub async fn apply_fork_choice(
    store: &Store,
    head_hash: H256,
    safe_hash: H256,
    finalized_hash: H256,
    max_reorg_depth: Option<u64>,
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
    // finalized block and the head is at or below it on the canonical chain. Skipping for
    // unfinalized canonical ancestors is no longer permitted - those must trigger a reorg.
    //
    // `head.number < latest` is the strict-ancestor check; equality (head IS the current
    // canonical head) falls through to normal FCU so the CL can still build a payload on
    // top, mirroring geth's `head == current_head` carve-out in `eth/catalyst/api.go`.
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
        && head.number < latest
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

    // execution-apis PR 786 point 6: -38006 TooDeepReorg is returned when the reorg
    // depth exceeds the limitation specific to the client software. ethrex's limit
    // is finality-bounded: the ceiling is derived from the distance to the last
    // known finalized block (or to the lowest journal entry when no finalized block
    // is known), capped further by the operator-configured `max_reorg_depth` if set.
    // See `compute_reorg_ceiling` for the three-case logic.
    //
    // This check is intentionally placed before the finalized/safe connectivity
    // checks so that an over-depth reorg is rejected without those further reads. The CL is
    // authoritative on fork choice and the EL must honor what the CL sends if it
    // physically can.
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
    let ceiling = compute_reorg_ceiling(store, &finalized_hash, latest, max_reorg_depth).await?;
    if reorg_depth > ceiling {
        return Err(InvalidForkChoice::TooDeepReorg {
            reorg_depth,
            limit: ceiling,
        });
    }

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

    // If the state can't be constructed from the DB, the caller starts a sync
    // toward the head instead of ignoring the FCU.
    // TODO(#5564): handle arbitrary reorgs
    if !store.has_state_root(link_header.state_root)? {
        warn!(
            link_block=%link_block_hash,
            link_number=%link_header.number,
            head_number=%head.number,
            "FCU head state not reachable from DB state. Starting sync toward head. This is expected if the consensus client is currently syncing."
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

/// Entry point for engine-API fork-choice updates; handles both shallow and deep reorgs.
///
/// ## Deep-reorg apply flow (issue #6685)
///
/// 1. **Operator / CL submits FCU.** `apply_fork_choice_with_deep_reorg` is the
///    single entry point. It first attempts a normal (shallow) `apply_fork_choice`.
/// 2. **`StateNotReachable` triggers the deep path.** If the pivot block's state was
///    flushed past the layer-cache edge `D`, `apply_fork_choice` returns
///    `StateNotReachable` and `reorg_apply_deep` takes over.
/// 3. **`ReorgGuard` flag set; concurrent FCUs short-circuit to `SYNCING`.**
///    `blockchain.enter_reorg()` sets the in-progress flag for the lifetime of this
///    call. Any concurrent FCU sees the flag and returns `InvalidForkChoice::Syncing`
///    immediately, preventing a second reorg from racing the first.
/// 4. **`install_overlay_for_reorg` builds the overlay and swaps the cache.**
///    Journal entries `[T, D]` (inclusive, where `T = pivot+1`) are replayed in
///    descending order to produce an `Overlay` that exposes the virtual state at
///    `pivot`. The layer cache is atomically replaced with a fresh empty cache
///    carrying the overlay. Reads now cascade: layer cache -> overlay -> disk.
/// 5. **Side-chain blocks `[T .. new_head]` are executed via the normal path.**
///    `Blockchain::add_block` is called for each block in ascending order. Reads
///    hit the layer cache (new-chain diffs) then the overlay (pivot baseline) then
///    disk (unchanged old-chain state).
/// 6. **First commit folds the overlay atomically.**
///    The first new-chain block that triggers a layer commit (via the reconciliation
///    path added in PR #6689) writes the overlay entries plus the new layer together
///    in a single RocksDB write batch, then clears the overlay.
/// 7. **`AbortReorgGuard` resets cache on any failure.**
///    An `AbortReorgGuard` is armed immediately after overlay install. On any error
///    (side-chain execution failure, missing block body, fork-choice update error,
///    or panic via unwinding) the guard calls `Store::abort_reorg`, discarding the
///    overlay and any partial new-chain layers so the next FCU starts clean.
///
/// For shallow reorgs and no-op cases the call falls through to `apply_fork_choice`
/// and behaves identically.
///
/// The reorg depth ceiling is finality-bounded; see [`compute_reorg_ceiling`] for
/// the three-case logic. The operator can further restrict depth via
/// `--max-reorg-depth` (`BlockchainOptions::max_reorg_depth`).
///
/// Tracking issue: #6685. Merged PRs: #6686, #6687, #6689, and this PR.
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
    let max_reorg_depth = blockchain.options.max_reorg_depth;
    match apply_fork_choice(store, head_hash, safe_hash, finalized_hash, max_reorg_depth).await {
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
#[tracing::instrument(level = "info", skip_all, fields(namespace = "deep_reorg"))]
async fn reorg_apply_deep(
    blockchain: &Blockchain,
    head_hash: H256,
    safe_hash: H256,
    finalized_hash: H256,
) -> Result<BlockHeader, InvalidForkChoice> {
    // Atomically claim the reorg slot. If another FCU is already mid-apply, the
    // test-and-set fails and we short-circuit to SYNCING rather than racing on
    // the shared overlay/layer cache. The guard clears the flag on every exit
    // path (success, error, panic via unwinding). The pre-check in
    // `apply_fork_choice_with_deep_reorg` is only a cheap fast-path; this is the
    // authoritative gate.
    let Some(_reorg_guard) = blockchain.enter_reorg() else {
        return Err(InvalidForkChoice::Syncing);
    };

    metrics!(
        use ethrex_metrics::reorg::METRICS_REORG;
        METRICS_REORG.deep_reorg_attempts_total.inc();
    );

    let store = blockchain.store();

    let head = store
        .get_block_header_by_hash(head_hash)?
        .ok_or(InvalidForkChoice::Syncing)?;

    // `find_link_with_canonical_chain` returns an empty branch in two distinct
    // cases that must NOT be conflated for replay purposes:
    //
    // 1. Head is already canonical (its state was just unreachable from disk
    //    after enough commits past it). Overlay install is the entire fix; no
    //    side-chain to replay.
    // 2. Head is NOT canonical but its direct parent IS. Branch is empty
    //    because no non-canonical ancestors were found before the canonical
    //    parent, but we still need to add head itself via the replay loop.
    //
    // Pre-compute head_is_canonical so the replay list below can disambiguate.
    let head_is_canonical = is_canonical(store, head.number, head_hash).await?;

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

    metrics!(
        use ethrex_metrics::reorg::METRICS_REORG;
        let reorg_depth = head.number.saturating_sub(pivot_number);
        METRICS_REORG.reorg_depth_hist.observe(reorg_depth as f64);
    );

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

    // Emit overlay size metrics (O(N) over entries, but only once at install time).
    metrics!(
        use ethrex_metrics::reorg::METRICS_REORG;
        let (ov_entries, ov_bytes) = store.reorg_overlay_size_hint().unwrap_or((0, 0));
        METRICS_REORG.overlay_entries.set(ov_entries as i64);
        METRICS_REORG.overlay_bytes.set(ov_bytes as i64);
    );

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
    // Skip the replay entirely only when head is already canonical (case 1
    // above). For case 2 (parent canonical, head not), branch is empty but we
    // still need to replay head.
    let replay_iter: Vec<(BlockNumber, H256)> = if head_is_canonical {
        Vec::new()
    } else {
        new_canonical_blocks
            .iter()
            .rev()
            .copied()
            .chain(std::iter::once((head.number, head_hash)))
            .collect()
    };

    #[cfg(feature = "metrics")]
    let mut first_block = true;
    for (number, block_hash) in replay_iter {
        let block = match store.get_block_by_hash(block_hash).await? {
            Some(b) => b,
            None => {
                warn!(%number, %block_hash, "deep-reorg: side-chain block body missing");
                return Err(InvalidForkChoice::UnlinkedHead);
            }
        };
        let parent_hash = block.header.parent_hash;
        // The first add_block triggers the Section 9 reconciliation that folds the
        // overlay into the first new-chain disk commit. Time just the add_block
        // window so the histogram isolates the reconcile path (overlay-fold +
        // commit) rather than the bulk side-chain replay.
        #[cfg(feature = "metrics")]
        let reconcile_start = first_block.then(std::time::Instant::now);
        if let Err(e) = blockchain.add_block(block) {
            error!(%number, %block_hash, error = %e, "deep-reorg: side-chain block execution failed");
            // `parent_hash` is the last block we replayed successfully (or the
            // pivot for the first iteration), i.e. the deepest still-valid head
            // on the new chain ; the correct `latestValidHash` for an INVALID
            // response.
            return Err(map_chain_error_for_fcu(e, parent_hash));
        }
        #[cfg(feature = "metrics")]
        if let Some(start) = reconcile_start {
            first_block = false;
            metrics!(
                use ethrex_metrics::reorg::METRICS_REORG;
                METRICS_REORG
                    .reconcile_duration_hist
                    .observe(start.elapsed().as_secs_f64());
            );
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
    metrics!(
        use ethrex_metrics::reorg::METRICS_REORG;
        METRICS_REORG.deep_reorg_success_total.inc();
        // Overlay has been folded into disk by reconciliation; reset gauges.
        METRICS_REORG.overlay_entries.set(0);
        METRICS_REORG.overlay_bytes.set(0);
        // Emit updated journal length (O(1) via first_key / last_key).
        let journal_len = match (
            store.lowest_state_history_block_number(),
            store.highest_state_history_block_number(),
        ) {
            (Ok(Some(lo)), Ok(Some(hi))) => hi.saturating_sub(lo).saturating_add(1) as i64,
            _ => 0,
        };
        METRICS_REORG.journal_length.set(journal_len);
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
        if self.armed {
            metrics!(
                use ethrex_metrics::reorg::METRICS_REORG;
                METRICS_REORG.deep_reorg_aborts_total.inc();
                METRICS_REORG.overlay_entries.set(0);
                METRICS_REORG.overlay_bytes.set(0);
            );
            if let Err(e) = self.store.abort_reorg() {
                error!(error = %e, "AbortReorgGuard: abort_reorg failed during cleanup");
            }
        }
    }
}

/// Maps a [`ChainError`] from a side-chain block execution to the closest
/// [`InvalidForkChoice`] variant.
///
/// A block that fails validation or execution is genuinely invalid, so we emit
/// `InvalidAncestor(last_valid_hash)` ; the engine API then responds `INVALID`
/// with a `latestValidHash`, telling the CL to abandon this branch. Collapsing
/// these to `StateNotReachable` (which responds `SYNCING`) would hide the
/// verdict and let the CL retry the same invalid branch indefinitely.
///
/// Infrastructure errors (missing parent block/state, DB/trie/RLP faults) are
/// NOT a statement about block validity ; they stay `StateNotReachable` so the
/// FCU is retried rather than the branch wrongly rejected. `EvmError` is treated
/// as infrastructure here: genuine transaction-level EVM faults are already
/// reclassified into `ChainError::InvalidBlock` by `From<EvmError>`, so a bare
/// `EvmError` at this layer is a state/db problem, not an invalid block.
///
/// `last_valid_hash` is the failing block's parent ; the deepest block on the
/// new chain that replayed successfully.
fn map_chain_error_for_fcu(err: ChainError, last_valid_hash: H256) -> InvalidForkChoice {
    match err {
        ChainError::InvalidBlock(_) | ChainError::InvalidTransaction(_) => {
            InvalidForkChoice::InvalidAncestor(last_valid_hash)
        }
        ChainError::ParentNotFound
        | ChainError::ParentStateNotFound
        | ChainError::StoreError(_)
        | ChainError::TrieError(_)
        | ChainError::RLPDecodeError(_)
        | ChainError::EvmError(_)
        | ChainError::WitnessGeneration(_)
        | ChainError::Custom(_)
        | ChainError::UnknownPayload => InvalidForkChoice::StateNotReachable,
    }
}
