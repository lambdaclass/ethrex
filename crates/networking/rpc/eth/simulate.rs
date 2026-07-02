//! `eth_simulateV1`: simulate a chain of blocks with per-block state/block
//! overrides and call lists (execution-apis `ethSimulate`).
//!
//! Request parsing and response serialization live here; the execution engine
//! is [`ethrex_blockchain::simulate`].

use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use ethrex_blockchain::{
    Blockchain,
    simulate::{
        MAX_SIMULATE_BLOCKS, SimBlockOverrides, SimulatedBlock, SimulatedCallError,
        SimulationBlockSpec, SimulationError, SimulationRequest,
    },
};
use ethrex_common::types::{ChainConfig, GenericTransaction};
use ethrex_common::{H256, serde_utils};
use ethrex_crypto::NativeCrypto;
use ethrex_vm::TxValidationError;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use ethrex_rlp::encode::RLPEncode;

use crate::{
    rpc::{RpcApiContext, RpcHandler},
    types::{
        block::{BlockBodyWrapper, FullBlockBody, RpcBlock},
        block_identifier::{BlockIdentifier, BlockIdentifierOrHash},
        block_override::BlockOverrideSet,
        receipt::RpcLog,
        state_override::StateOverrideSet,
        transaction::RpcTransaction,
    },
    utils::RpcErr,
};

/// Wall-clock budget for the whole simulation (mirrors the `debug_traceCall`
/// timeout; geth's `--rpc.evmtimeout` defaults to 5s as well).
const SIMULATE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug)]
pub struct EthSimulateRequest {
    pub payload: SimulatePayload,
    /// Second param: the base block to simulate on. Defaults to `latest`.
    pub block: BlockIdentifierOrHash,
}

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SimulatePayload {
    pub block_state_calls: Vec<BlockStateCall>,
    #[serde(default)]
    pub trace_transfers: bool,
    #[serde(default)]
    pub validation: bool,
    #[serde(default)]
    pub return_full_transactions: bool,
}

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BlockStateCall {
    #[serde(default)]
    pub block_overrides: Option<BlockOverrideSet>,
    #[serde(default)]
    pub state_overrides: Option<StateOverrideSet>,
    #[serde(default)]
    pub calls: Vec<GenericTransaction>,
}

/// One simulated block in the response: the `eth_getBlockByHash` shape
/// extended with per-call results.
///
/// Serialize-only: flattening `RpcBlock` (which itself flattens an untagged
/// body enum) is only fragile in the Deserialize direction, so this type must
/// never derive `Deserialize`.
#[derive(Debug, Serialize)]
pub struct RpcSimulatedBlock {
    #[serde(flatten)]
    pub block: RpcBlock,
    pub calls: Vec<SimulateCallResult>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SimulateCallResult {
    /// "0x1" success / "0x0" failure.
    #[serde(with = "serde_utils::bool")]
    pub status: bool,
    #[serde(with = "serde_utils::bytes")]
    pub return_data: Bytes,
    #[serde(with = "serde_utils::u64::hex_str")]
    pub gas_used: u64,
    /// Gas consumed before refunds.
    #[serde(with = "serde_utils::u64::hex_str")]
    pub max_used_gas: u64,
    /// Always serialized; empty for failed calls.
    pub logs: Vec<SimulateRpcLog>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<SimulateCallErrorJson>,
}

/// `RpcLog` plus `blockTimestamp`, which simulate results carry in each log
/// object. A wrapper keeps `eth_getLogs`/receipt responses untouched.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SimulateRpcLog {
    #[serde(flatten)]
    pub log: RpcLog,
    #[serde(with = "serde_utils::u64::hex_str")]
    pub block_timestamp: u64,
}

#[derive(Debug, Serialize)]
pub struct SimulateCallErrorJson {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
}

impl RpcHandler for EthSimulateRequest {
    fn parse(params: &Option<Vec<Value>>) -> Result<EthSimulateRequest, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.is_empty() || params.len() > 2 {
            return Err(RpcErr::BadParams(format!(
                "Expected one or two params, got {}",
                params.len()
            )));
        }
        // The method's spec pins invalid payloads to -32602; the blanket
        // serde_json conversion would yield BadParams (-32000).
        let payload: SimulatePayload = serde_json::from_value(params[0].clone())
            .map_err(|error| invalid_params(format!("invalid simulate payload: {error}")))?;
        if payload.block_state_calls.is_empty() {
            return Err(invalid_params(
                "empty input: blockStateCalls must not be empty".to_string(),
            ));
        }
        if payload.block_state_calls.len() as u64 > MAX_SIMULATE_BLOCKS {
            return Err(RpcErr::EthSimulate {
                code: -38026,
                message: format!(
                    "client limit exceeded: too many blocks: {} > {MAX_SIMULATE_BLOCKS}",
                    payload.block_state_calls.len()
                ),
                data: None,
            });
        }
        let block = match params.get(1) {
            Some(value) if !value.is_null() => BlockIdentifierOrHash::parse(value.clone(), 1)?,
            _ => BlockIdentifierOrHash::Identifier(BlockIdentifier::default()),
        };
        Ok(EthSimulateRequest { payload, block })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let base_header = match self.block.resolve_block_header(&context.storage).await? {
            Some(header) => header,
            // The result schema is an array, so eth_call's `Null` convention
            // does not apply; unresolvable base blocks are an error.
            None => return Err(RpcErr::BadParams("header not found".to_owned())),
        };
        let chain_config = context.storage.get_chain_config();
        let request = SimulationRequest {
            blocks: self
                .payload
                .block_state_calls
                .iter()
                .cloned()
                .map(|entry| block_state_call_to_spec(entry, &base_header, &chain_config))
                .collect(),
            base: base_header,
            validation: self.payload.validation,
            trace_transfers: self.payload.trace_transfers,
        };
        let blockchain: Arc<Blockchain> = context.blockchain.clone();
        let return_full_transactions = self.payload.return_full_transactions;

        let operation = move || -> Result<Value, RpcErr> {
            let simulated = blockchain
                .simulate_v1(request)
                .map_err(simulation_error_to_rpc)?;
            let blocks = simulated
                .into_iter()
                .map(|block| build_simulated_block(block, return_full_transactions))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(serde_json::to_value(blocks)?)
        };
        // Up to 256 blocks of EVM execution: run off the async runtime, with
        // a wall-clock cap.
        tokio::time::timeout(SIMULATE_TIMEOUT, tokio::task::spawn_blocking(operation))
            .await
            .map_err(|_| RpcErr::Internal("eth_simulateV1 timeout".to_string()))?
            .map_err(|_| RpcErr::Internal("Unexpected runtime error".to_string()))?
    }
}

fn invalid_params(message: String) -> RpcErr {
    RpcErr::EthSimulate {
        code: -32602,
        message,
        data: None,
    }
}

/// Convert one JSON `blockStateCalls` entry into the engine's spec type.
fn block_state_call_to_spec(
    entry: BlockStateCall,
    base_header: &ethrex_common::types::BlockHeader,
    chain_config: &ChainConfig,
) -> SimulationBlockSpec {
    let block_overrides = entry.block_overrides.unwrap_or_default();
    // Fork-schedule hint for the blobBaseFee inversion: the resolved block
    // timestamp is not known until the engine sanitizes the chain, so the
    // override (or the base-relative default) approximates it. Only matters
    // for simulations crossing a blob-schedule fork boundary.
    let timestamp_hint = block_overrides
        .time
        .unwrap_or(base_header.timestamp.saturating_add(12));
    let excess_blob_gas = block_overrides.resolved_excess_blob_gas(chain_config, timestamp_hint);
    SimulationBlockSpec {
        overrides: SimBlockOverrides {
            number: block_overrides.number,
            time: block_overrides.time,
            gas_limit: block_overrides.gas_limit,
            coinbase: block_overrides.coinbase,
            prev_randao: block_overrides.random,
            base_fee_per_gas: block_overrides.base_fee_per_gas,
            excess_blob_gas,
            difficulty: block_overrides.difficulty,
            withdrawals: block_overrides.withdrawals.unwrap_or_default(),
        },
        state_overrides: entry
            .state_overrides
            .map(StateOverrideSet::into_overrides)
            .unwrap_or_default(),
        calls: entry.calls,
    }
}

/// Hydrate one engine result into its response shape: the block serialized
/// like `eth_getBlockByHash` plus per-call results with fully-qualified logs
/// (per-block `logIndex`, tx hash/index, block hash/number/timestamp).
fn build_simulated_block(
    simulated: SimulatedBlock,
    return_full_transactions: bool,
) -> Result<RpcSimulatedBlock, RpcErr> {
    let block_hash = simulated.block.header.hash();
    let block_number = simulated.block.header.number;
    let block_timestamp = simulated.block.header.timestamp;
    let tx_hashes: Vec<H256> = simulated
        .block
        .body
        .transactions
        .iter()
        .map(|tx| tx.hash(&NativeCrypto))
        .collect();

    let mut log_index: u64 = 0;
    let calls = simulated
        .calls
        .into_iter()
        .enumerate()
        .map(|(tx_index, call)| {
            let logs = call
                .logs
                .into_iter()
                .map(|log| {
                    let rpc_log = SimulateRpcLog {
                        log: RpcLog {
                            log: log.into(),
                            log_index,
                            removed: false,
                            transaction_hash: tx_hashes.get(tx_index).copied().unwrap_or_default(),
                            transaction_index: tx_index as u64,
                            block_hash,
                            block_number,
                        },
                        block_timestamp,
                    };
                    log_index += 1;
                    rpc_log
                })
                .collect();
            SimulateCallResult {
                status: call.success,
                return_data: call.return_data,
                gas_used: call.gas_used,
                max_used_gas: call.max_used_gas,
                logs,
                error: call.error.map(|error| match error {
                    SimulatedCallError::Revert { output } => {
                        let data = format!("0x{}", hex::encode(&output));
                        SimulateCallErrorJson {
                            code: 3,
                            message: "execution reverted".to_string(),
                            data: Some(data),
                        }
                    }
                    SimulatedCallError::Halt { reason } => SimulateCallErrorJson {
                        code: -32015,
                        message: reason,
                        data: None,
                    },
                }),
            }
        })
        .collect();

    let block = if return_full_transactions {
        // RpcBlock::build recovers senders from signatures, which are zeroed
        // in simulated transactions — build the full body from the engine's
        // out-of-band senders instead.
        let transactions = simulated
            .block
            .body
            .transactions
            .iter()
            .enumerate()
            .map(|(index, tx)| {
                RpcTransaction::build_with_sender(
                    tx.clone(),
                    Some(block_number),
                    Some(block_hash),
                    Some(index),
                    simulated.senders.get(index).copied().unwrap_or_default(),
                )
            })
            .collect();
        let size = simulated.block.length() as u64;
        RpcBlock {
            hash: block_hash,
            size,
            header: simulated.block.header,
            body: BlockBodyWrapper::Full(FullBlockBody {
                transactions,
                uncles: Vec::new(),
                withdrawals: simulated.block.body.withdrawals.unwrap_or_default(),
            }),
        }
    } else {
        RpcBlock::build(
            simulated.block.header,
            simulated.block.body,
            block_hash,
            false,
        )?
    };
    Ok(RpcSimulatedBlock { block, calls })
}

fn simulation_error_to_rpc(error: SimulationError) -> RpcErr {
    let message = error.to_string();
    let code = match &error {
        SimulationError::TooManyBlocks => -38026,
        SimulationError::BlockNumberNotAscending { .. } => -38020,
        SimulationError::TimestampNotAscending { .. } => -38021,
        SimulationError::BlockGasLimitReached { .. } => -38015,
        SimulationError::InvalidTx(validation_error) => validation_error_code(validation_error),
        SimulationError::InvalidParams(_) => -32602,
        // geth reports invalid movePrecompileToAddress usage as a plain
        // server error.
        SimulationError::PrecompileOverride(_) => -32000,
        SimulationError::Internal(_) => -32603,
    };
    RpcErr::EthSimulate {
        code,
        message,
        data: None,
    }
}

/// Spec error codes for transaction validation failures (geth
/// `internal/ethapi/errors.go` `txValidationError`).
fn validation_error_code(error: &TxValidationError) -> i32 {
    match error {
        TxValidationError::NonceMismatch { expected, actual } => {
            if actual < expected {
                -38010 // nonce too low
            } else {
                -38011 // nonce too high
            }
        }
        TxValidationError::NonceIsMax => -38011,
        TxValidationError::InsufficientMaxFeePerGas
        | TxValidationError::InsufficientMaxFeePerBlobGas { .. } => -38012,
        TxValidationError::IntrinsicGasTooLow
        | TxValidationError::IntrinsicGasBelowFloorGasCost => -38013,
        TxValidationError::InsufficientAccountFunds
        | TxValidationError::GasLimitPriceProductOverflow => -38014,
        TxValidationError::SenderNotEOA(_) => -38024,
        TxValidationError::InitcodeSizeExceeded { .. } => -38025,
        TxValidationError::PriorityGreaterThanMaxFeePerGas { .. }
        | TxValidationError::Type3TxPreFork
        | TxValidationError::Type3TxZeroBlobs
        | TxValidationError::Type3TxInvalidBlobVersionedHash
        | TxValidationError::Type3TxBlobCountExceeded { .. }
        | TxValidationError::Type3TxContractCreation
        | TxValidationError::Type4TxPreFork
        | TxValidationError::Type4TxAuthorizationListIsEmpty
        | TxValidationError::Type4TxContractCreation => -32602,
        _ => -32603,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::RpcErrorMetadata;
    use ethrex_common::Address;
    use ethrex_common::types::{Block, BlockBody, BlockHeader, Log, Receipt, TxKind};
    use serde_json::json;

    fn parse(params: Vec<Value>) -> Result<EthSimulateRequest, RpcErr> {
        EthSimulateRequest::parse(&Some(params))
    }

    fn error_code(err: RpcErr) -> i32 {
        RpcErrorMetadata::from(err).code
    }

    #[test]
    fn parse_defaults_block_to_latest_and_flags_to_false() {
        let is_latest = |request: &EthSimulateRequest| {
            matches!(
                request.block,
                BlockIdentifierOrHash::Identifier(BlockIdentifier::Tag(_))
            )
        };
        let request = parse(vec![json!({"blockStateCalls": [{}]})]).unwrap();
        assert!(is_latest(&request));
        assert!(!request.payload.trace_transfers);
        assert!(!request.payload.validation);
        assert!(!request.payload.return_full_transactions);
        // Explicit null block param behaves like a missing one.
        let request = parse(vec![json!({"blockStateCalls": [{}]}), Value::Null]).unwrap();
        assert!(is_latest(&request));
    }

    #[test]
    fn parse_accepts_tag_number_and_hash_block_params() {
        for block in [
            json!("latest"),
            json!("0x10"),
            json!("0x1234567890123456789012345678901234567890123456789012345678901234"),
            json!({"blockNumber": "0x10"}),
        ] {
            parse(vec![json!({"blockStateCalls": [{}]}), block]).unwrap();
        }
    }

    #[test]
    fn parse_rejects_wrong_arity() {
        assert!(parse(vec![]).is_err());
        assert!(
            parse(vec![
                json!({"blockStateCalls": [{}]}),
                json!("latest"),
                json!("extra")
            ])
            .is_err()
        );
    }

    #[test]
    fn parse_maps_payload_errors_to_invalid_params() {
        // Missing blockStateCalls.
        assert_eq!(error_code(parse(vec![json!({})]).unwrap_err()), -32602);
        // Empty blockStateCalls.
        assert_eq!(
            error_code(parse(vec![json!({"blockStateCalls": []})]).unwrap_err()),
            -32602
        );
        // Unknown payload key.
        assert_eq!(
            error_code(parse(vec![json!({"blockStateCalls": [{}], "bogus": true})]).unwrap_err()),
            -32602
        );
        // Unknown blockStateCall key.
        assert_eq!(
            error_code(parse(vec![json!({"blockStateCalls": [{"bogus": true}]})]).unwrap_err()),
            -32602
        );
    }

    #[test]
    fn parse_rejects_too_many_block_state_calls() {
        let entries: Vec<Value> = (0..257).map(|_| json!({})).collect();
        let err = parse(vec![json!({"blockStateCalls": entries})]).unwrap_err();
        assert_eq!(error_code(err), -38026);
    }

    #[test]
    fn parse_call_objects_with_missing_fields() {
        // `{}` and from-only call objects are valid; missing `to` is a create.
        let request = parse(vec![json!({"blockStateCalls": [{"calls": [
            {},
            {"from": "0xc000000000000000000000000000000000000000"},
            {"input": "0x00"},
            {"data": "0x00"},
        ]}]})])
        .unwrap();
        let calls = &request.payload.block_state_calls[0].calls;
        assert_eq!(calls.len(), 4);
        assert!(matches!(calls[0].to, TxKind::Create));
        assert_eq!(calls[0].from, Address::zero());
        assert_eq!(calls[2].input, calls[3].input);
    }

    #[test]
    fn simulate_error_codes_map_to_spec() {
        assert_eq!(
            error_code(simulation_error_to_rpc(SimulationError::TooManyBlocks)),
            -38026
        );
        assert_eq!(
            error_code(simulation_error_to_rpc(
                SimulationError::BlockNumberNotAscending { given: 1, prev: 2 }
            )),
            -38020
        );
        assert_eq!(
            error_code(simulation_error_to_rpc(
                SimulationError::TimestampNotAscending { given: 1, prev: 2 }
            )),
            -38021
        );
        assert_eq!(
            error_code(simulation_error_to_rpc(
                SimulationError::BlockGasLimitReached {
                    requested: 2,
                    remaining: 1
                }
            )),
            -38015
        );
        assert_eq!(
            error_code(simulation_error_to_rpc(SimulationError::InvalidTx(
                TxValidationError::NonceMismatch {
                    expected: 5,
                    actual: 1
                }
            ))),
            -38010
        );
        assert_eq!(
            error_code(simulation_error_to_rpc(SimulationError::InvalidTx(
                TxValidationError::NonceMismatch {
                    expected: 1,
                    actual: 5
                }
            ))),
            -38011
        );
        assert_eq!(
            error_code(simulation_error_to_rpc(SimulationError::InvalidTx(
                TxValidationError::InsufficientMaxFeePerGas
            ))),
            -38012
        );
        assert_eq!(
            error_code(simulation_error_to_rpc(SimulationError::InvalidTx(
                TxValidationError::IntrinsicGasTooLow
            ))),
            -38013
        );
        assert_eq!(
            error_code(simulation_error_to_rpc(SimulationError::InvalidTx(
                TxValidationError::InsufficientAccountFunds
            ))),
            -38014
        );
    }

    /// Exact-JSON tripwire for the `#[serde(flatten)]` of `RpcBlock` (which
    /// itself flattens header + untagged body): a change in the serialized
    /// shape of simulate results should fail here, not in hive.
    #[test]
    fn serialize_simulated_block_shape() {
        use ethrex_blockchain::simulate::SimulatedCallResult;

        let header = BlockHeader {
            number: 101,
            timestamp: 1012,
            gas_limit: 30_000_000,
            base_fee_per_gas: Some(0),
            ..Default::default()
        };
        let body = BlockBody {
            transactions: vec![],
            ommers: vec![],
            withdrawals: None,
        };
        let block = Block::new(header, body);
        let hash = block.header.hash();
        let simulated = SimulatedBlock {
            block,
            receipts: Vec::<Receipt>::new(),
            calls: vec![
                SimulatedCallResult {
                    success: true,
                    return_data: Bytes::new(),
                    gas_used: 21000,
                    max_used_gas: 21000,
                    logs: vec![Log {
                        address: Address::repeat_byte(0xee),
                        topics: vec![H256::zero()],
                        data: Bytes::from_static(&[0x01]),
                    }],
                    error: None,
                },
                SimulatedCallResult {
                    success: false,
                    return_data: Bytes::new(),
                    gas_used: 30000,
                    max_used_gas: 30000,
                    logs: vec![],
                    error: Some(SimulatedCallError::Revert {
                        output: Bytes::from_static(&[0xab, 0xcd]),
                    }),
                },
            ],
            senders: vec![Address::zero(), Address::zero()],
        };
        let value = serde_json::to_value(build_simulated_block(simulated, false).unwrap()).unwrap();

        // Block fields are flattened at the top level.
        assert_eq!(value["number"], json!("0x65"));
        assert_eq!(value["hash"], json!(format!("{hash:#x}")));
        assert!(value["transactions"].is_array());
        // Success call: status/gas/maxUsedGas/logs with blockTimestamp.
        let success = &value["calls"][0];
        assert_eq!(success["status"], json!("0x1"));
        assert_eq!(success["gasUsed"], json!("0x5208"));
        assert_eq!(success["maxUsedGas"], json!("0x5208"));
        assert_eq!(success["returnData"], json!("0x"));
        assert!(success.get("error").is_none());
        let log = &success["logs"][0];
        assert_eq!(log["logIndex"], json!("0x0"));
        assert_eq!(log["transactionIndex"], json!("0x0"));
        assert_eq!(log["blockNumber"], json!("0x65"));
        assert_eq!(log["blockTimestamp"], json!("0x3f4"));
        assert_eq!(log["removed"], json!(false));
        // Failed call: status 0x0, empty logs, revert error with data.
        let failure = &value["calls"][1];
        assert_eq!(failure["status"], json!("0x0"));
        assert_eq!(failure["logs"], json!([]));
        assert_eq!(failure["error"]["code"], json!(3));
        assert_eq!(failure["error"]["data"], json!("0xabcd"));
    }

    #[test]
    fn block_override_aliases_parse() {
        let set: BlockOverrideSet = serde_json::from_value(json!({
            "feeRecipient": "0x000000000000000000000000000000000000beef",
            "prevRandao": "0x000000000000000000000000000000000000000000000000000000000000dead",
            "blobBaseFee": "0x100",
            "withdrawals": [],
        }))
        .unwrap();
        assert!(set.coinbase.is_some());
        assert!(set.random.is_some());
        assert!(set.blob_base_fee_per_gas.is_some());
        assert_eq!(set.withdrawals.as_deref(), Some(&[][..]));
        assert!(!set.is_empty());
    }
}

/// End-to-end scenarios ported from the execution-apis `eth_simulateV1` `.io`
/// corpus, adapted to the `fixtures/genesis/l1.json` in-memory chain (the
/// hive fixtures use a pre-merge genesis ethrex cannot import, so responses
/// are asserted by JSON path rather than whole-body equality).
#[cfg(test)]
mod integration_tests {
    use crate::rpc::map_http_requests;
    use crate::test_utils::{default_context_with_storage, setup_store};
    use crate::utils::{RpcErr, RpcErrorMetadata, RpcRequest};
    use serde_json::{Value, json};

    /// Funded EOA from fixtures/genesis/l1.json (10^9 ETH).
    const RICH: &str = "0x00000a8d3f37af8def18832962ee008d8dca4f7b";
    /// Fresh addresses with no genesis state.
    const FRESH_A: &str = "0xc000000000000000000000000000000000000000";
    const FRESH_B: &str = "0xc100000000000000000000000000000000000000";

    async fn simulate(params: Value) -> Result<Value, RpcErr> {
        let storage = setup_store().await;
        let context = default_context_with_storage(storage).await;
        let request: RpcRequest = serde_json::from_value(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "eth_simulateV1",
            "params": params,
        }))
        .unwrap();
        map_http_requests(&request, context).await
    }

    fn error_code(err: RpcErr) -> i32 {
        RpcErrorMetadata::from(err).code
    }

    #[tokio::test]
    async fn simple_transfer_with_balance_override() {
        // `ethSimulate-simple.io`: fund a fresh account via stateOverrides,
        // then transfer out of it.
        let result = simulate(json!([{
            "blockStateCalls": [{
                "stateOverrides": {FRESH_A: {"balance": "0xde0b6b3a7640000"}},
                "calls": [{"from": FRESH_A, "to": FRESH_B, "value": "0x1"}],
            }],
        }, "latest"]))
        .await
        .unwrap();
        let blocks = result.as_array().unwrap();
        assert_eq!(blocks.len(), 1);
        // Genesis is block 0, so the simulated block is 1.
        assert_eq!(blocks[0]["number"], json!("0x1"));
        let call = &blocks[0]["calls"][0];
        assert_eq!(call["status"], json!("0x1"));
        assert_eq!(call["gasUsed"], json!("0x5208"));
        assert_eq!(call["maxUsedGas"], json!("0x5208"));
        assert_eq!(call["returnData"], json!("0x"));
        assert_eq!(call["logs"], json!([]));
        // Hash-only transactions by default.
        assert!(blocks[0]["transactions"][0].is_string());
    }

    #[tokio::test]
    async fn empty_block_state_call_produces_empty_block() {
        // `ethSimulate-empty.io`.
        let result = simulate(json!([{"blockStateCalls": [{}]}, "latest"]))
            .await
            .unwrap();
        let blocks = result.as_array().unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0]["calls"], json!([]));
        assert_eq!(blocks[0]["transactions"], json!([]));
        assert_eq!(blocks[0]["gasUsed"], json!("0x0"));
    }

    #[tokio::test]
    async fn block_numbers_must_ascend() {
        // `ethSimulate-block-num-order-38020.io`.
        let err = simulate(json!([{
            "blockStateCalls": [
                {"blockOverrides": {"number": "0x10"}},
                {"blockOverrides": {"number": "0x5"}},
            ],
        }, "latest"]))
        .await
        .unwrap_err();
        assert_eq!(error_code(err), -38020);
    }

    #[tokio::test]
    async fn timestamps_must_increase() {
        // `ethSimulate-block-timestamp-order-38021.io`: equal timestamps are
        // rejected too.
        let err = simulate(json!([{
            "blockStateCalls": [
                {"blockOverrides": {"time": "0x6668e112"}},
                {"blockOverrides": {"time": "0x6668e112"}},
            ],
        }, "latest"]))
        .await
        .unwrap_err();
        assert_eq!(error_code(err), -38021);
    }

    #[tokio::test]
    async fn explicit_gas_above_block_limit_aborts() {
        // `ethSimulate-run-out-of-gas-in-block-38015.io` (adapted): a small
        // gasLimit override plus an explicit call gas beyond it.
        let err = simulate(json!([{
            "blockStateCalls": [{
                "blockOverrides": {"gasLimit": "0x5208"},
                "calls": [{"from": RICH, "to": FRESH_B, "gas": "0x10000"}],
            }],
        }, "latest"]))
        .await
        .unwrap_err();
        assert_eq!(error_code(err), -38015);
    }

    #[tokio::test]
    async fn validation_requires_base_fee() {
        // `ethSimulate-basefee-too-low-with-validation-38012.io`: with
        // validation the block keeps a real base fee, so zero-fee calls fail.
        let err = simulate(json!([{
            "validation": true,
            "blockStateCalls": [{
                "calls": [{"from": RICH, "to": FRESH_B, "value": "0x1"}],
            }],
        }, "latest"]))
        .await
        .unwrap_err();
        assert_eq!(error_code(err), -38012);
    }

    #[tokio::test]
    async fn insufficient_funds_aborts() {
        // `ethSimulate-simple-no-funds.io`: value transfers from an unfunded
        // account fail the balance check in both validation modes.
        let err = simulate(json!([{
            "blockStateCalls": [{
                "calls": [{"from": FRESH_A, "to": FRESH_B, "value": "0x1"}],
            }],
        }, "latest"]))
        .await
        .unwrap_err();
        assert_eq!(error_code(err), -38014);
    }

    /// A nonce-gapped call (explicit nonce above the account nonce) under
    /// `validation: true` — the sender is funded and the fees cover the real
    /// base fee, so the nonce check is the one that fires.
    fn gapped_nonce_call(validation: bool) -> Value {
        json!([{
            "validation": validation,
            "blockStateCalls": [{
                "calls": [{
                    "from": RICH,
                    "to": FRESH_B,
                    "nonce": "0x5",
                    "gas": "0x5208",
                    "maxFeePerGas": "0x77359400",
                }],
            }],
        }, "latest"])
    }

    #[tokio::test]
    async fn validation_enforces_nonce_too_high() {
        // Unlike lax simulators, a gapped nonce must abort the request with
        // -38011 under validation, matching geth.
        let err = simulate(gapped_nonce_call(true)).await.unwrap_err();
        assert_eq!(error_code(err), -38011);
    }

    #[tokio::test]
    async fn validation_enforces_nonce_too_low() {
        // Account nonce raised to 0xa via override; explicit nonce 0x1 is
        // stale -> -38010 under validation.
        let err = simulate(json!([{
            "validation": true,
            "blockStateCalls": [{
                "stateOverrides": {RICH: {"nonce": "0xa"}},
                "calls": [{
                    "from": RICH,
                    "to": FRESH_B,
                    "nonce": "0x1",
                    "gas": "0x5208",
                    "maxFeePerGas": "0x77359400",
                }],
            }],
        }, "latest"]))
        .await
        .unwrap_err();
        assert_eq!(error_code(err), -38010);
    }

    #[tokio::test]
    async fn validation_accepts_correct_nonce() {
        // Same call with the right nonce sanity-checks that validation mode
        // does not over-reject.
        let result = simulate(json!([{
            "validation": true,
            "blockStateCalls": [{
                "calls": [{
                    "from": RICH,
                    "to": FRESH_B,
                    "nonce": "0x0",
                    "gas": "0x5208",
                    "maxFeePerGas": "0x77359400",
                }],
            }],
        }, "latest"]))
        .await
        .unwrap();
        assert_eq!(result[0]["calls"][0]["status"], json!("0x1"));
    }

    #[tokio::test]
    async fn gapped_nonce_passes_without_validation() {
        // `ethSimulate-transaction-too-high-nonce.io`: without validation the
        // explicit gapped nonce is ignored, like eth_call.
        let result = simulate(gapped_nonce_call(false)).await.unwrap();
        assert_eq!(result[0]["calls"][0]["status"], json!("0x1"));
    }

    #[tokio::test]
    async fn trace_transfers_emits_synthetic_log() {
        // `ethSimulate-eth-send-should-produce-logs.io`.
        let result = simulate(json!([{
            "traceTransfers": true,
            "blockStateCalls": [{
                "calls": [{"from": RICH, "to": FRESH_B, "value": "0x7b"}],
            }],
        }, "latest"]))
        .await
        .unwrap();
        let call = &result[0]["calls"][0];
        assert_eq!(call["status"], json!("0x1"));
        let log = &call["logs"][0];
        assert_eq!(
            log["address"],
            json!("0xeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee")
        );
        assert_eq!(
            log["topics"][0],
            json!("0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef")
        );
        // Topics 1/2 are the padded from/to addresses; data is the value.
        assert_eq!(
            log["topics"][1],
            json!(format!("0x000000000000000000000000{}", &RICH[2..]))
        );
        assert_eq!(
            log["data"],
            json!("0x000000000000000000000000000000000000000000000000000000000000007b")
        );
        assert_eq!(log["logIndex"], json!("0x0"));
        assert_eq!(log["removed"], json!(false));
        assert!(log["blockTimestamp"].is_string());
        // The synthetic log stays out of the block's logs bloom.
        assert_eq!(
            result[0]["logsBloom"],
            json!(format!("0x{}", "0".repeat(512)))
        );
    }

    #[tokio::test]
    async fn state_carries_across_simulated_blocks() {
        // `ethSimulate-transfer-over-BlockStateCalls.io` (adapted): block 1
        // funds FRESH_A from the rich account; block 2 spends from FRESH_A.
        let result = simulate(json!([{
            "blockStateCalls": [
                {"calls": [{"from": RICH, "to": FRESH_A, "value": "0xde0b6b3a7640000"}]},
                {"calls": [{"from": FRESH_A, "to": FRESH_B, "value": "0x1"}]},
            ],
        }, "latest"]))
        .await
        .unwrap();
        let blocks = result.as_array().unwrap();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0]["calls"][0]["status"], json!("0x1"));
        assert_eq!(blocks[1]["calls"][0]["status"], json!("0x1"));
        assert_eq!(blocks[0]["number"], json!("0x1"));
        assert_eq!(blocks[1]["number"], json!("0x2"));
        // Chained parent hash: block 2's parentHash is block 1's hash.
        assert_eq!(blocks[1]["parentHash"], blocks[0]["hash"]);
        // State roots differ (both blocks mutate state).
        assert_ne!(blocks[0]["stateRoot"], blocks[1]["stateRoot"]);
    }

    #[tokio::test]
    async fn gap_blocks_are_included_in_response() {
        // `ethSimulate-blocknumber-increment.io` (adapted): jumping to
        // base+3 yields the two gap blocks as well.
        let result = simulate(json!([{
            "blockStateCalls": [{"blockOverrides": {"number": "0x3"}}],
        }, "latest"]))
        .await
        .unwrap();
        let blocks = result.as_array().unwrap();
        assert_eq!(blocks.len(), 3);
        let numbers: Vec<&Value> = blocks.iter().map(|b| &b["number"]).collect();
        assert_eq!(numbers, vec![&json!("0x1"), &json!("0x2"), &json!("0x3")]);
        // Timestamps advance by 12 per block.
        let ts: Vec<u64> = blocks
            .iter()
            .map(|b| {
                u64::from_str_radix(
                    b["timestamp"].as_str().unwrap().trim_start_matches("0x"),
                    16,
                )
                .unwrap()
            })
            .collect();
        assert_eq!(ts[1] - ts[0], 12);
        assert_eq!(ts[2] - ts[1], 12);
    }

    #[tokio::test]
    async fn return_full_transactions() {
        // `ethSimulate-simple-validation-fulltx.io` (adapted, without
        // validation): transactions come back as objects.
        let result = simulate(json!([{
            "returnFullTransactions": true,
            "blockStateCalls": [{
                "calls": [{"from": RICH, "to": FRESH_B, "value": "0x1"}],
            }],
        }, "latest"]))
        .await
        .unwrap();
        let tx = &result[0]["transactions"][0];
        assert!(tx.is_object());
        // Spec default tx type is EIP-1559 (0x2).
        assert_eq!(tx["type"], json!("0x2"));
        assert_eq!(tx["nonce"], json!("0x0"));
        assert!(tx["hash"].is_string());
    }

    #[tokio::test]
    async fn block_overrides_reflect_in_result_header() {
        // `ethSimulate-override-block-num.io` (adapted) + simulate-spelled
        // field names (feeRecipient/prevRandao).
        let result = simulate(json!([{
            "blockStateCalls": [{
                "blockOverrides": {
                    "number": "0x5",
                    "time": "0x6668e200",
                    "gasLimit": "0x1000000",
                    "feeRecipient": "0x000000000000000000000000000000000000beef",
                    "baseFeePerGas": "0x7",
                },
            }],
        }, "latest"]))
        .await
        .unwrap();
        let block = result.as_array().unwrap().last().unwrap().clone();
        assert_eq!(block["number"], json!("0x5"));
        assert_eq!(block["timestamp"], json!("0x6668e200"));
        assert_eq!(block["gasLimit"], json!("0x1000000"));
        assert_eq!(
            block["miner"],
            json!("0x000000000000000000000000000000000000beef")
        );
        assert_eq!(block["baseFeePerGas"], json!("0x7"));
    }

    /// Identity precompile (0x04).
    const IDENTITY: &str = "0x0000000000000000000000000000000000000004";
    const MOVE_DEST: &str = "0xc900000000000000000000000000000000000000";

    #[tokio::test]
    async fn move_precompile_executes_at_destination_only() {
        // `ethSimulate-move-ecrecover-and-call-old-and-new.io` (adapted to the
        // identity precompile): after the move, the destination echoes input
        // and the source behaves like an empty account.
        let result = simulate(json!([{
            "blockStateCalls": [{
                "stateOverrides": {IDENTITY: {"movePrecompileToAddress": MOVE_DEST}},
                "calls": [
                    {"from": RICH, "to": MOVE_DEST, "input": "0xdeadbeef"},
                    {"from": RICH, "to": IDENTITY, "input": "0xdeadbeef"},
                ],
            }],
        }, "latest"]))
        .await
        .unwrap();
        let calls = &result[0]["calls"];
        assert_eq!(calls[0]["status"], json!("0x1"));
        assert_eq!(calls[0]["returnData"], json!("0xdeadbeef"));
        assert_eq!(calls[1]["status"], json!("0x1"));
        assert_eq!(calls[1]["returnData"], json!("0x"));
    }

    #[tokio::test]
    async fn move_is_scoped_to_its_block() {
        // The move applies to block 1 only; block 2 sees the canonical
        // precompile layout again.
        let result = simulate(json!([{
            "blockStateCalls": [
                {"stateOverrides": {IDENTITY: {"movePrecompileToAddress": MOVE_DEST}},
                 "calls": [{"from": RICH, "to": MOVE_DEST, "input": "0x01"}]},
                {"calls": [
                    {"from": RICH, "to": MOVE_DEST, "input": "0x02"},
                    {"from": RICH, "to": IDENTITY, "input": "0x03"},
                ]},
            ],
        }, "latest"]))
        .await
        .unwrap();
        assert_eq!(result[0]["calls"][0]["returnData"], json!("0x01"));
        // Block 2: destination is a plain account again, source echoes again.
        assert_eq!(result[1]["calls"][0]["returnData"], json!("0x"));
        assert_eq!(result[1]["calls"][1]["returnData"], json!("0x03"));
    }

    #[tokio::test]
    async fn overriding_code_at_a_precompile_disables_it() {
        // geth removes precompile behavior for any overridden precompile
        // address: the deployed bytecode executes instead (PUSH1 1 MSTORE
        // MSTORE8-free simple return of one word).
        let result = simulate(json!([{
            "blockStateCalls": [{
                "stateOverrides": {IDENTITY: {"code": "0x600160005260206000f3"}},
                "calls": [{"from": RICH, "to": IDENTITY, "input": "0xdeadbeef"}],
            }],
        }, "latest"]))
        .await
        .unwrap();
        let call = &result[0]["calls"][0];
        assert_eq!(call["status"], json!("0x1"));
        assert_eq!(
            call["returnData"],
            json!("0x0000000000000000000000000000000000000000000000000000000000000001")
        );
    }

    #[tokio::test]
    async fn moving_a_non_precompile_fails() {
        // `ethSimulate-try-to-move-non-precompile.io`.
        let err = simulate(json!([{
            "blockStateCalls": [{
                "stateOverrides": {FRESH_A: {"movePrecompileToAddress": MOVE_DEST}},
            }],
        }, "latest"]))
        .await
        .unwrap_err();
        let metadata = RpcErrorMetadata::from(err);
        assert_eq!(metadata.code, -32000);
        assert!(
            metadata.message.contains("is not a precompile"),
            "unexpected message: {}",
            metadata.message
        );
    }

    #[tokio::test]
    async fn moving_to_an_overridden_address_fails() {
        let err = simulate(json!([{
            "blockStateCalls": [{
                "stateOverrides": {
                    IDENTITY: {"movePrecompileToAddress": FRESH_A},
                    FRESH_A: {"balance": "0x1"},
                },
            }],
        }, "latest"]))
        .await
        .unwrap_err();
        let metadata = RpcErrorMetadata::from(err);
        assert_eq!(metadata.code, -32000);
        assert!(
            metadata.message.contains("is already overridden"),
            "unexpected message: {}",
            metadata.message
        );
    }
}
