use ethrex_common::{H256, U256};
use serde::{Deserialize, Serialize};

/// Public output variables exposed by the zkVM execution program. Some of these are part of
/// the program input.
#[cfg(feature = "l2")]
#[derive(Serialize, Deserialize)]
pub struct ProgramOutput {
    /// initial state trie root hash
    pub initial_state_hash: H256,
    /// final state trie root hash
    pub final_state_hash: H256,
    /// merkle root of all messages in a batch
    pub l1messages_merkle_root: H256,
    /// hash of all the privileged transactions made in a batch
    pub privileged_transactions_hash: H256,
    /// blob commitment versioned hash
    pub blob_versioned_hash: H256,
    /// hash of the last block in a batch
    pub last_block_hash: H256,
    /// chain_id of the network
    pub chain_id: U256,
    /// amount of non-privileged transactions
    pub non_privileged_count: U256,
}

#[cfg(feature = "l2")]
impl ProgramOutput {
    pub fn encode(&self) -> Vec<u8> {
        [
            self.initial_state_hash.to_fixed_bytes(),
            self.final_state_hash.to_fixed_bytes(),
            self.l1messages_merkle_root.to_fixed_bytes(),
            self.privileged_transactions_hash.to_fixed_bytes(),
            self.blob_versioned_hash.to_fixed_bytes(),
            self.last_block_hash.to_fixed_bytes(),
            self.chain_id.to_big_endian(),
            self.non_privileged_count.to_big_endian(),
        ]
        .concat()
    }
}

/// Public output variables exposed by the zkVM execution program. Some of these are part of
/// the program input.
#[cfg(not(feature = "l2"))]
#[derive(Serialize, Deserialize)]
pub struct ProgramOutput {
    /// initial state trie root hash
    pub initial_state_hash: H256,
    /// final state trie root hash
    pub final_state_hash: H256,
    /// hash of the last block in a batch
    pub last_block_hash: H256,
    /// chain_id of the network
    pub chain_id: U256,
    /// amount of non-privileged transactions
    pub non_privileged_count: U256,
}

#[cfg(not(feature = "l2"))]
impl ProgramOutput {
    pub fn encode(&self) -> Vec<u8> {
        [
            self.initial_state_hash.to_fixed_bytes(),
            self.final_state_hash.to_fixed_bytes(),
            self.last_block_hash.to_fixed_bytes(),
            self.chain_id.to_big_endian(),
            self.non_privileged_count.to_big_endian(),
        ]
        .concat()
    }
}
