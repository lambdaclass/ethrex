use serde::{Deserialize, Serialize};

/// Output of the L1 stateless validation program (EIP-8025).
///
/// The output is a 41-byte commitment: the `hash_tree_root` of the
/// `NewPayloadRequest` (32 bytes), a validity flag (1 byte), and
/// `chain_id` (8 bytes).
#[derive(Serialize, Deserialize)]
pub struct ProgramOutput {
    /// The `hash_tree_root` of the `NewPayloadRequest`.
    pub new_payload_request_root: [u8; 32],
    /// Whether execution was valid.
    pub valid: bool,
    /// Chain ID from the stateless validation chain configuration.
    pub chain_id: u64,
}

impl ProgramOutput {
    /// Encode the output to 41 bytes: `root ++ valid ++ chain_id`.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(41);
        out.extend_from_slice(&self.new_payload_request_root);
        out.push(u8::from(self.valid));
        out.extend_from_slice(&self.chain_id.to_le_bytes());
        out
    }
}
