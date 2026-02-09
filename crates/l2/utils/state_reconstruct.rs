/// Utility functions for state reconstruction.
/// Used by the based block fetcher and reconstruct command.
use ethereum_types::H256;
use ethrex_common::types::BlobsBundle;
use ethrex_common::types::balance_diff::BalanceDiff;
use ethrex_common::{
    U256,
    types::{Block, BlockNumber, PrivilegedL2Transaction, batch::Batch},
};

use ethrex_l2_common::messages::{L2Message, get_balance_diffs, get_block_l2_out_messages};
use ethrex_l2_common::privileged_transactions::{
    get_block_l1_in_messages, get_block_l2_in_messages,
};
use ethrex_l2_common::{
    messages::{L1Message, get_block_l1_messages, get_l1_message_hash},
    privileged_transactions::compute_privileged_transactions_hash,
};
use ethrex_storage::Store;
use std::collections::BTreeMap;

use crate::utils::error::UtilsError;

pub async fn get_batch(
    store: &Store,
    batch: &[Block],
    batch_number: U256,
    commit_tx: Option<H256>,
    blobs_bundle: BlobsBundle,
    chain_id: u64,
) -> Result<Batch, UtilsError> {
    let l1_in_messages: Vec<PrivilegedL2Transaction> = batch
        .iter()
        .flat_map(|block| get_block_l1_in_messages(&block.body.transactions, chain_id))
        .collect();
    let l1_in_messages_hashes = l1_in_messages
        .iter()
        .filter_map(|tx| tx.get_privileged_hash())
        .collect();
    let l1_in_messages_rolling_hash = compute_privileged_transactions_hash(l1_in_messages_hashes)?;

    let l2_in_messages: Vec<PrivilegedL2Transaction> = batch
        .iter()
        .flat_map(|block| get_block_l2_in_messages(&block.body.transactions, chain_id))
        .collect();

    let mut l2_in_message_hashes = BTreeMap::new();
    for tx in &l2_in_messages {
        let tx_hash = tx
            .get_privileged_hash()
            .ok_or(UtilsError::InvalidPrivilegedTransaction)?;
        l2_in_message_hashes
            .entry(tx.chain_id)
            .or_insert_with(Vec::new)
            .push(tx_hash);
    }
    let mut l2_in_message_rolling_hashes = Vec::new();
    for (chain_id, hashes) in &l2_in_message_hashes {
        let rolling_hash = compute_privileged_transactions_hash(hashes.clone())?;
        l2_in_message_rolling_hashes.push((*chain_id, rolling_hash));
    }

    let non_privileged_transactions_usize = batch
        .iter()
        .map(|block| block.body.transactions.len())
        .sum::<usize>()
        - l1_in_messages.len()
        - l2_in_messages.len();

    let non_privileged_transactions: u64 = non_privileged_transactions_usize.try_into()?;

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

    let (l1_out_message_hashes, balance_diffs) =
        get_batch_message_hashes_and_balance_diffs(store, batch, chain_id).await?;

    Ok(Batch {
        number: batch_number.as_u64(),
        first_block: first_block.header.number,
        last_block: last_block.header.number,
        state_root: new_state_root,
        l1_in_messages_rolling_hash,
        l2_in_message_rolling_hashes,
        l1_out_message_hashes,
        non_privileged_transactions,
        blobs_bundle,
        commit_tx,
        verify_tx: None,
        balance_diffs,
    })
}

async fn get_batch_message_hashes_and_balance_diffs(
    store: &Store,
    batch: &[Block],
    chain_id: u64,
) -> Result<(Vec<H256>, Vec<BalanceDiff>), UtilsError> {
    let mut l1_message_hashes = Vec::new();
    let mut l2_messages = Vec::new();

    for block in batch {
        let (l1_block_messages, l2_block_messages) =
            extract_block_messages(store, block.header.number, chain_id).await?;

        for l1_msg in l1_block_messages.iter() {
            l1_message_hashes.push(get_l1_message_hash(l1_msg));
        }

        for l2_msg in l2_block_messages.iter() {
            l2_messages.push(l2_msg.clone());
        }
    }

    let balance_diffs = get_balance_diffs(&l2_messages);

    Ok((l1_message_hashes, balance_diffs))
}

async fn extract_block_messages(
    store: &Store,
    block_number: BlockNumber,
    chain_id: u64,
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
        get_block_l2_out_messages(&receipts, chain_id),
    ))
}
