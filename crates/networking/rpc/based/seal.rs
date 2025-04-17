use crate::{rpc::RpcApiContext, utils::RpcErr};
use ethrex_common::H256;
use serde::{Deserialize, Serialize};
use tree_hash::TreeHash;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SealV0 {
    /// How many frags for this block were in this sequence
    pub total_frags: u64,
    // Header fields
    pub block_number: u64,
    pub gas_used: u64,
    pub gas_limit: u64,
    pub parent_hash: H256,
    pub transactions_root: H256,
    pub receipts_root: H256,
    pub state_root: H256,
    pub block_hash: H256,
}

impl SealV0 {
    pub fn handle(&self, _context: RpcApiContext) -> Result<serde_json::Value, RpcErr> {
        tracing::debug!("handling seal");
        Ok(serde_json::Value::Null)
    }
}

impl TreeHash for SealV0 {
    fn tree_hash_type() -> tree_hash::TreeHashType {
        tree_hash::TreeHashType::Container
    }

    fn tree_hash_packed_encoding(&self) -> tree_hash::PackedEncoding {
        unreachable!("Struct should never be packed.")
    }

    fn tree_hash_packing_factor() -> usize {
        unreachable!("Struct should never be packed.")
    }

    fn tree_hash_root(&self) -> tree_hash::Hash256 {
        let total_frags = self.total_frags.tree_hash_root();
        let block_number = self.block_number.tree_hash_root();
        let gas_used = self.gas_used.tree_hash_root();
        let gas_limit = self.gas_limit.tree_hash_root();
        let parent_hash = self.parent_hash.as_fixed_bytes().tree_hash_root();
        let transactions_root = self.transactions_root.as_fixed_bytes().tree_hash_root();
        let receipts_root = self.receipts_root.as_fixed_bytes().tree_hash_root();
        let state_root = self.state_root.as_fixed_bytes().tree_hash_root();
        let block_hash = self.block_hash.as_fixed_bytes().tree_hash_root();

        let leaves = [
            total_frags.as_slice(),
            block_number.as_slice(),
            gas_used.as_slice(),
            gas_limit.as_slice(),
            parent_hash.as_slice(),
            transactions_root.as_slice(),
            receipts_root.as_slice(),
            state_root.as_slice(),
            block_hash.as_slice(),
        ];
        let mut hasher = tree_hash::MerkleHasher::with_leaves(leaves.len());

        // PANIC: the following `expect`s would only fail if we exceed the declared
        // number of leaves, which is impossible by construction
        // See https://docs.rs/tree_hash/0.9.1/tree_hash/struct.MerkleHasher.html#method.write
        for leaf in leaves {
            hasher.write(leaf).expect("could not hash leaf");
        }

        hasher.finish().expect("could not finish tree hash")
    }
}
