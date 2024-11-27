use ethrex_storage::error::StoreError;
use ethrex_vm::EvmError;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::authentication::AuthenticationError;
use ethrex_blockchain::error::MempoolError;

#[derive(Debug, Deserialize)]
pub enum RpcErr {
    MethodNotFound(String),
    WrongParam(String),
    BadParams(String),
    MissingParam(String),
    BadHexFormat(u64),
    UnsuportedFork(String),
    Internal(String),
    Vm(String),
    Revert { data: String },
    Halt { reason: String, gas_used: u64 },
    AuthenticationError(AuthenticationError),
    InvalidForkChoiceState(String),
    InvalidPayloadAttributes(String),
    UnknownPayload(String),
}

impl From<RpcErr> for RpcErrorMetadata {
    fn from(value: RpcErr) -> Self {
        match value {
            RpcErr::MethodNotFound(bad_method) => RpcErrorMetadata {
                code: -32601,
                data: None,
                message: format!("Method not found: {bad_method}"),
            },
            RpcErr::WrongParam(field) => RpcErrorMetadata {
                code: -32602,
                data: None,
                message: format!("Field '{}' is incorrect or has an unknown format", field),
            },
            RpcErr::BadParams(context) => RpcErrorMetadata {
                code: -32000,
                data: None,
                message: format!("Invalid params: {context}"),
            },
            RpcErr::MissingParam(parameter_name) => RpcErrorMetadata {
                code: -32000,
                data: None,
                message: format!("Expected parameter: {parameter_name} is missing"),
            },
            RpcErr::UnsuportedFork(context) => RpcErrorMetadata {
                code: -38005,
                data: None,
                message: format!("Unsupported fork: {context}"),
            },
            RpcErr::BadHexFormat(arg_number) => RpcErrorMetadata {
                code: -32602,
                data: None,
                message: format!("invalid argument {arg_number} : hex string without 0x prefix"),
            },
            RpcErr::Internal(context) => RpcErrorMetadata {
                code: -32603,
                data: None,
                message: format!("Internal Error: {context}"),
            },
            RpcErr::Vm(context) => RpcErrorMetadata {
                code: -32015,
                data: None,
                message: format!("Vm execution error: {context}"),
            },
            RpcErr::Revert { data } => RpcErrorMetadata {
                // This code (3) was hand-picked to match hive tests.
                // Could not find proper documentation about it.
                code: 3,
                data: Some(data.clone()),
                message: format!(
                    "execution reverted: {}",
                    get_message_from_revert_data(&data)
                ),
            },
            RpcErr::Halt { reason, gas_used } => RpcErrorMetadata {
                // Just copy the `Revert` error code.
                // Haven't found an example of this one yet.
                code: 3,
                data: None,
                message: format!("execution halted: reason={}, gas_used={}", reason, gas_used),
            },
            RpcErr::AuthenticationError(auth_error) => match auth_error {
                AuthenticationError::InvalidIssuedAtClaim => RpcErrorMetadata {
                    code: -32000,
                    data: None,
                    message: "Auth failed: Invalid iat claim".to_string(),
                },
                AuthenticationError::TokenDecodingError => RpcErrorMetadata {
                    code: -32000,
                    data: None,
                    message: "Auth failed: Invalid or missing token".to_string(),
                },
                AuthenticationError::MissingAuthentication => RpcErrorMetadata {
                    code: -32000,
                    data: None,
                    message: "Auth failed: Missing authentication header".to_string(),
                },
            },
            RpcErr::InvalidForkChoiceState(data) => RpcErrorMetadata {
                code: -38002,
                data: Some(data),
                message: "Invalid forkchoice state".to_string(),
            },
            RpcErr::InvalidPayloadAttributes(data) => RpcErrorMetadata {
                code: -38003,
                data: Some(data),
                message: "Invalid forkchoice state".to_string(),
            },
            RpcErr::UnknownPayload(context) => RpcErrorMetadata {
                code: -38001,
                data: None,
                message: format!("Unknown payload: {context}"),
            },
        }
    }
}

impl From<serde_json::Error> for RpcErr {
    fn from(error: serde_json::Error) -> Self {
        Self::BadParams(error.to_string())
    }
}

// TODO: Actually return different errors for each case
// here we are returning a BadParams error
impl From<MempoolError> for RpcErr {
    fn from(err: MempoolError) -> Self {
        match err {
            MempoolError::StoreError(err) => Self::Internal(err.to_string()),
            other_err => Self::BadParams(other_err.to_string()),
        }
    }
}

pub enum RpcNamespace {
    Engine,
    Eth,
    Admin,
    Debug,
    Web3,
    Net,
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
pub struct RpcErrorMetadata {
    pub code: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
    pub message: String,
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

/// Failure to read from DB will always constitute an internal error
impl From<StoreError> for RpcErr {
    fn from(value: StoreError) -> Self {
        RpcErr::Internal(value.to_string())
    }
}

impl From<EvmError> for RpcErr {
    fn from(value: EvmError) -> Self {
        RpcErr::Vm(value.to_string())
    }
}

fn get_message_from_revert_data(_data: &str) -> String {
    // TODO
    // Hive tests are not failing when revert message does not match, but currently it is not matching
    // It should be fixed
    // See https://github.com/ethereum/go-ethereum/blob/8fd43c80132434dca896d8ae5004ae2aac1450d3/accounts/abi/abi.go#L275
    "".to_owned()
}

pub fn parse_json_hex(hex: &serde_json::Value) -> Result<u64, String> {
    if let Value::String(maybe_hex) = hex {
        let trimmed = maybe_hex.trim_start_matches("0x");
        let maybe_parsed = u64::from_str_radix(trimmed, 16);
        maybe_parsed.map_err(|_| format!("Could not parse given hex {}", maybe_hex))
    } else {
        Err(format!("Could not parse given hex {}", hex))
    }
}

#[cfg(test)]
pub mod test_utils {
    use std::{net::SocketAddr, str::FromStr};

    use ethrex_core::H512;
    use ethrex_net::types::Node;
    use ethrex_storage::{EngineType, Store};

    use crate::start_api;

    pub const TEST_GENESIS: &str = include_str!("../../../test_data/genesis-l1.json");
    pub fn example_p2p_node() -> Node {
        let node_id_1 = H512::from_str("d860a01f9722d78051619d1e2351aba3f43f943f6f00718d1b9baa4101932a1f5011f16bb2b1bb35db20d6fe28fa0bf09636d26a87d31de9ec6203eeedb1f666").unwrap();
        Node {
            ip: "127.0.0.1".parse().unwrap(),
            udp_port: 30303,
            tcp_port: 30303,
            node_id: node_id_1,
        }
    }

    // Util to start an api for testing on ports 8500 and 8501,
    // mostly for when hive is missing some endpoints to test
    // like eth_uninstallFilter.
    // Here's how you would use it:
    // ```
    // let server_handle = tokio::spawn(async move { start_stest_api().await })
    // ...
    // assert!(something_that_needs_the_server)
    // ...
    // server_handle.abort()
    // ```
    pub async fn start_test_api() {
        let http_addr: SocketAddr = "127.0.0.1:8500".parse().unwrap();
        let authrpc_addr: SocketAddr = "127.0.0.1:8501".parse().unwrap();
        let storage =
            Store::new("", EngineType::InMemory).expect("Failed to create in-memory storage");
        storage
            .add_initial_state(serde_json::from_str(TEST_GENESIS).unwrap())
            .expect("Failed to build test genesis");

        let jwt_secret = Default::default();
        let local_p2p_node = example_p2p_node();
        start_api(http_addr, authrpc_addr, storage, jwt_secret, local_p2p_node).await;
    }
}
