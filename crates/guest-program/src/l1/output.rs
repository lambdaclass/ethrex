use ethrex_common::{H256, U256};
use serde::{Deserialize, Serialize};

/// Output of the L1 stateless validation program.
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

/// EIP-8025 output: 33 bytes = hash_tree_root(NewPayloadRequest) + valid flag.
#[cfg(feature = "eip-8025")]
pub struct Eip8025ProgramOutput {
    /// hash_tree_root of the NewPayloadRequest (SSZ Merkleization).
    pub new_payload_request_root: [u8; 32],
    /// Whether block validation succeeded.
    pub valid: bool,
}

#[cfg(feature = "eip-8025")]
impl Eip8025ProgramOutput {
    /// Encode the output to 33 bytes for commitment.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(33);
        out.extend_from_slice(&self.new_payload_request_root);
        out.push(self.valid as u8);
        out
    }
}
