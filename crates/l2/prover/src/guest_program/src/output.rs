#[cfg(feature = "l2")]
use ethrex_common::types::balance_diff::BalanceDiff;
use ethrex_common::{H256, U256};
use serde::{Deserialize, Serialize};

/// Public output variables exposed by the zkVM execution program. Some of these are part of
/// the program input.
#[derive(Serialize, Deserialize)]
pub struct ProgramOutput {
    /// initial state trie root hash
    pub initial_state_hash: H256,
    /// final state trie root hash
    pub final_state_hash: H256,
    #[cfg(feature = "l2")]
    /// merkle root of all L1 output messages in a batch
    pub l1_out_messages_merkle_root: H256,
    #[cfg(feature = "l2")]
    /// merkle root of all L2 output messages in a batch
    pub l2_out_messages_merkle_root: H256,
    #[cfg(feature = "l2")]
    /// hash of all the deposit transactions included in a batch
    pub l1_in_message_hash: H256,
    #[cfg(feature = "l2")]
    /// rolling hash of all L2 in messages included in a batch
    pub l2_in_message_rolling_hashes: Vec<(u64, H256)>,
    #[cfg(feature = "l2")]
    /// blob commitment versioned hash
    pub blob_versioned_hash: H256,
    /// hash of the last block in a batch
    pub last_block_hash: H256,
    /// chain_id of the network
    pub chain_id: U256,
    /// amount of non-privileged transactions
    pub non_privileged_count: U256,
    #[cfg(feature = "l2")]
    /// balance diffs for each chain id
    pub balance_diffs: Vec<BalanceDiff>,
}

impl ProgramOutput {
    pub fn encode(&self) -> Vec<u8> {
        let mut encoded = [
            self.initial_state_hash.to_fixed_bytes(),
            self.final_state_hash.to_fixed_bytes(),
            #[cfg(feature = "l2")]
            self.l1_out_messages_merkle_root.to_fixed_bytes(),
            #[cfg(feature = "l2")]
            self.l1_in_message_hash.to_fixed_bytes(),
            #[cfg(feature = "l2")]
            self.blob_versioned_hash.to_fixed_bytes(),
            self.last_block_hash.to_fixed_bytes(),
            self.chain_id.to_big_endian(),
            self.non_privileged_count.to_big_endian(),
            #[cfg(feature = "l2")]
            self.l2_out_messages_merkle_root.to_fixed_bytes(),
        ]
        .concat();
        #[cfg(feature = "l2")]
        for diff in &self.balance_diffs {
            encoded.extend_from_slice(&diff.chain_id.to_big_endian());
            encoded.extend_from_slice(&diff.value.to_big_endian());
            encoded.extend(diff.message_hashes.iter().flat_map(|h| h.to_fixed_bytes()));
        }

        for (chain_id, hash) in &self.l2_in_message_rolling_hashes {
            encoded.extend_from_slice(&chain_id.to_be_bytes());
            encoded.extend_from_slice(&hash.to_fixed_bytes());
        }

        encoded
    }
}
