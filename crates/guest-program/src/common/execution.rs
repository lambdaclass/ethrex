use std::sync::Arc;

use ethrex_common::types::block_access_list::BlockAccessList;
use ethrex_common::types::block_execution_witness::{ExecutionWitness, GuestProgramState};
use ethrex_common::types::{Block, Receipt, Transaction, validate_block_body};
use ethrex_common::{
    H256, U256, validate_block_access_list_hash, validate_block_pre_execution, validate_gas_used,
    validate_receipts_root_and_logs_bloom, validate_requests_hash,
};
use ethrex_crypto::Crypto;
use ethrex_vm::{Evm, GuestProgramStateWrapper, VmDatabase};

use crate::common::ExecutionError;
use crate::report_cycles;

/// Result of executing a batch of blocks.
pub struct BatchExecutionResult {
    /// Receipts for each block (outer vec) and each transaction (inner vec).
    pub receipts: Vec<Vec<Receipt>>,
    /// Initial state trie root hash.
    pub initial_state_hash: H256,
    /// Final state trie root hash.
    pub final_state_hash: H256,
    /// Hash of the last block in the batch.
    pub last_block_hash: H256,
    /// Number of non-privileged transactions in the batch.
    pub non_privileged_count: U256,
    /// Chain ID from the execution witness.
    pub chain_id: u64,
    /// Per-block recomputed burned fees (EIP-8079, LStar+). `None` for pre-LStar forks.
    /// One entry per block in the same order as `receipts`.
    pub burned_fees: Vec<Option<u64>>,
    /// Per-block recomputed Block Access List (EIP-7928, Amsterdam+). `None` for pre-Amsterdam.
    /// One entry per block in the same order as `receipts`.
    pub bals: Vec<Option<BlockAccessList>>,
}

/// The witnesses of a block's dependencies are verified natively, before
/// execution, by the node. Stateless execution does not link the verifier, so it
/// must not vouch for a block that depends on them: executing its transactions
/// would accept signatures nobody checked. Such a block is rejected.
pub fn reject_unverified_dependency_witnesses(block: &Block) -> Result<(), ExecutionError> {
    let has_dependencies = block.body.transactions.iter().any(
        |tx| matches!(tx, Transaction::FrameTransaction(frame_tx) if frame_tx.has_dependencies()),
    );
    if has_dependencies {
        return Err(ExecutionError::UnverifiedDependencyWitnesses(
            block.header.number,
        ));
    }
    Ok(())
}

/// Execute a batch of blocks using the provided VM factory.
///
/// This is the core execution logic shared by both L1 and L2 programs.
/// The VM factory closure allows L1 and L2 to create their respective VM types
/// without coupling this code to either.
///
/// # Arguments
/// * `blocks` - The blocks to execute
/// * `execution_witness` - Database containing all data necessary to execute
/// * `elasticity_multiplier` - Value used to calculate base fee
/// * `vm_factory` - Closure that creates an EVM instance for a given block index
pub fn execute_blocks<F>(
    blocks: &[Block],
    execution_witness: ExecutionWitness,
    elasticity_multiplier: u64,
    vm_factory: F,
    crypto: Arc<dyn Crypto + Send + Sync>,
) -> Result<BatchExecutionResult, ExecutionError>
where
    F: Fn(&GuestProgramStateWrapper, usize) -> Result<Evm, ExecutionError>,
{
    let chain_id = execution_witness.chain_config.chain_id;

    let ethrex_guest_program_state: GuestProgramState =
        report_cycles("ethrex_guest_program_state_initialization", || {
            GuestProgramState::from_witness(execution_witness, crypto.as_ref())
                .map_err(ExecutionError::GuestProgramState)
        })?;

    let mut wrapped_db = GuestProgramStateWrapper::new(ethrex_guest_program_state, crypto.clone());

    let chain_config = wrapped_db.get_chain_config().map_err(|_| {
        ExecutionError::Internal("No chain config in execution witness".to_string())
    })?;

    // Hashing is expensive in zkVMs - initialize block header hashes once
    report_cycles("initialize_block_header_hashes", || {
        wrapped_db.initialize_block_header_hashes(blocks)
    })?;

    // Validate execution witness' block hashes
    report_cycles("get_first_invalid_block_hash", || {
        if let Ok(Some(invalid_block_header)) = wrapped_db.get_first_invalid_block_hash() {
            return Err(ExecutionError::InvalidBlockHash(invalid_block_header));
        }
        Ok(())
    })?;

    // Validate initial state
    let parent_block_header = wrapped_db
        .get_block_parent_header(
            blocks
                .first()
                .ok_or(ExecutionError::EmptyBatch)?
                .header
                .number,
        )
        .map_err(ExecutionError::GuestProgramState)?;

    let initial_state_hash = report_cycles("state_trie_root", || {
        wrapped_db
            .state_trie_root()
            .map_err(ExecutionError::GuestProgramState)
    })?;

    if initial_state_hash != parent_block_header.state_root {
        return Err(ExecutionError::InvalidInitialStateTrie);
    }

    // Execute blocks
    let mut parent_block_header = &parent_block_header;
    let mut acc_receipts = Vec::new();
    let mut acc_burned_fees: Vec<Option<u64>> = Vec::new();
    let mut acc_bals: Vec<Option<BlockAccessList>> = Vec::new();
    let mut non_privileged_count: usize = 0;

    for (i, block) in blocks.iter().enumerate() {
        // Validate that the block header and body match (transactions root, withdrawals root)
        report_cycles("validate_block_body", || {
            validate_block_body(&block.header, &block.body, crypto.as_ref())
                .map_err(ExecutionError::BlockBodyValidation)
        })?;

        // Validate the block header pre-execution
        report_cycles("validate_block_pre_execution", || {
            validate_block_pre_execution(
                block,
                parent_block_header,
                &chain_config,
                elasticity_multiplier,
            )
            .map_err(ExecutionError::BlockValidation)
        })?;

        reject_unverified_dependency_witnesses(block)?;

        // Create VM using the provided factory
        let mut vm = report_cycles("setup_evm", || vm_factory(&wrapped_db, i))?;

        // Execute block
        let (result, bal) = report_cycles("execute_block", || {
            vm.execute_block(block, parent_block_header.timestamp)
                .map_err(ExecutionError::Evm)
        })?;

        let receipts = result.receipts;
        let block_gas_used = result.block_gas_used;
        let block_burned_fees = result.burned_fees;

        let account_updates = report_cycles("get_state_transitions", || {
            vm.get_state_transitions().map_err(ExecutionError::Evm)
        })?;

        // Apply state transitions to the db (needed for both next block execution
        // and final state validation via state_trie_root())
        report_cycles("apply_account_updates", || {
            wrapped_db
                .apply_account_updates(&account_updates)
                .map_err(ExecutionError::GuestProgramState)
        })?;

        // Count non-privileged transactions
        non_privileged_count += block
            .body
            .transactions
            .iter()
            .filter(|tx| !tx.is_privileged())
            .count();

        // Validate gas and receipts
        report_cycles("validate_gas_and_receipts", || {
            validate_gas_used(block_gas_used, &block.header).map_err(ExecutionError::GasValidation)
        })?;

        report_cycles("validate_receipts_root_and_logs_bloom", || {
            validate_receipts_root_and_logs_bloom(&block.header, &receipts, crypto.as_ref())
                .map_err(ExecutionError::ReceiptsRootValidation)
        })?;

        report_cycles("validate_requests_hash", || {
            validate_requests_hash(&block.header, &chain_config, &result.requests)
                .map_err(ExecutionError::RequestsRootValidation)
        })?;

        if let Some(bal) = &bal {
            report_cycles("validate_block_access_list_hash", || {
                validate_block_access_list_hash(
                    &block.header,
                    &chain_config,
                    bal,
                    block.body.transactions.len(),
                    crypto.as_ref(),
                )
                .map_err(ExecutionError::BlockValidation)
            })?;
        }

        acc_receipts.push(receipts);
        acc_burned_fees.push(block_burned_fees);
        acc_bals.push(bal);
        parent_block_header = &block.header;
    }

    // Validate final state
    let last_block = blocks.last().ok_or(ExecutionError::EmptyBatch)?;

    let final_state_hash = report_cycles("get_final_state_root", || {
        wrapped_db
            .state_trie_root()
            .map_err(ExecutionError::GuestProgramState)
    })?;

    if final_state_hash != last_block.header.state_root {
        return Err(ExecutionError::InvalidFinalStateTrie);
    }

    let last_block_hash = last_block.header.compute_block_hash(crypto.as_ref());

    Ok(BatchExecutionResult {
        receipts: acc_receipts,
        initial_state_hash,
        final_state_hash,
        last_block_hash,
        non_privileged_count: non_privileged_count.into(),
        chain_id,
        burned_fees: acc_burned_fees,
        bals: acc_bals,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use ethrex_common::H256;
    use ethrex_common::types::{
        BlockBody, BlockHeader, FRAME_SIG_SCHEME_LEANSPHINCS, Frame, FrameSignature,
        FrameTransaction,
    };

    fn block_with(transactions: Vec<Transaction>) -> Block {
        Block::new(
            BlockHeader {
                number: 7,
                ..Default::default()
            },
            BlockBody {
                transactions,
                ommers: Vec::new(),
                withdrawals: Some(Vec::new()),
                dependency_witnesses: Some(Vec::new()),
            },
        )
    }

    /// A frame transaction, with a LEANSPHINCS entry in its signature list when
    /// `depends` is set.
    fn frame_tx(depends: bool) -> Transaction {
        let signatures = if depends {
            vec![FrameSignature {
                scheme: FRAME_SIG_SCHEME_LEANSPHINCS,
                signer: None,
                signer_key: Some(H256::repeat_byte(0xAA)),
                msg: Bytes::new(),
                signature: Bytes::new(),
            }]
        } else {
            Vec::new()
        };
        Transaction::FrameTransaction(FrameTransaction {
            frames: vec![Frame::default()],
            signatures,
            ..Default::default()
        })
    }

    #[test]
    fn stateless_execution_rejects_a_block_with_dependencies() {
        // Any other block is left to the rest of the validation.
        reject_unverified_dependency_witnesses(&block_with(Vec::new())).expect("no transactions");
        reject_unverified_dependency_witnesses(&block_with(vec![frame_tx(false)]))
            .expect("a frame transaction without dependencies");

        // A LEANSPHINCS entry stands for a signature this path cannot check.
        let result = reject_unverified_dependency_witnesses(&block_with(vec![
            frame_tx(false),
            frame_tx(true),
        ]));
        assert!(
            matches!(
                result,
                Err(ExecutionError::UnverifiedDependencyWitnesses(7))
            ),
            "got {result:?}"
        );
    }
}
