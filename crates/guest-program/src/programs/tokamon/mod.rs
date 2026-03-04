pub mod execution;
pub mod types;

use crate::traits::{GuestProgram, GuestProgramError, ResourceLimits, backends};

/// Tokamon Guest Program — location-based reward/stamp game.
///
/// This program proves game state transitions for the Tokamon game:
/// spot management (CreateSpot) and claim/game mechanics (ClaimReward,
/// FeedTokamon, Battle).
///
/// Reference: <https://github.com/tokamak-network/tokamon/tree/deploy/thanos-sepolia>
///
/// ## Serialization
///
/// The Tokamon guest binary reads rkyv-serialized
/// [`types::TokammonProgramInput`] from the zkVM stdin.
/// [`serialize_input`](GuestProgram::serialize_input) is a pass-through
/// because the prover already supplies the correct bytes.
///
/// [`encode_output`](GuestProgram::encode_output) is also a pass-through;
/// the guest binary calls [`types::TokammonProgramOutput::encode`] internally.
pub struct TokammonGuestProgram;

impl TokammonGuestProgram {
    fn non_empty(elf: &[u8]) -> Option<&[u8]> {
        if elf.is_empty() || elf == [0] {
            None
        } else {
            Some(elf)
        }
    }
}

impl GuestProgram for TokammonGuestProgram {
    fn program_id(&self) -> &str {
        "tokamon"
    }

    fn elf(&self, backend: &str) -> Option<&[u8]> {
        match backend {
            backends::SP1 => Self::non_empty(crate::ZKVM_SP1_TOKAMON_ELF),
            _ => None,
        }
    }

    fn vk_bytes(&self, _backend: &str) -> Option<Vec<u8>> {
        None
    }

    fn program_type_id(&self) -> u8 {
        3 // Tokamon
    }

    fn serialize_input(&self, raw_input: &[u8]) -> Result<Vec<u8>, GuestProgramError> {
        // The prover serializes TokammonProgramInput to rkyv bytes before
        // calling this method.  Pass-through is correct.
        Ok(raw_input.to_vec())
    }

    fn encode_output(&self, raw_output: &[u8]) -> Result<Vec<u8>, GuestProgramError> {
        // The zkVM guest binary calls TokammonProgramOutput::encode()
        // internally and commits the result as public values.
        Ok(raw_output.to_vec())
    }

    fn resource_limits(&self) -> ResourceLimits {
        ResourceLimits {
            max_input_bytes: Some(64 * 1024 * 1024), // 64 MB
            max_proving_duration: Some(std::time::Duration::from_secs(1800)), // 30 minutes
        }
    }

    fn version(&self) -> &str {
        "0.1.0"
    }
}

#[cfg(test)]
mod tests {
    use super::execution::execution_program;
    use super::types::{ActionType, GameAction, TokammonProgramInput};
    use super::*;

    #[test]
    fn program_id_is_tokamon() {
        let gp = TokammonGuestProgram;
        assert_eq!(gp.program_id(), "tokamon");
    }

    #[test]
    fn program_type_id_is_three() {
        let gp = TokammonGuestProgram;
        assert_eq!(gp.program_type_id(), 3);
    }

    #[test]
    fn sp1_elf_lookup() {
        let gp = TokammonGuestProgram;
        let result = gp.elf(crate::traits::backends::SP1);
        if crate::ZKVM_SP1_TOKAMON_ELF.is_empty() {
            assert!(result.is_none());
        } else {
            assert!(result.is_some());
        }
    }

    #[test]
    fn unsupported_backend_returns_none() {
        let gp = TokammonGuestProgram;
        assert!(gp.elf("risc0").is_none());
        assert!(gp.elf("nonexistent").is_none());
    }

    #[test]
    fn serialize_input_is_identity() {
        let gp = TokammonGuestProgram;
        let data = b"test data";
        assert_eq!(gp.serialize_input(data).unwrap(), data);
    }

    // ── Execution tests ────────────────────────────────────────────

    fn claim_action(nonce: u64) -> GameAction {
        GameAction {
            player: [0x11; 20],
            action_type: ActionType::ClaimReward,
            target_id: nonce,
            payload: vec![],
        }
    }

    fn create_spot_action(target_id: u64) -> GameAction {
        // 16 bytes minimum: lat(8) + lon(8)
        GameAction {
            player: [0x22; 20],
            action_type: ActionType::CreateSpot,
            target_id,
            payload: vec![0u8; 16],
        }
    }

    #[test]
    fn execution_produces_deterministic_output() {
        let input = TokammonProgramInput {
            initial_state_root: [0xAA; 32],
            actions: vec![claim_action(0), create_spot_action(1)],
        };
        let output = execution_program(input.clone()).expect("should succeed");

        assert_eq!(output.initial_state_root, [0xAA; 32]);
        assert_eq!(output.action_count, 2);
        assert_eq!(output.rewards_claimed, 1);
        assert_eq!(output.spots_created, 1);
        assert_ne!(output.final_state_root, output.initial_state_root);

        // Same input must produce the same output (deterministic).
        let output2 = execution_program(input).expect("should succeed");
        assert_eq!(output.final_state_root, output2.final_state_root);
    }

    #[test]
    fn execution_rejects_empty_batch() {
        let input = TokammonProgramInput {
            initial_state_root: [0; 32],
            actions: vec![],
        };
        assert!(execution_program(input).is_err());
    }

    #[test]
    fn execution_rejects_short_create_spot_payload() {
        let input = TokammonProgramInput {
            initial_state_root: [0; 32],
            actions: vec![GameAction {
                player: [0x33; 20],
                action_type: ActionType::CreateSpot,
                target_id: 0,
                payload: vec![0u8; 4], // too short (need 16)
            }],
        };
        assert!(execution_program(input).is_err());
    }

    #[test]
    fn execution_rejects_short_battle_payload() {
        let input = TokammonProgramInput {
            initial_state_root: [0; 32],
            actions: vec![GameAction {
                player: [0x44; 20],
                action_type: ActionType::Battle,
                target_id: 0,
                payload: vec![0u8; 2], // too short (need 8)
            }],
        };
        assert!(execution_program(input).is_err());
    }

    #[test]
    fn feed_tokamon_needs_no_payload() {
        let input = TokammonProgramInput {
            initial_state_root: [0; 32],
            actions: vec![GameAction {
                player: [0x55; 20],
                action_type: ActionType::FeedTokamon,
                target_id: 42,
                payload: vec![],
            }],
        };
        let output = execution_program(input).expect("should succeed");
        assert_eq!(output.action_count, 1);
    }

    #[test]
    fn output_encode_length() {
        let input = TokammonProgramInput {
            initial_state_root: [0xBB; 32],
            actions: vec![claim_action(0)],
        };
        let output = execution_program(input).expect("should succeed");
        let encoded = output.encode();
        // 32 + 32 + 8 + 8 + 8 = 88 bytes
        assert_eq!(encoded.len(), 88);
    }

    #[test]
    fn rkyv_roundtrip() {
        let input = TokammonProgramInput {
            initial_state_root: [0xCC; 32],
            actions: vec![claim_action(0), create_spot_action(1)],
        };
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&input).expect("rkyv serialize");
        let restored: TokammonProgramInput =
            rkyv::from_bytes::<TokammonProgramInput, rkyv::rancor::Error>(&bytes)
                .expect("rkyv deserialize");
        assert_eq!(restored.initial_state_root, input.initial_state_root);
        assert_eq!(restored.actions.len(), 2);
    }
}
