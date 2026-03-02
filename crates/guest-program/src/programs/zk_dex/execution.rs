use ethrex_crypto::keccak::keccak_hash;

use super::types::{DexProgramInput, DexProgramOutput};

/// Errors that can occur during ZK-DEX execution.
#[derive(Debug, thiserror::Error)]
pub enum DexExecutionError {
    #[error("Empty transfer batch")]
    EmptyBatch,
    #[error("Transfer amount is zero at index {0}")]
    ZeroAmount(usize),
    #[error("Self-transfer at index {0}")]
    SelfTransfer(usize),
}

/// Execute a batch of ZK-DEX token transfers.
///
/// Validates each transfer and computes a deterministic `final_state_root`
/// by hashing the initial state root together with all transfer data.
///
/// # State transition model
///
/// The final state root is computed as:
/// ```text
/// state = initial_state_root
/// for each transfer:
///     state = keccak256(state || from || to || token || amount || nonce)
/// final_state_root = state
/// ```
///
/// This is a simplified model — a production implementation would maintain
/// an actual Merkle tree of balances and verify sufficient funds.
pub fn execution_program(input: DexProgramInput) -> Result<DexProgramOutput, DexExecutionError> {
    if input.transfers.is_empty() {
        return Err(DexExecutionError::EmptyBatch);
    }

    let mut state = input.initial_state_root;

    for (i, transfer) in input.transfers.iter().enumerate() {
        if transfer.amount == 0 {
            return Err(DexExecutionError::ZeroAmount(i));
        }
        if transfer.from == transfer.to {
            return Err(DexExecutionError::SelfTransfer(i));
        }

        // Hash the current state with this transfer to produce the next state.
        let mut preimage = Vec::with_capacity(32 + 20 + 20 + 20 + 8 + 8);
        preimage.extend_from_slice(&state);
        preimage.extend_from_slice(&transfer.from);
        preimage.extend_from_slice(&transfer.to);
        preimage.extend_from_slice(&transfer.token);
        preimage.extend_from_slice(&transfer.amount.to_le_bytes());
        preimage.extend_from_slice(&transfer.nonce.to_le_bytes());

        state = keccak_hash(&preimage);
    }

    Ok(DexProgramOutput {
        initial_state_root: input.initial_state_root,
        final_state_root: state,
        transfer_count: input.transfers.len() as u64,
    })
}
