use serde::{Deserialize, Serialize};

#[cfg(not(feature = "eip-8025"))]
use ethrex_common::{H256, U256};

/// Output of the L1 stateless validation program.
#[cfg(not(feature = "eip-8025"))]
#[derive(Serialize, Deserialize)]
pub struct ProgramOutput {
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

#[cfg(not(feature = "eip-8025"))]
impl ProgramOutput {
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

/// Output of the L1 stateless validation program (EIP-8025).
///
/// The output is a 33-byte commitment: the `hash_tree_root` of the
/// `NewPayloadRequest` (32 bytes) followed by a validity flag (1 byte).
#[cfg(feature = "eip-8025")]
#[derive(Serialize, Deserialize)]
pub struct ProgramOutput {
    /// The `hash_tree_root` of the `NewPayloadRequest`.
    pub new_payload_request_root: [u8; 32],
    /// Whether execution was valid.
    pub valid: bool,
}

#[cfg(feature = "eip-8025")]
impl ProgramOutput {
    /// Encode the output to 33 bytes: `root ++ valid`.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(33);
        out.extend_from_slice(&self.new_payload_request_root);
        out.push(u8::from(self.valid));
        out
    }
}
