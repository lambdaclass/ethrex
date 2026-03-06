use ethrex_rpc::utils::RpcErrorMetadata;
use ethrex_storage::error::StoreError;
use ethrex_storage_rollup::RollupStoreError;

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

impl From<ethrex_crypto::CryptoError> for RpcErr {
    fn from(error: ethrex_crypto::CryptoError) -> Self {
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
