use crate::{utils::RpcErr, RpcHandler};
use ethrex_common::{Address, H256, U256};
use serde::{Deserialize, Serialize};
use ssz_types::{typenum, VariableList};
use std::collections::HashMap;
use tree_hash::TreeHash;

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

impl RpcHandler for EnvV0 {
    fn parse(params: &Option<Vec<serde_json::Value>>) -> Result<Self, RpcErr> {
        tracing::info!("parsing env");

        let Some(params) = params else {
            return Err(RpcErr::InvalidEnv("Expected some params".to_string()));
        };

        let envelope: HashMap<String, serde_json::Value> = serde_json::from_value(
            params
                .first()
                .ok_or(RpcErr::InvalidEnv("Expected envelope".to_string()))?
                .clone(),
        )
        .map_err(|e| RpcErr::InvalidEnv(e.to_string()))?;

        // TODO: Parse and validate gateway's signature

        serde_json::from_value(
            envelope
                .get("message")
                .ok_or_else(|| RpcErr::InvalidEnv("Expected message".to_string()))?
                .clone(),
        )
        .map_err(|e| RpcErr::InvalidEnv(e.to_string()))
    }

    fn handle(
        &self,
        _context: crate::RpcApiContext,
    ) -> Result<serde_json::Value, crate::utils::RpcErr> {
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
        let mut hasher = tree_hash::MerkleHasher::with_leaves(10);
        hasher
            .write(self.number.tree_hash_root().as_slice())
            .expect("could not tree hash number");
        hasher
            .write(
                self.parent_hash
                    .as_fixed_bytes()
                    .tree_hash_root()
                    .as_slice(),
            )
            .expect("could not tree hash parent_hash");
        hasher
            .write(encode_address(&self.beneficiary).as_slice())
            .expect("could not tree hash beneficiary");
        hasher
            .write(self.timestamp.tree_hash_root().as_slice())
            .expect("could not tree hash timestamp");
        hasher
            .write(self.gas_limit.tree_hash_root().as_slice())
            .expect("could not tree hash gas_limit");
        hasher
            .write(self.basefee.tree_hash_root().as_slice())
            .expect("could not tree hash basefee");
        hasher
            .write(encode_u256(&self.difficulty).as_slice())
            .expect("could not tree hash difficulty");
        hasher
            .write(self.prevrandao.as_fixed_bytes().tree_hash_root().as_slice())
            .expect("could not tree hash prevrandao");
        hasher
            .write(self.extra_data.tree_hash_root().as_slice())
            .expect("could not tree hash extra_data");
        hasher
            .write(
                self.parent_beacon_block_root
                    .as_fixed_bytes()
                    .tree_hash_root()
                    .as_slice(),
            )
            .expect("could not tree hash parent_beacon_block_root");
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
