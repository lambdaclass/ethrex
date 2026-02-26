//! `debug_timeTravel` RPC handler.
//!
//! Replays a transaction at opcode granularity and returns a window of
//! execution steps, enabling time-travel debugging over JSON-RPC.

use std::time::Duration;

use ethrex_common::{Address, H256};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokamak_debugger::{
    engine::ReplayEngine,
    types::{ReplayConfig, StepRecord},
};

use crate::{
    rpc::{RpcApiContext, RpcHandler},
    utils::RpcErr,
};

const DEFAULT_REEXEC: u32 = 128;
const DEFAULT_COUNT: usize = 20;
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

pub struct DebugTimeTravelRequest {
    tx_hash: H256,
    options: TimeTravelOptions,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct TimeTravelOptions {
    #[serde(default)]
    step_index: Option<usize>,
    #[serde(default)]
    count: Option<usize>,
    #[serde(default)]
    reexec: Option<u32>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TimeTravelResponse {
    trace: TraceSummary,
    current_step_index: usize,
    steps: Vec<StepView>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TraceSummary {
    total_steps: usize,
    gas_used: u64,
    success: bool,
    output: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StepView {
    step_index: usize,
    pc: usize,
    opcode: u8,
    opcode_name: String,
    depth: usize,
    gas_remaining: i64,
    stack_top: Vec<String>,
    stack_depth: usize,
    memory_size: usize,
    code_address: Address,
}

fn step_to_view(step: &StepRecord) -> StepView {
    let opcode_name = step.opcode_name();
    let stack_top = step.stack_top.iter().map(|v| format!("{v:#x}")).collect();
    StepView {
        step_index: step.step_index,
        pc: step.pc,
        opcode: step.opcode,
        opcode_name,
        depth: step.depth,
        gas_remaining: step.gas_remaining,
        stack_top,
        stack_depth: step.stack_depth,
        memory_size: step.memory_size,
        code_address: step.code_address,
    }
}

impl RpcHandler for DebugTimeTravelRequest {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.is_empty() || params.len() > 2 {
            return Err(RpcErr::BadParams("Expected 1 or 2 params".to_owned()));
        }
        let tx_hash: H256 = serde_json::from_value(params[0].clone())?;
        let options = if params.len() == 2 {
            serde_json::from_value(params[1].clone())?
        } else {
            TimeTravelOptions::default()
        };
        Ok(DebugTimeTravelRequest { tx_hash, options })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let reexec = self.options.reexec.unwrap_or(DEFAULT_REEXEC);
        let step_index = self.options.step_index.unwrap_or(0);
        let count = self.options.count.unwrap_or(DEFAULT_COUNT);

        // 1. Prepare EVM state up to the target transaction
        let (vm, block, tx_index) = context
            .blockchain
            .prepare_state_for_tx(self.tx_hash, reexec)
            .await
            .map_err(|err| RpcErr::Internal(err.to_string()))?;

        // 2. Build execution environment for the target TX
        let tx = block
            .body
            .transactions
            .get(tx_index)
            .ok_or(RpcErr::Internal(
                "Transaction index out of range".to_owned(),
            ))?
            .clone();
        let block_header = block.header.clone();
        let env = vm
            .setup_env_for_tx(&tx, &block_header)
            .map_err(|err| RpcErr::Internal(err.to_string()))?;
        let mut db = vm.db;

        // 3. Record trace in a blocking task (CPU-intensive)
        let config = ReplayConfig::default();
        let engine = tokio::time::timeout(
            DEFAULT_TIMEOUT,
            tokio::task::spawn_blocking(move || ReplayEngine::record(&mut db, env, &tx, config)),
        )
        .await
        .map_err(|_| RpcErr::Internal("Time travel timeout".to_owned()))?
        .map_err(|_| RpcErr::Internal("Unexpected runtime error".to_owned()))?
        .map_err(|err| RpcErr::Internal(err.to_string()))?;

        // 4. Extract the requested window of steps
        let trace = engine.trace();
        let steps: Vec<StepView> = engine
            .steps_range(step_index, count)
            .iter()
            .map(step_to_view)
            .collect();

        let response = TimeTravelResponse {
            trace: TraceSummary {
                total_steps: trace.steps.len(),
                gas_used: trace.gas_used,
                success: trace.success,
                output: format!("0x{}", hex::encode(&trace.output)),
            },
            current_step_index: step_index,
            steps,
        };

        Ok(serde_json::to_value(response)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_tx_hash_only() {
        let params = Some(vec![serde_json::json!(
            "0x0000000000000000000000000000000000000000000000000000000000000001"
        )]);
        let req = DebugTimeTravelRequest::parse(&params).expect("should parse");
        assert_eq!(req.options.step_index, None);
        assert_eq!(req.options.count, None);
        assert_eq!(req.options.reexec, None);
    }

    #[test]
    fn parse_with_options() {
        let params = Some(vec![
            serde_json::json!("0x0000000000000000000000000000000000000000000000000000000000000001"),
            serde_json::json!({"stepIndex": 5, "count": 10, "reexec": 64}),
        ]);
        let req = DebugTimeTravelRequest::parse(&params).expect("should parse");
        assert_eq!(req.options.step_index, Some(5));
        assert_eq!(req.options.count, Some(10));
        assert_eq!(req.options.reexec, Some(64));
    }

    #[test]
    fn parse_empty_params() {
        let params = Some(vec![]);
        let result = DebugTimeTravelRequest::parse(&params);
        assert!(result.is_err());
    }

    #[test]
    fn parse_invalid_hash() {
        let params = Some(vec![serde_json::json!("not-a-hash")]);
        let result = DebugTimeTravelRequest::parse(&params);
        assert!(result.is_err());
    }

    #[test]
    fn step_view_serialization() {
        let view = StepView {
            step_index: 0,
            pc: 10,
            opcode: 0x01,
            opcode_name: "ADD".to_string(),
            depth: 0,
            gas_remaining: 99994,
            stack_top: vec!["0x7".to_string(), "0x3".to_string()],
            stack_depth: 3,
            memory_size: 0,
            code_address: Address::zero(),
        };
        let json = serde_json::to_value(&view).expect("should serialize");
        assert_eq!(json["stepIndex"], 0);
        assert_eq!(json["pc"], 10);
        assert_eq!(json["opcode"], 1);
        assert_eq!(json["opcodeName"], "ADD");
        assert_eq!(json["gasRemaining"], 99994);
        assert_eq!(json["stackTop"][0], "0x7");
        assert_eq!(json["stackDepth"], 3);
        assert_eq!(json["memorySize"], 0);
    }

    #[test]
    fn trace_summary_serialization() {
        let summary = TraceSummary {
            total_steps: 1337,
            gas_used: 21009,
            success: true,
            output: "0x".to_string(),
        };
        let json = serde_json::to_value(&summary).expect("should serialize");
        assert_eq!(json["totalSteps"], 1337);
        assert_eq!(json["gasUsed"], 21009);
        assert_eq!(json["success"], true);
        assert_eq!(json["output"], "0x");
    }
}
