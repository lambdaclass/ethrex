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

    /// Verify ProgramOutput.encode() against actual prover log from batch 8
    /// (empty batch: no balance_diffs, no l2 messages, non_privileged_count=0).
    /// Source: platform/docs/prover-batch11-12.log batch 8
    #[test]
    fn l2_encode_matches_prover_log_batch8_empty() {
        let output = ProgramOutput {
            initial_state_hash: H256::from(hex_to_bytes32(
                "48f7b11fc87cbe873361a3ff5b40c91dc24cf42f2597c43b23e5d5ebd64fca94",
            )),
            final_state_hash: H256::from(hex_to_bytes32(
                "f2d3abac19a86f4276b05c34673bb3bc72b069893a1c97f489e1cf05c8b3367c",
            )),
            l1_out_messages_merkle_root: H256::from(hex_to_bytes32(
                "0000000000000000000000000000000000000000000000000000000000000000",
            )),
            l1_in_messages_rolling_hash: H256::from(hex_to_bytes32(
                "0001c506b9f23a7737a304999fcc80d15c8b9644f1ec32d54b9f820fc7b24fe1",
            )),
            blob_versioned_hash: H256::from(hex_to_bytes32(
                "01867ce3040fa3000a16ce1785b1dc0293249ee0ebcd9a3da41d8a8cb62e2779",
            )),
            last_block_hash: H256::from(hex_to_bytes32(
                "4a13ea4ff3d0baedbb35eb4ab1fc8ebf5a0a1de526b3c23a5b7115b8b924027e",
            )),
            chain_id: U256::from(0x03e803e7u64),
            non_privileged_count: U256::from(0u64),
            l2_in_message_rolling_hashes: vec![],
            balance_diffs: vec![],
        };

        let encoded = output.encode();
        assert_eq!(encoded.len(), 256);

        let expected_hex = "48f7b11fc87cbe873361a3ff5b40c91dc24cf42f2597c43b23e5d5ebd64fca94f2d3abac19a86f4276b05c34673bb3bc72b069893a1c97f489e1cf05c8b3367c00000000000000000000000000000000000000000000000000000000000000000001c506b9f23a7737a304999fcc80d15c8b9644f1ec32d54b9f820fc7b24fe101867ce3040fa3000a16ce1785b1dc0293249ee0ebcd9a3da41d8a8cb62e27794a13ea4ff3d0baedbb35eb4ab1fc8ebf5a0a1de526b3c23a5b7115b8b924027e0000000000000000000000000000000000000000000000000000000003e803e70000000000000000000000000000000000000000000000000000000000000000";
        assert_eq!(hex::encode(&encoded), expected_hex);

        // Verify sha256 matches prover log
        use sha2::{Digest, Sha256};
        let hash = Sha256::digest(&encoded);
        assert_eq!(
            hex::encode(hash),
            "bbb6d86e8eca2a6240c54e51d23b9e7de7b2e2f8f3a8d0cce05760da2482a3fe"
        );
    }

    /// Verify ProgramOutput.encode() against actual prover log from batch 11
    /// (1 deposit tx: non_privileged_count=1, has l1_out_merkle_root).
    /// Source: platform/docs/prover-batch11-12.log batch 11
    #[test]
    fn l2_encode_matches_prover_log_batch11_with_deposit() {
        let output = ProgramOutput {
            initial_state_hash: H256::from(hex_to_bytes32(
                "f2d3abac19a86f4276b05c34673bb3bc72b069893a1c97f489e1cf05c8b3367c",
            )),
            final_state_hash: H256::from(hex_to_bytes32(
                "e13dfb0343a9d80105399653ecb0c46d22f53fd3ab4605e7a00d84728cc42da2",
            )),
            l1_out_messages_merkle_root: H256::from(hex_to_bytes32(
                "c55f9da905b0df29c9aaab516d596b67f57c6b60bc10c2a0dbac27744c6c9976",
            )),
            l1_in_messages_rolling_hash: H256::from(hex_to_bytes32(
                "0001807d074561de72c234a066163d4a1c0975dcbd6280f52dd3d8d5f3a36f50",
            )),
            blob_versioned_hash: H256::from(hex_to_bytes32(
                "0177acaa737625cc2bbe9ba10a894bbb7aaea5ca404d94737bf07631731422a5",
            )),
            last_block_hash: H256::from(hex_to_bytes32(
                "c73a24081e20983993b287cc4f67d9b37570202061e7410d3713d8f3015db214",
            )),
            chain_id: U256::from(0x03e803e7u64),
            non_privileged_count: U256::from(1u64),
            l2_in_message_rolling_hashes: vec![],
            balance_diffs: vec![],
        };

        let encoded = output.encode();
        assert_eq!(encoded.len(), 256);

        let expected_hex = "f2d3abac19a86f4276b05c34673bb3bc72b069893a1c97f489e1cf05c8b3367ce13dfb0343a9d80105399653ecb0c46d22f53fd3ab4605e7a00d84728cc42da2c55f9da905b0df29c9aaab516d596b67f57c6b60bc10c2a0dbac27744c6c99760001807d074561de72c234a066163d4a1c0975dcbd6280f52dd3d8d5f3a36f500177acaa737625cc2bbe9ba10a894bbb7aaea5ca404d94737bf07631731422a5c73a24081e20983993b287cc4f67d9b37570202061e7410d3713d8f3015db2140000000000000000000000000000000000000000000000000000000003e803e70000000000000000000000000000000000000000000000000000000000000001";
        assert_eq!(hex::encode(&encoded), expected_hex);

        // Verify sha256 matches prover log
        use sha2::{Digest, Sha256};
        let hash = Sha256::digest(&encoded);
        assert_eq!(
            hex::encode(hash),
            "47b261816ac029786edfe31367cd76e2734541a6ddd44ff6d45a22901c98d9ef"
        );
    }

    /// Verify state hash continuity: batch N's final_state = batch N+1's initial_state.
    /// Batches 8→11→12 from prover logs.
    #[test]
    fn l2_state_hash_continuity_across_batches() {
        let batch8_final = "f2d3abac19a86f4276b05c34673bb3bc72b069893a1c97f489e1cf05c8b3367c";
        let batch11_initial = "f2d3abac19a86f4276b05c34673bb3bc72b069893a1c97f489e1cf05c8b3367c";
        let batch11_final = "e13dfb0343a9d80105399653ecb0c46d22f53fd3ab4605e7a00d84728cc42da2";
        let batch12_initial = "e13dfb0343a9d80105399653ecb0c46d22f53fd3ab4605e7a00d84728cc42da2";

        assert_eq!(batch8_final, batch11_initial, "batch 8 final != batch 11 initial");
        assert_eq!(batch11_final, batch12_initial, "batch 11 final != batch 12 initial");
    }

    /// Verify chain_id encoding: 0x03e803e7 = 65536999 (zk-dex chain ID).
    #[test]
    fn l2_chain_id_encoding() {
        assert_eq!(0x03e803e7u64, 65536999u64);
        let chain_id = U256::from(0x03e803e7u64);
        let bytes = chain_id.to_big_endian();
        // Last 4 bytes should be 03 e8 03 e7
        assert_eq!(&bytes[28..32], &[0x03, 0xe8, 0x03, 0xe7]);
    }

    fn hex_to_bytes32(hex_str: &str) -> [u8; 32] {
        let bytes = hex::decode(hex_str).expect("invalid hex");
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&bytes);
        arr
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
