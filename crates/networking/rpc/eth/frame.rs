use crate::{rpc::RpcApiContext, utils::RpcErr};
use ethrex_blockchain::vm::StoreVmDatabase;
use ethrex_common::{
    Address, Bytes, U256,
    constants::TX_MAX_GAS_LIMIT_AMSTERDAM,
    serde_utils,
    types::{BlockHeader, Frame, FrameSignature, FrameTransaction, GenericTransaction, TxType},
};
use ethrex_vm::{Evm, backends::FrameSimulationResult};
use serde::{Deserialize, Deserializer};
use serde_json::Value;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FrameRequest {
    #[serde(with = "serde_utils::u8::hex_str")]
    mode: u8,
    #[serde(default, with = "serde_utils::u8::hex_str")]
    flags: u8,
    target: Option<Address>,
    #[serde(default, with = "serde_utils::u64::hex_str_opt")]
    execution_gas: Option<u64>,
    #[serde(default, with = "serde_utils::u64::hex_str_opt")]
    state_gas: Option<u64>,
    #[serde(default)]
    value: U256,
    #[serde(default, with = "serde_utils::bytes")]
    data: Bytes,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignatureRequest {
    #[serde(with = "serde_utils::u8::hex_str")]
    scheme: u8,
    #[serde(default, deserialize_with = "empty_signer")]
    signer: Option<Address>,
    #[serde(default, deserialize_with = "nullable_bytes")]
    msg: Bytes,
    #[serde(default, deserialize_with = "nullable_bytes")]
    signature: Bytes,
}

fn empty_signer<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<Address>, D::Error> {
    let value = Option::<String>::deserialize(deserializer)?;
    match value.as_deref() {
        None | Some("0x") => Ok(None),
        Some(value) => value.parse().map(Some).map_err(serde::de::Error::custom),
    }
}

fn nullable_bytes<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Bytes, D::Error> {
    let value = Option::<String>::deserialize(deserializer)?;
    let value = value.as_deref().unwrap_or("0x");
    let data = value
        .strip_prefix("0x")
        .ok_or_else(|| serde::de::Error::custom("expected hex bytes"))?;
    hex::decode(data)
        .map(Bytes::from)
        .map_err(serde::de::Error::custom)
}

#[derive(Clone, Debug, Deserialize)]
pub struct FrameTransactionRequest {
    pub frames: Vec<FrameRequest>,
    #[serde(default)]
    signatures: Vec<SignatureRequest>,
}

pub fn parse_transaction(
    value: &Value,
) -> Result<(GenericTransaction, Option<FrameTransactionRequest>), RpcErr> {
    let frame = if value.get("frames").is_some() {
        let frame: FrameTransactionRequest = serde_json::from_value(value.clone())?;
        if frame.frames.is_empty() {
            return Err(RpcErr::BadParams("frames must not be empty".into()));
        }
        Some(frame)
    } else {
        None
    };
    if frame.is_some()
        && let Some(kind) = value.get("type")
    {
        let kind: TxType = serde_json::from_value(kind.clone())?;
        if kind != TxType::Frame {
            return Err(RpcErr::BadParams(
                "frames require transaction type 0x6".into(),
            ));
        }
    }
    let mut value = value.clone();
    if frame.is_some() {
        value
            .as_object_mut()
            .ok_or_else(|| RpcErr::BadParams("expected transaction object".into()))?
            .entry("to")
            .or_insert(Value::Null);
    }
    let mut transaction: GenericTransaction = serde_json::from_value(value)?;
    if frame.is_some() {
        transaction.r#type = TxType::Frame;
    } else if transaction.r#type == TxType::Frame {
        return Err(RpcErr::BadParams(
            "frame transaction requires frames".into(),
        ));
    }
    Ok((transaction, frame))
}

impl FrameTransactionRequest {
    pub async fn prepare(
        &self,
        request: &GenericTransaction,
        header: &BlockHeader,
        context: &RpcApiContext,
    ) -> Result<(FrameTransaction, FrameSimulationResult), RpcErr> {
        let nonce = match request.nonce {
            Some(nonce) => nonce,
            None => context
                .storage
                .get_nonce_by_account_address(header.number, request.from)
                .await?
                .unwrap_or_default(),
        };
        let mut tx = FrameTransaction {
            inner_hash: Default::default(),
            cached_canonical: Default::default(),
            sender: request.from,
            chain_id: request
                .chain_id
                .unwrap_or(context.storage.get_chain_config().chain_id),
            nonce,
            max_fee_per_gas: request
                .max_fee_per_gas
                .map(U256::from)
                .unwrap_or(request.gas_price),
            max_priority_fee_per_gas: request
                .max_priority_fee_per_gas
                .map(U256::from)
                .unwrap_or_default(),
            max_fee_per_blob_gas: request.max_fee_per_blob_gas.unwrap_or_default(),
            blob_versioned_hashes: request.blob_versioned_hashes.clone(),
            frames: self
                .frames
                .iter()
                .map(|frame| Frame {
                    mode: frame.mode,
                    flags: frame.flags,
                    target: frame.target,
                    gas_limit: frame.execution_gas.unwrap_or_default(),
                    state_gas_limit: frame.state_gas.unwrap_or_default(),
                    value: frame.value,
                    data: frame.data.clone(),
                })
                .collect(),
            signatures: self
                .signatures
                .iter()
                .map(|signature| FrameSignature {
                    scheme: signature.scheme,
                    signer: signature.signer,
                    msg: signature.msg.clone(),
                    signature: signature.signature.clone(),
                })
                .collect(),
        };
        tx.validate_static_constraints()
            .map_err(RpcErr::BadParams)?;
        let execution_cap = TX_MAX_GAS_LIMIT_AMSTERDAM.min(header.gas_limit);
        let sized = with_signature_sizes(&tx);
        let intrinsic = sized.mandatory_gas().saturating_add(sized.data_cost());
        let explicit = tx.total_frame_execution_gas();
        let remaining = execution_cap
            .checked_sub(intrinsic.saturating_add(explicit))
            .ok_or_else(|| {
                RpcErr::BadParams("frame execution limits exceed the transaction gas cap".into())
            })?;
        let missing = self
            .frames
            .iter()
            .filter(|frame| frame.execution_gas.is_none())
            .count() as u64;
        let state_budget = header.gas_limit / self.frames.len() as u64;
        for (frame, request) in tx.frames.iter_mut().zip(&self.frames) {
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
        let omitted = self
            .frames
            .iter()
            .any(|frame| frame.execution_gas.is_none() || frame.state_gas.is_none());
        if !omitted || !initial.report.is_success() {
            return Ok((tx, initial));
        }
        for (index, request) in self.frames.iter().enumerate() {
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
        let (_, frame) = parse_transaction(&request()).unwrap();
        let frame = frame.unwrap();
        assert_eq!(frame.frames[0].execution_gas, None);
        assert_eq!(frame.frames[0].state_gas, None);
        assert_eq!(frame.frames[0].target, None);
        assert_eq!(frame.frames[1].flags, 0);
        assert!(frame.frames[1].data.is_empty());
        let mut value = request();
        value["frames"][0]["executionGas"] = json!("0x0");
        assert_eq!(
            parse_transaction(&value).unwrap().1.unwrap().frames[0].execution_gas,
            Some(0)
        );
        for value in [json!(null), json!("0x")] {
            let entry: SignatureRequest = serde_json::from_value(
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
        let (generic, frame) = parse_transaction(&value).unwrap();
        let (tx, result) = frame
            .unwrap()
            .prepare(&generic, &header, &context)
            .await
            .unwrap();
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
}
