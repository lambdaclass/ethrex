use ethrex_common::H256;
use ethrex_rpc::utils::RpcErrorMetadata;
use ethrex_storage::error::StoreError;
use ethrex_storage_rollup::RollupStoreError;
use keccak_hash::keccak;
use serde_json::Value;

#[derive(Debug, thiserror::Error)]
pub enum RpcErr {
    #[error("L1 RPC Error: {0}")]
    L1RpcErr(#[from] ethrex_rpc::RpcErr),
    #[error("Internal Error: {0}")]
    Internal(String),
    #[error("Invalid ethrex L2 message: {0}")]
    InvalidEthrexL2Message(String),
}

impl From<RpcErr> for RpcErrorMetadata {
    fn from(value: RpcErr) -> Self {
        match value {
            RpcErr::L1RpcErr(l1_rpc_err) => l1_rpc_err.into(),
            RpcErr::Internal(context) => RpcErrorMetadata {
                code: -32603,
                data: None,
                message: format!("Internal Error: {context}"),
            },
            RpcErr::InvalidEthrexL2Message(reason) => RpcErrorMetadata {
                code: -39000,
                data: None,
                message: format!("Invalid Ethex L2 message: {reason}",),
            },
        }
    }
}

impl From<serde_json::Error> for RpcErr {
    fn from(error: serde_json::Error) -> Self {
        Self::L1RpcErr(error.into())
    }
}

impl From<secp256k1::Error> for RpcErr {
    fn from(error: secp256k1::Error) -> Self {
        Self::L1RpcErr(error.into())
    }
}

pub enum RpcNamespace {
    L1RpcNamespace(ethrex_rpc::RpcNamespace),
    EthrexL2,
}

pub fn resolve_namespace(method: &str) -> Result<RpcNamespace, RpcErr> {
    let maybe_namespace =
        method
            .split('_')
            .next()
            .ok_or(RpcErr::L1RpcErr(ethrex_rpc::RpcErr::MethodNotFound(
                method.to_string(),
            )))?;
    match maybe_namespace {
        "ethrex" => Ok(RpcNamespace::EthrexL2),
        _ => ethrex_rpc::utils::resolve_namespace(maybe_namespace, method.to_string())
            .map(RpcNamespace::L1RpcNamespace)
            .map_err(RpcErr::L1RpcErr),
    }
}

/// Failure to read from DB will always constitute an internal error
impl From<StoreError> for RpcErr {
    fn from(value: StoreError) -> Self {
        RpcErr::Internal(value.to_string())
    }
}

impl From<RollupStoreError> for RpcErr {
    fn from(value: RollupStoreError) -> Self {
        RpcErr::Internal(value.to_string())
    }
}

pub fn parse_json_hex(hex: &serde_json::Value) -> Result<u64, String> {
    let Value::String(maybe_hex) = hex else {
        return Err(format!("Could not parse given hex {hex}"));
    };
    let trimmed = maybe_hex.trim_start_matches("0x");
    let maybe_parsed = u64::from_str_radix(trimmed, 16);
    maybe_parsed.map_err(|_| format!("Could not parse given hex {maybe_hex}"))
}

pub fn merkle_proof(data: Vec<H256>, mut index: usize) -> Option<Vec<H256>> {
    if index >= data.len() {
        return None;
    }

    let mut proof = vec![];
    let mut current = data.clone();
    let mut first = true;
    while current.len() > 1 || first {
        first = false;
        proof.push(*current.get(index ^ 1).or(current.get(index))?);
        index /= 2;
        current = current
            .chunks(2)
            .map(|chunk| -> H256 {
                let left = *chunk.first().unwrap_or(&H256::zero());
                let right = *chunk.get(1).unwrap_or(&left);
                keccak([left.as_bytes(), right.as_bytes()].concat())
                    .as_fixed_bytes()
                    .into()
            })
            .collect();
    }
    Some(proof)
}
