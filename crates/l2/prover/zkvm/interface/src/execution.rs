use crate::io::{ProgramInput, ProgramOutput};
use ethrex_blockchain::error::ChainError;
use ethrex_blockchain::{
    validate_block, validate_gas_used, validate_receipts_root, validate_requests_hash,
};
use ethrex_common::Address;
use ethrex_common::types::AccountUpdate;
use ethrex_common::types::{
    block_execution_witness::ExecutionWitnessError, block_execution_witness::ExecutionWitnessResult,
};
use ethrex_common::{
    H256,
    types::{Block, BlockHeader},
};
#[cfg(feature = "l2")]
use ethrex_l2_common::l1_messages::L1Message;
use ethrex_vm::{Evm, EvmEngine, EvmError, ProverDBError};
use std::collections::HashMap;

#[cfg(feature = "l2")]
use ethrex_common::types::{
    BlobsBundleError, Commitment, PrivilegedL2Transaction, Proof, Receipt, blob_from_bytes,
    kzg_commitment_to_versioned_hash,
};
#[cfg(feature = "l2")]
use ethrex_l2_common::{
    deposits::{DepositError, compute_deposit_logs_hash, get_block_deposits},
    l1_messages::{L1MessagingError, compute_merkle_root, get_block_l1_messages},
    state_diff::{StateDiff, StateDiffError, prepare_state_diff},
};
#[cfg(feature = "l2")]
use kzg_rs::{Blob, Bytes48, KzgProof, get_kzg_settings};

#[derive(Debug, thiserror::Error)]
pub enum StatelessExecutionError {
    #[error("ProverDB error: {0}")]
    ProverDBError(#[from] ProverDBError),
    #[error("Block validation error: {0}")]
    BlockValidationError(ChainError),
    #[error("Gas validation error: {0}")]
    GasValidationError(ChainError),
    #[error("L1Message validation error: {0}")]
    RequestsRootValidationError(ChainError),
    #[error("Receipts validation error: {0}")]
    ReceiptsRootValidationError(ChainError),
    #[error("EVM error: {0}")]
    EvmError(EvmError),
    #[cfg(feature = "l2")]
    #[error("L1Message calculation error: {0}")]
    L1MessageError(#[from] L1MessagingError),
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
    #[error("Execution witness error: {0}")]
    ExecutionWitness(#[from] ExecutionWitnessError),
    #[error("Invalid initial state trie")]
    InvalidInitialStateTrie,
    #[error("Invalid final state trie")]
    InvalidFinalStateTrie,
    #[error("Missing deposit hash")]
    MissingDepositHash,
    #[error("Failed to apply account updates {0}")]
    ApplyAccountUpdates(String),
    #[error("No block headers required, should at least require parent header")]
    NoHeadersRequired,
    #[error("Unreachable code reached: {0}")]
    Unreachable(String),
    #[error("Invalid hash of block {0} (it's not the parent hash of its successor)")]
    InvalidBlockHash(u64),
    #[error("Invalid parent block header")]
    InvalidParentBlockHeader,
    #[error("Failed to calculate deposit hash")]
    InvalidDeposit,
    #[error("Internal error: {0}")]
    Internal(String),
}

#[cfg(feature = "l2")]
impl From<kzg_rs::KzgError> for StatelessExecutionError {
    fn from(value: kzg_rs::KzgError) -> Self {
        StatelessExecutionError::KzgError(value)
    }
}

pub fn execution_program(input: ProgramInput) -> Result<ProgramOutput, StatelessExecutionError> {
    let ProgramInput {
        blocks,
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
            &mut db,
            elasticity_multiplier,
            blob_commitment,
            blob_proof,
        );
    }
    stateless_validation_l1(&blocks, &mut db, elasticity_multiplier)
}

pub fn stateless_validation_l1(
    blocks: &[Block],
    db: &mut ExecutionWitnessResult,

    elasticity_multiplier: u64,
) -> Result<ProgramOutput, StatelessExecutionError> {
    let StatelessResult {
        initial_state_hash,
        final_state_hash,
        last_block_hash,
        ..
    } = execute_stateless(blocks, db, elasticity_multiplier)?;
    Ok(ProgramOutput {
        initial_state_hash,
        final_state_hash,
        #[cfg(feature = "l2")]
        l1messages_merkle_root: H256::zero(),
        #[cfg(feature = "l2")]
        deposit_logs_hash: H256::zero(),
        #[cfg(feature = "l2")]
        blob_versioned_hash: H256::zero(),
        last_block_hash,
    })
}

#[cfg(feature = "l2")]
pub fn stateless_validation_l2(
    blocks: &[Block],
    db: &mut ExecutionWitnessResult,
    elasticity_multiplier: u64,
    blob_commitment: Commitment,
    blob_proof: Proof,
) -> Result<ProgramOutput, StatelessExecutionError> {
    let mut initial_db = db.clone();

    let StatelessResult {
        receipts,
        initial_state_hash,
        final_state_hash,
        account_updates,
        last_block_header,
        last_block_hash,
    } = execute_stateless(blocks, db, elasticity_multiplier)?;

    let (l1messages, deposits) = get_batch_l1messages_and_deposits(blocks, &receipts)?;
    let (l1messages_merkle_root, deposit_logs_hash) =
        compute_l1messages_and_deposits_digests(&l1messages, &deposits)?;

    // TODO: this could be replaced with something like a ProverConfig in the future.
    let validium = (blob_commitment, blob_proof) == ([0; 48], [0; 48]);

    // Check state diffs are valid
    let blob_versioned_hash = if !validium {
        initial_db
            .rebuild_tries()
            .map_err(|_| StatelessExecutionError::InvalidInitialStateTrie)?;
        let state_diff = prepare_state_diff(
            last_block_header,
            &initial_db,
            &l1messages,
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
        l1messages_merkle_root,
        deposit_logs_hash,
        blob_versioned_hash,
        last_block_hash,
    })
}

struct StatelessResult {
    receipts: Vec<Vec<ethrex_common::types::Receipt>>,
    initial_state_hash: H256,
    final_state_hash: H256,
    account_updates: HashMap<Address, AccountUpdate>,
    last_block_header: BlockHeader,
    last_block_hash: H256,
}

fn execute_stateless(
    blocks: &[Block],
    db: &mut ExecutionWitnessResult,
    elasticity_multiplier: u64,
) -> Result<StatelessResult, StatelessExecutionError> {
    db.rebuild_tries()
        .map_err(StatelessExecutionError::ExecutionWitness)?;

    // Validate block hashes, except parent block hash (latest block hash)
    if let Ok(Some(invalid_block_header)) = db.get_first_invalid_block_hash() {
        return Err(StatelessExecutionError::InvalidBlockHash(
            invalid_block_header,
        ));
    }

    // Validate parent block header
    let parent_block_header = db
        .get_block_parent_header(
            blocks
                .first()
                .ok_or(StatelessExecutionError::EmptyBatchError)?
                .header
                .number,
        )
        .map_err(StatelessExecutionError::ExecutionWitness)?;
    let first_block_header = &blocks
        .first()
        .ok_or(StatelessExecutionError::EmptyBatchError)?
        .header;
    if parent_block_header.hash() != first_block_header.parent_hash {
        return Err(StatelessExecutionError::InvalidParentBlockHeader);
    }

    // Validate the initial state
    let initial_state_hash = db
        .state_trie_root()
        .map_err(StatelessExecutionError::ExecutionWitness)?;

    if initial_state_hash != parent_block_header.state_root {
        return Err(StatelessExecutionError::InvalidInitialStateTrie);
    }

    // Execute blocks
    let mut parent_block_header = parent_block_header;
    let mut acc_account_updates: HashMap<Address, AccountUpdate> = HashMap::new();
    let mut acc_receipts = Vec::new();
    for block in blocks {
        // Validate the block
        validate_block(
            block,
            parent_block_header,
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
        db.apply_account_updates(&account_updates)
            .map_err(StatelessExecutionError::ExecutionWitness)?;

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
        validate_receipts_root(&block.header, &receipts)
            .map_err(StatelessExecutionError::ReceiptsRootValidationError)?;
        // validate_requests_hash doesn't do anything for l2 blocks as this verifies l1 requests (messages, deposits and consolidations)
        validate_requests_hash(&block.header, &db.chain_config, &result.requests)
            .map_err(StatelessExecutionError::RequestsRootValidationError)?;
        parent_block_header = &block.header;
        acc_receipts.push(receipts);
    }

    // Calculate final state root hash and check
    let last_block = blocks
        .last()
        .ok_or(StatelessExecutionError::EmptyBatchError)?;
    let last_block_state_root = last_block.header.state_root;

    let last_block_hash = last_block.header.hash();
    let final_state_hash = db
        .state_trie_root()
        .map_err(StatelessExecutionError::ExecutionWitness)?;
    if final_state_hash != last_block_state_root {
        return Err(StatelessExecutionError::InvalidFinalStateTrie);
    }

    Ok(StatelessResult {
        receipts: acc_receipts,
        initial_state_hash,
        final_state_hash,
        account_updates: acc_account_updates,
        last_block_header: last_block.header.clone(),
        last_block_hash,
    })
}

#[cfg(feature = "l2")]
fn get_batch_l1messages_and_deposits(
    blocks: &[Block],
    receipts: &[Vec<Receipt>],
) -> Result<(Vec<L1Message>, Vec<PrivilegedL2Transaction>), StatelessExecutionError> {
    let mut l1messages = vec![];
    let mut deposits = vec![];

    for (block, receipts) in blocks.iter().zip(receipts) {
        let txs = &block.body.transactions;
        deposits.extend(get_block_deposits(txs));
        l1messages.extend(get_block_l1_messages(txs, receipts));
    }

    Ok((l1messages, deposits))
}

#[cfg(feature = "l2")]
fn compute_l1messages_and_deposits_digests(
    l1messages: &[L1Message],
    deposits: &[PrivilegedL2Transaction],
) -> Result<(H256, H256), StatelessExecutionError> {
    use ethrex_l2_common::l1_messages::get_l1_message_hash;

    let message_hashes: Vec<_> = l1messages.iter().map(get_l1_message_hash).collect();
    let deposit_hashes: Vec<_> = deposits
        .iter()
        .map(PrivilegedL2Transaction::get_deposit_hash)
        .map(|hash| hash.ok_or(StatelessExecutionError::InvalidDeposit))
        .collect::<Result<_, _>>()?;

    let l1message_merkle_root = compute_merkle_root(&message_hashes)?;
    let deposit_logs_hash =
        compute_deposit_logs_hash(deposit_hashes).map_err(StatelessExecutionError::DepositError)?;

    Ok((l1message_merkle_root, deposit_logs_hash))
}

#[cfg(feature = "l2")]
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
