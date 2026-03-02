use rkyv::{Archive, Deserialize as RDeserialize, Serialize as RSerialize};
use serde::{Deserialize, Serialize};

/// Action types for the Tokamon game.
///
/// Each variant represents a specific game action that mutates the
/// global game state.
#[derive(
    Serialize, Deserialize, RSerialize, RDeserialize, Archive, Clone, Debug, PartialEq, Eq,
)]
pub enum ActionType {
    /// Create a new reward spot at a location.
    CreateSpot,
    /// Player claims a reward from a spot.
    ClaimReward,
    /// Player feeds (levels up) a collected Tokamon.
    FeedTokamon,
    /// Two players battle their Tokamon.
    Battle,
}

/// A single game action inside a Tokamon batch.
#[derive(Serialize, Deserialize, RSerialize, RDeserialize, Archive, Clone, Debug)]
pub struct GameAction {
    /// Player address (20 bytes).
    pub player: [u8; 20],
    /// Type of action.
    pub action_type: ActionType,
    /// Target entity ID (spot ID, tokamon ID, or opponent ID).
    pub target_id: u64,
    /// Action-specific payload (e.g. location coordinates, battle seed).
    pub payload: Vec<u8>,
}

/// Input for the Tokamon guest program.
///
/// Represents a batch of game actions to be proven.
#[derive(Serialize, Deserialize, RSerialize, RDeserialize, Archive, Clone, Debug)]
pub struct TokammonProgramInput {
    /// Merkle root of the game state before this batch.
    pub initial_state_root: [u8; 32],
    /// Ordered list of game actions in this batch.
    pub actions: Vec<GameAction>,
}

/// Output of the Tokamon guest program.
///
/// Committed as public values by the zkVM so the L1 verifier can
/// check the state transition.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct TokammonProgramOutput {
    /// Game state root before execution.
    pub initial_state_root: [u8; 32],
    /// Game state root after executing all actions.
    pub final_state_root: [u8; 32],
    /// Number of actions successfully processed.
    pub action_count: u64,
    /// Number of new spots created.
    pub spots_created: u64,
    /// Number of rewards claimed.
    pub rewards_claimed: u64,
}

impl TokammonProgramOutput {
    /// Encode the output to bytes for L1 commitment verification.
    ///
    /// Layout: `initial_state_root (32) || final_state_root (32) ||
    ///          action_count (8 BE) || spots_created (8 BE) || rewards_claimed (8 BE)`
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(88);
        buf.extend_from_slice(&self.initial_state_root);
        buf.extend_from_slice(&self.final_state_root);
        buf.extend_from_slice(&self.action_count.to_be_bytes());
        buf.extend_from_slice(&self.spots_created.to_be_bytes());
        buf.extend_from_slice(&self.rewards_claimed.to_be_bytes());
        buf
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify the 88-byte layout: initial_root(32) + final_root(32) +
    /// action_count(8) + spots_created(8) + rewards_claimed(8).
    #[test]
    fn tokamon_encode_layout() {
        let output = TokammonProgramOutput {
            initial_state_root: [0x11; 32],
            final_state_root: [0x22; 32],
            action_count: 10,
            spots_created: 3,
            rewards_claimed: 7,
        };
        let encoded = output.encode();
        assert_eq!(encoded.len(), 88);
        assert_eq!(&encoded[0..32], &[0x11; 32]);
        assert_eq!(&encoded[32..64], &[0x22; 32]);
        assert_eq!(u64::from_be_bytes(encoded[64..72].try_into().unwrap()), 10);
        assert_eq!(u64::from_be_bytes(encoded[72..80].try_into().unwrap()), 3);
        assert_eq!(u64::from_be_bytes(encoded[80..88].try_into().unwrap()), 7);
    }

    #[test]
    fn tokamon_encode_zero_values() {
        let output = TokammonProgramOutput {
            initial_state_root: [0; 32],
            final_state_root: [0; 32],
            action_count: 0,
            spots_created: 0,
            rewards_claimed: 0,
        };
        let encoded = output.encode();
        assert_eq!(encoded.len(), 88);
        assert!(encoded.iter().all(|&b| b == 0));
    }

    #[test]
    fn tokamon_encode_max_values() {
        let output = TokammonProgramOutput {
            initial_state_root: [0xFF; 32],
            final_state_root: [0xFF; 32],
            action_count: u64::MAX,
            spots_created: u64::MAX,
            rewards_claimed: u64::MAX,
        };
        let encoded = output.encode();
        assert_eq!(encoded.len(), 88);
        assert!(encoded.iter().all(|&b| b == 0xFF));
    }
}
