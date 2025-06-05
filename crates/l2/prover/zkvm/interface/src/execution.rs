#[cfg(feature = "l2")]
use crate::deposits::{get_block_deposits, get_deposit_hash};
#[cfg(feature = "l2")]
use crate::withdrawals::{get_block_withdrawals, get_withdrawals_merkle_root};

use crate::io::{ProgramInput, ProgramOutput};
use ethrex_blockchain::error::ChainError;
use ethrex_blockchain::{validate_block, validate_gas_used};
use ethrex_common::types::{Block, BlockHeader};
use ethrex_common::H256;
use ethrex_vm::{Evm, EvmEngine, EvmError, ProverDB, ProverDBError};

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
    WithdrawalError(crate::withdrawals::Error),
    #[cfg(feature = "l2")]
    #[error("Deposit calculation error: {0}")]
    DepositError(crate::deposits::DepositError),
    #[error("Batch has no blocks")]
    EmptyBatchError,
    #[error("Invalid database")]
    InvalidDatabase,
    #[error("Invalid initial state trie")]
    InvalidInitialStateTrie,
    #[error("Invalid final state trie")]
    InvalidFinalStateTrie,
    #[error("Missing deposit hash")]
    MissingDepositHash,
}

pub fn execution_program(input: ProgramInput) -> Result<ProgramOutput, StatelessExecutionError> {
    let ProgramInput {
        blocks,
        parent_block_header,
        mut db,
        elasticity_multiplier,
    } = input;
    if cfg!(feature = "l2") {
        #[cfg(feature = "l2")]
        return stateless_validation_l2(
            &blocks,
            &parent_block_header,
            &mut db,
            elasticity_multiplier,
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
    })
}

#[cfg(feature = "l2")]
pub fn stateless_validation_l2(
    blocks: &[Block],
    parent_block_header: &BlockHeader,
    db: &mut ProverDB,
    elasticity_multiplier: u64,
) -> Result<ProgramOutput, StatelessExecutionError> {
    let StatelessResult {
        receipts,
        initial_state_hash,
        final_state_hash,
    } = execute_stateless(blocks, parent_block_header, db, elasticity_multiplier)?;

    let mut withdrawals = vec![];
    let mut deposits_hashes = vec![];

    // Get L2 withdrawals and deposits for this block
    for (block, receipts) in blocks.iter().zip(receipts) {
        let block_withdrawals = get_block_withdrawals(&block.body.transactions, &receipts)
            .map_err(StatelessExecutionError::WithdrawalError)?;
        let block_deposits = get_block_deposits(&block.body.transactions);
        let mut block_deposits_hashes = Vec::with_capacity(block_deposits.len());
        for deposit in block_deposits {
            if let Some(hash) = deposit.get_deposit_hash() {
                block_deposits_hashes.push(hash);
            } else {
                return Err(StatelessExecutionError::MissingDepositHash);
            }
        }
        withdrawals.extend(block_withdrawals);
        deposits_hashes.extend(block_deposits_hashes);
    }

    // Calculate L2 withdrawals root
    let withdrawals_merkle_root = get_withdrawals_merkle_root(withdrawals)
        .map_err(StatelessExecutionError::WithdrawalError)?;

    // Calculate L2 deposits logs root
    let deposit_logs_hash =
        get_deposit_hash(deposits_hashes).map_err(StatelessExecutionError::DepositError)?;

    Ok(ProgramOutput {
        initial_state_hash,
        final_state_hash,
        withdrawals_merkle_root,
        deposit_logs_hash,
    })
}

struct StatelessResult {
    #[cfg(feature = "l2")]
    receipts: Vec<Vec<ethrex_common::types::Receipt>>,
    initial_state_hash: H256,
    final_state_hash: H256,
}

fn execute_stateless(
    blocks: &[Block],
    parent_block_header: &BlockHeader,
    db: &mut ProverDB,
    elasticity_multiplier: u64,
) -> Result<StatelessResult, StatelessExecutionError> {
    let _ = db.rebuild_tries();
    // Validate the initial state
    let initial_state_hash = db
        .state_trie_root()
        .map_err(|_| StatelessExecutionError::InvalidInitialStateTrie)?;

    if initial_state_hash != parent_block_header.state_root {
        return Err(StatelessExecutionError::InvalidInitialStateTrie);
    }

    let mut parent_header = parent_block_header;
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
        db.apply_account_updates_from_trie(&account_updates);

        validate_gas_used(&receipts, &block.header)
            .map_err(StatelessExecutionError::GasValidationError)?;
        parent_header = &block.header;
        acc_receipts.push(receipts);
    }

    // Calculate final state root hash and check
    let last_block = blocks
        .last()
        .ok_or(StatelessExecutionError::EmptyBatchError)?;
    let last_block_state_root = last_block.header.state_root;
    let final_state_hash = db
        .state_trie_root()
        .map_err(|_| StatelessExecutionError::InvalidDatabase)?;

    if final_state_hash != last_block_state_root {
        return Err(StatelessExecutionError::InvalidFinalStateTrie);
    }
    Ok(StatelessResult {
        #[cfg(feature = "l2")]
        receipts: acc_receipts,
        initial_state_hash,
        final_state_hash,
    })
}
