use ethrex_common::{
    H256,
    types::{BlockHash, BlockHeader, BlockNumber},
};
use ethrex_metrics::metrics;
use ethrex_storage::{Store, error::StoreError};
use tracing::{error, info};

use crate::{
    error::{self, InvalidForkChoice},
    is_canonical,
};

/// Maximum reorg depth supported by the layer cache.
const LAYER_CACHE_MAX_DEPTH: u64 = 128;

/// Information about a reorg that occurred during fork choice.
/// The caller must re-execute blocks from the fork point to the new head
/// to rebuild state.
#[derive(Debug)]
pub struct ReorgData {
    /// Blocks on the new fork that need re-execution, ordered oldest to newest.
    pub blocks_to_reexecute: Vec<(BlockNumber, BlockHash)>,
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
pub async fn apply_fork_choice(
    store: &Store,
    head_hash: H256,
    safe_hash: H256,
    finalized_hash: H256,
) -> Result<(BlockHeader, Option<ReorgData>), InvalidForkChoice> {
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

    let Some(_link_header) = store.get_block_header_by_hash(link_block_hash)? else {
        // Probably unreachable, but we return this error just in case.
        error!("Link block not found although it was just retrieved from the DB");
        return Err(InvalidForkChoice::UnlinkedHead);
    };

    // Detect reorg: if new canonical blocks exist and the fork point is
    // behind the current latest, the canonical chain is being switched.
    let reorg_data = if !new_canonical_blocks.is_empty() && link_block_number < latest {
        let reorg_depth = latest - link_block_number;
        if reorg_depth > LAYER_CACHE_MAX_DEPTH {
            return Err(InvalidForkChoice::ReorgTooDeep(LAYER_CACHE_MAX_DEPTH));
        }
        info!("Reorg detected: depth={reorg_depth}, reloading binary trie from checkpoint");

        // Reload binary trie from disk checkpoint. This clears layers and root map.
        let checkpoint = store
            .reload_binary_trie()
            .map_err(InvalidForkChoice::StoreError)?;
        info!("Binary trie reloaded from checkpoint at block {checkpoint}");

        // Collect blocks that need re-execution, oldest to newest.
        // If the trie checkpoint is behind the fork point, we must first
        // replay canonical blocks from checkpoint+1..=fork_point, then
        // the new fork's blocks.
        let mut blocks_to_reexecute: Vec<(BlockNumber, BlockHash)> = Vec::new();

        // Canonical blocks between checkpoint and fork point.
        for n in (checkpoint + 1)..=link_block_number {
            if let Ok(Some(hash)) = store.get_canonical_block_hash(n).await {
                blocks_to_reexecute.push((n, hash));
            }
        }

        // New fork blocks (new_canonical_blocks is newest-to-oldest, reverse it).
        let mut fork_blocks: Vec<(BlockNumber, BlockHash)> =
            new_canonical_blocks.iter().copied().collect();
        fork_blocks.reverse();
        blocks_to_reexecute.extend(fork_blocks);
        // Add the head block itself.
        blocks_to_reexecute.push((head.number, head_hash));

        Some(ReorgData {
            blocks_to_reexecute,
        })
    } else {
        None
    };

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

    Ok((head, reorg_data))
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
