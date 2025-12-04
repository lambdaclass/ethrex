/// Utility functions for state reconstruction.
/// Used by the based block fetcher and reconstruct command.
use ethereum_types::H256;
use ethrex_common::types::BlobsBundle;
use ethrex_common::types::balance_diff::BalanceDiff;
use ethrex_common::{
    U256,
    types::{Block, BlockNumber, PrivilegedL2Transaction, Transaction, batch::Batch},
};
use ethrex_l2_common::messages::{
    L2Message, get_balance_diffs, get_block_l2_messages, get_l2_message_hash,
};
use ethrex_l2_common::{
    messages::{L1Message, get_block_l1_messages, get_l1_message_hash},
    privileged_transactions::compute_privileged_transactions_hash,
};
use ethrex_storage::Store;

use crate::utils::error::UtilsError;

pub async fn get_batch(
    store: &Store,
    batch: &[Block],
    batch_number: U256,
    commit_tx: Option<H256>,
    blobs_bundle: BlobsBundle,
) -> Result<Batch, UtilsError> {
    let chain_id = store.get_chain_config().chain_id;
    let privileged_transactions: Vec<PrivilegedL2Transaction> = batch
        .iter()
        .flat_map(|block| {
            block.body.transactions.iter().filter_map(|tx| {
                if let Transaction::PrivilegedL2Transaction(tx) = tx {
                    if tx.chain_id == chain_id {
                        Some(tx.clone())
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
        })
        .collect();
    let privileged_transaction_hashes = privileged_transactions
        .iter()
        .filter_map(|tx| tx.get_privileged_hash())
        .collect();

    let privileged_transactions_hash =
        compute_privileged_transactions_hash(privileged_transaction_hashes)?;

    let first_block = batch.first().ok_or(UtilsError::RetrievalError(
        "Batch is empty. This shouldn't happen.".to_owned(),
    ))?;

    let last_block = batch.last().ok_or(UtilsError::RetrievalError(
        "Batch is empty. This shouldn't happen.".to_owned(),
    ))?;

    let new_state_root = store
        .state_trie(last_block.hash())?
        .ok_or(UtilsError::InconsistentStorage(
            "This block should be in the store".to_owned(),
        ))?
        .hash_no_commit();

    let (l1_message_hashes, l2_message_hashes, balance_diffs) =
        get_batch_message_hashes_and_balance_diffs(store, batch).await?;

    Ok(Batch {
        number: batch_number.as_u64(),
        first_block: first_block.header.number,
        last_block: last_block.header.number,
        state_root: new_state_root,
        privileged_transactions_hash,
        l1_message_hashes,
        blobs_bundle,
        commit_tx,
        verify_tx: None,
        balance_diffs,
        l2_message_hashes,
    })
}

async fn get_batch_message_hashes_and_balance_diffs(
    store: &Store,
    batch: &[Block],
) -> Result<(Vec<H256>, Vec<H256>, Vec<BalanceDiff>), UtilsError> {
    let mut l1_message_hashes = Vec::new();
    let mut l2_messages = Vec::new();

    for block in batch {
        let (l1_block_messages, l2_block_messages) =
            extract_block_messages(store, block.header.number).await?;

        for l1_msg in l1_block_messages.iter() {
            l1_message_hashes.push(get_l1_message_hash(l1_msg));
        }

        for l2_msg in l2_block_messages.iter() {
            l2_messages.push(l2_msg.clone());
        }
    }

    let balance_diffs = get_balance_diffs(&l2_messages);

    let l2_message_hashes = l2_messages.iter().map(get_l2_message_hash).collect();

    Ok((l1_message_hashes, l2_message_hashes, balance_diffs))
}

async fn extract_block_messages(
    store: &Store,
    block_number: BlockNumber,
) -> Result<(Vec<L1Message>, Vec<L2Message>), UtilsError> {
    let Some(block_body) = store.get_block_body(block_number).await? else {
        return Err(UtilsError::InconsistentStorage(format!(
            "Block {block_number} is supposed to be in store at this point"
        )));
    };

    let mut receipts = vec![];
    for index in 0..block_body.transactions.len() {
        let receipt = store
            .get_receipt(
                block_number,
                index.try_into().map_err(|_| {
                    UtilsError::ConversionError("Failed to convert index to u64".to_owned())
                })?,
            )
            .await?
            .ok_or(UtilsError::RetrievalError(
                "Transactions in a block should have a receipt".to_owned(),
            ))?;
        receipts.push(receipt);
    }
    Ok((
        get_block_l1_messages(&receipts),
        get_block_l2_messages(&receipts),
    ))
}
