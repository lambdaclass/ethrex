# ethrex-rpc

JSON-RPC API implementation for the ethrex Ethereum client.

## Overview

This crate provides the JSON-RPC interface that allows external clients to interact with the ethrex node. It implements the standard Ethereum JSON-RPC API as well as the Engine API for consensus client communication.

## Architecture

The RPC server runs three separate endpoints:

1. **HTTP Server** (default: `127.0.0.1:8545`)
   - Public JSON-RPC endpoint for standard Ethereum methods
   - Handles `eth_*`, `debug_*`, `net_*`, `admin_*`, `web3_*`, `txpool_*` namespaces

2. **WebSocket Server** (optional)
   - Same methods as HTTP with persistent connections
   - Useful for subscriptions and real-time updates

3. **Auth RPC Server** (default: `127.0.0.1:8551`)
   - JWT-authenticated endpoint for Engine API
   - Used by consensus clients (Lighthouse, Prysm, etc.)
   - Handles `engine_*` namespace methods

## Supported Methods

### eth namespace
- Account: `eth_getBalance`, `eth_getCode`, `eth_getStorageAt`, `eth_getTransactionCount`, `eth_getProof`
- Blocks: `eth_getBlockByNumber`, `eth_getBlockByHash`, `eth_blockNumber`, `eth_getBlockReceipts`
- Transactions: `eth_sendRawTransaction`, `eth_getTransactionByHash`, `eth_getTransactionReceipt`, `eth_call`
- Gas: `eth_estimateGas`, `eth_gasPrice`, `eth_maxPriorityFeePerGas`, `eth_feeHistory`, `eth_blobBaseFee`
- Filters: `eth_newFilter`, `eth_getFilterChanges`, `eth_uninstallFilter`, `eth_getLogs`
- Misc: `eth_chainId`, `eth_syncing`, `eth_createAccessList`

### engine namespace (requires JWT auth)
- Fork choice: `engine_forkchoiceUpdatedV1/V2/V3`
- Payloads: `engine_newPayloadV1/V2/V3/V4`, `engine_getPayloadV1/V2/V3/V4/V5`
- Bodies: `engine_getPayloadBodiesByHashV1`, `engine_getPayloadBodiesByRangeV1`
- Blobs: `engine_getBlobsV1/V2/V3`
- Capabilities: `engine_exchangeCapabilities`

### debug namespace
- `debug_getRawHeader`, `debug_getRawBlock`, `debug_getRawTransaction`, `debug_getRawReceipts`
- `debug_executionWitness`, `debug_traceTransaction`, `debug_traceBlockByNumber`

### admin namespace
- `admin_nodeInfo`, `admin_peers`, `admin_addPeer`, `admin_setLogLevel`

### net namespace
- `net_version`, `net_peerCount`

### txpool namespace
- `txpool_content`, `txpool_status`

## Usage

```rust
use ethrex_rpc::{start_api, RpcApiContext};

// Start the RPC servers
start_api(
    "127.0.0.1:8545".parse()?,  // HTTP address
    Some("127.0.0.1:8546".parse()?),  // WebSocket address (optional)
    "127.0.0.1:8551".parse()?,  // Auth RPC address
    storage,
    blockchain,
    jwt_secret,
    // ... other parameters
).await?;
```

## Implementing Custom Handlers

Each RPC method is implemented as a struct that implements the `RpcHandler` trait:

```rust
use ethrex_rpc::{RpcHandler, RpcApiContext, RpcErr};
use serde_json::Value;

struct MyCustomMethod {
    param: String,
}

impl RpcHandler for MyCustomMethod {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        let params = params.as_ref().ok_or(RpcErr::MissingParam("params"))?;
        Ok(Self {
            param: serde_json::from_value(params[0].clone())?,
        })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        // Implementation here
        Ok(serde_json::json!({"result": self.param}))
    }
}
```

## Error Handling

RPC errors follow the JSON-RPC 2.0 specification with Ethereum-specific error codes:

| Code | Meaning |
|------|---------|
| -32601 | Method not found |
| -32602 | Invalid params |
| -32603 | Internal error |
| -32000 | Server error |
| -38001 | Unknown payload |
| -38002 | Invalid fork choice state |
| -38003 | Invalid payload attributes |
| -38004 | Too large request |
| -38005 | Unsupported fork |
| 3 | Execution reverted |

## Features

- `jemalloc_profiling`: Enable heap profiling endpoints at `/debug/pprof/allocs`
