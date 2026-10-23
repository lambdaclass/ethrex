//! The JSON-RPC a wallet talks to: a subset of `eth`, `net` and `web3` served from the
//! partial node, plus `partial_status`. Requests, responses and error codes reuse the
//! ethrex RPC crate's types, so clients see the same shapes as from a full node.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::post;
use axum::{Json, Router};
use bytes::Bytes;
use ethrex_common::types::{BlockBody, GenericTransaction};
use ethrex_common::{Address, H256, U256};
use ethrex_rpc::types::block::RpcBlock;
use ethrex_rpc::types::block_identifier::{BlockIdentifier, BlockIdentifierOrHash, BlockTag};
use ethrex_rpc::types::receipt::{RpcReceipt, RpcReceiptBlockInfo, RpcReceiptTxInfo};
use ethrex_rpc::types::transaction::RpcTransaction;
use ethrex_rpc::utils::{RpcErr, RpcRequest};
use ethrex_rpc::{RpcRequestWrapper, rpc_response, validate_batch};
use ethrex_vm::ExecutionResult;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use tower_http::cors::CorsLayer;
use tracing::info;

use crate::node::{BlockSel, Node, NodeError};
use crate::txbook::SubmittedTx;

/// Serves `node` at `addr` until `shutdown`.
pub async fn serve(
    node: Node,
    addr: SocketAddr,
    shutdown: CancellationToken,
) -> Result<(), std::io::Error> {
    let router = Router::new()
        .route("/", post(handle_http))
        .layer(CorsLayer::permissive())
        .with_state(Arc::new(node));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!(%addr, "partial node JSON-RPC listening");
    axum::serve(listener, router)
        .with_graceful_shutdown(async move { shutdown.cancelled().await })
        .await
}

async fn handle_http(
    State(node): State<Arc<Node>>,
    body: String,
) -> Result<Json<Value>, StatusCode> {
    let wrapper: RpcRequestWrapper =
        serde_json::from_str(&body).map_err(|_| StatusCode::BAD_REQUEST)?;
    if let Some(error) = validate_batch(&wrapper) {
        return Ok(Json(error));
    }
    let response = match wrapper {
        RpcRequestWrapper::Single(request) => {
            let result = dispatch(&request, &node).await;
            rpc_response(request.id, result).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        }
        RpcRequestWrapper::Multiple(requests) => {
            let mut responses = Vec::with_capacity(requests.len());
            for request in requests {
                let result = dispatch(&request, &node).await;
                responses.push(
                    rpc_response(request.id, result)
                        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
                );
            }
            Value::Array(responses)
        }
    };
    Ok(Json(response))
}

/// Routes one request.
pub async fn dispatch(request: &RpcRequest, node: &Node) -> Result<Value, RpcErr> {
    let params = request.params.as_deref().unwrap_or(&[]);
    match request.method.as_str() {
        "web3_clientVersion" => Ok(json!(node.client_version())),
        "net_version" => Ok(json!(node.chain_id().to_string())),
        "net_listening" => Ok(json!(true)),
        "eth_chainId" => Ok(json!(hex_u64(node.chain_id()))),
        "eth_blockNumber" => Ok(json!(hex_u64(node.head().number()))),
        "eth_syncing" => Ok(json!(false)),
        "eth_accounts" => Ok(json!([])),
        "eth_getBlockByNumber" => {
            let sel = block_sel(param(params, 0)?, 0)?;
            let full = param(params, 1).ok().and_then(parse_bool).unwrap_or(false);
            block(node, sel, full)
        }
        "eth_getBlockByHash" => {
            let hash: H256 = parse(param(params, 0)?, "block hash")?;
            let full = param(params, 1).ok().and_then(parse_bool).unwrap_or(false);
            block(node, BlockSel::Hash(hash), full)
        }
        "eth_getBalance" => {
            let address: Address = parse(param(params, 0)?, "address")?;
            let sel = block_sel_or_latest(params, 1)?;
            let account = node.account(address, sel).await.map_err(to_rpc)?;
            Ok(json!(hex_u256(account.map_or(U256::zero(), |a| a.balance))))
        }
        "eth_getTransactionCount" => {
            let address: Address = parse(param(params, 0)?, "address")?;
            let sel = block_sel_or_latest(params, 1)?;
            let nonce = node.transaction_count(address, sel).await.map_err(to_rpc)?;
            Ok(json!(hex_u64(nonce)))
        }
        "eth_getCode" => {
            let address: Address = parse(param(params, 0)?, "address")?;
            let sel = block_sel_or_latest(params, 1)?;
            let code = node.code(address, sel).await.map_err(to_rpc)?;
            Ok(json!(hex_bytes(&code)))
        }
        "eth_getStorageAt" => {
            let address: Address = parse(param(params, 0)?, "address")?;
            let slot: U256 = parse(param(params, 1)?, "storage slot")?;
            let sel = block_sel_or_latest(params, 2)?;
            let value = node.storage(address, slot, sel).await.map_err(to_rpc)?;
            Ok(json!(format!("{:#066x}", value)))
        }
        "eth_call" => {
            let tx = call_request(param(params, 0)?)?;
            let sel = block_sel_or_latest(params, 1)?;
            match node.call(tx, sel).await.map_err(to_rpc)? {
                ExecutionResult::Success { output, .. } => Ok(json!(hex_bytes(&output))),
                ExecutionResult::Revert { output, .. } => Err(RpcErr::Revert {
                    data: hex_bytes(&output),
                }),
                ExecutionResult::Halt { reason, gas_used } => {
                    Err(RpcErr::Halt { reason, gas_used })
                }
            }
        }
        "eth_estimateGas" => {
            let tx = call_request(param(params, 0)?)?;
            let sel = block_sel_or_latest(params, 1)?;
            let gas = node.estimate_gas(tx, sel).await.map_err(to_rpc)?;
            Ok(json!(hex_u64(gas)))
        }
        "eth_gasPrice" => Ok(json!(hex_u64(node.gas_price().await.map_err(to_rpc)?))),
        "eth_maxPriorityFeePerGas" => Ok(json!(hex_u64(
            node.max_priority_fee().await.map_err(to_rpc)?
        ))),
        "eth_feeHistory" => {
            let count: u64 = parse_quantity(param(params, 0)?, "block count")?;
            let newest = block_sel(param(params, 1)?, 1)?;
            let percentiles: Vec<f64> = match params.get(2) {
                Some(raw) if raw.get() != "null" => {
                    serde_json::from_str(raw.get()).map_err(|e| RpcErr::BadParams(e.to_string()))?
                }
                _ => Vec::new(),
            };
            if percentiles.windows(2).any(|w| w[0] > w[1])
                || percentiles.iter().any(|p| !(0.0..=100.0).contains(p))
            {
                return Err(RpcErr::BadParams(
                    "reward percentiles must be ascending within 0..=100".into(),
                ));
            }
            let history = node
                .fee_history(count, newest, &percentiles)
                .await
                .map_err(to_rpc)?;
            let mut value = json!({
                "oldestBlock": hex_u64(history.oldest_block),
                "baseFeePerGas": history.base_fee_per_gas.iter().map(|f| hex_u64(*f)).collect::<Vec<_>>(),
                "baseFeePerBlobGas": history.base_fee_per_blob_gas.iter().map(|f| hex_u256(*f)).collect::<Vec<_>>(),
                "gasUsedRatio": history.gas_used_ratio,
                "blobGasUsedRatio": history.blob_gas_used_ratio,
            });
            if !percentiles.is_empty()
                && let Value::Object(map) = &mut value
            {
                map.insert(
                    "reward".into(),
                    json!(
                        history
                            .reward
                            .iter()
                            .map(|block| block.iter().map(|r| hex_u64(*r)).collect::<Vec<_>>())
                            .collect::<Vec<_>>()
                    ),
                );
            }
            Ok(value)
        }
        "eth_sendRawTransaction" => {
            let raw: Bytes = parse(param(params, 0)?, "raw transaction")?;
            if raw.is_empty() {
                return Err(RpcErr::BadParams("empty transaction".into()));
            }
            let hash = node.submit(&raw).await.map_err(to_rpc)?;
            Ok(json!(format!("{hash:#x}")))
        }
        "eth_getTransactionReceipt" => {
            let hash: H256 = parse(param(params, 0)?, "transaction hash")?;
            receipt(node, hash)
        }
        "eth_getTransactionByHash" => {
            let hash: H256 = parse(param(params, 0)?, "transaction hash")?;
            transaction(node, hash)
        }
        "partial_status" => status(node).await,
        "eth_getLogs"
        | "eth_newFilter"
        | "eth_newBlockFilter"
        | "eth_newPendingTransactionFilter"
        | "eth_getFilterChanges"
        | "eth_getFilterLogs"
        | "eth_uninstallFilter"
        | "eth_getProof"
        | "eth_getBlockReceipts"
        | "eth_getTransactionByBlockHashAndIndex"
        | "eth_getTransactionByBlockNumberAndIndex"
        | "eth_getBlockTransactionCountByHash"
        | "eth_getBlockTransactionCountByNumber"
        | "eth_createAccessList"
        | "eth_blobBaseFee" => Err(unsupported(&request.method)),
        other => Err(RpcErr::MethodNotFound(other.to_string())),
    }
}

fn unsupported(method: &str) -> RpcErr {
    RpcErr::MethodNotServedHere {
        method: method.to_string(),
        reason: "a partial stateful node serves only the state it holds".to_string(),
    }
}

fn to_rpc(err: NodeError) -> RpcErr {
    match err {
        NodeError::Reverted { data } => RpcErr::Revert {
            data: hex_bytes(&data),
        },
        NodeError::Halted { reason } => RpcErr::Halt {
            reason,
            gas_used: 0,
        },
        NodeError::Estimate(crate::exec::EstimateError::Reverted { data }) => RpcErr::Revert {
            data: hex_bytes(&data),
        },
        NodeError::Estimate(crate::exec::EstimateError::Halted { reason }) => RpcErr::Halt {
            reason,
            gas_used: 0,
        },
        NodeError::Estimate(crate::exec::EstimateError::Refused(reason)) => RpcErr::Vm(reason),
        NodeError::Exec(reason) => RpcErr::Vm(reason),
        NodeError::InvalidTransaction(reason) => RpcErr::BadParams(reason),
        NodeError::Unsupported { what } => RpcErr::MethodNotServedHere {
            method: String::new(),
            reason: what,
        },
        other => RpcErr::Internal(other.to_string()),
    }
}

fn param(
    params: &[Box<serde_json::value::RawValue>],
    index: usize,
) -> Result<&serde_json::value::RawValue, RpcErr> {
    params
        .get(index)
        .map(AsRef::as_ref)
        .ok_or_else(|| RpcErr::MissingParam(format!("parameter {index}")))
}

fn parse<T: serde::de::DeserializeOwned>(
    raw: &serde_json::value::RawValue,
    what: &str,
) -> Result<T, RpcErr> {
    serde_json::from_str(raw.get()).map_err(|e| RpcErr::WrongParam(format!("{what}: {e}")))
}

fn parse_bool(raw: &serde_json::value::RawValue) -> Option<bool> {
    serde_json::from_str(raw.get()).ok()
}

fn parse_quantity(raw: &serde_json::value::RawValue, what: &str) -> Result<u64, RpcErr> {
    let value: Value =
        serde_json::from_str(raw.get()).map_err(|e| RpcErr::WrongParam(format!("{what}: {e}")))?;
    match value {
        Value::Number(n) => n
            .as_u64()
            .ok_or_else(|| RpcErr::WrongParam(format!("{what}: not a quantity"))),
        Value::String(s) => {
            let digits = s.trim_start_matches("0x");
            u64::from_str_radix(digits, 16).map_err(|e| RpcErr::WrongParam(format!("{what}: {e}")))
        }
        _ => Err(RpcErr::WrongParam(format!("{what}: not a quantity"))),
    }
}

fn block_sel_or_latest(
    params: &[Box<serde_json::value::RawValue>],
    index: usize,
) -> Result<BlockSel, RpcErr> {
    match params.get(index) {
        None => Ok(BlockSel::Latest),
        Some(raw) if raw.get() == "null" => Ok(BlockSel::Latest),
        Some(raw) => block_sel(raw, index),
    }
}

fn block_sel(raw: &serde_json::value::RawValue, index: usize) -> Result<BlockSel, RpcErr> {
    let arg_index = u64::try_from(index).unwrap_or(0);
    let id = BlockIdentifierOrHash::parse_param(raw, arg_index)?;
    Ok(match id {
        BlockIdentifierOrHash::Hash(hash) => BlockSel::Hash(hash),
        BlockIdentifierOrHash::Identifier(BlockIdentifier::Number(number)) => {
            BlockSel::Number(number)
        }
        BlockIdentifierOrHash::Identifier(BlockIdentifier::Tag(tag)) => match tag {
            BlockTag::Latest => BlockSel::Latest,
            BlockTag::Pending => BlockSel::Pending,
            BlockTag::Safe => BlockSel::Safe,
            BlockTag::Finalized => BlockSel::Finalized,
            BlockTag::Earliest => {
                return Err(RpcErr::MethodNotServedHere {
                    method: String::new(),
                    reason: "the earliest block is not held".into(),
                });
            }
        },
    })
}

fn call_request(raw: &serde_json::value::RawValue) -> Result<GenericTransaction, RpcErr> {
    let tx: GenericTransaction = parse(raw, "call request")?;
    if !tx.blob_versioned_hashes.is_empty()
        || !tx.blobs.is_empty()
        || tx.max_fee_per_blob_gas.is_some()
    {
        return Err(RpcErr::BadParams(
            "blob fields are not supported in calls".into(),
        ));
    }
    if tx
        .authorization_list
        .as_ref()
        .is_some_and(|l| !l.is_empty())
    {
        return Err(RpcErr::BadParams(
            "authorization lists are not supported in calls".into(),
        ));
    }
    Ok(tx)
}

fn block(node: &Node, sel: BlockSel, full_transactions: bool) -> Result<Value, RpcErr> {
    let header = match node.header(sel) {
        Ok(header) => header,
        Err(NodeError::Unsupported { .. }) | Err(NodeError::NotReady) => return Ok(Value::Null),
        Err(err) => return Err(to_rpc(err)),
    };
    // Bodies are not held: the block is served with its header and no transactions.
    let body = BlockBody {
        transactions: Vec::new(),
        ommers: Vec::new(),
        withdrawals: header.header().withdrawals_root.map(|_| Vec::new()),
    };
    let block = RpcBlock::build(
        header.header().clone(),
        body,
        header.hash(),
        full_transactions,
    )?;
    serde_json::to_value(block).map_err(|e| RpcErr::Internal(e.to_string()))
}

fn transaction(node: &Node, hash: H256) -> Result<Value, RpcErr> {
    let Some(submitted) = node.transaction(hash) else {
        return Ok(Value::Null);
    };
    let (block_number, block_hash, index) = match &submitted {
        SubmittedTx::Pending(_) => (None, None, None),
        SubmittedTx::Located(located) => (
            Some(located.block.number),
            Some(located.block.hash),
            usize::try_from(located.index).ok(),
        ),
    };
    let rpc = RpcTransaction::build(submitted.tx().clone(), block_number, block_hash, index)?;
    serde_json::to_value(rpc).map_err(|e| RpcErr::Internal(e.to_string()))
}

fn receipt(node: &Node, hash: H256) -> Result<Value, RpcErr> {
    let Some(receipt) = node.receipt(hash) else {
        return Ok(Value::Null);
    };
    let Some(SubmittedTx::Located(located)) = node.transaction(hash) else {
        return Ok(Value::Null);
    };
    let Ok(header) = node.header(BlockSel::Hash(receipt.block().hash)) else {
        return Ok(Value::Null);
    };
    let base_fee = header.header().base_fee_per_gas;
    let blob_gas_price = crate::node::next_base_fee(header.header(), &node.chain_config());
    let tx_info = RpcReceiptTxInfo::from_transaction(
        located.tx.clone(),
        receipt.index(),
        receipt.gas_used(),
        blob_gas_price,
        base_fee,
    )?;
    let block_info = RpcReceiptBlockInfo::from_block_header(header.header().clone());
    let rpc = RpcReceipt::new(
        receipt.receipt().clone(),
        tx_info,
        block_info,
        receipt.first_log_index(),
        receipt.block_timestamp(),
    );
    serde_json::to_value(rpc).map_err(|e| RpcErr::Internal(e.to_string()))
}

async fn status(node: &Node) -> Result<Value, RpcErr> {
    let status = node.status().await;
    Ok(json!({
        "anchor": { "number": status.anchor.number, "hash": format!("{:#x}", status.anchor.hash), "timestamp": status.anchor_timestamp },
        "finalized": status.finalized.map(|f| json!({ "number": f.number, "hash": format!("{:#x}", f.hash) })),
        "lightClient": status.heads.map(|h| json!({
            "optimisticSlot": h.optimistic.slot,
            "optimisticBeaconRoot": format!("{:#x}", h.optimistic.beacon_root),
            "optimisticExecutionBlockHash": format!("{:#x}", h.optimistic.execution_block_hash),
            "finalizedSlot": h.finalized.slot,
            "finalizedExecutionBlockHash": format!("{:#x}", h.finalized.execution_block_hash),
        })),
        "chainProofs": {
            "mode": format!("{:?}", status.proof_mode).to_lowercase(),
            "verified": status.proofs_verified,
            "provenPayloads": status.proven_payloads,
            "anchorProven": status.anchor_proven,
        },
        "held": { "accounts": status.held_accounts, "slots": status.held_slots, "pins": status.pins, "learned": status.learned },
        "pendingTransactions": status.pending_txs,
        "peers": { "state": status.peers.state_peers, "eth": status.peers.eth_peers },
    }))
}

fn hex_u64(value: u64) -> String {
    format!("{value:#x}")
}

fn hex_u256(value: U256) -> String {
    format!("{value:#x}")
}

fn hex_bytes(bytes: &[u8]) -> String {
    format!("0x{}", hex::encode(bytes))
}
