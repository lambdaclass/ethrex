use ethereum_types::U256;
use ethrex_common::{
    H256, InvalidBlockError,
    constants::EMPTY_WITHDRAWALS_HASH,
    types::{BlockBody, BlockHeader, InvalidBlockBodyError, Receipt, Transaction, Withdrawal},
};
use ethrex_crypto::Crypto;
use ethrex_rlp::encode::RLPEncode;
use std::collections::BTreeMap;

use crate::Trie;

/// Computes the MPT root hash of a list of transactions (EIP-2718 encoded, keyed by index).
pub fn compute_transactions_root(transactions: &[Transaction], crypto: &dyn Crypto) -> H256 {
    let iter = transactions
        .iter()
        .enumerate()
        .map(|(idx, tx)| (idx.encode_to_vec(), tx.encode_canonical_to_vec()));
    Trie::compute_hash_from_unsorted_iter(iter, crypto)
}

/// Computes the MPT root hash of a list of receipts (keyed by index).
pub fn compute_receipts_root(receipts: &[Receipt], crypto: &dyn Crypto) -> H256 {
    let iter = receipts
        .iter()
        .enumerate()
        .map(|(idx, receipt)| (idx.encode_to_vec(), receipt.encode_inner_with_bloom(crypto)));
    Trie::compute_hash_from_unsorted_iter(iter, crypto)
}

/// Computes the MPT root hash of a list of withdrawals (EIP-4895, keyed by index).
pub fn compute_withdrawals_root(withdrawals: &[Withdrawal], crypto: &dyn Crypto) -> H256 {
    let iter = withdrawals
        .iter()
        .enumerate()
        .map(|(idx, withdrawal)| (idx.encode_to_vec(), withdrawal.encode_to_vec()));
    Trie::compute_hash_from_unsorted_iter(iter, crypto)
}

/// Computes the MPT root hash for an account's storage trie.
///
/// Zero-value slots are excluded (they represent deleted/empty slots).
pub fn compute_storage_root(storage: &BTreeMap<U256, U256>, crypto: &dyn Crypto) -> H256 {
    let iter = storage.iter().filter_map(|(k, v)| {
        (!v.is_zero()).then_some((
            crypto.keccak256(&k.to_big_endian()).to_vec(),
            v.encode_to_vec(),
        ))
    });
    Trie::compute_hash_from_unsorted_iter(iter, crypto)
}

/// Computes the MPT state root from an iterator of `(hashed_address_bytes, encoded_account_state)` pairs.
pub fn compute_state_root(
    iter: impl Iterator<Item = (Vec<u8>, Vec<u8>)>,
    crypto: &dyn Crypto,
) -> H256 {
    Trie::compute_hash_from_unsorted_iter(iter, crypto)
}

/// Validates that the block body matches the block header.
///
/// Checks transactions root, empty ommers, and withdrawals root.
pub fn validate_block_body(
    block_header: &BlockHeader,
    block_body: &BlockBody,
    crypto: &dyn Crypto,
) -> Result<(), InvalidBlockBodyError> {
    let computed_tx_root = compute_transactions_root(&block_body.transactions, crypto);

    if block_header.transactions_root != computed_tx_root {
        return Err(InvalidBlockBodyError::TransactionsRootNotMatch);
    }

    if !block_body.ommers.is_empty() {
        return Err(InvalidBlockBodyError::OmmersIsNotEmpty);
    }

    match (block_header.withdrawals_root, &block_body.withdrawals) {
        (Some(withdrawals_root), Some(withdrawals)) => {
            let computed_withdrawals_root = compute_withdrawals_root(withdrawals, crypto);
            if withdrawals_root != computed_withdrawals_root {
                return Err(InvalidBlockBodyError::WithdrawalsRootNotMatch);
            }
        }
        (Some(withdrawals_root), None) => {
            if withdrawals_root != *EMPTY_WITHDRAWALS_HASH {
                return Err(InvalidBlockBodyError::WithdrawalsRootNotMatch);
            }
        }
        (None, None) => {}
        _ => return Err(InvalidBlockBodyError::WithdrawalsRootNotMatch),
    }

    Ok(())
}

/// Validates that the receipts root in the block header matches the given receipts.
pub fn validate_receipts_root(
    block_header: &BlockHeader,
    receipts: &[Receipt],
    crypto: &dyn Crypto,
) -> Result<(), InvalidBlockError> {
    let receipts_root = compute_receipts_root(receipts, crypto);
    if receipts_root == block_header.receipts_root {
        Ok(())
    } else {
        Err(InvalidBlockError::ReceiptsRootMismatch)
    }
}
