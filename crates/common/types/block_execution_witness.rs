use crate::serde_utils;
use serde::{Deserialize, Serialize};

/// RPC-friendly representation of an execution witness.
///
/// This is the format returned by the `debug_executionWitness` RPC method.
/// The trie nodes are pre-serialized to avoid expensive traversal on every RPC request.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RpcExecutionWitness {
    #[serde(
        serialize_with = "serde_utils::bytes::vec::serialize",
        deserialize_with = "serde_utils::bytes::vec::deserialize"
    )]
    pub state: Vec<bytes::Bytes>,
    #[serde(
        default,
        serialize_with = "serde_utils::bytes::vec::serialize",
        deserialize_with = "serde_utils::bytes::vec::deserialize"
    )]
    pub keys: Vec<bytes::Bytes>,
    #[serde(
        serialize_with = "serde_utils::bytes::vec::serialize",
        deserialize_with = "serde_utils::bytes::vec::deserialize"
    )]
    pub codes: Vec<bytes::Bytes>,
    #[serde(
        serialize_with = "serde_utils::bytes::vec::serialize",
        deserialize_with = "serde_utils::bytes::vec::deserialize"
    )]
    pub headers: Vec<bytes::Bytes>,
}
