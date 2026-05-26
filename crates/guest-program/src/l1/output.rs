use ethrex_common::{H256, U256};
use serde::{Deserialize, Serialize};

/// Legacy (pre-Hegotá) output of the L1 stateless validation program.
#[derive(Serialize, Deserialize)]
pub struct LegacyProgramOutput {
    /// Initial state trie root hash.
    pub initial_state_hash: H256,
    /// Final state trie root hash.
    pub final_state_hash: H256,
    /// Hash of the last block in the batch.
    pub last_block_hash: H256,
    /// Chain ID of the network.
    pub chain_id: U256,
    /// Number of transactions in the batch.
    pub transaction_count: U256,
}

impl LegacyProgramOutput {
    /// Encode the output to bytes for commitment.
    pub fn encode(&self) -> Vec<u8> {
        [
            self.initial_state_hash.to_fixed_bytes(),
            self.final_state_hash.to_fixed_bytes(),
            self.last_block_hash.to_fixed_bytes(),
            self.chain_id.to_big_endian(),
            self.transaction_count.to_big_endian(),
        ]
        .concat()
    }
}

/// Hegotá / EIP-8025 output of the L1 stateless validation program.
///
/// The output is a 41-byte commitment: the `hash_tree_root` of the
/// `NewPayloadRequest` (32 bytes), a validity flag (1 byte), and
/// `chain_id` (8 bytes).
#[derive(Serialize, Deserialize)]
pub struct Eip8025ProgramOutput {
    /// The `hash_tree_root` of the `NewPayloadRequest`.
    pub new_payload_request_root: [u8; 32],
    /// Whether execution was valid.
    pub valid: bool,
    /// Chain ID from the stateless validation chain configuration.
    pub chain_id: u64,
}

impl Eip8025ProgramOutput {
    /// Encode the output to 41 bytes: `root ++ valid ++ chain_id`.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(41);
        out.extend_from_slice(&self.new_payload_request_root);
        out.push(u8::from(self.valid));
        out.extend_from_slice(&self.chain_id.to_le_bytes());
        out
    }
}

/// Output of the L1 stateless validation program.
///
/// The variant is selected at runtime by the prover-coordinator from the block's
/// fork (`is_hegota_activated`). Guest binaries are specialized for one variant
/// and choose at compile time via their local `eip-8025` feature.
#[derive(Serialize, Deserialize)]
pub enum ProgramOutput {
    Legacy(LegacyProgramOutput),
    Eip8025(Eip8025ProgramOutput),
}

impl ProgramOutput {
    /// Encode the output to bytes for commitment. Delegates to the inner variant.
    pub fn encode(&self) -> Vec<u8> {
        match self {
            ProgramOutput::Legacy(inner) => inner.encode(),
            ProgramOutput::Eip8025(inner) => inner.encode(),
        }
    }
}
