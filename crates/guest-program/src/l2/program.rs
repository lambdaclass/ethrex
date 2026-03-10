use ethrex_l2_common::messages::get_balance_diffs;
use ethrex_vm::{Evm, GuestProgramStateWrapper};

use crate::common::{BatchExecutionResult, execute_blocks};
use crate::crypto::get_crypto_provider;
use crate::l2::blobs::verify_blob;
use crate::l2::error::L2ExecutionError;
use crate::l2::input::ProgramInput;
use crate::l2::messages::{compute_message_digests, get_batch_messages};
use crate::l2::output::ProgramOutput;

/// Execute the L2 stateless validation program.
///
/// This validates and executes a batch of L2 blocks, verifying state transitions,
/// message passing, and blob data without access to the full blockchain state.
pub fn execution_program(input: ProgramInput) -> Result<ProgramOutput, L2ExecutionError> {
    let ProgramInput {
        blocks,
        execution_witness,
        elasticity_multiplier,
        fee_configs,
        blob_commitment,
        blob_proof,
    } = input;

    let crypto = get_crypto_provider();

    // Execute blocks using the common execution logic
    let BatchExecutionResult {
        receipts,
        initial_state_hash,
        final_state_hash,
        last_block_hash,
        non_privileged_count,
        chain_id,
    } = execute_blocks(
        &blocks,
        execution_witness,
        elasticity_multiplier,
        |db: &GuestProgramStateWrapper, i: usize| -> Result<Evm, crate::common::ExecutionError> {
            // L2 VM factory - requires fee config for each block
            let fee_config = fee_configs.get(i).cloned().ok_or_else(|| {
                crate::common::ExecutionError::Internal(
                    "FeeConfig not provided for L2 execution".to_string(),
                )
            })?;
            Evm::new_for_l2(db.clone(), fee_config, crypto.clone())
                .map_err(crate::common::ExecutionError::Evm)
        },
    )?;

    // Extract and process messages
    let batch_messages = get_batch_messages(&blocks, &receipts, chain_id);
    let message_digests = compute_message_digests(&batch_messages)?;
    let balance_diffs = get_balance_diffs(&batch_messages.l2_out_messages);

    // Verify blob proof
    let blob_versioned_hash = verify_blob(&blocks, &fee_configs, blob_commitment, blob_proof)?;

    Ok(ProgramOutput {
        initial_state_hash,
        final_state_hash,
        l1_out_messages_merkle_root: message_digests.l1_out_messages_merkle_root,
        l1_in_messages_rolling_hash: message_digests.l1_in_messages_rolling_hash,
        l2_in_message_rolling_hashes: message_digests.l2_in_message_rolling_hashes,
        blob_versioned_hash,
        last_block_hash,
        chain_id: chain_id.into(),
        non_privileged_count,
        balance_diffs,
    })
}
