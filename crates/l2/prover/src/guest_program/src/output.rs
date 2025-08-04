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
}

impl ProgramOutput {
    pub fn encode(&self) -> Vec<u8> {
        [
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
        ]
        .concat()
    }
}
