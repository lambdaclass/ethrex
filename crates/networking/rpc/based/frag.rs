use crate::{utils::RpcErr, RpcApiContext};
use serde::{Deserialize, Serialize};
use ssz_types::{typenum, VariableList};
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
    #[serde(rename = "seq")]
    pub sequence: u64,
    /// Whether this is the last frag in the sequence
    pub is_last: bool,
    /// Ordered list of EIP-2718 encoded transactions
    #[serde(rename = "txs", with = "ssz_types::serde_utils::list_of_hex_var_list")]
    pub transactions: Transactions,
}

impl FragV0 {
    pub fn handle(&self, _context: RpcApiContext) -> Result<serde_json::Value, RpcErr> {
        tracing::debug!("handling frag");
        Ok(serde_json::Value::Null)
    }
}
