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
