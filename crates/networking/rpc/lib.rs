//! # ethrex RPC
//!
//! This crate implements the Ethereum JSON-RPC API for the ethrex node.
//!
//! ## Overview
//!
//! The RPC server provides three interfaces:
//! - **HTTP API**: Public JSON-RPC endpoint for client requests (`eth_*`, `debug_*`, `net_*`, etc.)
//! - **WebSocket API**: Optional WebSocket endpoint for subscriptions and real-time updates
//! - **Auth RPC API**: Authenticated endpoint for consensus client communication (`engine_*` methods)
//!
//! ## Supported Namespaces
//!
//! - `eth`: Standard Ethereum methods (blocks, transactions, accounts, gas estimation)
//! - `engine`: Consensus layer methods for block building and fork choice
//! - `debug`: Debugging methods (raw blocks, execution witnesses, tracing)
//! - `net`: Network information methods
//! - `admin`: Node administration methods
//! - `web3`: Web3 utility methods
//! - `txpool`: Transaction pool inspection methods
//!
//! ## Usage
//!
//! ```ignore
//! use ethrex_rpc::{start_api, RpcApiContext};
//!
//! // Start the RPC server
//! start_api(
//!     http_addr,
//!     ws_addr,
//!     authrpc_addr,
//!     storage,
//!     blockchain,
//!     jwt_secret,
//!     // ... other parameters
//! ).await?;
//! ```
//!
//! ## Implementing Custom RPC Handlers
//!
//! Implement the [`RpcHandler`] trait to create custom RPC endpoints:
//!
//! ```ignore
//! use ethrex_rpc::{RpcHandler, RpcApiContext, RpcErr};
//!
//! struct MyHandler { /* fields */ }
//!
//! impl RpcHandler for MyHandler {
//!     fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
//!         // Parse JSON-RPC parameters
//!     }
//!
//!     async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
//!         // Handle the request
//!     }
//! }
//! ```

// This is added because otherwise some tests would fail due to reaching the recursion limit
#![recursion_limit = "400"]

mod admin;
mod authentication;
pub mod debug;
pub mod engine;
mod eth;
mod mempool;
mod net;
pub mod rpc;
mod tracing;

pub mod clients;
pub mod types;
pub mod utils;
pub use clients::{EngineClient, EthClient};

pub use rpc::{start_api, start_block_executor};

pub mod test_utils;

// TODO: These exports are needed by ethrex-l2-rpc, but we do not want to
// export them in the public API of this crate.
pub use eth::{
    filter::{ActiveFilters, clean_outdated_filters},
    gas_price::GasPrice,
    gas_tip_estimator::GasTipEstimator,
    simulate::SimulateV1Request,
    transaction::EstimateGasRequest,
};
pub use rpc::{
    ClientVersion, NodeData, RpcApiContext, RpcHandler, RpcRequestWrapper, map_debug_requests,
    map_eth_requests, map_http_requests, rpc_response, shutdown_signal,
};
pub use utils::{RpcErr, RpcErrorMetadata, RpcNamespace};
