use crate::{
    io::{ProgramInput, ProgramOutput},
    trie::{update_tries, verify_db},
};
use ethrex_blockchain::error::ChainError;
use ethrex_blockchain::{validate_block, validate_gas_used};
#[cfg(feature = "l2")]
use ethrex_common::types::BlobsBundleError;
use ethrex_common::types::{
    blob_from_bytes, kzg_commitment_to_versioned_hash, AccountUpdate, Block, BlockHeader,
    Commitment, PrivilegedL2Transaction, Proof, Receipt, Transaction,
};
use ethrex_common::{Address, H256};
use ethrex_l2_common::withdrawals::{get_block_withdrawals, get_withdrawal_hash};
#[cfg(feature = "l2")]
use ethrex_l2_common::{
    deposits::{compute_deposit_logs_hash, get_block_deposits, DepositError},
    state_diff::{prepare_state_diff, StateDiff, StateDiffError},
    withdrawals::{compute_withdrawals_merkle_root, WithdrawalError},
};
use ethrex_vm::{Evm, EvmEngine, EvmError, ProverDB, ProverDBError};
use kzg_rs::{get_kzg_settings, Blob, Bytes48, KzgProof};
use std::collections::HashMap;

#[derive(Debug, thiserror::Error)]
pub enum StatelessExecutionError {
    #[error("ProverDB error: {0}")]
    ProverDBError(ProverDBError),
    #[error("Trie error: {0}")]
    TrieError(crate::trie::Error),
    #[error("Block validation error: {0}")]
    BlockValidationError(ChainError),
    #[error("Gas validation error: {0}")]
    GasValidationError(ChainError),
    #[error("EVM error: {0}")]
    EvmError(EvmError),
    #[cfg(feature = "l2")]
    #[error("Withdrawal calculation error: {0}")]
    WithdrawalError(#[from] WithdrawalError),
    #[cfg(feature = "l2")]
    #[error("Deposit calculation error: {0}")]
    DepositError(#[from] DepositError),
    #[cfg(feature = "l2")]
    #[error("State diff error: {0}")]
    StateDiffError(#[from] StateDiffError),
    #[cfg(feature = "l2")]
    #[error("Blobs bundle error: {0}")]
    BlobsBundleError(#[from] BlobsBundleError),
    #[cfg(feature = "l2")]
    #[error("KZG error (proof couldn't be verified): {0}")]
    KzgError(kzg_rs::KzgError),
    #[cfg(feature = "l2")]
    #[error("Invalid KZG blob proof")]
    InvalidBlobProof,
    #[cfg(feature = "l2")]
    #[error("Invalid state diff")]
    InvalidStateDiff,
    #[error("Batch has no blocks")]
    EmptyBatchError,
    #[error("Invalid database")]
    InvalidDatabase,
    #[error("Invalid initial state trie")]
    InvalidInitialStateTrie,
    #[error("Invalid final state trie")]
    InvalidFinalStateTrie,
    #[error("Failed to calculate deposit hash")]
    InvalidDeposit,
    #[error("Internal error: {0}")]
    Internal(String),
}

impl From<kzg_rs::KzgError> for StatelessExecutionError {
    fn from(value: kzg_rs::KzgError) -> Self {
        StatelessExecutionError::KzgError(value)
    }
}

pub fn execution_program(input: ProgramInput) -> Result<ProgramOutput, StatelessExecutionError> {
    let ProgramInput {
        blocks,
        parent_block_header,
        mut db,
        elasticity_multiplier,
        #[cfg(feature = "l2")]
        blob_commitment,
        #[cfg(feature = "l2")]
        blob_proof,
    } = input;
    if cfg!(feature = "l2") {
        #[cfg(feature = "l2")]
        return stateless_validation_l2(
            &blocks,
            &parent_block_header,
            &mut db,
            elasticity_multiplier,
            blob_commitment,
            blob_proof,
        );
    }
    stateless_validation_l1(
        &blocks,
        &parent_block_header,
        &mut db,
        elasticity_multiplier,
    )
}

pub fn stateless_validation_l1(
    blocks: &[Block],
    parent_block_header: &BlockHeader,
    db: &mut ProverDB,
    elasticity_multiplier: u64,
) -> Result<ProgramOutput, StatelessExecutionError> {
    let StatelessResult {
        initial_state_hash,
        final_state_hash,
        ..
    } = execute_stateless(blocks, parent_block_header, db, elasticity_multiplier)?;
    Ok(ProgramOutput {
        initial_state_hash,
        final_state_hash,
        #[cfg(feature = "l2")]
        withdrawals_merkle_root: H256::zero(),
        #[cfg(feature = "l2")]
        deposit_logs_hash: H256::zero(),
        #[cfg(feature = "l2")]
        blob_versioned_hash: H256::zero(),
    })
}

#[cfg(feature = "l2")]
pub fn stateless_validation_l2(
    blocks: &[Block],
    parent_block_header: &BlockHeader,
    db: &mut ProverDB,
    elasticity_multiplier: u64,
    blob_commitment: Commitment,
    blob_proof: Proof,
) -> Result<ProgramOutput, StatelessExecutionError> {
    let StatelessResult {
        receipts,
        initial_state_hash,
        final_state_hash,
        account_updates,
        last_block_header,
    } = execute_stateless(blocks, parent_block_header, db, elasticity_multiplier)?;

    let (withdrawals, deposits) = get_batch_withdrawals_and_deposits(blocks, &receipts)?;
    let (withdrawals_merkle_root, deposit_logs_hash) =
        compute_withdrawals_and_deposits_digests(&withdrawals, &deposits)?;

    // TODO: this could be replaced with something like a ProverConfig in the future.
    let validium = (blob_commitment, blob_proof) == ([0; 48], [0; 48]);

    // Check state diffs are valid
    let blob_versioned_hash = if !validium {
        let state_diff = prepare_state_diff(
            last_block_header,
            db,
            &withdrawals,
            &deposits,
            account_updates.values().cloned().collect(),
        )?;
        verify_blob(state_diff, blob_commitment, blob_proof)?
    } else {
        H256::zero()
    };

    Ok(ProgramOutput {
        initial_state_hash,
        final_state_hash,
        withdrawals_merkle_root,
        deposit_logs_hash,
        blob_versioned_hash,
    })
}

struct StatelessResult {
    receipts: Vec<Vec<ethrex_common::types::Receipt>>,
    initial_state_hash: H256,
    final_state_hash: H256,
    account_updates: HashMap<Address, AccountUpdate>,
    last_block_header: BlockHeader,
}

fn execute_stateless(
    blocks: &[Block],
    parent_block_header: &BlockHeader,
    db: &mut ProverDB,
    elasticity_multiplier: u64,
) -> Result<StatelessResult, StatelessExecutionError> {
    // Tries used for validating initial state root
    let (mut state_trie, mut storage_tries) = db
        .get_tries()
        .map_err(StatelessExecutionError::ProverDBError)?;

    // Validate the initial state
    let initial_state_hash = state_trie.hash_no_commit();
    if initial_state_hash != parent_block_header.state_root {
        return Err(StatelessExecutionError::InvalidInitialStateTrie);
    }
    if !verify_db(db, &state_trie, &storage_tries).map_err(StatelessExecutionError::TrieError)? {
        return Err(StatelessExecutionError::InvalidDatabase);
    };

    let mut parent_header = parent_block_header;
    let mut acc_account_updates: HashMap<Address, AccountUpdate> = HashMap::new();
    let mut acc_receipts = Vec::new();

    for block in blocks {
        // Validate the block
        validate_block(
            block,
            parent_header,
            &db.chain_config,
            elasticity_multiplier,
        )
        .map_err(StatelessExecutionError::BlockValidationError)?;

        // Execute block
        let mut vm = Evm::new(EvmEngine::LEVM, db.clone());
        let result = vm
            .execute_block(block)
            .map_err(StatelessExecutionError::EvmError)?;
        let receipts = result.receipts;
        let account_updates = vm
            .get_state_transitions()
            .map_err(StatelessExecutionError::EvmError)?;

        // Update db for the next block
        db.apply_account_updates(&account_updates);

        // Update acc_account_updates
        for account in account_updates {
            let address = account.address;
            if let Some(existing) = acc_account_updates.get_mut(&address) {
                existing.merge(account);
            } else {
                acc_account_updates.insert(address, account);
            }
        }

        validate_gas_used(&receipts, &block.header)
            .map_err(StatelessExecutionError::GasValidationError)?;
        parent_header = &block.header;
        acc_receipts.push(receipts);
    }

    // Update state trie
    let update_list: Vec<AccountUpdate> = acc_account_updates.values().cloned().collect();
    update_tries(&mut state_trie, &mut storage_tries, &update_list)
        .map_err(StatelessExecutionError::TrieError)?;

    // Calculate final state root hash and check
    let last_block = blocks
        .last()
        .ok_or(StatelessExecutionError::EmptyBatchError)?;
    let last_block_state_root = last_block.header.state_root;
    let final_state_hash = state_trie.hash_no_commit();
    if final_state_hash != last_block_state_root {
        return Err(StatelessExecutionError::InvalidFinalStateTrie);
    }
    Ok(StatelessResult {
        receipts: acc_receipts,
        initial_state_hash,
        final_state_hash,
        account_updates: acc_account_updates,
        last_block_header: parent_header.clone(),
    })
}

fn get_batch_withdrawals_and_deposits(
    blocks: &[Block],
    receipts: &[Vec<Receipt>],
) -> Result<(Vec<Transaction>, Vec<PrivilegedL2Transaction>), StatelessExecutionError> {
    let mut withdrawals = vec![];
    let mut deposits = vec![];

    for (block, receipts) in blocks.iter().zip(receipts) {
        let txs = &block.body.transactions;
        deposits.extend(get_block_deposits(txs));
        withdrawals.extend(get_block_withdrawals(txs, receipts));
    }

    Ok((withdrawals, deposits))
}

fn compute_withdrawals_and_deposits_digests(
    withdrawals: &[Transaction],
    deposits: &[PrivilegedL2Transaction],
) -> Result<(H256, H256), StatelessExecutionError> {
    let withdrawal_hashes: Vec<_> = withdrawals
        .iter()
        .map(get_withdrawal_hash)
        .collect::<Result<_, _>>()?;
    let deposit_hashes: Vec<_> = deposits
        .iter()
        .map(PrivilegedL2Transaction::get_deposit_hash)
        .map(|hash| hash.ok_or(StatelessExecutionError::InvalidDeposit))
        .collect::<Result<_, _>>()?;

    let withdrawals_merkle_root = compute_withdrawals_merkle_root(&withdrawal_hashes)?;
    let deposit_logs_hash =
        compute_deposit_logs_hash(deposit_hashes).map_err(StatelessExecutionError::DepositError)?;

    Ok((withdrawals_merkle_root, deposit_logs_hash))
}

fn verify_blob(
    state_diff: StateDiff,
    blob_commitment: Commitment,
    blob_proof: Proof,
) -> Result<H256, StatelessExecutionError> {
    let encoded_state_diff = state_diff.encode()?;
    let blob_data = blob_from_bytes(encoded_state_diff)?;
    let blob = Blob::from_slice(&blob_data)?;

    let is_blob_proof_valid = KzgProof::verify_blob_kzg_proof(
        blob,
        &Bytes48::from_slice(&blob_commitment)?,
        &Bytes48::from_slice(&blob_proof)?,
        &get_kzg_settings(),
    )?;

    if !is_blob_proof_valid {
        return Err(StatelessExecutionError::InvalidBlobProof);
    }

    Ok(kzg_commitment_to_versioned_hash(&blob_commitment))
}
