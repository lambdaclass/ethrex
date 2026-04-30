use ethrex_common::{
    H256,
    types::{BlockHash, BlockHeader, BlockNumber},
};
use ethrex_metrics::metrics;
use ethrex_storage::{Store, error::StoreError};
use tracing::{error, warn};

use crate::{
    error::{self, InvalidForkChoice},
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
    if let Some(stored_finalized) = store.get_finalized_block_number().await?
        && head.number <= stored_finalized
        && head_is_canonical
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

    // execution-apis PR 786: depth of reorg is the number of canonical blocks that would
    // be replaced by the new head. The shared canonical ancestor is `head` itself when
    // head is canonical (the FCU truncates the canonical chain), or one below the lowest
    // sidechain block in `new_canonical_blocks` otherwise. When the branch is empty and
    // head is non-canonical, head's parent is the canonical link.
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

    // Spec check (execution-apis PR 786): reject only when the reorg would replace
    // blocks at or below the finalized prefix. Reorgs strictly within unfinalized
    // history are legitimate fork-choice swings the EL must honor at any depth.
    if let Some(stored_finalized) = store.get_finalized_block_number().await?
        && canonical_link_height < stored_finalized
    {
        return Err(InvalidForkChoice::TooDeepReorg {
            reorg_depth,
            limit: latest.saturating_sub(stored_finalized),
        });
    }

    // Implementation cap: ethrex's state-history retention can only undo up to
    // REORG_DEPTH_LIMIT blocks. Even an unfinalized reorg deeper than this must be
    // rejected because the state to revert to is not in the DB.
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
