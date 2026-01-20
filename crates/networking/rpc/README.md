# ethrex-rpc

JSON-RPC API implementation for the ethrex Ethereum client.

## Overview

This crate implements the Ethereum JSON-RPC specification, providing both client-facing APIs and consensus layer communication. It exposes three distinct server interfaces to handle different types of requests.

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                       RPC Servers                                │
│  ┌─────────────────┐  ┌─────────────────┐  ┌─────────────────┐ │
│  │   HTTP Server   │  │   WebSocket     │  │   Auth RPC      │ │
│  │  (Port 8545)    │  │   Server        │  │  (Port 8551)    │ │
│  │                 │  │                 │  │  JWT Auth       │ │
│  └─────────────────┘  └─────────────────┘  └─────────────────┘ │
│         │                    │                    │             │
│         └────────────────────┼────────────────────┘             │
│                              ▼                                   │
│  ┌───────────────────────────────────────────────────────────┐ │
│  │                    RpcApiContext                          │ │
│  │  Storage | Blockchain | SyncManager | PeerHandler | ...   │ │
│  └───────────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────────────┘
                              │
          ┌───────────────────┼───────────────────┐
          ▼                   ▼                   ▼
┌─────────────────┐ ┌─────────────────┐ ┌─────────────────┐
│      eth_*      │ │    engine_*     │ │    debug_*      │
│  Standard API   │ │  Consensus API  │ │   Debugging     │
└─────────────────┘ └─────────────────┘ └─────────────────┘
```

## Quick Start

```rust
use ethrex_rpc::start_api;

// Start all RPC servers
start_api(
    "127.0.0.1:8545".parse()?,   // HTTP
    Some("127.0.0.1:8546".parse()?), // WebSocket (optional)
    "127.0.0.1:8551".parse()?,   // Auth RPC (Engine API)
    storage,
    blockchain,
    jwt_secret,
    local_p2p_node,
    local_node_record,
    syncer,
    peer_handler,
    client_version,
    log_filter_handler,
    gas_ceil,
    extra_data,
).await?;
```

## Supported Namespaces

### eth_* (Standard Ethereum API)

| Method | Description |
|--------|-------------|
| `eth_chainId` | Return the chain ID |
| `eth_blockNumber` | Return the current block number |
| `eth_getBlockByNumber` | Get block by number |
| `eth_getBlockByHash` | Get block by hash |
| `eth_getBalance` | Get account balance |
| `eth_getCode` | Get contract code |
| `eth_getStorageAt` | Get storage slot value |
| `eth_getTransactionCount` | Get account nonce |
| `eth_getTransactionByHash` | Get transaction by hash |
| `eth_getTransactionReceipt` | Get transaction receipt |
| `eth_sendRawTransaction` | Submit signed transaction |
| `eth_call` | Execute call without state change |
| `eth_estimateGas` | Estimate gas for transaction |
| `eth_gasPrice` | Get current gas price |
| `eth_maxPriorityFeePerGas` | Get max priority fee |
| `eth_feeHistory` | Get fee history |
| `eth_getLogs` | Get logs matching filter |
| `eth_newFilter` | Create new log filter |
| `eth_getFilterChanges` | Poll filter for changes |
| `eth_uninstallFilter` | Remove filter |
| `eth_getProof` | Get Merkle proof for account |
| `eth_createAccessList` | Generate access list for transaction |
| `eth_syncing` | Get sync status |
| `eth_getBlockReceipts` | Get all receipts for block |
| `eth_blobBaseFee` | Get current blob base fee |

### engine_* (Consensus Layer API)

These methods are used by consensus clients (e.g., Lighthouse, Prysm) and require JWT authentication.

| Method | Description |
|--------|-------------|
| `engine_newPayloadV1/V2/V3/V4` | Submit new block payload |
| `engine_forkchoiceUpdatedV1/V2/V3` | Update fork choice and optionally start payload building |
| `engine_getPayloadV1/V2/V3/V4/V5` | Retrieve built payload |
| `engine_getPayloadBodiesByHashV1` | Get payload bodies by hash |
| `engine_getPayloadBodiesByRangeV1` | Get payload bodies by range |
| `engine_getBlobsV1/V2/V3` | Get blobs for payload |
| `engine_exchangeCapabilities` | Exchange supported methods |
| `engine_exchangeTransitionConfigurationV1` | Exchange transition config |

### debug_* (Debugging)

| Method | Description |
|--------|-------------|
| `debug_getRawHeader` | Get RLP-encoded block header |
| `debug_getRawBlock` | Get RLP-encoded block |
| `debug_getRawTransaction` | Get RLP-encoded transaction |
| `debug_getRawReceipts` | Get RLP-encoded receipts |
| `debug_executionWitness` | Get execution witness for stateless validation |
| `debug_traceTransaction` | Trace transaction execution |
| `debug_traceBlockByNumber` | Trace all transactions in block |

### net_* (Network)

| Method | Description |
|--------|-------------|
| `net_version` | Return network ID |
| `net_peerCount` | Return connected peer count |

### admin_* (Administration)

| Method | Description |
|--------|-------------|
| `admin_nodeInfo` | Get node information |
| `admin_peers` | List connected peers |
| `admin_addPeer` | Add a peer manually |
| `admin_setLogLevel` | Change log verbosity |

### web3_* (Utilities)

| Method | Description |
|--------|-------------|
| `web3_clientVersion` | Return client version string |

### txpool_* (Transaction Pool)

| Method | Description |
|--------|-------------|
| `txpool_content` | Get all pending transactions |
| `txpool_status` | Get pool statistics |

## Module Structure

| Module | Description |
|--------|-------------|
| `eth` | Standard Ethereum RPC method handlers |
| `engine` | Engine API handlers for consensus clients |
| `debug` | Debugging and tracing handlers |
| `admin` | Node administration handlers |
| `net` | Network information handlers |
| `mempool` | Transaction pool inspection handlers |
| `authentication` | JWT authentication for Engine API |
| `types` | RPC-specific type definitions |
| `utils` | Error types and utilities |
| `clients` | RPC client implementations |
| `tracing` | Transaction tracing support |

## Core Types

### RpcApiContext

Shared context passed to all handlers:

```rust
pub struct RpcApiContext {
    pub storage: Store,
    pub blockchain: Arc<Blockchain>,
    pub active_filters: ActiveFilters,
    pub syncer: Option<Arc<SyncManager>>,
    pub peer_handler: Option<PeerHandler>,
    pub node_data: NodeData,
    pub gas_tip_estimator: Arc<TokioMutex<GasTipEstimator>>,
    pub log_filter_handler: Option<reload::Handle<EnvFilter, Registry>>,
    pub gas_ceil: u64,
    pub block_worker_channel: UnboundedSender<...>,
}
```

### RpcHandler Trait

Interface for implementing RPC methods:

```rust
pub trait RpcHandler: Sized {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr>;
    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr>;
}
```

### RpcErr

Error types for RPC responses:

```rust
pub enum RpcErr {
    MethodNotFound(String),
    WrongParam(String),
    BadParams(String),
    MissingParam(String),
    TooLargeRequest,
    BadHexFormat(String),
    UnsuportedFork(String),
    Internal(String),
    Vm(String),
    Revert { gas_used: u64, output: Bytes },
    Halt { reason: String, gas_used: u64 },
    AuthenticationError(String),
    InvalidForkChoiceState(String),
    InvalidPayloadAttributes(String),
    UnknownPayload(String),
}
```

## Server Configuration

### HTTP Server (Port 8545 default)

- Handles all public RPC methods
- CORS enabled (permissive)
- Supports batch requests

### WebSocket Server (Optional)

- Same methods as HTTP
- Persistent connections
- Real-time updates

### Auth RPC Server (Port 8551 default)

- JWT authentication required
- Engine API methods only
- 256MB body limit for large payloads
- Consensus client heartbeat monitoring

## Feature Flags

| Feature | Description | Default |
|---------|-------------|---------|
| `jemalloc_profiling` | Enable heap profiling endpoints (Linux only) | No |

## Profiling Endpoints

When `jemalloc_profiling` is enabled (Linux only):

| Endpoint | Description |
|----------|-------------|
| `GET /debug/pprof/allocs` | Get heap profile (pprof format) |
| `GET /debug/pprof/allocs/flamegraph` | Get heap flamegraph (SVG) |

## Authentication

Engine API uses JWT authentication per EIP-3675:

1. Consensus client includes `Authorization: Bearer <token>` header
2. Server validates JWT signature using shared secret
3. Token expiration is checked (60 second validity)

## RPC Clients

The `clients` module provides typed RPC clients for making requests:

```rust
use ethrex_rpc::clients::{EthClient, EngineClient};

// Ethereum RPC client
let eth_client = EthClient::new("http://localhost:8545");
let balance = eth_client.get_balance(address, "latest").await?;

// Engine API client (with JWT)
let engine_client = EngineClient::new("http://localhost:8551", jwt_secret);
let payload = engine_client.get_payload(payload_id).await?;
```

## Filter System

Log filters for `eth_newFilter` have automatic cleanup:
- Filters expire after 5 minutes of inactivity
- Background task periodically removes stale filters
- Test mode uses 1 second timeout

## Block Executor

A dedicated thread handles block execution from `engine_newPayload`:
- Prevents async runtime blocking
- Sequential block processing
- Uses unbounded channel for submission

## Dependencies

- `axum` - HTTP/WebSocket server framework
- `ethrex-blockchain` - Block validation and execution
- `ethrex-storage` - Database access
- `ethrex-vm` - EVM execution for `eth_call`
- `ethrex-p2p` - Peer handler access
- `jsonwebtoken` - JWT authentication
- `tower-http` - CORS middleware

For detailed API documentation:
```bash
cargo doc --package ethrex-rpc --open
```
