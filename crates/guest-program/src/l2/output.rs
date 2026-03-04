use ethrex_common::types::balance_diff::BalanceDiff;
use ethrex_common::{H256, U256};
use serde::{Deserialize, Serialize};

/// Output of the L2 stateless validation program.
#[derive(Serialize, Deserialize)]
pub struct ProgramOutput {
    /// Initial state trie root hash.
    pub initial_state_hash: H256,
    /// Final state trie root hash.
    pub final_state_hash: H256,
    /// Merkle root of all L1 output messages in a batch.
    pub l1_out_messages_merkle_root: H256,
    /// Rolling hash of all deposit transactions included in a batch.
    pub l1_in_messages_rolling_hash: H256,
    /// Rolling hash of all L2 in messages included in a batch (per chain ID).
    pub l2_in_message_rolling_hashes: Vec<(u64, H256)>,
    /// Blob commitment versioned hash.
    pub blob_versioned_hash: H256,
    /// Hash of the last block in the batch.
    pub last_block_hash: H256,
    /// Chain ID of the network.
    pub chain_id: U256,
    /// Number of non-privileged transactions in the batch.
    pub non_privileged_count: U256,
    /// Balance diffs for each chain ID.
    pub balance_diffs: Vec<BalanceDiff>,
}

impl ProgramOutput {
    /// Encode the output to bytes for commitment.
    pub fn encode(&self) -> Vec<u8> {
        let mut encoded = [
            self.initial_state_hash.to_fixed_bytes(),
            self.final_state_hash.to_fixed_bytes(),
            self.l1_out_messages_merkle_root.to_fixed_bytes(),
            self.l1_in_messages_rolling_hash.to_fixed_bytes(),
            self.blob_versioned_hash.to_fixed_bytes(),
            self.last_block_hash.to_fixed_bytes(),
            self.chain_id.to_big_endian(),
            self.non_privileged_count.to_big_endian(),
        ]
        .concat();

        for balance_diff in &self.balance_diffs {
            encoded.extend_from_slice(&balance_diff.chain_id.to_big_endian());
            encoded.extend_from_slice(&balance_diff.value.to_big_endian());
            for value_per_token in &balance_diff.value_per_token {
                encoded.extend_from_slice(&value_per_token.token_l1.to_fixed_bytes());
                encoded.extend_from_slice(&value_per_token.token_src_l2.to_fixed_bytes());
                encoded.extend_from_slice(&value_per_token.token_dst_l2.to_fixed_bytes());
                encoded.extend_from_slice(&value_per_token.value.to_big_endian());
            }
            encoded.extend(
                balance_diff
                    .message_hashes
                    .iter()
                    .flat_map(|h| h.to_fixed_bytes()),
            );
        }

        for (chain_id, hash) in &self.l2_in_message_rolling_hashes {
            encoded.extend_from_slice(&chain_id.to_be_bytes());
            encoded.extend_from_slice(&hash.to_fixed_bytes());
        }

        encoded
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify that the 8 fixed fields occupy exactly 256 bytes (8 × 32)
    /// and each field is at the expected byte offset.
    #[test]
    fn l2_encode_fixed_fields_layout() {
        let output = ProgramOutput {
            initial_state_hash: H256::from([0x01; 32]),
            final_state_hash: H256::from([0x02; 32]),
            l1_out_messages_merkle_root: H256::from([0x03; 32]),
            l1_in_messages_rolling_hash: H256::from([0x04; 32]),
            l2_in_message_rolling_hashes: vec![],
            blob_versioned_hash: H256::from([0x05; 32]),
            last_block_hash: H256::from([0x06; 32]),
            chain_id: U256::from(7u64),
            non_privileged_count: U256::from(8u64),
            balance_diffs: vec![],
        };
        let encoded = output.encode();
        // 8 fixed fields × 32 bytes = 256 bytes (no variable parts).
        assert_eq!(encoded.len(), 256);
        // Field positions: each 32 bytes apart.
        assert_eq!(&encoded[0..32], &[0x01; 32]); // initial_state_hash
        assert_eq!(&encoded[32..64], &[0x02; 32]); // final_state_hash
        assert_eq!(&encoded[64..96], &[0x03; 32]); // l1_out_messages_merkle_root
        assert_eq!(&encoded[96..128], &[0x04; 32]); // l1_in_messages_rolling_hash
        assert_eq!(&encoded[128..160], &[0x05; 32]); // blob_versioned_hash
        assert_eq!(&encoded[160..192], &[0x06; 32]); // last_block_hash
        // chain_id = 7, big-endian in 32 bytes
        assert_eq!(encoded[255 - 32], 7); // chain_id last byte of its slot
        // non_privileged_count = 8, big-endian in 32 bytes
        assert_eq!(encoded[255], 8); // non_privileged_count last byte
    }

    /// Verify encoding with balance diffs includes the variable-length portion.
    #[test]
    fn l2_encode_with_balance_diffs() {
        let output = ProgramOutput {
            initial_state_hash: H256::zero(),
            final_state_hash: H256::zero(),
            l1_out_messages_merkle_root: H256::zero(),
            l1_in_messages_rolling_hash: H256::zero(),
            l2_in_message_rolling_hashes: vec![],
            blob_versioned_hash: H256::zero(),
            last_block_hash: H256::zero(),
            chain_id: U256::zero(),
            non_privileged_count: U256::zero(),
            balance_diffs: vec![BalanceDiff {
                chain_id: U256::from(1u64),
                value: U256::from(100u64),
                value_per_token: vec![],
                message_hashes: vec![],
            }],
        };
        let encoded = output.encode();
        // 256 (fixed) + 32 (chain_id) + 32 (value) = 320
        assert_eq!(encoded.len(), 320);
    }

    /// Verify encoding with l2_in_message_rolling_hashes appends (u64 BE + H256) tuples.
    #[test]
    fn l2_encode_with_l2_message_hashes() {
        let output = ProgramOutput {
            initial_state_hash: H256::zero(),
            final_state_hash: H256::zero(),
            l1_out_messages_merkle_root: H256::zero(),
            l1_in_messages_rolling_hash: H256::zero(),
            l2_in_message_rolling_hashes: vec![
                (42u64, H256::from([0xAA; 32])),
                (99u64, H256::from([0xBB; 32])),
            ],
            blob_versioned_hash: H256::zero(),
            last_block_hash: H256::zero(),
            chain_id: U256::zero(),
            non_privileged_count: U256::zero(),
            balance_diffs: vec![],
        };
        let encoded = output.encode();
        // 256 (fixed) + 2 × (8 + 32) = 256 + 80 = 336
        assert_eq!(encoded.len(), 336);
        // First rolling hash entry: chain_id=42 at offset 256.
        let chain_id_bytes = &encoded[256..264];
        assert_eq!(u64::from_be_bytes(chain_id_bytes.try_into().unwrap()), 42);
        assert_eq!(&encoded[264..296], &[0xAA; 32]);
        // Second entry at offset 296.
        let chain_id_bytes2 = &encoded[296..304];
        assert_eq!(u64::from_be_bytes(chain_id_bytes2.try_into().unwrap()), 99);
        assert_eq!(&encoded[304..336], &[0xBB; 32]);
    }
}
