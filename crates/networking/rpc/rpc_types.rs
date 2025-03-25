use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::errors::{RpcErr, RpcErrorMetadata};

pub enum RpcNamespace {
    Engine,
    Eth,
    Admin,
    Debug,
    Web3,
    Net,
    #[cfg(feature = "l2")]
    EthrexL2,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RpcRequestId {
    Number(u64),
    String(String),
}

#[derive(Serialize, Deserialize, Debug)]
pub struct RpcRequest {
    pub id: RpcRequestId,
    pub jsonrpc: String,
    pub method: String,
    pub params: Option<Vec<Value>>,
}

impl RpcRequest {
    pub fn namespace(&self) -> Result<RpcNamespace, RpcErr> {
        let mut parts = self.method.split('_');
        if let Some(namespace) = parts.next() {
            match namespace {
                "engine" => Ok(RpcNamespace::Engine),
                "eth" => Ok(RpcNamespace::Eth),
                "admin" => Ok(RpcNamespace::Admin),
                "debug" => Ok(RpcNamespace::Debug),
                "web3" => Ok(RpcNamespace::Web3),
                "net" => Ok(RpcNamespace::Net),
                #[cfg(feature = "l2")]
                "ethrex" => Ok(RpcNamespace::EthrexL2),
                _ => Err(RpcErr::MethodNotFound(self.method.clone())),
            }
        } else {
            Err(RpcErr::MethodNotFound(self.method.clone()))
        }
    }
}

impl Default for RpcRequest {
    fn default() -> Self {
        RpcRequest {
            id: RpcRequestId::Number(1),
            jsonrpc: "2.0".to_string(),
            method: "".to_string(),
            params: None,
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct RpcSuccessResponse {
    pub id: RpcRequestId,
    pub jsonrpc: String,
    pub result: Value,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct RpcErrorResponse {
    pub id: RpcRequestId,
    pub jsonrpc: String,
    pub error: RpcErrorMetadata,
}