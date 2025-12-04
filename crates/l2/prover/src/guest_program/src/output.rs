#[cfg(feature = "l2")]
use ethrex_common::Address;
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
    /// merkle root of all messages in a batch
    pub l1messages_merkle_root: H256,
    #[cfg(feature = "l2")]
    /// hash of all the privileged transactions made in a batch
    pub privileged_transactions_hash: H256,
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
    /// merkle root of all l2 messages in a batch
    pub l2messages_merkle_root: H256,
    #[cfg(feature = "l2")]
    /// balance diffs for each chain id
    pub balance_diffs: Vec<(U256, Vec<(Address, Address, Address, U256)>)>,
}

impl ProgramOutput {
    pub fn encode(&self) -> Vec<u8> {
        let mut encoded = [
            self.initial_state_hash.to_fixed_bytes(),
            self.final_state_hash.to_fixed_bytes(),
            #[cfg(feature = "l2")]
            self.l1messages_merkle_root.to_fixed_bytes(),
            #[cfg(feature = "l2")]
            self.privileged_transactions_hash.to_fixed_bytes(),
            #[cfg(feature = "l2")]
            self.blob_versioned_hash.to_fixed_bytes(),
            self.last_block_hash.to_fixed_bytes(),
            self.chain_id.to_big_endian(),
            self.non_privileged_count.to_big_endian(),
            #[cfg(feature = "l2")]
            self.l2messages_merkle_root.to_fixed_bytes(),
        ]
        .concat();

        #[cfg(feature = "l2")]
        {
            for (chain_id, balance_diff) in &self.balance_diffs {
                encoded.extend_from_slice(&chain_id.to_big_endian());
                for &(token_l1, token_l2, other_chain_token_l2, amount) in balance_diff {
                    encoded.extend_from_slice(&[0u8; 12]);
                    encoded.extend_from_slice(&token_l1.to_fixed_bytes());
                    encoded.extend_from_slice(&[0u8; 12]);
                    encoded.extend_from_slice(&token_l2.to_fixed_bytes());
                    encoded.extend_from_slice(&[0u8; 12]);
                    encoded.extend_from_slice(&other_chain_token_l2.to_fixed_bytes());
                    encoded.extend_from_slice(&amount.to_big_endian());
                }
            }
        }
        encoded
    }
}
