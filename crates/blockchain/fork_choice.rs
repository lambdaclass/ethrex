use ethrex_core::{
    types::{Block, BlockHash, BlockHeader, BlockNumber},
    H256,
};
use ethrex_storage::{error::StoreError, Store};

use crate::{
    error::{self, InvalidForkChoice},
    is_canonical,
};
use tracing::error;

/// Applies new fork choice data to the current blockchain. It performs validity checks:
/// - The finalized, safe and head hashes must correspond to already saved blocks.
/// - The saved blocks should be in the correct order (finalized <= safe <= head).
/// - They must be connected.
///
/// After the validity checks, the canonical chain is updated so that all head's ancestors
/// and itself are made canonical.
///
/// If the fork choice state is applied correctly, the head block header is returned.
pub fn apply_fork_choice(
    store: &Store,
    head_hash: H256,
    safe_hash: H256,
    finalized_hash: H256,
) -> Result<BlockHeader, InvalidForkChoice> {
    if head_hash.is_zero() {
        return Err(InvalidForkChoice::InvalidHeadHash);
    }

    // We get the block bodies even if we only use headers them so we check that they are
    // stored too.

    let finalized_res = if !finalized_hash.is_zero() {
        store.get_block_by_hash(finalized_hash)?
    } else {
        None
    };

    let safe_res = if !safe_hash.is_zero() {
        store.get_block_by_hash(safe_hash)?
    } else {
        None
    };

    let head_res = store.get_block_by_hash(head_hash)?;

    if !safe_hash.is_zero() {
        check_order(&safe_res, &head_res)?;
    }

    if !finalized_hash.is_zero() && !safe_hash.is_zero() {
        check_order(&finalized_res, &safe_res)?;
    }

    let Some(head_block) = head_res else {
        if let Some(block) = store.get_pending_block(head_hash)? {
            trigger_sync(block);
        };
        return Err(InvalidForkChoice::Syncing);
    };

    let head = head_block.header;

    total_difficulty_check(&head_hash, &head, store)?;

    // TODO(#791): should we panic here? We should never not have a latest block number.
    let Some(latest) = store.get_latest_block_number()? else {
        return Err(StoreError::Custom("Latest block number not found".to_string()).into());
    };

    // If the head block is an already present head ancestor, skip the update.
    if is_canonical(store, head.number, head_hash)? && head.number < latest {
        return Err(InvalidForkChoice::NewHeadAlreadyCanonical);
    }

    // Find blocks that will be part of the new canonical chain.
    let Some(new_canonical_blocks) = find_link_with_canonical_chain(store, &head)? else {
        return Err(InvalidForkChoice::Disconnected(
            error::ForkChoiceElement::Head,
            error::ForkChoiceElement::Safe,
        ));
    };

    let link_block_number = match new_canonical_blocks.last() {
        Some((number, _)) => *number,
        None => head.number,
    };

    // Check that finalized and safe blocks are part of the new canonical chain.
    if let Some(ref finalized_block) = finalized_res {
        let finalized = &finalized_block.header;
        if !((is_canonical(store, finalized.number, finalized_hash)?
            && finalized.number <= link_block_number)
            || (finalized.number == head.number && finalized_hash == head_hash)
            || new_canonical_blocks.contains(&(finalized.number, finalized_hash)))
        {
            return Err(InvalidForkChoice::Disconnected(
                error::ForkChoiceElement::Head,
                error::ForkChoiceElement::Finalized,
            ));
        };
    }

    if let Some(ref safe_block) = safe_res {
        let safe = &safe_block.header;
        if !((is_canonical(store, safe.number, safe_hash)? && safe.number <= link_block_number)
            || (safe.number == head.number && safe_hash == head_hash)
            || new_canonical_blocks.contains(&(safe.number, safe_hash)))
        {
            return Err(InvalidForkChoice::Disconnected(
                error::ForkChoiceElement::Head,
                error::ForkChoiceElement::Safe,
            ));
        };
    }

    // Finished all validations.

    // Make all ancestors to head canonical.
    for (number, hash) in new_canonical_blocks {
        store.set_canonical_block(number, hash)?;
    }

    // Remove anything after the head from the canonical chain.
    for number in (head.number + 1)..(latest + 1) {
        store.unset_canonical_block(number)?;
    }

    // Make head canonical and label all special blocks correctly.
    store.set_canonical_block(head.number, head_hash)?;
    if let Some(finalized) = finalized_res {
        store.update_finalized_block_number(finalized.header.number)?;
    }
    if let Some(safe) = safe_res {
        store.update_safe_block_number(safe.header.number)?;
    }
    store.update_latest_block_number(head.number)?;

    Ok(head)
}

// Trigger a backfill sync from the block until we find a valid block that we're familiar with or
// something goes wrong.
fn trigger_sync(head_block: Block) {
    // TODO(#438): add immediate reorg if all needed blocks are pending.
    error!(
        "A sync for block {} should be triggered but it's not yet supported.",
        head_block.header.compute_block_hash()
    );
}

// Checks that block 1 is prior to block 2 and that if the second is present, the first one is too.
fn check_order(block_1: &Option<Block>, block_2: &Option<Block>) -> Result<(), InvalidForkChoice> {
    // We don't need to perform the check if the hashes are null
    match (block_1, block_2) {
        (None, Some(_)) => Err(InvalidForkChoice::ElementNotFound(
            error::ForkChoiceElement::Finalized,
        )),
        (Some(b1), Some(b2)) => {
            if b1.header.number > b2.header.number {
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
fn find_link_with_canonical_chain(
    store: &Store,
    block: &BlockHeader,
) -> Result<Option<Vec<(BlockNumber, BlockHash)>>, StoreError> {
    let mut block_number = block.number;
    let block_hash = block.compute_block_hash();
    let mut header = block.clone();
    let mut branch = Vec::new();

    if is_canonical(store, block_number, block_hash)? {
        return Ok(Some(branch));
    }

    let Some(genesis_number) = store.get_earliest_block_number()? else {
        return Err(StoreError::Custom(
            "Earliest block number not found. Node setup must have been faulty.".to_string(),
        ));
    };

    while block_number > genesis_number {
        block_number -= 1;
        let parent_hash = header.parent_hash;

        // Check that the parent exists.
        let parent_header = match store.get_block_header_by_hash(parent_hash) {
            Ok(Some(header)) => header,
            Ok(None) => return Ok(None),
            Err(error) => return Err(error),
        };

        if is_canonical(store, block_number, parent_hash)? {
            return Ok(Some(branch));
        } else {
            branch.push((block_number, parent_hash));
        }

        header = parent_header;
    }

    Ok(None)
}

fn total_difficulty_check<'a>(
    head_block_hash: &'a H256,
    head_block: &'a BlockHeader,
    storage: &'a Store,
) -> Result<(), InvalidForkChoice> {
    // This check is performed only for genesis or for blocks with difficulty.
    if head_block.difficulty.is_zero() && head_block.number != 0 {
        return Ok(());
    }

    let total_difficulty = storage
        .get_block_total_difficulty(*head_block_hash)?
        .ok_or(StoreError::Custom(
            "Block difficulty not found for head block".to_string(),
        ))?;

    let terminal_total_difficulty = storage
        .get_chain_config()?
        .terminal_total_difficulty
        .ok_or(StoreError::Custom(
            "Terminal total difficulty not found in chain config".to_string(),
        ))?;

    // Check that the header is post-merge.
    if total_difficulty < terminal_total_difficulty.into() {
        return Err(InvalidForkChoice::PreMergeBlock);
    }

    if head_block.number == 0 {
        return Ok(());
    }

    // Non genesis checks

    let parent_total_difficulty = storage
        .get_block_total_difficulty(head_block.parent_hash)?
        .ok_or(StoreError::Custom(
            "Block difficulty not found for parent block".to_string(),
        ))?;

    // TODO(#790): is this check necessary and correctly implemented?
    if parent_total_difficulty >= terminal_total_difficulty.into() {
        Err((StoreError::Custom(
            "Parent block is already post terminal total difficulty".to_string(),
        ))
        .into())
    } else {
        Ok(())
    }
}
