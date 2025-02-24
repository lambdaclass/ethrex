use crate::{utils::RpcErr, RpcHandler};
use serde::{Deserialize, Serialize};
use ssz_types::{typenum, VariableList};
use std::collections::HashMap;
use tree_hash_derive::TreeHash;

pub type MaxBytesPerTransaction = typenum::U1073741824;
pub type MaxTransactionsPerPayload = typenum::U1048576;
pub type Transaction = VariableList<u8, MaxBytesPerTransaction>;
pub type Transactions = VariableList<Transaction, MaxTransactionsPerPayload>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TreeHash)]
#[serde(rename_all = "camelCase")]
pub struct FragV0 {
    /// Block in which this frag will be included
    pub block_number: u64,
    /// Index of this frag. Frags need to be applied sequentially by index, up to [`SealV0::total_frags`]
    pub seq: u64,
    /// Whether this is the last frag in the sequence
    pub is_last: bool,
    /// Ordered list of EIP-2718 encoded transactions
    #[serde(with = "ssz_types::serde_utils::list_of_hex_var_list")]
    pub txs: Transactions,
}

impl RpcHandler for FragV0 {
    fn parse(params: &Option<Vec<serde_json::Value>>) -> Result<Self, crate::utils::RpcErr> {
        tracing::info!("parsing frag");

        let Some(params) = params else {
            return Err(RpcErr::InvalidFrag("Expected some params".to_string()));
        };

        let envelope: HashMap<String, serde_json::Value> = serde_json::from_value(
            params
                .first()
                .ok_or(RpcErr::InvalidFrag("Expected envelope".to_string()))?
                .clone(),
        )
        .map_err(|e| RpcErr::InvalidFrag(e.to_string()))?;

        // TODO: Parse and validate gateway's signature

        serde_json::from_value(
            envelope
                .get("message")
                .ok_or_else(|| RpcErr::InvalidFrag("Expected message".to_string()))?
                .clone(),
        )
        .map_err(|e| RpcErr::InvalidFrag(e.to_string()))
    }

    fn handle(
        &self,
        _context: crate::RpcApiContext,
    ) -> Result<serde_json::Value, crate::utils::RpcErr> {
        tracing::info!("handling frag");
        Ok(serde_json::Value::Null)
    }
}
