use std::collections::BTreeMap;

use ethrex_common::H256;
use ethrex_common::types::{Block, PrivilegedL2Transaction, Receipt};
use ethrex_l2_common::merkle_tree::compute_merkle_root;
use ethrex_l2_common::messages::{
    L1Message, L2Message, get_block_l1_messages, get_block_l2_out_messages, get_l1_message_hash,
};
use ethrex_l2_common::privileged_transactions::{
    compute_privileged_transactions_hash, get_block_l1_in_messages, get_block_l2_in_messages,
};

use crate::l2::L2ExecutionError;

/// Messages and privileged transactions extracted from a batch of blocks.
pub struct BatchMessages {
    pub l1_out_messages: Vec<L1Message>,
    pub l2_out_messages: Vec<L2Message>,
    pub l1_in_messages: Vec<PrivilegedL2Transaction>,
    pub l2_in_messages: Vec<PrivilegedL2Transaction>,
}

/// Computed digests for messages and privileged transactions.
pub struct MessageDigests {
    pub l1_out_messages_merkle_root: H256,
    pub l1_in_messages_rolling_hash: H256,
    pub l2_in_message_rolling_hashes: Vec<(u64, H256)>,
}

/// Extract messages and privileged transactions from a batch of blocks.
pub fn get_batch_messages(
    blocks: &[Block],
    receipts: &[Vec<Receipt>],
    chain_id: u64,
) -> BatchMessages {
    let mut l1_out_messages = vec![];
    let mut l2_out_messages = vec![];
    let mut l1_in_messages = vec![];
    let mut l2_in_messages = vec![];

    for (block, receipts) in blocks.iter().zip(receipts) {
        let txs = &block.body.transactions;
        l1_in_messages.extend(get_block_l1_in_messages(txs, chain_id));
        l2_in_messages.extend(get_block_l2_in_messages(txs, chain_id));
        l1_out_messages.extend(get_block_l1_messages(receipts));
        l2_out_messages.extend(get_block_l2_out_messages(receipts, chain_id));
    }

    BatchMessages {
        l1_out_messages,
        l2_out_messages,
        l1_in_messages,
        l2_in_messages,
    }
}

/// Compute message digests (merkle roots, rolling hashes) for a batch.
pub fn compute_message_digests(
    batch_messages: &BatchMessages,
) -> Result<MessageDigests, L2ExecutionError> {
    // L1 out messages merkle root
    let l1_out_message_hashes: Vec<_> = batch_messages
        .l1_out_messages
        .iter()
        .map(get_l1_message_hash)
        .collect();
    let l1_out_messages_merkle_root = compute_merkle_root(&l1_out_message_hashes);

    // L1 in messages rolling hash
    let l1_in_message_hashes: Vec<_> = batch_messages
        .l1_in_messages
        .iter()
        .map(PrivilegedL2Transaction::get_privileged_hash)
        .map(|hash| hash.ok_or(L2ExecutionError::InvalidPrivilegedTransaction))
        .collect::<Result<_, _>>()?;

    let l1_in_messages_rolling_hash = compute_privileged_transactions_hash(l1_in_message_hashes)?;

    // L2 in messages rolling hashes (per chain ID)
    // We need to guarantee that the rolling hashes are computed in the same order
    // both in the prover and committer.
    let mut l2_in_hashes_per_chain_id = BTreeMap::new();

    for tx in &batch_messages.l2_in_messages {
        let tx_hash = tx
            .get_privileged_hash()
            .ok_or(L2ExecutionError::InvalidPrivilegedTransaction)?;
        l2_in_hashes_per_chain_id
            .entry(tx.chain_id)
            .or_insert_with(Vec::new)
            .push(tx_hash);
    }

    let mut l2_in_message_rolling_hashes = Vec::new();
    for (chain_id, hashes) in &l2_in_hashes_per_chain_id {
        let rolling_hash = compute_privileged_transactions_hash(hashes.clone())?;
        l2_in_message_rolling_hashes.push((*chain_id, rolling_hash));
    }

    Ok(MessageDigests {
        l1_out_messages_merkle_root,
        l1_in_messages_rolling_hash,
        l2_in_message_rolling_hashes,
    })
}
