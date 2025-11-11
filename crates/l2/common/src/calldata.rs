use ethrex_common::{Address, Bytes, U256};
use serde::{Deserialize, Serialize};

/// Struct representing the possible solidity types for function arguments
/// - `Uint` -> `uint256`
/// - `Address` -> `address`
/// - `Bool` -> `bool`
/// - `Bytes` -> `bytes`
/// - `String` -> `string`
/// - `Array` -> `T[]`
/// - `Tuple` -> `(X_1, ..., X_k)`
/// - `FixedArray` -> `T[k]`
/// - `FixedBytes` -> `bytesN`
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
pub enum Value {
    Address(Address),
    Uint(U256),
    Int(U256),
    Bool(bool),
    Bytes(Bytes),
    String(String),
    Array(Vec<Value>),
    Tuple(Vec<Value>),
    FixedArray(Vec<Value>),
    FixedBytes(Bytes),
}
