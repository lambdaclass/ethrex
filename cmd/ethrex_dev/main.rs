//! ethrex-dev - On-Demand Ethereum L1 Block Builder
//!
//! A development node that builds blocks immediately when transactions arrive.

use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::{Arc, Mutex},
};

use axum::{Json, Router, extract::State, routing::post};
use bytes::Bytes;
use clap::Parser;
use ethrex_block_builder::{BlockBuilder, BlockBuilderConfig, CastMsg, display_banner};
use ethrex_common::{Address, H512, types::Transaction};
use ethrex_config::networks::Network;
use ethrex_p2p::types::{Node, NodeRecord};
use ethrex_rpc::{
    GasTipEstimator, NodeData, RpcApiContext, RpcErr, RpcRequestWrapper, map_http_requests,
    rpc_response,
    types::transaction::SendRawTransactionRequest,
    utils::{RpcRequest, RpcRequestId},
};
use serde_json::Value;
use spawned_concurrency::tasks::GenServerHandle;
use tokio::{net::TcpListener, sync::mpsc::unbounded_channel};
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

/// CLI arguments for ethrex-dev.
#[derive(Parser, Debug)]
#[command(name = "ethrex-dev")]
#[command(about = "On-demand Ethereum L1 block builder for development")]
struct Args {
    /// RPC server host
    #[arg(long, default_value = "127.0.0.1")]
    host: String,

    /// RPC server port
    #[arg(short, long, default_value = "8545")]
    port: u16,

    /// Block time in milliseconds (enables interval mode)
    #[arg(long)]
    block_time: Option<u64>,

    /// Coinbase address for block rewards
    #[arg(long)]
    coinbase: Option<String>,
}

/// Extended RPC context with block builder handle.
#[derive(Clone)]
struct DevRpcContext {
    inner: RpcApiContext,
    block_builder: GenServerHandle<BlockBuilder>,
}

#[tokio::main]
async fn main() -> eyre::Result<()> {
    // Initialize logging - suppress most logs, only show errors
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::new("error"))
        .init();

    let args = Args::parse();

    // Parse coinbase address
    let coinbase = if let Some(addr_str) = &args.coinbase {
        let addr_hex = addr_str.strip_prefix("0x").unwrap_or(addr_str);
        let bytes = hex::decode(addr_hex)?;
        Address::from_slice(&bytes)
    } else {
        Address::zero()
    };

    // Create block builder configuration
    let config = BlockBuilderConfig {
        coinbase,
        block_time_ms: args.block_time,
        ..Default::default()
    };

    // Spawn the block builder - get shared store and blockchain
    let (block_builder, store, blockchain) = BlockBuilder::spawn(config.clone()).await?;

    // Get genesis for gas limit
    let network = Network::LocalDevnet;
    let genesis = network.get_genesis()?;

    // Create RPC context using the same store and blockchain as the block builder
    let (block_worker_sender, _block_worker_receiver) = unbounded_channel();

    let rpc_context = RpcApiContext {
        storage: store,
        blockchain,
        active_filters: Arc::new(Mutex::new(HashMap::new())),
        syncer: None,
        peer_handler: None,
        node_data: NodeData {
            jwt_secret: Bytes::new(),
            local_p2p_node: Node::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0, 0, H512::zero()),
            local_node_record: NodeRecord::default(),
            client_version: "ethrex-dev/0.1.0".to_string(),
            extra_data: Bytes::new(),
        },
        gas_tip_estimator: Arc::new(tokio::sync::Mutex::new(GasTipEstimator::new())),
        log_filter_handler: None,
        gas_ceil: genesis.gas_limit,
        block_worker_channel: block_worker_sender,
        dev_tx_sender: None,
    };

    let dev_context = DevRpcContext {
        inner: rpc_context,
        block_builder,
    };

    // Display banner
    display_banner(&args.host, args.port)?;

    // Create router
    let router = Router::new()
        .route("/", post(handle_rpc_request))
        .with_state(dev_context);

    // Start server
    let addr: SocketAddr = format!("{}:{}", args.host, args.port).parse()?;
    let listener = TcpListener::bind(addr).await?;

    println!("    Server running. Press Ctrl+C to stop.\n");

    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

/// Handle incoming RPC requests with logging.
async fn handle_rpc_request(
    State(context): State<DevRpcContext>,
    body: String,
) -> Result<Json<Value>, axum::http::StatusCode> {
    let res = match serde_json::from_str::<RpcRequestWrapper>(&body) {
        Ok(RpcRequestWrapper::Single(request)) => {
            // Log the RPC call
            println!("    >> {}", request.method);

            let res = handle_single_request(&request, &context).await;
            rpc_response(request.id, res).map_err(|_| axum::http::StatusCode::BAD_REQUEST)?
        }
        Ok(RpcRequestWrapper::Multiple(requests)) => {
            let mut responses = Vec::new();
            for req in requests {
                // Log the RPC call
                println!("    >> {}", req.method);

                let res = handle_single_request(&req, &context).await;
                responses.push(
                    rpc_response(req.id, res).map_err(|_| axum::http::StatusCode::BAD_REQUEST)?,
                );
            }
            serde_json::to_value(responses).map_err(|_| axum::http::StatusCode::BAD_REQUEST)?
        }
        Err(_) => rpc_response(
            RpcRequestId::String("".to_string()),
            Err(RpcErr::BadParams("Invalid request body".to_string())),
        )
        .map_err(|_| axum::http::StatusCode::BAD_REQUEST)?,
    };
    Ok(Json(res))
}

/// Handle a single RPC request, intercepting eth_sendRawTransaction.
async fn handle_single_request(req: &RpcRequest, context: &DevRpcContext) -> Result<Value, RpcErr> {
    // Special handling for eth_sendRawTransaction - send to block builder
    if req.method == "eth_sendRawTransaction" {
        return handle_send_raw_transaction(req, context).await;
    }

    // For all other requests, use the standard handler
    map_http_requests(req, context.inner.clone()).await
}

/// Handle eth_sendRawTransaction by sending to the block builder.
async fn handle_send_raw_transaction(
    req: &RpcRequest,
    context: &DevRpcContext,
) -> Result<Value, RpcErr> {
    // Extract raw transaction hex from params
    let params = req
        .params
        .as_ref()
        .ok_or_else(|| RpcErr::BadParams("Missing params".to_string()))?;
    let raw_tx_hex = params
        .first()
        .and_then(|v| v.as_str())
        .ok_or_else(|| RpcErr::BadParams("Missing raw transaction hex".to_string()))?;

    // Decode hex to bytes
    let raw_tx_hex = raw_tx_hex.strip_prefix("0x").unwrap_or(raw_tx_hex);
    let raw_tx_bytes =
        hex::decode(raw_tx_hex).map_err(|e| RpcErr::BadParams(format!("Invalid hex: {e}")))?;

    // Parse the transaction using decode_canonical
    let tx_request = SendRawTransactionRequest::decode_canonical(&raw_tx_bytes)
        .map_err(|e| RpcErr::BadParams(format!("Invalid transaction: {e}")))?;

    let (tx, blobs_bundle) = match tx_request {
        SendRawTransactionRequest::EIP4844(wrapped) => (
            Transaction::EIP4844Transaction(wrapped.tx),
            Some(wrapped.blobs_bundle),
        ),
        other => (other.to_transaction(), None),
    };

    let tx_hash = tx.hash();

    // Also add to mempool for queries
    if let Some(bundle) = blobs_bundle.clone() {
        if let Transaction::EIP4844Transaction(ref eip4844_tx) = tx {
            context
                .inner
                .blockchain
                .add_blob_transaction_to_pool(eip4844_tx.clone(), bundle)
                .await
                .map_err(|e| RpcErr::Internal(e.to_string()))?;
        }
    } else {
        context
            .inner
            .blockchain
            .add_transaction_to_pool(tx.clone())
            .await
            .map_err(|e| RpcErr::Internal(e.to_string()))?;
    }

    // Send to block builder (async, don't wait)
    let mut builder_handle = context.block_builder.clone();
    builder_handle
        .cast(CastMsg::SubmitTransaction {
            tx: Box::new(tx),
            blobs_bundle,
        })
        .await
        .map_err(|e| RpcErr::Internal(e.to_string()))?;

    // Return transaction hash immediately (Ethereum behavior)
    serde_json::to_value(format!("{tx_hash:#x}")).map_err(|e| RpcErr::Internal(e.to_string()))
}

/// Shutdown signal handler.
async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("failed to install Ctrl+C handler");
    println!("\n    Shutting down...");
}
