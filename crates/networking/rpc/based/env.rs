use ethrex_common::{Address, H256, U256};
use serde::{Deserialize, Serialize};
use ssz_types::{typenum, VariableList};
use tree_hash::TreeHash;

use crate::{utils::RpcErr, RpcApiContext};

pub type MaxExtraDataSize = typenum::U256;
pub type ExtraData = VariableList<u8, MaxExtraDataSize>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvV0 {
    pub number: u64,
    pub parent_hash: H256,
    pub beneficiary: Address,
    pub timestamp: u64,
    pub gas_limit: u64,
    #[serde(rename = "baseFee")]
    pub basefee: u64,
    pub difficulty: U256,
    pub prevrandao: H256,
    #[serde(with = "ssz_types::serde_utils::hex_var_list")]
    pub extra_data: ExtraData,
    pub parent_beacon_block_root: H256,
}

impl EnvV0 {
    pub fn handle(&self, _context: RpcApiContext) -> Result<serde_json::Value, RpcErr> {
        tracing::info!("handling env");
        Ok(serde_json::Value::Null)
    }
}

impl TreeHash for EnvV0 {
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
        let number = self.number.tree_hash_root();
        let parent_hash = self.parent_hash.as_fixed_bytes().tree_hash_root();
        let beneficiary = encode_address(&self.beneficiary);
        let timestamp = self.timestamp.tree_hash_root();
        let gas_limit = self.gas_limit.tree_hash_root();
        let basefee = self.basefee.tree_hash_root();
        let difficulty = encode_u256(&self.difficulty);
        let prevrandao = self.prevrandao.as_fixed_bytes().tree_hash_root();
        let extra_data = self.extra_data.tree_hash_root();
        let parent_beacon_block_root = self
            .parent_beacon_block_root
            .as_fixed_bytes()
            .tree_hash_root();

        let leaves = [
            number.as_slice(),
            parent_hash.as_slice(),
            beneficiary.as_slice(),
            timestamp.as_slice(),
            gas_limit.as_slice(),
            basefee.as_slice(),
            difficulty.as_slice(),
            prevrandao.as_slice(),
            extra_data.as_slice(),
            parent_beacon_block_root.as_slice(),
        ];

        let mut hasher = tree_hash::MerkleHasher::with_leaves(10);

        // PANIC: the following `expect`s would only fail if we exceed the declared
        // number of leaves, which is impossible by construction
        // See https://docs.rs/tree_hash/0.9.1/tree_hash/struct.MerkleHasher.html#method.write
        for leaf in &leaves {
            hasher.write(leaf).expect("could not write leaf to hasher");
        }

        hasher.finish().expect("could not finish tree hash")
    }
}

fn encode_u256(value: &U256) -> tree_hash::Hash256 {
    tree_hash::Hash256::from(&value.to_little_endian())
}

fn encode_address(value: &Address) -> tree_hash::Hash256 {
    let mut result = [0; 32];
    result[0..20].copy_from_slice(value.as_bytes());
    tree_hash::Hash256::from_slice(&result)
}
