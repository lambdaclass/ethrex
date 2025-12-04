use crate::input::ProgramInput;
use crate::output::ProgramOutput;
use crate::report_cycles;

use ethrex_blockchain::error::ChainError;
use ethrex_blockchain::{
    validate_block, validate_gas_used, validate_receipts_root, validate_requests_hash,
    validate_state_root,
};
use ethrex_common::types::AccountUpdate;
use ethrex_common::types::block_execution_witness::ExecutionWitness;
use ethrex_common::types::fee_config::FeeConfig;
use ethrex_common::types::{
    block_execution_witness::GuestProgramState, block_execution_witness::GuestProgramStateError,
};
use ethrex_common::{Address, U256};
use ethrex_common::{H256, types::Block};
use ethrex_l2_common::privileged_transactions::get_block_l1_privileged_transactions;
use ethrex_rlp::encode::RLPEncode;
use ethrex_vm::{Evm, EvmError, GuestProgramStateWrapper, VmDatabase};
use std::collections::{BTreeMap, HashMap};

#[cfg(not(feature = "l2"))]
use ethrex_common::types::ELASTICITY_MULTIPLIER;
#[cfg(feature = "l2")]
use ethrex_common::types::{
    BlobsBundleError, Commitment, PrivilegedL2Transaction, Proof, Receipt, blob_from_bytes,
    kzg_commitment_to_versioned_hash,
};
#[cfg(feature = "l2")]
use ethrex_l2_common::{
    messages::{L1Message, L2Message, get_block_l1_messages, get_block_l2_messages},
    privileged_transactions::{PrivilegedTransactionError, compute_privileged_transactions_hash},
};

#[derive(Debug, thiserror::Error)]
pub enum StatelessExecutionError {
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
    #[error("Blobs bundle error: {0}")]
    BlobsBundleError(#[from] BlobsBundleError),
    #[cfg(feature = "l2")]
    #[error("KZG error (proof couldn't be verified): {0}")]
    KzgError(#[from] ethrex_crypto::kzg::KzgError),
    #[cfg(feature = "l2")]
    #[error("Invalid KZG blob proof")]
    InvalidBlobProof,
    #[cfg(feature = "l2")]
    #[error("FeeConfig not provided for L2 execution")]
    FeeConfigNotFound,
    #[error("Batch has no blocks")]
    EmptyBatchError,
    #[error("Invalid database")]
    InvalidDatabase,
    #[error("Execution witness error: {0}")]
    GuestProgramState(#[from] GuestProgramStateError),
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
    #[error("Failed to convert integer")]
    TryIntoError(#[from] std::num::TryFromIntError),
}

pub fn execution_program(input: ProgramInput) -> Result<ProgramOutput, StatelessExecutionError> {
    let ProgramInput {
        blocks,
        execution_witness,
        elasticity_multiplier,
        fee_configs: _fee_configs,
        #[cfg(feature = "l2")]
        blob_commitment,
        #[cfg(feature = "l2")]
        blob_proof,
    } = input;

    let chain_id = execution_witness.chain_config.chain_id;

    if cfg!(feature = "l2") {
        #[cfg(feature = "l2")]
        return stateless_validation_l2(
            &blocks,
            execution_witness,
            elasticity_multiplier,
            _fee_configs,
            blob_commitment,
            blob_proof,
            chain_id,
        );
    }

    stateless_validation_l1(blocks, execution_witness, elasticity_multiplier, chain_id)
}

pub fn stateless_validation_l1(
    blocks: Vec<Block>,
    execution_witness: ExecutionWitness,
    elasticity_multiplier: u64,
    chain_id: u64,
) -> Result<ProgramOutput, StatelessExecutionError> {
    let guest_program_state: GuestProgramState =
        report_cycles("guest_program_state_initialization", || {
            execution_witness
                .try_into()
                .map_err(StatelessExecutionError::GuestProgramState)
        })?;

    let mut wrapped_db = GuestProgramStateWrapper::new(guest_program_state);

    let chain_config = wrapped_db.get_chain_config().map_err(|_| {
        StatelessExecutionError::Internal("No chain config in execution witness".to_string())
    })?;

    // Hashing is an expensive operation in zkVMs, this way we avoid hashing twice
    // (once in get_first_invalid_block_hash(), later in validate_block()).
    report_cycles("initialize_block_header_hashes", || {
        wrapped_db.initialize_block_header_hashes(&blocks)
    })?;

    // Validate execution witness' block hashes, except parent block hash (latest block hash).
    report_cycles("get_first_invalid_block_hash", || {
        if let Ok(Some(invalid_block_header)) = wrapped_db.get_first_invalid_block_hash() {
            return Err(StatelessExecutionError::InvalidBlockHash(
                invalid_block_header,
            ));
        }
        Ok(())
    })?;

    // Validate the initial state
    let parent_block_header = wrapped_db
        .get_block_parent_header(
            blocks
                .first()
                .ok_or(StatelessExecutionError::EmptyBatchError)?
                .header
                .number,
        )
        .map_err(StatelessExecutionError::GuestProgramState)?;

    let initial_state_hash = report_cycles("state_trie_root", || {
        wrapped_db
            .state_trie_root()
            .map_err(StatelessExecutionError::GuestProgramState)
    })?;

    if initial_state_hash != parent_block_header.state_root {
        return Err(StatelessExecutionError::InvalidInitialStateTrie);
    }

    // Execute blocks
    let mut parent_block_header = &parent_block_header;
    let mut acc_account_updates: BTreeMap<Address, AccountUpdate> = BTreeMap::new();
    let mut non_privileged_count = 0;

    for block in blocks.iter() {
        // Validate the block
        report_cycles("validate_block", || {
            validate_block(
                block,
                parent_block_header,
                &chain_config,
                elasticity_multiplier,
            )
            .map_err(StatelessExecutionError::BlockValidationError)
        })?;

        let mut vm = report_cycles("setup_evm", || {
            let vm = Evm::new_for_l1(wrapped_db.clone());
            Ok::<_, StatelessExecutionError>(vm)
        })?;

        let result = report_cycles("execute_block", || {
            vm.execute_block(block)
                .map_err(StatelessExecutionError::EvmError)
        })?;

        let account_updates = report_cycles("get_state_transitions", || {
            vm.get_state_transitions()
                .map_err(StatelessExecutionError::EvmError)
        })?;

        // Update db for the next block
        report_cycles("apply_account_updates", || {
            wrapped_db
                .apply_account_updates(&account_updates)
                .map_err(StatelessExecutionError::GuestProgramState)
        })?;
        // Update acc_account_updates
        for account in account_updates {
            let address = account.address;
            if let Some(existing) = acc_account_updates.get_mut(&address) {
                existing.merge(account);
            } else {
                acc_account_updates.insert(address, account);
            }
        }

        report_cycles("validate_gas_and_receipts", || {
            validate_gas_used(&result.receipts, &block.header)
                .map_err(StatelessExecutionError::GasValidationError)
        })?;

        report_cycles("validate_receipts_root", || {
            validate_receipts_root(&block.header, &result.receipts)
                .map_err(StatelessExecutionError::ReceiptsRootValidationError)
        })?;

        // validate_requests_hash doesn't do anything for l2 blocks as this verifies l1 requests (messages, privileged transactions and consolidations)
        report_cycles("validate_requests_hash", || {
            validate_requests_hash(&block.header, &chain_config, &result.requests)
                .map_err(StatelessExecutionError::RequestsRootValidationError)
        })?;

        non_privileged_count += block.body.transactions.len();
        parent_block_header = &block.header;
    }

    let final_state_root = report_cycles("get_final_state_root", || {
        wrapped_db
            .state_trie_root()
            .map_err(StatelessExecutionError::GuestProgramState)
    })?;

    let last_block = blocks
        .last()
        .ok_or(StatelessExecutionError::EmptyBatchError)?;

    report_cycles("validate_state_root", || {
        validate_state_root(&last_block.header, final_state_root)
            .map_err(|_chain_err| StatelessExecutionError::InvalidFinalStateTrie)
    })?;

    Ok(ProgramOutput {
        initial_state_hash,
        final_state_hash: final_state_root,
        #[cfg(feature = "l2")]
        l1messages_merkle_root: H256::zero(),
        #[cfg(feature = "l2")]
        privileged_transactions_hash: H256::zero(),
        #[cfg(feature = "l2")]
        blob_versioned_hash: H256::zero(),
        last_block_hash: last_block.header.hash(),
        chain_id: chain_id.into(),
        non_privileged_count: non_privileged_count.into(),
        #[cfg(feature = "l2")]
        l2messages_merkle_root: H256::zero(),
        #[cfg(feature = "l2")]
        balance_diffs: vec![],
    })
}

#[cfg(feature = "l2")]
pub fn stateless_validation_l2(
    blocks: &[Block],
    execution_witness: ExecutionWitness,
    elasticity_multiplier: u64,
    fee_configs: Option<Vec<FeeConfig>>,
    blob_commitment: Commitment,
    blob_proof: Proof,
    chain_id: u64,
) -> Result<ProgramOutput, StatelessExecutionError> {
    use ethrex_l2_common::messages::get_balance_diffs;

    let StatelessResult {
        receipts,
        initial_state_hash,
        final_state_hash,
        last_block_hash,
        non_privileged_count,
    } = execute_stateless(
        blocks,
        execution_witness,
        elasticity_multiplier,
        fee_configs.clone(),
    )?;

    let (l1messages, l2messages, privileged_transactions) =
        get_batch_messages_and_privileged_transactions(blocks, &receipts, chain_id)?;

    let (l1messages_merkle_root, l2messages_merkle_root, privileged_transactions_hash) =
        compute_messages_and_privileged_transactions_digests(
            &l1messages,
            &l2messages,
            &privileged_transactions,
        )?;

    let balance_diffs = get_balance_diffs(&l2messages)
        .iter()
        .map(|diff| (diff.chain_id, diff.value))
        .collect();

    // TODO: this could be replaced with something like a ProverConfig in the future.
    let validium = (blob_commitment, &blob_proof) == ([0; 48], &[0; 48]);

    // Check blobs are valid
    let blob_versioned_hash = if !validium {
        let fee_configs = fee_configs.ok_or_else(|| StatelessExecutionError::FeeConfigNotFound)?;
        verify_blob(blocks, &fee_configs, blob_commitment, blob_proof)?
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
        l2messages_merkle_root,
        balance_diffs,
    })
}

#[cfg_attr(not(feature = "l2"), expect(dead_code))]
struct StatelessResult {
    receipts: Vec<Vec<ethrex_common::types::Receipt>>,
    initial_state_hash: H256,
    final_state_hash: H256,
    last_block_hash: H256,
    non_privileged_count: U256,
}

fn execute_stateless(
    blocks: &[Block],
    execution_witness: ExecutionWitness,
    elasticity_multiplier: u64,
    fee_configs: Option<Vec<FeeConfig>>,
) -> Result<StatelessResult, StatelessExecutionError> {
    let guest_program_state: GuestProgramState = execution_witness
        .try_into()
        .map_err(StatelessExecutionError::GuestProgramState)?;

    #[cfg(feature = "l2")]
    let fee_configs = fee_configs.ok_or_else(|| StatelessExecutionError::FeeConfigNotFound)?;

    let mut wrapped_db = GuestProgramStateWrapper::new(guest_program_state);
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
        .map_err(StatelessExecutionError::GuestProgramState)?;
    let initial_state_hash = wrapped_db
        .state_trie_root()
        .map_err(StatelessExecutionError::GuestProgramState)?;
    if initial_state_hash != parent_block_header.state_root {
        return Err(StatelessExecutionError::InvalidInitialStateTrie);
    }

    // Execute blocks
    let mut parent_block_header = parent_block_header;
    let mut acc_account_updates: HashMap<Address, AccountUpdate> = HashMap::new();
    let mut acc_receipts = Vec::new();
    let mut non_privileged_count = 0;

    for (i, block) in blocks.iter().enumerate() {
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
        let mut vm = Evm::new_for_l2(
            wrapped_db.clone(),
            fee_configs
                .get(i)
                .cloned()
                .ok_or_else(|| StatelessExecutionError::FeeConfigNotFound)?,
        )?;
        #[cfg(not(feature = "l2"))]
        let mut vm = Evm::new_for_l1(wrapped_db.clone());
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
            .map_err(StatelessExecutionError::GuestProgramState)?;

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
            - get_block_l1_privileged_transactions(&block.body.transactions, chain_config.chain_id)
                .len();

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
        .map_err(StatelessExecutionError::GuestProgramState)?;
    if final_state_hash != last_block_state_root {
        return Err(StatelessExecutionError::InvalidFinalStateTrie);
    }

    Ok(StatelessResult {
        receipts: acc_receipts,
        initial_state_hash,
        final_state_hash,
        last_block_hash,
        non_privileged_count: non_privileged_count.into(),
    })
}

#[cfg(feature = "l2")]
type MessagesAndPrivilegedTransactions =
    (Vec<L1Message>, Vec<L2Message>, Vec<PrivilegedL2Transaction>);

#[cfg(feature = "l2")]
fn get_batch_messages_and_privileged_transactions(
    blocks: &[Block],
    receipts: &[Vec<Receipt>],
    chain_id: u64,
) -> Result<MessagesAndPrivilegedTransactions, StatelessExecutionError> {
    let mut l1messages = vec![];
    let mut privileged_transactions = vec![];
    let mut l2messages = vec![];

    for (block, receipts) in blocks.iter().zip(receipts) {
        let txs = &block.body.transactions;
        privileged_transactions.extend(get_block_l1_privileged_transactions(txs, chain_id));
        l1messages.extend(get_block_l1_messages(receipts));
        l2messages.extend(get_block_l2_messages(receipts));
    }

    Ok((l1messages, l2messages, privileged_transactions))
}

#[cfg(feature = "l2")]
fn compute_messages_and_privileged_transactions_digests(
    l1messages: &[L1Message],
    l2messages: &[L2Message],
    privileged_transactions: &[PrivilegedL2Transaction],
) -> Result<(H256, H256, H256), StatelessExecutionError> {
    use ethrex_l2_common::{
        merkle_tree::compute_merkle_root,
        messages::{get_l1_message_hash, get_l2_message_hash},
    };

    let l1_message_hashes: Vec<_> = l1messages.iter().map(get_l1_message_hash).collect();
    let l2_message_hashes: Vec<_> = l2messages.iter().map(get_l2_message_hash).collect();
    let privileged_transactions_hashes: Vec<_> = privileged_transactions
        .iter()
        .map(PrivilegedL2Transaction::get_privileged_hash)
        .map(|hash| hash.ok_or(StatelessExecutionError::InvalidPrivilegedTransaction))
        .collect::<Result<_, _>>()?;

    let l1message_merkle_root = compute_merkle_root(&l1_message_hashes);
    let l2message_merkle_root = compute_merkle_root(&l2_message_hashes);
    let privileged_transactions_hash =
        compute_privileged_transactions_hash(privileged_transactions_hashes)
            .map_err(StatelessExecutionError::PrivilegedTransactionError)?;

    Ok((
        l1message_merkle_root,
        l2message_merkle_root,
        privileged_transactions_hash,
    ))
}

#[cfg(feature = "l2")]
fn verify_blob(
    blocks: &[Block],
    fee_configs: &[FeeConfig],
    commitment: Commitment,
    proof: Proof,
) -> Result<H256, StatelessExecutionError> {
    use bytes::Bytes;
    use ethrex_crypto::kzg::verify_blob_kzg_proof;

    let len: u64 = blocks.len().try_into()?;
    let mut blob_data = Vec::new();

    blob_data.extend(len.to_be_bytes());

    for block in blocks {
        blob_data.extend(block.encode_to_vec());
    }

    for fee_config in fee_configs {
        blob_data.extend(fee_config.to_vec());
    }

    let blob_data = blob_from_bytes(Bytes::from(blob_data))?;

    if !verify_blob_kzg_proof(blob_data, commitment, proof)? {
        return Err(StatelessExecutionError::InvalidBlobProof);
    }

    Ok(kzg_commitment_to_versioned_hash(&commitment))
}
