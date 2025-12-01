/// Utility functions for state reconstruction.
/// Used by the based block fetcher and reconstruct command.
use ethereum_types::H256;
use ethrex_common::types::BlobsBundle;
use ethrex_common::{
    U256,
    types::{Block, BlockNumber, PrivilegedL2Transaction, Transaction, batch::Batch},
};
use ethrex_l2_common::{
    l1_messages::{L1Message, get_block_l1_messages, get_l1_message_hash},
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
    let privileged_transactions: Vec<PrivilegedL2Transaction> = batch
        .iter()
        .flat_map(|block| {
            block.body.transactions.iter().filter_map(|tx| {
                if let Transaction::PrivilegedL2Transaction(tx) = tx {
                    Some(tx.clone())
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

    Ok(Batch {
        number: batch_number.as_u64(),
        first_block: first_block.header.number,
        last_block: last_block.header.number,
        state_root: new_state_root,
        privileged_transactions_hash,
        message_hashes: get_batch_message_hashes(store, batch).await?,
        blobs_bundle,
        commit_tx,
        verify_tx: None,
    })
}

async fn get_batch_message_hashes(store: &Store, batch: &[Block]) -> Result<Vec<H256>, UtilsError> {
    let mut message_hashes = Vec::new();

    for block in batch {
        let block_messages = extract_block_messages(store, block.header.number).await?;

        for msg in &block_messages {
            message_hashes.push(get_l1_message_hash(msg));
        }
    }

    Ok(message_hashes)
}

async fn extract_block_messages(
    store: &Store,
    block_number: BlockNumber,
) -> Result<Vec<L1Message>, UtilsError> {
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
    Ok(get_block_l1_messages(&receipts))
}
