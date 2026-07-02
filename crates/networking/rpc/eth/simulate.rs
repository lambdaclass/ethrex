//! `eth_simulateV1`: simulate a chain of blocks with per-block state/block
//! overrides and call lists (execution-apis `ethSimulate`).
//!
//! Request parsing and response serialization live here; the execution engine
//! is [`ethrex_blockchain::simulate`].

use std::collections::BTreeMap;
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

use crate::{
    rpc::{RpcApiContext, RpcHandler},
    types::{
        block::RpcBlock,
        block_identifier::{BlockIdentifier, BlockIdentifierOrHash},
        block_override::BlockOverrideSet,
        receipt::RpcLog,
        state_override::StateOverrideSet,
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

    let block = RpcBlock::build(
        simulated.block.header,
        simulated.block.body,
        block_hash,
        return_full_transactions,
    )?;
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
