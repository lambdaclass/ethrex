use crate::{utils::RpcErr, RpcHandler};
use ethrex_common::H256;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
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

impl RpcHandler for SealV0 {
    fn parse(params: &Option<Vec<serde_json::Value>>) -> Result<Self, crate::utils::RpcErr> {
        tracing::info!("parsing seal");

        let Some(params) = params else {
            return Err(RpcErr::InvalidSeal("Expected some params".to_string()));
        };

        let envelope: HashMap<String, serde_json::Value> = serde_json::from_value(
            params
                .first()
                .ok_or(RpcErr::InvalidSeal("Expected envelope".to_string()))?
                .clone(),
        )
        .map_err(|e| RpcErr::InvalidSeal(e.to_string()))?;

        // TODO: Parse and validate gateway's signature

        serde_json::from_value(
            envelope
                .get("message")
                .ok_or_else(|| RpcErr::InvalidSeal("Expected message".to_string()))?
                .clone(),
        )
        .map_err(|e| RpcErr::InvalidSeal(e.to_string()))
    }

    fn handle(
        &self,
        _context: crate::RpcApiContext,
    ) -> Result<serde_json::Value, crate::utils::RpcErr> {
        tracing::info!("handling seal");
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
        let mut hasher = tree_hash::MerkleHasher::with_leaves(10);
        hasher
            .write(self.total_frags.tree_hash_root().as_slice())
            .expect("could not tree hash total_frags");
        hasher
            .write(self.block_number.tree_hash_root().as_slice())
            .expect("could not tree hash block_number");
        hasher
            .write(self.gas_used.tree_hash_root().as_slice())
            .expect("could not tree hash gas_used");
        hasher
            .write(self.gas_limit.tree_hash_root().as_slice())
            .expect("could not tree hash gas_limit");
        hasher
            .write(
                self.parent_hash
                    .as_fixed_bytes()
                    .tree_hash_root()
                    .as_slice(),
            )
            .expect("could not tree hash parent_hash");
        hasher
            .write(
                self.transactions_root
                    .as_fixed_bytes()
                    .tree_hash_root()
                    .as_slice(),
            )
            .expect("could not tree hash transactions_root");
        hasher
            .write(
                self.receipts_root
                    .as_fixed_bytes()
                    .tree_hash_root()
                    .as_slice(),
            )
            .expect("could not tree hash receipts_root");
        hasher
            .write(self.state_root.as_fixed_bytes().tree_hash_root().as_slice())
            .expect("could not tree hash state_root");
        hasher
            .write(self.block_hash.as_fixed_bytes().tree_hash_root().as_slice())
            .expect("could not tree hash block_hash");
        hasher.finish().expect("could not finish tree hash")
    }
}
