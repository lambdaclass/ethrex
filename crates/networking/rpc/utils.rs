use ethrex_common::H256;
use ethrex_storage::error::StoreError;
use ethrex_vm::EvmError;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::authentication::AuthenticationError;
use ethrex_blockchain::error::MempoolError;

#[cfg(feature = "l2")]
use ethrex_storage_rollup::RollupStoreError;

#[derive(Debug, Deserialize)]
pub enum RpcErr {
    MethodNotFound(String),
    WrongParam(String),
    BadParams(String),
    MissingParam(String),
    TooLargeRequest,
    BadHexFormat(u64),
    UnsuportedFork(String),
    Internal(String),
    Vm(String),
    Revert {
        data: String,
    },
    Halt {
        reason: String,
        gas_used: u64,
    },
    AuthenticationError(AuthenticationError),
    InvalidForkChoiceState(String),
    InvalidPayloadAttributes(String),
    UnknownPayload(String),
    #[cfg(feature = "l2")]
    InvalidEthrexL2Message(String),
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
            RpcErr::TooLargeRequest => RpcErrorMetadata {
                code: -38004,
                data: None,
                message: "Too large request".to_string(),
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
            #[cfg(feature = "l2")]
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

impl From<secp256k1::Error> for RpcErr {
    fn from(err: secp256k1::Error) -> Self {
        Self::Internal(format!("Cryptography error: {err}"))
    }
}

pub enum RpcNamespace {
    Engine,
    Eth,
    Admin,
    Debug,
    Web3,
    Net,
    Mempool,
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
                // TODO: The namespace is set to match geth's namespace for compatibility, consider changing it in the future
                "txpool" => Ok(RpcNamespace::Mempool),
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

#[cfg(feature = "l2")]
impl From<RollupStoreError> for RpcErr {
    fn from(value: RollupStoreError) -> Self {
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

pub fn merkle_proof(data: Vec<H256>, mut index: usize) -> Option<Vec<H256>> {
    if index >= data.len() {
        return None;
    }
    use keccak_hash::keccak;

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

#[cfg(test)]
pub mod test_utils {
    use std::{net::SocketAddr, str::FromStr, sync::Arc};

    use ethrex_blockchain::Blockchain;
    use ethrex_common::H512;
    use ethrex_p2p::{
        peer_handler::PeerHandler,
        sync_manager::SyncManager,
        types::{Node, NodeRecord},
    };
    use ethrex_storage::{EngineType, Store};
    use k256::ecdsa::SigningKey;
    use tokio::sync::Mutex as TokioMutex;

    use crate::{
        eth::gas_tip_estimator::GasTipEstimator,
        rpc::{NodeData, RpcApiContext, start_api},
    };
    #[cfg(feature = "l2")]
    use ethrex_storage_rollup::{EngineTypeRollup, StoreRollup};
    #[cfg(feature = "l2")]
    use secp256k1::{SecretKey, rand};

    pub const TEST_GENESIS: &str = include_str!("../../../test_data/genesis-l1.json");
    pub fn example_p2p_node() -> Node {
        let public_key_1 = H512::from_str("d860a01f9722d78051619d1e2351aba3f43f943f6f00718d1b9baa4101932a1f5011f16bb2b1bb35db20d6fe28fa0bf09636d26a87d31de9ec6203eeedb1f666").unwrap();
        Node::new("127.0.0.1".parse().unwrap(), 30303, 30303, public_key_1)
    }

    pub fn example_local_node_record() -> NodeRecord {
        let public_key_1 = H512::from_str("d860a01f9722d78051619d1e2351aba3f43f943f6f00718d1b9baa4101932a1f5011f16bb2b1bb35db20d6fe28fa0bf09636d26a87d31de9ec6203eeedb1f666").unwrap();
        let node = Node::new("127.0.0.1".parse().unwrap(), 30303, 30303, public_key_1);
        let signer = SigningKey::random(&mut rand::rngs::OsRng);

        NodeRecord::from_node(&node, 1, &signer).unwrap()
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
            .await
            .expect("Failed to build test genesis");
        let blockchain = Arc::new(Blockchain::default_with_store(storage.clone()));
        let jwt_secret = Default::default();
        let local_p2p_node = example_p2p_node();
        #[cfg(feature = "l2")]
        let valid_delegation_addresses = Vec::new();
        #[cfg(feature = "l2")]
        let sponsor_pk = SecretKey::new(&mut rand::thread_rng());
        #[cfg(feature = "l2")]
        let rollup_store = StoreRollup::new("", EngineTypeRollup::InMemory)
            .expect("Failed to create in-memory storage");
        start_api(
            http_addr,
            authrpc_addr,
            storage,
            blockchain,
            jwt_secret,
            local_p2p_node,
            example_local_node_record(),
            SyncManager::dummy(),
            PeerHandler::dummy(),
            "ethrex/test".to_string(),
            #[cfg(feature = "l2")]
            valid_delegation_addresses,
            #[cfg(feature = "l2")]
            sponsor_pk,
            #[cfg(feature = "l2")]
            rollup_store,
        )
        .await
        .unwrap();
    }

    pub async fn default_context_with_storage(storage: Store) -> RpcApiContext {
        let blockchain = Arc::new(Blockchain::default_with_store(storage.clone()));
        RpcApiContext {
            storage,
            blockchain,
            active_filters: Default::default(),
            syncer: Arc::new(SyncManager::dummy()),
            peer_handler: PeerHandler::dummy(),
            node_data: NodeData {
                jwt_secret: Default::default(),
                local_p2p_node: example_p2p_node(),
                local_node_record: example_local_node_record(),
                client_version: "ethrex/test".to_string(),
            },
            gas_tip_estimator: Arc::new(TokioMutex::new(GasTipEstimator::new())),
            #[cfg(feature = "l2")]
            valid_delegation_addresses: Vec::new(),
            #[cfg(feature = "l2")]
            sponsor_pk: SecretKey::new(&mut rand::thread_rng()),
            #[cfg(feature = "l2")]
            rollup_store: StoreRollup::new("test-store", EngineTypeRollup::InMemory)
                .expect("Fail to create in-memory db test"),
        }
    }
}
