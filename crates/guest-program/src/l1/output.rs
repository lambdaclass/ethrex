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

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify 160-byte layout: 5 fields × 32 bytes, in the correct order.
    #[test]
    fn l1_encode_layout() {
        let output = ProgramOutput {
            initial_state_hash: H256::from([0x01; 32]),
            final_state_hash: H256::from([0x02; 32]),
            last_block_hash: H256::from([0x03; 32]),
            chain_id: U256::from(4u64),
            transaction_count: U256::from(5u64),
        };
        let encoded = output.encode();
        assert_eq!(encoded.len(), 160);
        assert_eq!(&encoded[0..32], &[0x01; 32]); // initial_state_hash
        assert_eq!(&encoded[32..64], &[0x02; 32]); // final_state_hash
        assert_eq!(&encoded[64..96], &[0x03; 32]); // last_block_hash
        // chain_id = 4 in big-endian 32 bytes (last byte = 4).
        assert_eq!(encoded[127], 4);
        // transaction_count = 5 in big-endian 32 bytes (last byte = 5).
        assert_eq!(encoded[159], 5);
    }
}
