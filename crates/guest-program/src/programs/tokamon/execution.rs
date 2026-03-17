use ethrex_crypto::keccak::keccak_hash;

use super::types::{ActionType, TokammonProgramInput, TokammonProgramOutput};

/// Errors that can occur during Tokamon execution.
#[derive(Debug, thiserror::Error)]
pub enum TokammonExecutionError {
    #[error("Empty action batch")]
    EmptyBatch,
    #[error("Invalid payload length at index {index}: expected {expected}, got {got}")]
    InvalidPayload {
        index: usize,
        expected: usize,
        got: usize,
    },
}

/// Minimum payload sizes per action type.
const CREATE_SPOT_PAYLOAD_MIN: usize = 16; // lat(8) + lon(8)
const BATTLE_PAYLOAD_MIN: usize = 8; // random seed

/// Execute a batch of Tokamon game actions.
///
/// Validates each action and computes a deterministic `final_state_root`
/// by hashing the game state with each action's data.
///
/// # State transition model
///
/// ```text
/// state = initial_state_root
/// for each action:
///     state = keccak256(state || player || action_type_byte || target_id || payload)
/// final_state_root = state
/// ```
///
/// This is a simplified model — a production implementation would maintain
/// an actual game-state Merkle tree with spots, tokamon inventories, etc.
pub fn execution_program(
    input: TokammonProgramInput,
) -> Result<TokammonProgramOutput, TokammonExecutionError> {
    if input.actions.is_empty() {
        return Err(TokammonExecutionError::EmptyBatch);
    }

    let mut state = input.initial_state_root;
    let mut spots_created: u64 = 0;
    let mut rewards_claimed: u64 = 0;

    for (i, action) in input.actions.iter().enumerate() {
        // Validate payload size for actions that require specific data.
        match action.action_type {
            ActionType::CreateSpot => {
                if action.payload.len() < CREATE_SPOT_PAYLOAD_MIN {
                    return Err(TokammonExecutionError::InvalidPayload {
                        index: i,
                        expected: CREATE_SPOT_PAYLOAD_MIN,
                        got: action.payload.len(),
                    });
                }
                spots_created += 1;
            }
            ActionType::ClaimReward => {
                rewards_claimed += 1;
            }
            ActionType::Battle => {
                if action.payload.len() < BATTLE_PAYLOAD_MIN {
                    return Err(TokammonExecutionError::InvalidPayload {
                        index: i,
                        expected: BATTLE_PAYLOAD_MIN,
                        got: action.payload.len(),
                    });
                }
            }
            ActionType::FeedTokamon => {
                // No special payload requirement.
            }
        }

        // Hash the current state with this action to produce the next state.
        let action_type_byte = match action.action_type {
            ActionType::CreateSpot => 0u8,
            ActionType::ClaimReward => 1,
            ActionType::FeedTokamon => 2,
            ActionType::Battle => 3,
        };

        let mut preimage = Vec::with_capacity(32 + 20 + 1 + 8 + action.payload.len());
        preimage.extend_from_slice(&state);
        preimage.extend_from_slice(&action.player);
        preimage.push(action_type_byte);
        preimage.extend_from_slice(&action.target_id.to_le_bytes());
        preimage.extend_from_slice(&action.payload);

        state = keccak_hash(&preimage);
    }

    Ok(TokammonProgramOutput {
        initial_state_root: input.initial_state_root,
        final_state_root: state,
        action_count: input.actions.len() as u64,
        spots_created,
        rewards_claimed,
    })
}
