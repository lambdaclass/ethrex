//! # ethrex RPC
//!
//! JSON-RPC API implementation for the ethrex Ethereum client.
//!
//! ## Overview
//!
//! This crate implements the Ethereum JSON-RPC specification, providing:
//! - **HTTP API** (port 8545): Public endpoint for `eth_*`, `debug_*`, `net_*`, `admin_*`, `web3_*`, `txpool_*`
//! - **WebSocket API**: Optional persistent connections for real-time updates
//! - **Auth RPC API** (port 8551): JWT-authenticated endpoint for `engine_*` (consensus client)
//!
//! ## Modules
//!
//! | Module | Description |
//! |--------|-------------|
//! | [`clients`] | RPC client implementations for making requests |
//! | [`types`] | RPC-specific type definitions |
//! | [`utils`] | Error types and utilities |
//! | [`debug`] | Debugging endpoint handlers |
//!
//! ## Supported Namespaces
//!
//! | Namespace | Methods | Auth Required |
//! |-----------|---------|---------------|
//! | `eth` | Blocks, transactions, accounts, gas estimation | No |
//! | `engine` | Fork choice, payload building/submission | Yes (JWT) |
//! | `debug` | Raw data, execution witnesses, tracing | No |
//! | `net` | Network information | No |
//! | `admin` | Node administration | No |
//! | `web3` | Client version | No |
//! | `txpool` | Transaction pool inspection | No |
//!
//! ## Quick Start
//!
//! ```ignore
//! use ethrex_rpc::{start_api, RpcApiContext};
//!
//! // Start all RPC servers
//! start_api(
//!     http_addr,      // e.g., 127.0.0.1:8545
//!     ws_addr,        // Optional WebSocket address
//!     authrpc_addr,   // e.g., 127.0.0.1:8551
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
//! use serde_json::Value;
//!
//! struct GetDataRequest { id: u64 }
//!
//! impl RpcHandler for GetDataRequest {
//!     fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
//!         let params = params.as_ref().ok_or(RpcErr::MissingParam("params"))?;
//!         Ok(Self { id: serde_json::from_value(params[0].clone())? })
//!     }
//!
//!     async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
//!         // Process request using context.storage, context.blockchain, etc.
//!         Ok(serde_json::json!({ "data": self.id }))
//!     }
//! }
//! ```
//!
//! ## Feature Flags
//!
//! | Feature | Description |
//! |---------|-------------|
//! | `jemalloc_profiling` | Enable heap profiling endpoints (Linux only) |

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

pub use rpc::{start_api, start_block_executor};

#[cfg(test)]
mod test_utils;

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
