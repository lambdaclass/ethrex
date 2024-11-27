use crate::authentication::authenticate;
use axum::{routing::post, Json, Router};
use axum_extra::{
    headers::{authorization::Bearer, Authorization},
    TypedHeader,
};
use bytes::Bytes;
use engine::{
    exchange_transition_config::ExchangeTransitionConfigV1Req,
    fork_choice::ForkChoiceUpdatedV3,
    payload::{GetPayloadV3Request, NewPayloadV3Request},
    ExchangeCapabilitiesRequest,
};
use eth::{
    account::{
        GetBalanceRequest, GetCodeRequest, GetProofRequest, GetStorageAtRequest,
        GetTransactionCountRequest,
    },
    block::{
        BlockNumberRequest, GetBlobBaseFee, GetBlockByHashRequest, GetBlockByNumberRequest,
        GetBlockReceiptsRequest, GetBlockTransactionCountRequest, GetRawBlockRequest,
        GetRawHeaderRequest, GetRawReceipts,
    },
    client::{ChainId, Syncing},
    fee_market::FeeHistoryRequest,
    filter::{self, ActiveFilters, DeleteFilterRequest, FilterChangesRequest, NewFilterRequest},
    gas_price::GasPrice,
    logs::LogsFilter,
    transaction::{
        CallRequest, CreateAccessListRequest, EstimateGasRequest, GetRawTransaction,
        GetTransactionByBlockHashAndIndexRequest, GetTransactionByBlockNumberAndIndexRequest,
        GetTransactionByHashRequest, GetTransactionReceiptRequest,
    },
};
use ethrex_net::sync::SyncManager;
use serde_json::Value;
use std::{
    collections::HashMap,
    future::IntoFuture,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{net::TcpListener, sync::Mutex as TokioMutex};
use tracing::info;
use types::transaction::SendRawTransactionRequest;
use utils::{
    RpcErr, RpcErrorMetadata, RpcErrorResponse, RpcNamespace, RpcRequest, RpcRequestId,
    RpcSuccessResponse,
};
mod admin;
mod authentication;
pub mod engine;
mod eth;
pub mod types;
pub mod utils;
mod web3;

use axum::extract::State;
use ethrex_net::types::Node;
use ethrex_storage::Store;

#[derive(Debug, Clone)]
pub struct RpcApiContext {
    storage: Store,
    jwt_secret: Bytes,
    local_p2p_node: Node,
    active_filters: ActiveFilters,
    syncer: Arc<TokioMutex<SyncManager>>,
}

trait RpcHandler: Sized {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr>;

    fn call(req: &RpcRequest, context: RpcApiContext) -> Result<Value, RpcErr> {
        let request = Self::parse(&req.params)?;
        request.handle(context)
    }

    fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr>;
}

const FILTER_DURATION: Duration = {
    if cfg!(test) {
        Duration::from_secs(1)
    } else {
        Duration::from_secs(5 * 60)
    }
};

pub async fn start_api(
    http_addr: SocketAddr,
    authrpc_addr: SocketAddr,
    storage: Store,
    jwt_secret: Bytes,
    local_p2p_node: Node,
    syncer: SyncManager,
) {
    // TODO: Refactor how filters are handled,
    // filters are used by the filters endpoints (eth_newFilter, eth_getFilterChanges, ...etc)
    let active_filters = Arc::new(Mutex::new(HashMap::new()));
    let service_context = RpcApiContext {
        storage: storage.clone(),
        jwt_secret,
        local_p2p_node,
        active_filters: active_filters.clone(),
        syncer: Arc::new(TokioMutex::new(syncer)),
    };

    // Periodically clean up the active filters for the filters endpoints.
    tokio::task::spawn(async move {
        let mut interval = tokio::time::interval(FILTER_DURATION);
        let filters = active_filters.clone();
        loop {
            interval.tick().await;
            tracing::info!("Running filter clean task");
            filter::clean_outdated_filters(filters.clone(), FILTER_DURATION);
            tracing::info!("Filter clean task complete");
        }
    });

    let http_router = Router::new()
        .route("/", post(handle_http_request))
        .with_state(service_context.clone());
    let http_listener = TcpListener::bind(http_addr).await.unwrap();

    let authrpc_router = Router::new()
        .route("/", post(handle_authrpc_request))
        .with_state(service_context);
    let authrpc_listener = TcpListener::bind(authrpc_addr).await.unwrap();

    let authrpc_server = axum::serve(authrpc_listener, authrpc_router)
        .with_graceful_shutdown(shutdown_signal())
        .into_future();
    let http_server = axum::serve(http_listener, http_router)
        .with_graceful_shutdown(shutdown_signal())
        .into_future();

    info!("Starting HTTP server at {http_addr}");
    info!("Starting Auth-RPC server at {}", authrpc_addr);

    let _ = tokio::try_join!(authrpc_server, http_server)
        .inspect_err(|e| info!("Error shutting down servers: {:?}", e));
}

async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("failed to install Ctrl+C handler");
}

pub async fn handle_http_request(
    State(service_context): State<RpcApiContext>,
    body: String,
) -> Json<Value> {
    let req: RpcRequest = serde_json::from_str(&body).unwrap();
    let res = map_http_requests(&req, service_context);
    rpc_response(req.id, res)
}

pub async fn handle_authrpc_request(
    State(service_context): State<RpcApiContext>,
    auth_header: Option<TypedHeader<Authorization<Bearer>>>,
    body: String,
) -> Json<Value> {
    let req: RpcRequest = serde_json::from_str(&body).unwrap();
    match authenticate(&service_context.jwt_secret, auth_header) {
        Err(error) => rpc_response(req.id, Err(error)),
        Ok(()) => {
            // Proceed with the request
            let res = map_authrpc_requests(&req, service_context);
            rpc_response(req.id, res)
        }
    }
}

/// Handle requests that can come from either clients or other users
pub fn map_http_requests(req: &RpcRequest, context: RpcApiContext) -> Result<Value, RpcErr> {
    match req.namespace() {
        Ok(RpcNamespace::Eth) => map_eth_requests(req, context),
        Ok(RpcNamespace::Admin) => map_admin_requests(req, context),
        Ok(RpcNamespace::Debug) => map_debug_requests(req, context),
        Ok(RpcNamespace::Web3) => map_web3_requests(req, context),
        _ => Err(RpcErr::MethodNotFound(req.method.clone())),
    }
}

/// Handle requests from consensus client
pub fn map_authrpc_requests(req: &RpcRequest, context: RpcApiContext) -> Result<Value, RpcErr> {
    match req.namespace() {
        Ok(RpcNamespace::Engine) => map_engine_requests(req, context),
        Ok(RpcNamespace::Eth) => map_eth_requests(req, context),
        _ => Err(RpcErr::MethodNotFound(req.method.clone())),
    }
}

pub fn map_eth_requests(req: &RpcRequest, context: RpcApiContext) -> Result<Value, RpcErr> {
    match req.method.as_str() {
        "eth_chainId" => ChainId::call(req, context),
        "eth_syncing" => Syncing::call(req, context),
        "eth_getBlockByNumber" => GetBlockByNumberRequest::call(req, context),
        "eth_getBlockByHash" => GetBlockByHashRequest::call(req, context),
        "eth_getBalance" => GetBalanceRequest::call(req, context),
        "eth_getCode" => GetCodeRequest::call(req, context),
        "eth_getStorageAt" => GetStorageAtRequest::call(req, context),
        "eth_getBlockTransactionCountByNumber" => {
            GetBlockTransactionCountRequest::call(req, context)
        }
        "eth_getBlockTransactionCountByHash" => GetBlockTransactionCountRequest::call(req, context),
        "eth_getTransactionByBlockNumberAndIndex" => {
            GetTransactionByBlockNumberAndIndexRequest::call(req, context)
        }
        "eth_getTransactionByBlockHashAndIndex" => {
            GetTransactionByBlockHashAndIndexRequest::call(req, context)
        }
        "eth_getBlockReceipts" => GetBlockReceiptsRequest::call(req, context),
        "eth_getTransactionByHash" => GetTransactionByHashRequest::call(req, context),
        "eth_getTransactionReceipt" => GetTransactionReceiptRequest::call(req, context),
        "eth_createAccessList" => CreateAccessListRequest::call(req, context),
        "eth_blockNumber" => BlockNumberRequest::call(req, context),
        "eth_call" => CallRequest::call(req, context),
        "eth_blobBaseFee" => GetBlobBaseFee::call(req, context),
        "eth_getTransactionCount" => GetTransactionCountRequest::call(req, context),
        "eth_feeHistory" => FeeHistoryRequest::call(req, context),
        "eth_estimateGas" => EstimateGasRequest::call(req, context),
        "eth_getLogs" => LogsFilter::call(req, context),
        "eth_newFilter" => {
            NewFilterRequest::stateful_call(req, context.storage, context.active_filters)
        }
        "eth_uninstallFilter" => {
            DeleteFilterRequest::stateful_call(req, context.storage, context.active_filters)
        }
        "eth_getFilterChanges" => {
            FilterChangesRequest::stateful_call(req, context.storage, context.active_filters)
        }
        "eth_sendRawTransaction" => SendRawTransactionRequest::call(req, context),
        "eth_getProof" => GetProofRequest::call(req, context),
        "eth_gasPrice" => GasPrice::call(req, context),
        unknown_eth_method => Err(RpcErr::MethodNotFound(unknown_eth_method.to_owned())),
    }
}

pub fn map_debug_requests(req: &RpcRequest, context: RpcApiContext) -> Result<Value, RpcErr> {
    match req.method.as_str() {
        "debug_getRawHeader" => GetRawHeaderRequest::call(req, context),
        "debug_getRawBlock" => GetRawBlockRequest::call(req, context),
        "debug_getRawTransaction" => GetRawTransaction::call(req, context),
        "debug_getRawReceipts" => GetRawReceipts::call(req, context),
        unknown_debug_method => Err(RpcErr::MethodNotFound(unknown_debug_method.to_owned())),
    }
}

pub fn map_engine_requests(req: &RpcRequest, context: RpcApiContext) -> Result<Value, RpcErr> {
    match req.method.as_str() {
        "engine_exchangeCapabilities" => ExchangeCapabilitiesRequest::call(req, context),
        "engine_forkchoiceUpdatedV3" => ForkChoiceUpdatedV3::call(req, context),
        "engine_newPayloadV3" => NewPayloadV3Request::call(req, context),
        "engine_exchangeTransitionConfigurationV1" => {
            ExchangeTransitionConfigV1Req::call(req, context)
        }
        "engine_getPayloadV3" => GetPayloadV3Request::call(req, context),
        unknown_engine_method => Err(RpcErr::MethodNotFound(unknown_engine_method.to_owned())),
    }
}

pub fn map_admin_requests(req: &RpcRequest, context: RpcApiContext) -> Result<Value, RpcErr> {
    match req.method.as_str() {
        "admin_nodeInfo" => admin::node_info(context.storage, context.local_p2p_node),
        unknown_admin_method => Err(RpcErr::MethodNotFound(unknown_admin_method.to_owned())),
    }
}

pub fn map_web3_requests(req: &RpcRequest, context: RpcApiContext) -> Result<Value, RpcErr> {
    match req.method.as_str() {
        "web3_clientVersion" => web3::client_version(req, context.storage),
        unknown_web3_method => Err(RpcErr::MethodNotFound(unknown_web3_method.to_owned())),
    }
}

fn rpc_response<E>(id: RpcRequestId, res: Result<Value, E>) -> Json<Value>
where
    E: Into<RpcErrorMetadata>,
{
    match res {
        Ok(result) => Json(
            serde_json::to_value(RpcSuccessResponse {
                id,
                jsonrpc: "2.0".to_string(),
                result,
            })
            .unwrap(),
        ),
        Err(error) => Json(
            serde_json::to_value(RpcErrorResponse {
                id,
                jsonrpc: "2.0".to_string(),
                error: error.into(),
            })
            .unwrap(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::test_utils::example_p2p_node;
    use ethrex_core::types::{ChainConfig, Genesis};
    use ethrex_storage::EngineType;
    use std::fs::File;
    use std::io::BufReader;

    // Maps string rpc response to RpcSuccessResponse as serde Value
    // This is used to avoid failures due to field order and allow easier string comparisons for responses
    fn to_rpc_response_success_value(str: &str) -> serde_json::Value {
        serde_json::to_value(serde_json::from_str::<RpcSuccessResponse>(str).unwrap()).unwrap()
    }

    #[test]
    fn admin_nodeinfo_request() {
        let body = r#"{"jsonrpc":"2.0", "method":"admin_nodeInfo", "params":[], "id":1}"#;
        let request: RpcRequest = serde_json::from_str(body).unwrap();
        let local_p2p_node = example_p2p_node();
        let storage =
            Store::new("temp.db", EngineType::InMemory).expect("Failed to create test DB");
        storage.set_chain_config(&example_chain_config()).unwrap();
        let context = RpcApiContext {
            local_p2p_node,
            storage,
            jwt_secret: Default::default(),
            active_filters: Default::default(),
            syncer: Arc::new(TokioMutex::new(SyncManager::dummy())),
        };
        let result = map_http_requests(&request, context);
        let rpc_response = rpc_response(request.id, result);
        let expected_response = to_rpc_response_success_value(
            r#"{"jsonrpc":"2.0","id":1,"result":{"enode":"enode://d860a01f9722d78051619d1e2351aba3f43f943f6f00718d1b9baa4101932a1f5011f16bb2b1bb35db20d6fe28fa0bf09636d26a87d31de9ec6203eeedb1f666@127.0.0.1:30303","id":"d860a01f9722d78051619d1e2351aba3f43f943f6f00718d1b9baa4101932a1f5011f16bb2b1bb35db20d6fe28fa0bf09636d26a87d31de9ec6203eeedb1f666","ip":"127.0.0.1","name":"ethrex/0.1.0/rust1.80","ports":{"discovery":30303,"listener":30303},"protocols":{"eth":{"chainId":3151908,"homesteadBlock":0,"daoForkBlock":null,"daoForkSupport":false,"eip150Block":0,"eip155Block":0,"eip158Block":0,"byzantiumBlock":0,"constantinopleBlock":0,"petersburgBlock":0,"istanbulBlock":0,"muirGlacierBlock":null,"berlinBlock":0,"londonBlock":0,"arrowGlacierBlock":null,"grayGlacierBlock":null,"mergeNetsplitBlock":0,"shanghaiTime":0,"cancunTime":0,"pragueTime":1718232101,"verkleTime":null,"terminalTotalDifficulty":0,"terminalTotalDifficultyPassed":true}}}}"#,
        );
        assert_eq!(rpc_response.to_string(), expected_response.to_string())
    }

    // Reads genesis file taken from https://github.com/ethereum/execution-apis/blob/main/tests/genesis.json
    fn read_execution_api_genesis_file() -> Genesis {
        let file = File::open("../../../test_data/genesis-execution-api.json")
            .expect("Failed to open genesis file");
        let reader = BufReader::new(file);
        serde_json::from_reader(reader).expect("Failed to deserialize genesis file")
    }

    #[test]
    fn create_access_list_simple_transfer() {
        // Create Request
        // Request taken from https://github.com/ethereum/execution-apis/blob/main/tests/eth_createAccessList/create-al-value-transfer.io
        let body = r#"{"jsonrpc":"2.0","id":1,"method":"eth_createAccessList","params":[{"from":"0x0c2c51a0990aee1d73c1228de158688341557508","nonce":"0x0","to":"0x0100000000000000000000000000000000000000","value":"0xa"},"0x00"]}"#;
        let request: RpcRequest = serde_json::from_str(body).unwrap();
        // Setup initial storage
        let storage =
            Store::new("temp.db", EngineType::InMemory).expect("Failed to create test DB");
        let genesis = read_execution_api_genesis_file();
        storage
            .add_initial_state(genesis)
            .expect("Failed to add genesis block to DB");
        let local_p2p_node = example_p2p_node();
        // Process request
        let context = RpcApiContext {
            local_p2p_node,
            storage,
            jwt_secret: Default::default(),
            active_filters: Default::default(),
            syncer: Arc::new(TokioMutex::new(SyncManager::dummy())),
        };
        let result = map_http_requests(&request, context);
        let response = rpc_response(request.id, result);
        let expected_response = to_rpc_response_success_value(
            r#"{"jsonrpc":"2.0","id":1,"result":{"accessList":[],"gasUsed":"0x5208"}}"#,
        );
        assert_eq!(response.to_string(), expected_response.to_string());
    }

    #[test]
    fn create_access_list_create() {
        // Create Request
        // Request taken from https://github.com/ethereum/execution-apis/blob/main/tests/eth_createAccessList/create-al-contract.io
        let body = r#"{"jsonrpc":"2.0","id":1,"method":"eth_createAccessList","params":[{"from":"0x0c2c51a0990aee1d73c1228de158688341557508","gas":"0xea60","gasPrice":"0x44103f2","input":"0x010203040506","nonce":"0x0","to":"0x7dcd17433742f4c0ca53122ab541d0ba67fc27df"},"0x00"]}"#;
        let request: RpcRequest = serde_json::from_str(body).unwrap();
        // Setup initial storage
        let storage =
            Store::new("temp.db", EngineType::InMemory).expect("Failed to create test DB");
        let genesis = read_execution_api_genesis_file();
        storage
            .add_initial_state(genesis)
            .expect("Failed to add genesis block to DB");
        let local_p2p_node = example_p2p_node();
        // Process request
        let context = RpcApiContext {
            local_p2p_node,
            storage,
            jwt_secret: Default::default(),
            active_filters: Default::default(),
            syncer: Arc::new(TokioMutex::new(SyncManager::dummy())),
        };
        let result = map_http_requests(&request, context);
        let response =
            serde_json::from_value::<RpcSuccessResponse>(rpc_response(request.id, result).0)
                .expect("Request failed");
        let expected_response_string = r#"{"jsonrpc":"2.0","id":1,"result":{"accessList":[{"address":"0x7dcd17433742f4c0ca53122ab541d0ba67fc27df","storageKeys":["0x0000000000000000000000000000000000000000000000000000000000000000","0x13a08e3cd39a1bc7bf9103f63f83273cced2beada9f723945176d6b983c65bd2"]}],"gasUsed":"0xca3c"}}"#;
        let expected_response =
            serde_json::from_str::<RpcSuccessResponse>(expected_response_string).unwrap();
        // Due to the scope of this test, we don't have the full state up to date which can cause variantions in gas used due to the difference in the blockchain state
        // So we will skip checking the gas_used and only check that the access list is correct
        // The gas_used will be checked when running the hive test framework
        assert_eq!(
            response.result["accessList"],
            expected_response.result["accessList"]
        )
    }

    fn example_chain_config() -> ChainConfig {
        ChainConfig {
            chain_id: 3151908_u64,
            homestead_block: Some(0),
            eip150_block: Some(0),
            eip155_block: Some(0),
            eip158_block: Some(0),
            byzantium_block: Some(0),
            constantinople_block: Some(0),
            petersburg_block: Some(0),
            istanbul_block: Some(0),
            berlin_block: Some(0),
            london_block: Some(0),
            merge_netsplit_block: Some(0),
            shanghai_time: Some(0),
            cancun_time: Some(0),
            prague_time: Some(1718232101),
            terminal_total_difficulty: Some(0),
            terminal_total_difficulty_passed: true,
            ..Default::default()
        }
    }
}
