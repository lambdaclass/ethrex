use super::receipt::{RpcFrameReceipt, RpcLog, RpcReceiptBlockInfo, RpcReceiptTxInfo};
use crate::utils::RpcErr;
use ethrex_common::{Address, Bytes, serde_utils};
use ethrex_vm::{VMError, backends::FrameSimulationResult};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct SimulationError {
    pub code: i32,
    pub message: String,
}

impl From<Option<VMError>> for SimulationError {
    fn from(error: Option<VMError>) -> Self {
        match error {
            None | Some(VMError::RevertOpcode) => Self {
                code: 3,
                message: "execution reverted".into(),
            },
            Some(error) => Self {
                code: -32015,
                message: format!("vm execution error: {error}"),
            },
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameResult {
    #[serde(flatten)]
    pub receipt: RpcFrameReceipt,
    #[serde(with = "serde_utils::bytes")]
    pub return_data: Bytes,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<SimulationError>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameCallResult {
    #[serde(with = "serde_utils::u8::hex_str")]
    pub status: u8,
    #[serde(with = "serde_utils::u64::hex_str")]
    pub gas_used: u64,
    #[serde(with = "serde_utils::u64::hex_str")]
    pub max_used_gas: u64,
    #[serde(with = "serde_utils::bytes")]
    pub return_data: Bytes,
    pub logs: Vec<RpcLog>,
    pub payer: Option<Address>,
    pub frame_results: Vec<FrameResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<SimulationError>,
}

impl FrameCallResult {
    /// The simulation block builder supplies the final block, transaction, and log positions.
    pub fn new(
        simulation: FrameSimulationResult,
        transaction: &RpcReceiptTxInfo,
        block: &RpcReceiptBlockInfo,
        mut log_index: u64,
    ) -> Result<Self, RpcErr> {
        let report = simulation.report;
        let frames = report
            .frame_results
            .ok_or_else(|| RpcErr::Internal("missing frame results".into()))?;
        if frames.is_empty() || frames.len() != simulation.outputs.len() {
            return Err(RpcErr::Internal(
                "frame results and outputs must have matching lengths".into(),
            ));
        }
        let mut frame_results = Vec::with_capacity(frames.len());
        let mut all_logs = Vec::new();
        for ((status, execution_gas_used, state_gas_used, logs), (return_data, error)) in
            frames.into_iter().zip(simulation.outputs)
        {
            let logs: Vec<_> = logs
                .into_iter()
                .map(|log| {
                    let log = RpcLog::new(log, log_index, transaction, block);
                    log_index += 1;
                    log
                })
                .collect();
            all_logs.extend(logs.iter().cloned());
            frame_results.push(FrameResult {
                receipt: RpcFrameReceipt {
                    status,
                    gas_used: execution_gas_used + state_gas_used,
                    execution_gas_used,
                    state_gas_used,
                    logs,
                },
                return_data,
                error: (status == 0).then(|| error.into()),
            });
        }
        let succeeded = frame_results.iter().all(|frame| frame.receipt.status == 1);
        let failed = frame_results.iter().find(|frame| frame.receipt.status == 0);
        let output = if succeeded {
            frame_results.last()
        } else {
            failed
        };
        let return_data = output
            .map(|frame| frame.return_data.clone())
            .unwrap_or_default();
        let error = if succeeded {
            None
        } else {
            Some(
                failed
                    .and_then(|frame| frame.error.clone())
                    .unwrap_or_else(|| None.into()),
            )
        };
        Ok(Self {
            status: u8::from(succeeded),
            gas_used: report.gas_spent,
            max_used_gas: report.gas_used,
            return_data,
            logs: all_logs,
            payer: report.payer_address,
            frame_results,
            error,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethrex_common::{H256, types::Log};
    use ethrex_vm::backends::errors::{ExceptionalHalt, ExecutionReport, TxResult};
    use serde_json::json;

    fn result(statuses: &[u8]) -> FrameCallResult {
        let log = Log {
            address: Address::repeat_byte(1),
            topics: vec![],
            data: Bytes::new(),
        };
        let simulation = FrameSimulationResult {
            report: ExecutionReport {
                result: TxResult::Success,
                gas_used: 20_000,
                gas_spent: 19_000,
                gas_refunded: 1000,
                state_gas_used: 512,
                output: Bytes::new(),
                logs: vec![],
                payer_address: Some(Address::repeat_byte(2)),
                frame_results: Some(
                    statuses
                        .iter()
                        .map(|status| {
                            (
                                *status,
                                if *status == 2 { 0 } else { 256 },
                                if *status == 1 { 512 } else { 0 },
                                if *status == 1 {
                                    vec![log.clone()]
                                } else {
                                    vec![]
                                },
                            )
                        })
                        .collect(),
                ),
            },
            outputs: statuses
                .iter()
                .enumerate()
                .map(|(index, status)| {
                    (
                        if *status == 2 {
                            Bytes::new()
                        } else {
                            Bytes::from(vec![index as u8])
                        },
                        (*status == 0).then_some(VMError::RevertOpcode),
                    )
                })
                .collect(),
            access_list: vec![],
        };
        let transaction = RpcReceiptTxInfo {
            transaction_hash: H256::repeat_byte(3),
            transaction_index: 2,
            from: Address::repeat_byte(1),
            to: None,
            contract_address: None,
            gas_used: 19_000,
            effective_gas_price: 0,
            blob_gas_price: None,
            blob_gas_used: None,
        };
        let block = RpcReceiptBlockInfo {
            block_hash: H256::repeat_byte(4),
            block_number: 5,
        };
        FrameCallResult::new(simulation, &transaction, &block, 7).unwrap()
    }

    #[test]
    fn frame_results_preserve_order_outputs_gas_and_surviving_logs() {
        let value = serde_json::to_value(result(&[1, 0, 2, 1])).unwrap();
        assert_eq!(value["status"], "0x0");
        assert_eq!(value["returnData"], "0x01");
        assert_eq!(value["payer"], "0x0202020202020202020202020202020202020202");
        assert_eq!(value["frameResults"][0]["gasUsed"], "0x300");
        assert_eq!(value["frameResults"][0]["executionGasUsed"], "0x100");
        assert_eq!(value["frameResults"][0]["stateGasUsed"], "0x200");
        assert_eq!(value["frameResults"][1]["error"]["code"], 3);
        assert_eq!(
            value["frameResults"][2],
            json!({"status":"0x2", "gasUsed":"0x0", "executionGasUsed":"0x0", "stateGasUsed":"0x0", "returnData":"0x", "logs":[]})
        );
        assert_eq!(value["logs"].as_array().unwrap().len(), 2);
        assert_eq!(value["frameResults"][0]["logs"][0], value["logs"][0]);
        assert_eq!(value["frameResults"][3]["logs"][0], value["logs"][1]);
        assert_eq!(value["logs"][1]["logIndex"], "0x8");
        assert_eq!(value["logs"][1]["transactionIndex"], "0x2");
    }

    #[test]
    fn successful_call_returns_last_frame_output() {
        let value = serde_json::to_value(result(&[1, 1])).unwrap();
        assert_eq!(value["status"], "0x1");
        assert_eq!(value["returnData"], "0x01");
        assert!(value.get("error").is_none());
    }

    #[test]
    fn exceptional_halts_use_the_simulation_error_code() {
        let error = SimulationError::from(Some(ExceptionalHalt::OutOfGas.into()));
        assert_eq!(error.code, -32015);
        assert!(error.message.starts_with("vm execution error"));
    }
    #[tokio::test]
    async fn frame_entry_out_of_gas_retains_exceptional_halt() {
        let context = crate::eth::frame::tests::context().await;
        let header = context.storage.get_block_header(0).unwrap().unwrap();
        let request = crate::eth::frame::parse_transaction(&json!({
            "type":"0x6", "from":"0x00000a8d3f37af8def18832962ee008d8dca4f7b",
            "frames":[
                {"mode":"0x1", "flags":"0x3", "executionGas":"0x64", "stateGas":"0x0"},
                {"mode":"0x2", "target":"0xc100000000000000000000000000000000000000", "executionGas":"0x0", "stateGas":"0x0"}
            ],
            "signatures":[{"scheme":"0x1"}]
        })).unwrap();
        let (_, simulation) = crate::eth::frame::prepare(&request, &header, &context)
            .await
            .unwrap();
        assert!(matches!(
            simulation.outputs[1].1,
            Some(VMError::ExceptionalHalt(ExceptionalHalt::OutOfGas))
        ));
    }
}
