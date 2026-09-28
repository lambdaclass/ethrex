use crate::{rpc::RpcApiContext, utils::RpcErr};
use ethrex_blockchain::vm::StoreVmDatabase;
use ethrex_common::{
    Bytes,
    constants::TX_MAX_GAS_LIMIT_AMSTERDAM,
    types::{BlockHeader, FrameTransaction, GenericTransaction, TxType},
};
use ethrex_vm::{Evm, backends::FrameSimulationResult};
use serde_json::Value;

pub fn parse_transaction(value: &Value) -> Result<GenericTransaction, RpcErr> {
    let mut transaction: GenericTransaction = serde_json::from_value(value.clone())?;
    if let Some(frames) = &transaction.frames {
        if frames.is_empty() {
            return Err(RpcErr::BadParams("frames must not be empty".into()));
        }
        if value.get("type").is_some() && transaction.r#type != TxType::Frame {
            return Err(RpcErr::BadParams(
                "frames require transaction type 0x6".into(),
            ));
        }
        transaction.r#type = TxType::Frame;
    } else if transaction.r#type == TxType::Frame {
        return Err(RpcErr::BadParams(
            "frame transaction requires frames".into(),
        ));
    }
    Ok(transaction)
}

pub async fn prepare(
    request: &GenericTransaction,
    header: &BlockHeader,
    context: &RpcApiContext,
) -> Result<(FrameTransaction, FrameSimulationResult), RpcErr> {
    let frames = request
        .frames
        .as_ref()
        .ok_or_else(|| RpcErr::BadParams("frame transaction requires frames".into()))?;
    let nonce = match request.nonce {
        Some(nonce) => nonce,
        None => context
            .storage
            .get_nonce_by_account_address(header.number, request.from)
            .await?
            .unwrap_or_default(),
    };
    let mut request = request.clone();
    request.nonce = Some(nonce);
    request.chain_id = Some(
        request
            .chain_id
            .unwrap_or(context.storage.get_chain_config().chain_id),
    );
    let mut tx = FrameTransaction::try_from(&request)
        .map_err(|error| RpcErr::BadParams(error.to_string()))?;
    let execution_cap = TX_MAX_GAS_LIMIT_AMSTERDAM.min(header.gas_limit);
    let sized = with_signature_sizes(&tx);
    let intrinsic = sized.mandatory_gas().saturating_add(sized.data_cost());
    let explicit = tx.total_frame_execution_gas();
    let remaining = execution_cap
        .checked_sub(intrinsic.saturating_add(explicit))
        .ok_or_else(|| {
            RpcErr::BadParams("frame execution limits exceed the transaction gas cap".into())
        })?;
    let missing = frames
        .iter()
        .filter(|frame| frame.execution_gas.is_none())
        .count() as u64;
    let state_budget = header.gas_limit / frames.len() as u64;
    for (frame, request) in tx.frames.iter_mut().zip(frames) {
        if request.execution_gas.is_none() {
            frame.gas_limit = remaining / missing;
        }
        if request.state_gas.is_none() && !frame.is_expiry_verifier() {
            frame.state_gas_limit = state_budget;
        }
    }
    let vm_db = StoreVmDatabase::new(context.storage.clone(), header.clone())?;
    let vm = context.blockchain.new_evm(vm_db)?;
    let initial = simulate(&vm, &tx, header)?;
    let omitted = frames
        .iter()
        .any(|frame| frame.execution_gas.is_none() || frame.state_gas.is_none());
    if !omitted || !initial.report.is_success() {
        return Ok((tx, initial));
    }
    for (index, request) in frames.iter().enumerate() {
        for state in [false, true] {
            if if state {
                request.state_gas.is_some()
            } else {
                request.execution_gas.is_some()
            } {
                continue;
            }
            let mut high = if state {
                tx.frames[index].state_gas_limit
            } else {
                tx.frames[index].gas_limit
            };
            let mut low = 0;
            while low < high {
                let middle = low + (high - low) / 2;
                if state {
                    tx.frames[index].state_gas_limit = middle;
                } else {
                    tx.frames[index].gas_limit = middle;
                }
                if simulate(&vm, &tx, header).is_ok_and(|result| result.report.is_success()) {
                    high = middle;
                } else {
                    low = middle + 1;
                }
            }
            if state {
                tx.frames[index].state_gas_limit = high;
            } else {
                tx.frames[index].gas_limit = high;
            }
        }
    }
    let result = simulate(&vm, &tx, header)?;
    Ok((tx, result))
}

// Reserve the worst-case calldata cost of fixed-size signatures that have not been signed yet.
fn with_signature_sizes(tx: &FrameTransaction) -> FrameTransaction {
    let mut tx = tx.clone();
    for signature in &mut tx.signatures {
        if signature.signature.is_empty() {
            let size = match signature.scheme {
                1 => 65,
                2 => 128,
                _ => 0,
            };
            signature.signature = Bytes::from(vec![1; size]);
        }
    }
    tx
}

pub fn estimated_max_gas(tx: &FrameTransaction) -> u64 {
    with_signature_sizes(tx).max_gas()
}

fn simulate(
    vm: &Evm,
    tx: &FrameTransaction,
    header: &BlockHeader,
) -> Result<FrameSimulationResult, RpcErr> {
    vm.clone().simulate_frame_tx(tx, header).map_err(Into::into)
}

pub fn require_success(result: &FrameSimulationResult) -> Result<(), RpcErr> {
    if !result.report.is_success() {
        return Err(RpcErr::Revert {
            data: format!("0x{}", hex::encode(&result.report.output)),
        });
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::{
        rpc::map_http_requests, test_utils::default_context_with_storage, utils::RpcRequest,
    };
    use ethrex_common::types::Genesis;
    use ethrex_storage::{EngineType, Store};
    use serde_json::json;

    const SENDER: &str = "0x00000a8d3f37af8def18832962ee008d8dca4f7b";
    const TARGET: &str = "0xc100000000000000000000000000000000000000";

    pub async fn context() -> RpcApiContext {
        let mut genesis: Value =
            serde_json::from_str(include_str!("../../../../fixtures/genesis/l1.json")).unwrap();
        genesis["config"]["amsterdamTime"] = json!(0);
        genesis["config"]["hegotaTime"] = json!(0);
        genesis["alloc"][SENDER]["nonce"] = json!("0x5");
        genesis["alloc"][TARGET] = json!({"balance":"0x1", "nonce":"0x0", "storage":{}, "code":"0x5f5450602a5f5260205ff3"});
        genesis["alloc"]["0xc200000000000000000000000000000000000000"] =
            json!({"balance":"0x1", "nonce":"0x0", "storage":{}, "code":"0x602b5f5260205ffd"});
        let genesis: Genesis = serde_json::from_value(genesis).unwrap();
        let mut storage = Store::new("test-store", EngineType::InMemory).unwrap();
        storage.add_initial_state(genesis).await.unwrap();
        default_context_with_storage(storage).await
    }

    fn request() -> Value {
        json!({"type":"0x6", "from":SENDER,
            "frames":[{"mode":"0x1", "flags":"0x3"}, {"mode":"0x2", "target":TARGET}],
            "signatures":[{"scheme":"0x1", "signer":null, "msg":null, "signature":null}]})
    }

    async fn rpc(
        method: &str,
        transaction: Value,
        context: RpcApiContext,
    ) -> Result<Value, RpcErr> {
        let request: RpcRequest = serde_json::from_value(
            json!({"jsonrpc":"2.0","id":1,"method":method,"params":[transaction,"latest"]}),
        )
        .unwrap();
        map_http_requests(&request, context).await
    }

    #[test]
    fn parse_frame_defaults_and_explicit_zero() {
        let transaction = parse_transaction(&request()).unwrap();
        let frames = transaction.frames.unwrap();
        assert_eq!(frames[0].execution_gas, None);
        assert_eq!(frames[0].state_gas, None);
        assert_eq!(frames[0].target, None);
        assert_eq!(frames[1].flags, 0);
        assert!(frames[1].data.is_empty());
        let mut value = request();
        value["frames"][0]["executionGas"] = json!("0x0");
        assert_eq!(
            parse_transaction(&value).unwrap().frames.unwrap()[0].execution_gas,
            Some(0)
        );
        for value in [json!(null), json!("0x")] {
            let entry: ethrex_common::types::FrameSignatureRequest = serde_json::from_value(
                json!({"scheme":"0x1","signer":value,"msg":value,"signature":value}),
            )
            .unwrap();
            assert_eq!(entry.signer, None);
            assert!(entry.msg.is_empty());
            assert!(entry.signature.is_empty());
        }
    }

    #[tokio::test]
    async fn frame_call_returns_last_output_without_changing_state() {
        let context = context().await;
        let result = rpc("eth_call", request(), context.clone()).await.unwrap();
        assert_eq!(result, json!(format!("0x{:064x}", 42)));
        assert_eq!(
            context
                .storage
                .get_nonce_by_account_address(0, SENDER.parse().unwrap())
                .await
                .unwrap(),
            Some(5)
        );
    }

    #[tokio::test]
    async fn frame_estimation_and_access_list_execute_frames() {
        let context = context().await;
        let estimate = rpc("eth_estimateGas", request(), context.clone())
            .await
            .unwrap();
        // 12,000 intrinsic + 950 frame overhead + 2,800 signature + 3,100 account access + 2,100 storage access + 20 execution + 1,040 signature bytes.
        assert_eq!(estimate, json!("0x55fa"));
        let list = rpc("eth_createAccessList", request(), context)
            .await
            .unwrap();
        assert!(
            list["accessList"]
                .as_array()
                .unwrap()
                .iter()
                .any(|entry| entry["address"] == TARGET)
        );
    }

    #[tokio::test]
    async fn frame_call_preserves_zero_and_rejects_invalid_signatures() {
        let context = context().await;
        let mut value = request();
        value["frames"][0]["executionGas"] = json!("0x0");
        assert!(rpc("eth_call", value, context.clone()).await.is_err());
        let mut value = request();
        value["signatures"][0]["signature"] = json!(format!("0x{}", "00".repeat(65)));
        assert!(rpc("eth_call", value, context).await.is_err());
    }
    #[tokio::test]
    async fn frame_call_returns_first_revert_data() {
        let mut value = request();
        value["frames"].as_array_mut().unwrap().insert(
            1,
            json!({"mode":"0x2", "target":"0xc200000000000000000000000000000000000000"}),
        );
        let error = rpc("eth_call", value, context().await).await.unwrap_err();
        assert!(matches!(error, RpcErr::Revert { data } if data == format!("0x{:064x}", 43)));
    }

    #[tokio::test]
    async fn placeholders_remain_invalid_for_consensus_execution() {
        let context = context().await;
        let header = context.storage.get_block_header(0).unwrap().unwrap();
        let mut value = request();
        value["maxFeePerGas"] = json!(format!("{:#x}", header.base_fee_per_gas.unwrap()));
        let generic = parse_transaction(&value).unwrap();
        let (tx, result) = prepare(&generic, &header, &context).await.unwrap();
        assert!(result.report.is_success());
        let vm_db = StoreVmDatabase::new(context.storage.clone(), header.clone()).unwrap();
        let mut vm = context.blockchain.new_evm(vm_db).unwrap();
        let error = vm
            .execute_tx(
                &ethrex_common::types::Transaction::FrameTransaction(tx),
                &header,
                &mut 0,
                generic.from,
            )
            .unwrap_err();
        assert!(error.to_string().contains("signature"), "{error}");
    }
    #[tokio::test]
    async fn existing_generic_simulation_api_executes_frames() {
        let context = context().await;
        let header = context.storage.get_block_header(0).unwrap().unwrap();
        let request = parse_transaction(&request()).unwrap();
        let (tx, _) = prepare(&request, &header, &context).await.unwrap();
        let generic = GenericTransaction::from(tx);
        let vm_db = StoreVmDatabase::new(context.storage.clone(), header.clone()).unwrap();
        let mut vm = context.blockchain.new_evm(vm_db).unwrap();
        let result = vm.simulate_tx_from_generic(&generic, &header).unwrap();
        assert_eq!(hex::encode(result.output()), format!("{:064x}", 42));
    }
}
