use crate::input::ProgramInput;
use crate::output::ProgramOutput;
use ethrex_blockchain::error::ChainError;
use ethrex_blockchain::{
    validate_block, validate_gas_used, validate_receipts_root, validate_requests_hash,
};
use ethrex_common::types::AccountUpdate;
use ethrex_common::types::{
    block_execution_witness::ExecutionWitnessError, block_execution_witness::ExecutionWitnessResult,
};
use ethrex_common::{Address, U256};
use ethrex_common::{
    H256,
    types::{Block, BlockHeader},
};
#[cfg(feature = "l2")]
use ethrex_l2_common::l1_messages::L1Message;
use ethrex_vm::{Evm, EvmEngine, EvmError, ExecutionWitnessWrapper, ProverDBError, VmDatabase};
use keccak_hash::keccak;
use std::collections::HashMap;

#[cfg(feature = "l2")]
use ethrex_common::{
    kzg::KzgError,
    types::{
        BlobsBundleError, Commitment, PrivilegedL2Transaction, Proof, Receipt, blob_from_bytes,
        kzg_commitment_to_versioned_hash,
    },
};
use ethrex_l2_common::{
    l1_messages::get_block_l1_messages,
    privileged_transactions::{
        PrivilegedTransactionError, compute_privileged_transactions_hash,
        get_block_privileged_transactions,
    },
    state_diff::{StateDiff, StateDiffError, prepare_state_diff},
};

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
    EvmError(#[from] EvmError),
    #[cfg(feature = "l2")]
    #[error("Privileged Transaction calculation error: {0}")]
    PrivilegedTransactionError(#[from] PrivilegedTransactionError),
    #[cfg(feature = "l2")]
    #[error("State diff error: {0}")]
    StateDiffError(#[from] StateDiffError),
    #[cfg(feature = "l2")]
    #[error("Blobs bundle error: {0}")]
    BlobsBundleError(#[from] BlobsBundleError),
    #[cfg(feature = "l2")]
    #[error("KZG error (proof couldn't be verified): {0}")]
    KzgError(#[from] KzgError),
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
    #[error("Missing privileged transaction hash")]
    MissingPrivilegedTransactionHash,
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
    #[error("Failed to calculate privileged transaction hash")]
    InvalidPrivilegedTransaction,
    #[error("Internal error: {0}")]
    Internal(String),
}

pub fn execution_program(input: ProgramInput) -> Result<ProgramOutput, StatelessExecutionError> {
    let ProgramInput {
        blocks,
        db,
        elasticity_multiplier,
        #[cfg(feature = "l2")]
        blob_commitment,
        #[cfg(feature = "l2")]
        blob_proof,
    } = input;

    let chain_id = db.chain_config.chain_id;

    if cfg!(feature = "l2") {
        #[cfg(feature = "l2")]
        return stateless_validation_l2(
            &blocks,
            db,
            elasticity_multiplier,
            blob_commitment,
            blob_proof,
            chain_id,
        );
    }

    stateless_validation_l1(&blocks, db, elasticity_multiplier, chain_id)
}

pub fn stateless_validation_l1(
    blocks: &[Block],
    db: ExecutionWitnessResult,
    elasticity_multiplier: u64,
    chain_id: u64,
) -> Result<ProgramOutput, StatelessExecutionError> {
    let StatelessResult {
        initial_state_hash,
        final_state_hash,
        last_block_hash,
        non_privileged_count,
        ..
    } = execute_stateless(blocks, db, elasticity_multiplier)?;
    Ok(ProgramOutput {
        initial_state_hash,
        final_state_hash,
        #[cfg(feature = "l2")]
        l1messages_merkle_root: H256::zero(),
        #[cfg(feature = "l2")]
        privileged_transactions_hash: H256::zero(),
        #[cfg(feature = "l2")]
        blob_versioned_hash: H256::zero(),
        last_block_hash,
        chain_id: chain_id.into(),
        non_privileged_count,
    })
}

#[cfg(feature = "l2")]
pub fn stateless_validation_l2(
    blocks: &[Block],
    db: ExecutionWitnessResult,
    elasticity_multiplier: u64,
    blob_commitment: Commitment,
    blob_proof: Proof,
    chain_id: u64,
) -> Result<ProgramOutput, StatelessExecutionError> {
    use std::collections::BTreeMap;

    let mut initial_db = ExecutionWitnessResult {
        block_headers: db.block_headers.clone(),
        chain_config: db.chain_config,
        codes_hashed: BTreeMap::new(), // This must be filled during stateless execution
        codes: db.codes.clone(),
        state_trie: None,
        storage_tries: BTreeMap::new(),
        parent_block_header: db.parent_block_header.clone(),
        state_nodes: BTreeMap::new(), // empty map because this must be filled during stateless execution
        nodes: db.nodes.clone(),
        touched_account_storage_slots: BTreeMap::new(),
        account_hashes_by_address: BTreeMap::new(), // This must be filled during stateless execution
    };

    let StatelessResult {
        receipts,
        initial_state_hash,
        final_state_hash,
        account_updates,
        last_block_header,
        last_block_hash,
        non_privileged_count,
    } = execute_stateless(blocks, db, elasticity_multiplier)?;

    let (l1messages, privileged_transactions) =
        get_batch_l1messages_and_privileged_transactions(blocks, &receipts)?;
    let (l1messages_merkle_root, privileged_transactions_hash) =
        compute_l1messages_and_privileged_transactions_digests(
            &l1messages,
            &privileged_transactions,
        )?;

    // TODO: this could be replaced with something like a ProverConfig in the future.
    let validium = (blob_commitment, blob_proof) == ([0; 48], [0; 48]);

    // Check state diffs are valid
    let blob_versioned_hash = if !validium {
        initial_db
            .rebuild_state_trie()
            .map_err(|_| StatelessExecutionError::InvalidInitialStateTrie)?;
        let wrapped_db = ExecutionWitnessWrapper::new(initial_db);
        let state_diff = prepare_state_diff(
            last_block_header,
            &wrapped_db,
            &l1messages,
            &privileged_transactions,
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
        privileged_transactions_hash,
        blob_versioned_hash,
        last_block_hash,
        chain_id: chain_id.into(),
        non_privileged_count,
    })
}

struct StatelessResult {
    receipts: Vec<Vec<ethrex_common::types::Receipt>>,
    initial_state_hash: H256,
    final_state_hash: H256,
    account_updates: HashMap<Address, AccountUpdate>,
    last_block_header: BlockHeader,
    last_block_hash: H256,
    non_privileged_count: U256,
}

fn execute_stateless(
    blocks: &[Block],
    mut db: ExecutionWitnessResult,
    elasticity_multiplier: u64,
) -> Result<StatelessResult, StatelessExecutionError> {
    // hash nodes
    db.state_nodes = db
        .nodes
        .drain(..)
        .map(|node| (keccak(&node), node))
        .collect();

    // hash codes
    db.codes_hashed = db
        .codes
        .drain(..)
        .map(|code| (keccak(&code), code))
        .collect();

    db.rebuild_state_trie()
        .map_err(|_| StatelessExecutionError::InvalidInitialStateTrie)?;

    let mut wrapped_db = ExecutionWitnessWrapper::new(db);
    let chain_config = wrapped_db.get_chain_config().map_err(|_| {
        StatelessExecutionError::Internal("No chain config in execution witness".to_string())
    })?;

    // Hashing is an expensive operation in zkVMs, this way we avoid hashing twice
    // (once in get_first_invalid_block_hash(), later in validate_block()).
    wrapped_db.initialize_block_header_hashes(blocks)?;

    // Validate execution witness' block hashes, except parent block hash (latest block hash).
    if let Ok(Some(invalid_block_header)) = wrapped_db.get_first_invalid_block_hash() {
        return Err(StatelessExecutionError::InvalidBlockHash(
            invalid_block_header,
        ));
    }

    // Validate the initial state
    let parent_block_header = &wrapped_db
        .get_block_parent_header(
            blocks
                .first()
                .ok_or(StatelessExecutionError::EmptyBatchError)?
                .header
                .number,
        )
        .map_err(StatelessExecutionError::ExecutionWitness)?;
    let initial_state_hash = wrapped_db
        .state_trie_root()
        .map_err(StatelessExecutionError::ExecutionWitness)?;
    if initial_state_hash != parent_block_header.state_root {
        return Err(StatelessExecutionError::InvalidInitialStateTrie);
    }

    // Execute blocks
    let mut parent_block_header = parent_block_header;
    let mut acc_account_updates: HashMap<Address, AccountUpdate> = HashMap::new();
    let mut acc_receipts = Vec::new();
    let mut non_privileged_count = 0;
    for block in blocks {
        // Validate the block
        validate_block(
            block,
            parent_block_header,
            &chain_config,
            elasticity_multiplier,
        )
        .map_err(StatelessExecutionError::BlockValidationError)?;

        // Execute block
        #[cfg(feature = "l2")]
        let mut vm = Evm::new_for_l2(EvmEngine::LEVM, wrapped_db.clone())?;
        #[cfg(not(feature = "l2"))]
        let mut vm = Evm::new_for_l1(EvmEngine::LEVM, wrapped_db.clone());
        let result = vm
            .execute_block(block)
            .map_err(StatelessExecutionError::EvmError)?;
        let receipts = result.receipts;
        let account_updates = vm
            .get_state_transitions()
            .map_err(StatelessExecutionError::EvmError)?;

        // Update db for the next block
        wrapped_db
            .apply_account_updates(&account_updates)
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

        non_privileged_count += block.body.transactions.len()
            - get_block_privileged_transactions(&block.body.transactions).len();

        validate_gas_used(&receipts, &block.header)
            .map_err(StatelessExecutionError::GasValidationError)?;
        validate_receipts_root(&block.header, &receipts)
            .map_err(StatelessExecutionError::ReceiptsRootValidationError)?;
        // validate_requests_hash doesn't do anything for l2 blocks as this verifies l1 requests (messages, privileged transactions and consolidations)
        validate_requests_hash(&block.header, &chain_config, &result.requests)
            .map_err(StatelessExecutionError::RequestsRootValidationError)?;
        acc_receipts.push(receipts);

        parent_block_header = &block.header;
    }

    // Calculate final state root hash and check
    let last_block = blocks
        .last()
        .ok_or(StatelessExecutionError::EmptyBatchError)?;
    let last_block_state_root = last_block.header.state_root;

    let last_block_hash = last_block.header.hash();
    let final_state_hash = wrapped_db
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
        non_privileged_count: non_privileged_count.into(),
    })
}

#[cfg(feature = "l2")]
fn get_batch_l1messages_and_privileged_transactions(
    blocks: &[Block],
    receipts: &[Vec<Receipt>],
) -> Result<(Vec<L1Message>, Vec<PrivilegedL2Transaction>), StatelessExecutionError> {
    let mut l1messages = vec![];
    let mut privileged_transactions = vec![];

    for (block, receipts) in blocks.iter().zip(receipts) {
        let txs = &block.body.transactions;
        privileged_transactions.extend(get_block_privileged_transactions(txs));
        l1messages.extend(get_block_l1_messages(receipts));
    }

    Ok((l1messages, privileged_transactions))
}

#[cfg(feature = "l2")]
fn compute_l1messages_and_privileged_transactions_digests(
    l1messages: &[L1Message],
    privileged_transactions: &[PrivilegedL2Transaction],
) -> Result<(H256, H256), StatelessExecutionError> {
    use ethrex_l2_common::{l1_messages::get_l1_message_hash, merkle_tree::compute_merkle_root};

    let message_hashes: Vec<_> = l1messages.iter().map(get_l1_message_hash).collect();
    let privileged_transactions_hashes: Vec<_> = privileged_transactions
        .iter()
        .map(PrivilegedL2Transaction::get_privileged_hash)
        .map(|hash| hash.ok_or(StatelessExecutionError::InvalidPrivilegedTransaction))
        .collect::<Result<_, _>>()?;

    let l1message_merkle_root = compute_merkle_root(&message_hashes);
    let privileged_transactions_hash =
        compute_privileged_transactions_hash(privileged_transactions_hashes)
            .map_err(StatelessExecutionError::PrivilegedTransactionError)?;

    Ok((l1message_merkle_root, privileged_transactions_hash))
}

#[cfg(feature = "l2")]
fn verify_blob(
    state_diff: StateDiff,
    commitment: Commitment,
    proof: Proof,
) -> Result<H256, StatelessExecutionError> {
    use ethrex_common::kzg::verify_blob_kzg_proof;

    let encoded_state_diff = state_diff.encode()?;
    let blob_data = blob_from_bytes(encoded_state_diff)?;

    if !verify_blob_kzg_proof(blob_data, commitment, proof)? {
        return Err(StatelessExecutionError::InvalidBlobProof);
    }

    Ok(kzg_commitment_to_versioned_hash(&commitment))
}
