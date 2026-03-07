use ethrex_common::types::ELASTICITY_MULTIPLIER;
use ethrex_vm::Evm;

use crate::common::{BatchExecutionResult, ExecutionError, execute_blocks};
use crate::l1::input::ProgramInput;
use crate::l1::output::ProgramOutput;

/// Execute the L1 stateless validation program.
///
/// This validates and executes a batch of L1 blocks, verifying state transitions
/// without access to the full blockchain state.
pub fn execution_program(input: ProgramInput) -> Result<ProgramOutput, ExecutionError> {
    let ProgramInput {
        blocks,
        execution_witness,
    } = input;

    let BatchExecutionResult {
        receipts: _,
        initial_state_hash,
        final_state_hash,
        last_block_hash,
        non_privileged_count,
        chain_id,
    } = execute_blocks(
        &blocks,
        execution_witness,
        ELASTICITY_MULTIPLIER,
        |db, _| {
            // L1 VM factory - simple creation without fee configs
            Ok(Evm::new_for_l1(db.clone()))
        },
    )?;

    Ok(ProgramOutput {
        initial_state_hash,
        final_state_hash,
        last_block_hash,
        chain_id: chain_id.into(),
        transaction_count: non_privileged_count,
    })
}

/// EIP-8025 execution program.
///
/// Validates a single block from a NewPayloadRequest inside the zkVM:
/// 1. Convert NewPayloadRequest -> Block
/// 2. Validate block_hash matches derived hash
/// 3. Validate versioned_blob_hashes
/// 4. Execute block statelessly (reuses execute_blocks)
/// 5. Compute hash_tree_root(NewPayloadRequest)
/// 6. Return (root, valid=true) on success
#[cfg(feature = "eip-8025")]
pub fn eip8025_execution_program(
    input: super::input::Eip8025ProgramInput,
) -> super::output::Eip8025ProgramOutput {
    let root = super::ssz::compute_new_payload_request_root(&input.new_payload_request)
        .expect("SSZ hash_tree_root computation failed");

    match eip8025_validate_and_execute(input) {
        Ok(()) => super::output::Eip8025ProgramOutput {
            new_payload_request_root: root,
            valid: true,
        },
        Err(_) => super::output::Eip8025ProgramOutput {
            new_payload_request_root: root,
            valid: false,
        },
    }
}

#[cfg(feature = "eip-8025")]
fn eip8025_validate_and_execute(
    input: super::input::Eip8025ProgramInput,
) -> Result<(), ExecutionError> {
    use ethrex_common::H256;

    let super::input::Eip8025ProgramInput {
        new_payload_request,
        execution_witness,
    } = input;

    // 1. Convert NewPayloadRequest -> Block
    let block = new_payload_request
        .to_block()
        .map_err(|e| ExecutionError::Internal(format!("Failed to convert payload to block: {e}")))?;

    // 2. Validate block_hash matches derived hash
    let derived_hash = block.hash();
    let expected_hash = new_payload_request.execution_payload.block_hash;
    if derived_hash != expected_hash {
        return Err(ExecutionError::Internal(format!(
            "Block hash mismatch: derived {derived_hash:#x}, expected {expected_hash:#x}"
        )));
    }

    // 3. Validate versioned_blob_hashes
    let tx_blob_hashes: Vec<H256> = block
        .body
        .transactions
        .iter()
        .flat_map(|tx| tx.blob_versioned_hashes())
        .collect();
    let expected_hashes = new_payload_request.versioned_hashes_h256();
    if tx_blob_hashes != expected_hashes {
        return Err(ExecutionError::Internal(
            "Versioned blob hashes mismatch".to_string(),
        ));
    }

    // 4. Execute block statelessly (reuses existing execute_blocks)
    let _result = execute_blocks(
        &[block],
        execution_witness,
        ELASTICITY_MULTIPLIER,
        |db, _| Ok(Evm::new_for_l1(db.clone())),
    )?;

    Ok(())
}
