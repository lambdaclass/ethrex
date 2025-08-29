// This is added because otherwise some tests would fail due to reaching the recursion limit
#![recursion_limit = "400"]

mod admin;
mod authentication;
pub mod debug;
mod engine;
mod eth;
mod mempool;
mod net;
mod rpc;
mod tracing;

pub mod clients;
pub mod types;
pub mod utils;
pub use clients::{EngineClient, EthClient};

pub use rpc::start_api;

// TODO: These exports are needed by ethrex-l2-rpc, but we do not want to
// export them in the public API of this crate.
pub use eth::{
    filter::{ActiveFilters, clean_outdated_filters},
    gas_price::GasPrice,
    gas_tip_estimator::GasTipEstimator,
    transaction::EstimateGasRequest,
};
pub use rpc::{
    NodeData, RpcApiContext, RpcHandler, RpcRequestWrapper, map_debug_requests, map_eth_requests,
    map_http_requests, rpc_response, shutdown_signal,
};
pub use utils::{RpcErr, RpcErrorMetadata, RpcNamespace};
