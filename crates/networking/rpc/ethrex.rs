//! ethrex-specific JSON-RPC methods (`ethrex_*` namespace).
//!
//! These are non-standard extensions that ethrex exposes outside the
//! standardized `eth_`/`debug_` namespaces. They live in a dedicated namespace
//! so operators can enable them on a public endpoint (`--http.api ethrex`)
//! without also exposing the whole `debug_` surface.

use ethrex_blockchain::vm::StoreVmDatabase;
use ethrex_common::{
    Address, U256,
    types::{
        BlockHeader, FRAME_RECEIPT_STATUS_SUCCESS, PrefixShape, Transaction, ValidationPrefix,
    },
};
use ethrex_vm::backends::{FrameValidationOutcome, levm::get_max_allowed_gas_limit};
use serde::Serialize;
use serde_json::Value;

use crate::{
    rpc::{RpcApiContext, RpcHandler},
    types::block_identifier::{BlockIdentifier, BlockIdentifierOrHash},
    utils::RpcErr,
};

/// `ethrex_simulateFrameTransaction` — dry-run the EIP-8141 validation prefix
/// (the same check the mempool runs on `eth_sendRawTransaction`) plus a full
/// multi-frame execution, WITHOUT submitting the transaction, so a client can
/// learn whether a frame transaction is valid and how much gas it consumes
/// before sending it.
#[derive(Debug)]
pub struct SimulateFrameTransactionRequest {
    /// Decoded type-`0x06` frame transaction (validated in `parse`).
    transaction: Transaction,
    /// Block the simulation runs against. Defaults to `latest`.
    block: Option<BlockIdentifierOrHash>,
}

/// Result of `ethrex_simulateFrameTransaction`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SimulateFrameTransactionResult {
    /// Whether the EIP-8141 validation prefix passed — the frame-specific
    /// admission check the mempool runs. This is NECESSARY but not SUFFICIENT
    /// for admission: standard gates (outer signatures, nonce, fees, per-tx gas
    /// cap, paymaster funding) are not all re-checked here. A `false` never
    /// under-rejects (the mempool uses this same prefix simulation).
    valid: bool,
    /// Recognized validation-prefix shape, or `null` if the prefix is
    /// structurally invalid.
    prefix_shape: Option<String>,
    /// The payer (paymaster or self-funded sender) established by the prefix,
    /// or `null` if none was established.
    payer: Option<Address>,
    /// The transaction's max cost (TXPARAM `0x06`), as a `0x`-hex wei value.
    /// Always present — it is a pure function of the transaction fields.
    max_cost: String,
    /// Reason the transaction is invalid — a validation-prefix failure or a
    /// pre-simulation gate such as the per-transaction gas cap. `null` when
    /// `valid` is true.
    violation: Option<String>,
    /// Accurate total gas used across all frames, as `0x`-hex. `null` when the
    /// prefix is invalid, the tx exceeds the simulation gas cap, or the full
    /// execution errored.
    gas_used: Option<String>,
    /// Per-frame gas used and success, when a full execution ran; `null`
    /// otherwise.
    frames: Option<Vec<FrameExecResult>>,
    /// Top-level execution summary: `"success"` (every frame succeeded) or
    /// `"reverted"` (at least one frame did not — see per-frame `frames`).
    /// `null` if the full execution was not run or errored.
    execution_status: Option<String>,
    /// Error string if the full execution could not run or complete (e.g. the
    /// tx exceeds the simulation gas cap, the body reverted the whole tx under
    /// the frame-tx exclusion model, or the payer was underfunded). `null`
    /// otherwise.
    execution_error: Option<String>,
}

/// Per-frame execution outcome for the full-execution step.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FrameExecResult {
    /// Gas used by this frame, as `0x`-hex.
    gas_used: String,
    /// Whether this frame completed successfully (did not revert/halt/skip).
    succeeded: bool,
}

/// Stable wire name for a validation-prefix shape (decoupled from the Rust
/// `Debug` representation, which must not leak into the public API).
fn prefix_shape_name(shape: &PrefixShape) -> &'static str {
    match shape {
        PrefixShape::SelfVerify => "SelfVerify",
        PrefixShape::DeploySelfVerify => "DeploySelfVerify",
        PrefixShape::OnlyVerifyPay => "OnlyVerifyPay",
        PrefixShape::DeployOnlyVerifyPay => "DeployOnlyVerifyPay",
    }
}

impl RpcHandler for SimulateFrameTransactionRequest {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.is_empty() || params.len() > 2 {
            return Err(RpcErr::BadParams(format!(
                "Expected one or two params and {} were provided",
                params.len()
            )));
        }

        let raw: String = serde_json::from_value(params[0].clone())
            .map_err(|error| RpcErr::BadParams(error.to_string()))?;
        let raw = raw
            .strip_prefix("0x")
            .ok_or_else(|| RpcErr::BadParams("rawTx is not 0x-prefixed".to_owned()))?;
        let bytes = hex::decode(raw).map_err(|error| RpcErr::BadParams(error.to_string()))?;

        let transaction = Transaction::decode_canonical(&bytes)
            .map_err(|error| RpcErr::BadParams(error.to_string()))?;
        if !matches!(transaction, Transaction::FrameTransaction(_)) {
            return Err(RpcErr::BadParams(
                "rawTx is not a type-0x06 frame transaction".to_owned(),
            ));
        }

        let block = match params.get(1) {
            Some(value) => Some(BlockIdentifierOrHash::parse(value.clone(), 1)?),
            None => None,
        };

        Ok(SimulateFrameTransactionRequest { transaction, block })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let block = self
            .block
            .clone()
            .unwrap_or(BlockIdentifierOrHash::Identifier(BlockIdentifier::default()));
        let header = match block.resolve_block_header(&context.storage).await? {
            Some(header) => header,
            _ => return Ok(Value::Null),
        };

        let Transaction::FrameTransaction(frame_tx) = &self.transaction else {
            // Guaranteed by `parse`; kept as a defensive guard.
            return Err(RpcErr::BadParams(
                "rawTx is not a type-0x06 frame transaction".to_owned(),
            ));
        };

        // `max_cost` is a pure function of the tx fields (no EVM pass), so it is
        // reported on every path, including structural rejection.
        let max_cost = to_hex_u256(frame_tx.max_cost());

        // Derive and structurally validate the prefix. A structural error means
        // the transaction is invalid without needing an EVM pass.
        let prefix = match frame_tx.validation_prefix() {
            Ok(prefix) => prefix,
            Err(error) => return structurally_invalid(error.to_string(), max_cost),
        };
        if let Err(error) = frame_tx.validate_prefix_structure(&prefix) {
            return structurally_invalid(error.to_string(), max_cost);
        }
        let prefix_shape = Some(prefix_shape_name(&prefix.shape).to_owned());

        // DoS guard, applied BEFORE any EVM work. Both the prefix simulation and
        // the full execution below run real opcodes bounded only by the tx's own
        // (attacker-controlled) per-frame gas limits — the prefix's MAX_VERIFY_GAS
        // ceiling is enforced only post-hoc, so an uncapped prefix frame would burn
        // unbounded CPU. Gate on the same per-tx cap `eth_estimateGas` uses (and
        // that the mempool checks before its own prefix sim); a tx above it is
        // rejected on submit (EIP-7825 / block gas limit) anyway.
        let fork = context.storage.get_chain_config().fork(header.timestamp);
        let max_allowed = get_max_allowed_gas_limit(header.gas_limit, fork);
        let total_gas_limit = frame_tx.total_gas_limit();
        if total_gas_limit > max_allowed {
            return to_value(SimulateFrameTransactionResult {
                valid: false,
                prefix_shape,
                payer: None,
                max_cost,
                violation: Some(format!(
                    "total gas limit {total_gas_limit} exceeds the per-transaction gas cap {max_allowed} (EIP-7825); not simulated"
                )),
                gas_used: None,
                frames: None,
                execution_status: None,
                execution_error: None,
            });
        }

        // Gas is bounded; run the validation-prefix simulation on a fresh,
        // throwaway state at the requested head — the same machinery the mempool
        // runs. Read-only: never touches the mempool or block building.
        let outcome = self.simulate_prefix(&context, &header, &prefix)?;
        let payer = outcome.accessed_paymaster.map(|(payer, _)| payer);

        if !outcome.passed {
            return to_value(SimulateFrameTransactionResult {
                valid: false,
                prefix_shape,
                payer,
                max_cost,
                violation: Some(
                    outcome
                        .violation
                        .unwrap_or_else(|| "validation prefix did not pass".to_owned()),
                ),
                gas_used: None,
                frames: None,
                execution_status: None,
                execution_error: None,
            });
        }

        // The prefix passed and gas is bounded (checked above); run a full
        // multi-frame execution on a SEPARATE fresh state (the prefix simulation
        // mutated its own throwaway state) for accurate total + per-frame gas.
        let (gas_used, frames, execution_status, execution_error) =
            self.execute_for_gas(&context, &header, frame_tx.sender);

        to_value(SimulateFrameTransactionResult {
            valid: true,
            prefix_shape,
            payer,
            max_cost,
            violation: None,
            gas_used,
            frames,
            execution_status,
            execution_error,
        })
    }
}

impl SimulateFrameTransactionRequest {
    /// Runs the EIP-8141 validation-prefix simulation over a fresh throwaway
    /// state at `header`.
    fn simulate_prefix(
        &self,
        context: &RpcApiContext,
        header: &BlockHeader,
        prefix: &ValidationPrefix,
    ) -> Result<FrameValidationOutcome, RpcErr> {
        let vm_db = StoreVmDatabase::new(context.storage.clone(), header.clone())?;
        let mut vm = context.blockchain.new_evm(vm_db)?;
        // EvmError maps to RpcErr::Vm (-32015) via From, matching eth_call/estimateGas.
        vm.simulate_frame_validation_prefix(&self.transaction, header, prefix, None)
            .map_err(RpcErr::from)
    }

    /// Executes the full transaction on a fresh throwaway state to measure
    /// total and per-frame gas. Returns `(gas_used, frames, status, error)`;
    /// on execution failure returns `(None, None, None, Some(error))` so the
    /// caller can still report the (valid) prefix outcome.
    fn execute_for_gas(
        &self,
        context: &RpcApiContext,
        header: &BlockHeader,
        sender: Address,
    ) -> (
        Option<String>,
        Option<Vec<FrameExecResult>>,
        Option<String>,
        Option<String>,
    ) {
        let vm_db = match StoreVmDatabase::new(context.storage.clone(), header.clone()) {
            Ok(db) => db,
            Err(error) => return (None, None, None, Some(error.to_string())),
        };
        let mut vm = match context.blockchain.new_evm(vm_db) {
            Ok(vm) => vm,
            Err(error) => return (None, None, None, Some(error.to_string())),
        };
        let mut cumulative_gas = 0u64;
        match vm.execute_tx(&self.transaction, header, &mut cumulative_gas, sender) {
            Ok((receipt, report)) => {
                let frames = receipt.frame_receipts.map(|frames| {
                    frames
                        .into_iter()
                        .map(|frame| FrameExecResult {
                            gas_used: format!("0x{:x}", frame.gas_used),
                            succeeded: frame.status == FRAME_RECEIPT_STATUS_SUCCESS,
                        })
                        .collect()
                });
                // For a frame tx the top-level result is Success iff every frame
                // succeeded, else a placeholder Revert (see execute_frame_tx);
                // per-frame detail is in `frames`.
                let status = if report.is_success() {
                    "success"
                } else {
                    "reverted"
                };
                (
                    Some(format!("0x{:x}", report.gas_used)),
                    frames,
                    Some(status.to_owned()),
                    None,
                )
            }
            Err(error) => (None, None, None, Some(error.to_string())),
        }
    }
}

/// Builds the `{valid: false, ...}` response for a structurally invalid prefix
/// (no EVM pass was run, so payer/prefixShape/gas are unknown; `maxCost` is a
/// pure function of the tx fields and is still reported).
fn structurally_invalid(violation: String, max_cost: String) -> Result<Value, RpcErr> {
    to_value(SimulateFrameTransactionResult {
        valid: false,
        prefix_shape: None,
        payer: None,
        max_cost,
        violation: Some(violation),
        gas_used: None,
        frames: None,
        execution_status: None,
        execution_error: None,
    })
}

fn to_hex_u256(value: U256) -> String {
    format!("0x{value:x}")
}

fn to_value(result: SimulateFrameTransactionResult) -> Result<Value, RpcErr> {
    serde_json::to_value(result).map_err(|error| RpcErr::Internal(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethrex_common::types::{EIP1559Transaction, FrameTransaction};
    use serde_json::json;

    /// Canonical (`type || payload`) hex, `0x`-prefixed, for a transaction.
    fn raw_hex(tx: &Transaction) -> String {
        let mut buf = Vec::new();
        tx.encode_canonical(&mut buf);
        format!("0x{}", hex::encode(buf))
    }

    #[test]
    fn parse_accepts_frame_tx_without_block() {
        let tx = Transaction::FrameTransaction(FrameTransaction::default());
        let params = Some(vec![json!(raw_hex(&tx))]);
        let parsed = SimulateFrameTransactionRequest::parse(&params).expect("frame tx accepted");
        assert!(matches!(
            parsed.transaction,
            Transaction::FrameTransaction(_)
        ));
        assert!(parsed.block.is_none());
    }

    #[test]
    fn parse_accepts_optional_block_tag() {
        let tx = Transaction::FrameTransaction(FrameTransaction::default());
        let params = Some(vec![json!(raw_hex(&tx)), json!("latest")]);
        let parsed = SimulateFrameTransactionRequest::parse(&params).expect("frame tx accepted");
        assert!(parsed.block.is_some());
    }

    #[test]
    fn parse_rejects_non_frame_tx() {
        let tx = Transaction::EIP1559Transaction(EIP1559Transaction::default());
        let params = Some(vec![json!(raw_hex(&tx))]);
        let err = SimulateFrameTransactionRequest::parse(&params).unwrap_err();
        assert!(matches!(err, RpcErr::BadParams(msg) if msg.contains("frame")));
    }

    #[test]
    fn parse_rejects_missing_0x_prefix() {
        let params = Some(vec![json!("abcdef")]);
        let err = SimulateFrameTransactionRequest::parse(&params).unwrap_err();
        assert!(matches!(err, RpcErr::BadParams(_)));
    }

    #[test]
    fn parse_rejects_empty_and_missing_params() {
        assert!(matches!(
            SimulateFrameTransactionRequest::parse(&Some(vec![])),
            Err(RpcErr::BadParams(_))
        ));
        assert!(matches!(
            SimulateFrameTransactionRequest::parse(&None),
            Err(RpcErr::BadParams(_))
        ));
    }

    #[test]
    fn parse_rejects_too_many_params() {
        let tx = Transaction::FrameTransaction(FrameTransaction::default());
        let params = Some(vec![json!(raw_hex(&tx)), json!("latest"), json!("extra")]);
        assert!(matches!(
            SimulateFrameTransactionRequest::parse(&params),
            Err(RpcErr::BadParams(_))
        ));
    }
}
