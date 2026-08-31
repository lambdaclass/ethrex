use std::{fs::File, io::BufReader, path::PathBuf};

use bytes::Bytes;
use ethrex_blockchain::{
    Blockchain,
    fork_choice::apply_fork_choice,
    payload::{BuildPayloadArgs, create_payload},
};
use ethrex_common::{
    Address, H160, H256,
    types::{Block, BlockHeader, DEFAULT_BUILDER_GAS_CEIL, ELASTICITY_MULTIPLIER, Genesis},
};
use ethrex_rpc::engine::fork_choice::{ForkChoiceUpdatedV3, ForkChoiceUpdatedV4};
use ethrex_rpc::engine::payload::GetPayloadV5Request;
use ethrex_rpc::rpc::RpcApiContext;
use ethrex_rpc::rpc::RpcHandler;
use ethrex_rpc::test_utils::default_context_with_storage;
use ethrex_rpc::types::fork_choice::{PayloadAttributesV4, PayloadAttributesV5};
use ethrex_rpc::types::payload::ExecutionPayloadResponse;
use ethrex_rpc::utils::{RpcErr, RpcErrorMetadata, RpcRequest};
use ethrex_storage::{EngineType, Store};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

async fn test_store() -> Store {
    let file = File::open(workspace_root().join("fixtures/genesis/execution-api.json"))
        .expect("Failed to open genesis file");
    let reader = BufReader::new(file);
    let genesis = serde_json::from_reader(reader).expect("Failed to deserialize genesis file");
    let mut store =
        Store::new("store.db", EngineType::InMemory).expect("Failed to build DB for testing");
    store
        .add_initial_state(genesis)
        .await
        .expect("Failed to add genesis state");
    store
}

async fn new_block(store: &Store, parent: &BlockHeader) -> Block {
    let args = BuildPayloadArgs {
        parent: parent.hash(),
        timestamp: parent.timestamp + 12,
        fee_recipient: H160::random(),
        random: H256::random(),
        withdrawals: Some(Vec::new()),
        beacon_root: Some(H256::random()),
        slot_number: None,
        version: 1,
        elasticity_multiplier: ELASTICITY_MULTIPLIER,
        gas_ceil: DEFAULT_BUILDER_GAS_CEIL,
        inclusion_list_transactions: None,
    };
    let blockchain = Blockchain::default_with_store(store.clone());
    let block = create_payload(&args, store, Bytes::new()).unwrap();
    blockchain.build_payload(block).unwrap().payload
}

async fn context_with_built_payload_at(timestamp: u64, payload_id: u64) -> RpcApiContext {
    let mut storage = test_store().await;
    let mut chain_config = storage.get_chain_config();
    chain_config.osaka_time = Some(0);
    chain_config.amsterdam_time = Some(10);
    storage.set_chain_config(&chain_config).await.unwrap();

    let genesis_header = storage.get_block_header(0).unwrap().unwrap();
    let args = BuildPayloadArgs {
        parent: genesis_header.hash(),
        timestamp,
        fee_recipient: H160::random(),
        random: H256::random(),
        withdrawals: Some(Vec::new()),
        beacon_root: Some(H256::random()),
        slot_number: None,
        version: 1,
        elasticity_multiplier: ELASTICITY_MULTIPLIER,
        gas_ceil: DEFAULT_BUILDER_GAS_CEIL,
        inclusion_list_transactions: None,
    };
    let payload = create_payload(&args, &storage, Bytes::new()).unwrap();
    let context = default_context_with_storage(storage).await;
    context
        .blockchain
        .clone()
        .initiate_payload_build(payload, payload_id, Vec::new())
        .await;

    context
}

#[tokio::test]
async fn get_payload_v5_accepts_osaka_payload_before_amsterdam() {
    let payload_id = 1;
    let context = context_with_built_payload_at(9, payload_id).await;

    let response = GetPayloadV5Request { payload_id }
        .handle(context)
        .await
        .unwrap();
    let response: ExecutionPayloadResponse = serde_json::from_value(response).unwrap();

    assert_eq!(response.execution_payload.timestamp, 9);
}

#[tokio::test]
async fn get_payload_v5_rejects_amsterdam_payload() {
    let payload_id = 1;
    let context = context_with_built_payload_at(10, payload_id).await;

    let err = GetPayloadV5Request { payload_id }
        .handle(context)
        .await
        .unwrap_err();

    assert!(matches!(err, RpcErr::UnsupportedFork(_)));
}

// Regression test for execution-apis PR #786: when engine_forkchoiceUpdatedV3
// receives a head that is a VALID canonical ancestor of the latest known
// finalized block, the response MUST be {payloadStatus: VALID, payloadId: null}
// and the client MUST NOT begin a payload build process — even when
// payloadAttributes is non-null.
#[tokio::test]
async fn test_fcu_v3_finalized_ancestor_returns_valid_with_null_payload_id() {
    let store = test_store().await;
    let genesis_header = store.get_block_header(0).unwrap().unwrap();
    let blockchain = Blockchain::default_with_store(store.clone());

    let block_1 = new_block(&store, &genesis_header).await;
    let hash_1 = block_1.hash();
    blockchain.add_block(block_1.clone()).unwrap();

    let block_2 = new_block(&store, &block_1.header).await;
    let hash_2 = block_2.hash();
    blockchain.add_block(block_2.clone()).unwrap();

    // head = block_2 (latest tip), safe = finalized = block_1.
    // After this, block_1 is canonical, finalized number == 1, latest == 2.
    apply_fork_choice(&store, hash_2, hash_1, hash_1, None)
        .await
        .expect("apply_fork_choice failed");

    // Now drive engine_forkchoiceUpdatedV3 with head = block_1 (finalized ancestor)
    // and non-null payloadAttributes. The guard in apply_fork_choice should
    // return InvalidForkChoice::NewHeadAlreadyCanonical, which the RPC layer
    // must translate into VALID + null payloadId without calling build_payload.
    let attrs_timestamp = block_1.header.timestamp + 12;
    let body = format!(
        r#"{{
            "jsonrpc": "2.0",
            "method": "engine_forkchoiceUpdatedV3",
            "params": [
                {{
                    "headBlockHash": "{hash_1:#x}",
                    "safeBlockHash": "{hash_1:#x}",
                    "finalizedBlockHash": "{hash_1:#x}"
                }},
                {{
                    "timestamp": "{attrs_timestamp:#x}",
                    "prevRandao": "0x0000000000000000000000000000000000000000000000000000000000000001",
                    "suggestedFeeRecipient": "0x0000000000000000000000000000000000000000",
                    "withdrawals": [],
                    "parentBeaconBlockRoot": "0x0000000000000000000000000000000000000000000000000000000000000002"
                }}
            ],
            "id": 1
        }}"#
    );
    let request: RpcRequest = serde_json::from_str(&body).expect("valid FCU request");

    let context = default_context_with_storage(store).await;
    let response = ForkChoiceUpdatedV3::call(&request, context)
        .await
        .expect("FCU V3 call should succeed");

    assert_eq!(
        response["payloadStatus"]["status"], "VALID",
        "payloadStatus.status must be VALID per execution-apis PR #786"
    );
    assert_eq!(
        response["payloadStatus"]["latestValidHash"],
        format!("{hash_1:#x}"),
        "latestValidHash must echo the head hash"
    );
    assert!(
        response["payloadId"].is_null(),
        "payloadId must be null when head is a finalized ancestor; got {:?}",
        response["payloadId"]
    );
}

// execution-apis#796: PayloadAttributesV4 carries a required CL-supplied
// targetGasLimit. An absent or null value fails deserialization, so the FCUv4
// request is rejected (see parse_v4); the e2e tests below exercise that path.
#[test]
fn payload_attributes_v4_parses_target_gas_limit_when_present() {
    let json = serde_json::json!({
        "timestamp": "0x65",
        "prevRandao": "0x0000000000000000000000000000000000000000000000000000000000000001",
        "suggestedFeeRecipient": "0x0000000000000000000000000000000000000002",
        "withdrawals": [],
        "parentBeaconBlockRoot": "0x0000000000000000000000000000000000000000000000000000000000000003",
        "slotNumber": "0x10",
        "targetGasLimit": "0x2faf080",
    });
    let attrs: PayloadAttributesV4 = serde_json::from_value(json).unwrap();
    assert_eq!(attrs.target_gas_limit, 50_000_000);
    assert_eq!(attrs.slot_number, 0x10);
}

#[test]
fn payload_attributes_v4_rejects_missing_target_gas_limit() {
    let json = serde_json::json!({
        "timestamp": "0x65",
        "prevRandao": "0x0000000000000000000000000000000000000000000000000000000000000001",
        "suggestedFeeRecipient": "0x0000000000000000000000000000000000000002",
        "withdrawals": [],
        "parentBeaconBlockRoot": "0x0000000000000000000000000000000000000000000000000000000000000003",
        "slotNumber": "0x10",
    });
    assert!(serde_json::from_value::<PayloadAttributesV4>(json).is_err());
}

#[test]
fn payload_attributes_v4_rejects_null_target_gas_limit() {
    let json = serde_json::json!({
        "timestamp": "0x65",
        "prevRandao": "0x0000000000000000000000000000000000000000000000000000000000000001",
        "suggestedFeeRecipient": "0x0000000000000000000000000000000000000002",
        "withdrawals": [],
        "parentBeaconBlockRoot": "0x0000000000000000000000000000000000000000000000000000000000000003",
        "slotNumber": "0x10",
        "targetGasLimit": null,
    });
    assert!(serde_json::from_value::<PayloadAttributesV4>(json).is_err());
}

// Builds an in-memory store from l1.json with Amsterdam (= upstream
// "Glamsterdam") activated at t=0 so the V4 validator paths added by
// execution-apis#796 are reachable.
async fn amsterdam_test_store() -> Store {
    let file = File::open(workspace_root().join("fixtures/genesis/l1.json"))
        .expect("Failed to open genesis file");
    let reader = BufReader::new(file);
    let mut genesis: Genesis =
        serde_json::from_reader(reader).expect("Failed to deserialize genesis file");
    genesis.config.amsterdam_time = Some(0);
    let mut store = Store::new("amsterdam-store.db", EngineType::InMemory)
        .expect("Failed to build DB for testing");
    store
        .add_initial_state(genesis)
        .await
        .expect("Failed to add genesis state");
    store
}

fn fcu_v4_request(head: H256, timestamp: u64, target_gas_limit: Option<&str>) -> RpcRequest {
    let target_field = match target_gas_limit {
        Some(hex) => format!(",\n                    \"targetGasLimit\": \"{hex}\""),
        None => String::new(),
    };
    let body = format!(
        r#"{{
            "jsonrpc": "2.0",
            "method": "engine_forkchoiceUpdatedV4",
            "params": [
                {{
                    "headBlockHash": "{head:#x}",
                    "safeBlockHash": "{head:#x}",
                    "finalizedBlockHash": "{head:#x}"
                }},
                {{
                    "timestamp": "{timestamp:#x}",
                    "prevRandao": "0x0000000000000000000000000000000000000000000000000000000000000001",
                    "suggestedFeeRecipient": "0x0000000000000000000000000000000000000000",
                    "withdrawals": [],
                    "parentBeaconBlockRoot": "0x0000000000000000000000000000000000000000000000000000000000000002",
                    "slotNumber": "0x1"{target_field}
                }}
            ],
            "id": 1
        }}"#
    );
    serde_json::from_str(&body).expect("valid FCU request")
}

// execution-apis#796: a CL-supplied targetGasLimit on an Amsterdam chain is
// accepted and the client begins a payload build.
#[tokio::test]
async fn fcu_v4_accepts_target_gas_limit_present() {
    let store = amsterdam_test_store().await;
    let genesis = store.get_block_header(0).unwrap().unwrap();
    let request = fcu_v4_request(genesis.hash(), genesis.timestamp + 12, Some("0x2faf080"));

    let context = default_context_with_storage(store).await;
    let response = ForkChoiceUpdatedV4::call(&request, context)
        .await
        .expect("FCU V4 call should succeed");

    assert!(
        !response["payloadId"].is_null(),
        "payloadId must be set when V4 attributes are valid; got {:?}",
        response["payloadId"]
    );
}

// Builds an FCUv4 request carrying a raw third `custodyColumns` parameter (EIP-8070).
fn fcu_v4_request_with_custody_columns(
    head: H256,
    timestamp: u64,
    custody_columns: &str,
) -> RpcRequest {
    let mut request = fcu_v4_request(head, timestamp, Some("0x2faf080"));
    let params = request.params.as_mut().expect("FCUv4 params");
    params.push(serde_json::from_str(custody_columns).expect("valid custodyColumns literal"));
    request
}

// EIP-8070: a CL that provides custody services passes a 16-byte custody bitarray as
// the third FCUv4 parameter. ethrex replicates every blob, so the set is accepted and
// ignored — it must not fail the call.
#[tokio::test]
async fn fcu_v4_accepts_custody_columns() {
    let store = amsterdam_test_store().await;
    let genesis = store.get_block_header(0).unwrap().unwrap();
    let request = fcu_v4_request_with_custody_columns(
        genesis.hash(),
        genesis.timestamp + 12,
        r#""0xffffffffffffffffffffffffffffffff""#,
    );

    let context = default_context_with_storage(store).await;
    let response = ForkChoiceUpdatedV4::call(&request, context)
        .await
        .expect("FCU V4 must accept custodyColumns");

    assert!(!response["payloadId"].is_null());
}

// A CL that provides no custody services sends `null`, which is equally valid.
#[tokio::test]
async fn fcu_v4_accepts_null_custody_columns() {
    let store = amsterdam_test_store().await;
    let genesis = store.get_block_header(0).unwrap().unwrap();
    let request =
        fcu_v4_request_with_custody_columns(genesis.hash(), genesis.timestamp + 12, "null");

    let context = default_context_with_storage(store).await;
    let response = ForkChoiceUpdatedV4::call(&request, context)
        .await
        .expect("FCU V4 must accept a null custodyColumns");

    assert!(!response["payloadId"].is_null());
}

// A non-null custodyColumns that is not exactly 16 bytes is `-32602: Invalid params`.
#[tokio::test]
async fn fcu_v4_rejects_wrong_length_custody_columns() {
    let store = amsterdam_test_store().await;
    let genesis = store.get_block_header(0).unwrap().unwrap();
    let request =
        fcu_v4_request_with_custody_columns(genesis.hash(), genesis.timestamp + 12, r#""0xff""#);

    let context = default_context_with_storage(store).await;
    let err = ForkChoiceUpdatedV4::call(&request, context)
        .await
        .expect_err("a short custodyColumns must be rejected");

    // The Amsterdam Engine API spec mandates -32602 here.
    assert_eq!(RpcErrorMetadata::from(err).code, -32602, "wrong error code");
}

// execution-apis#796: targetGasLimit is required on V4; an absent field is
// rejected at deserialization, so the FCUv4 request fails to parse.
#[tokio::test]
async fn fcu_v4_rejects_target_gas_limit_absent() {
    let store = amsterdam_test_store().await;
    let genesis = store.get_block_header(0).unwrap().unwrap();
    let request = fcu_v4_request(genesis.hash(), genesis.timestamp + 12, None);

    let context = default_context_with_storage(store).await;
    let err = ForkChoiceUpdatedV4::call(&request, context)
        .await
        .expect_err("FCU V4 must reject attributes without targetGasLimit");

    assert!(
        matches!(err, RpcErr::InvalidPayloadAttributes(_)),
        "got: {err:?}"
    );
}

// V4 attributes for a pre-Amsterdam timestamp are still rejected outright.
#[tokio::test]
async fn fcu_v4_rejects_pre_amsterdam_timestamp() {
    // execution-api.json has no amsterdamTime, so the chain is pre-Amsterdam.
    let store = test_store().await;
    let genesis = store.get_block_header(0).unwrap().unwrap();
    let request = fcu_v4_request(genesis.hash(), genesis.timestamp + 12, Some("0x2faf080"));

    let context = default_context_with_storage(store).await;
    let err = ForkChoiceUpdatedV4::call(&request, context)
        .await
        .expect_err("FCU V4 must reject pre-Amsterdam attributes");

    assert!(
        matches!(err, RpcErr::InvalidPayloadAttributes(_)),
        "got: {err:?}"
    );
}

// At the Amsterdam activation timestamp, engine_forkchoiceUpdatedV3 must reject
// otherwise-valid V3 payload attributes with UnsupportedFork; payload building
// from that timestamp onward requires engine_forkchoiceUpdatedV4.
#[tokio::test]
async fn forkchoice_updated_v3_rejects_amsterdam_payload_attributes() {
    let mut store = test_store().await;
    let mut chain_config = store.get_chain_config();
    let amsterdam_time = 24;
    chain_config.amsterdam_time = Some(amsterdam_time);
    store.set_chain_config(&chain_config).await.unwrap();

    let genesis_header = store.get_block_header(0).unwrap().unwrap();
    let genesis_hash = genesis_header.hash();
    let block = new_block(&store, &genesis_header).await;
    let block_hash = block.hash();
    Blockchain::default_with_store(store.clone())
        .add_block(block)
        .unwrap();

    let body = format!(
        r#"{{
            "jsonrpc": "2.0",
            "method": "engine_forkchoiceUpdatedV3",
            "params": [
                {{
                    "headBlockHash": "{block_hash:#x}",
                    "safeBlockHash": "{genesis_hash:#x}",
                    "finalizedBlockHash": "{genesis_hash:#x}"
                }},
                {{
                    "timestamp": "{amsterdam_time:#x}",
                    "prevRandao": "0x0000000000000000000000000000000000000000000000000000000000000001",
                    "suggestedFeeRecipient": "0x0000000000000000000000000000000000000000",
                    "withdrawals": [],
                    "parentBeaconBlockRoot": "0x0000000000000000000000000000000000000000000000000000000000000002"
                }}
            ],
            "id": 1
        }}"#
    );
    let request: RpcRequest = serde_json::from_str(&body).expect("valid FCU request");
    let context = default_context_with_storage(store).await;

    let err = ForkChoiceUpdatedV3::call(&request, context)
        .await
        .unwrap_err();

    assert!(matches!(err, ethrex_rpc::utils::RpcErr::UnsupportedFork(_)));
}

#[test]
fn payload_attributes_v5_round_trips_with_inclusion_list() {
    let json = r#"{
        "timestamp": "0x6846fb2",
        "prevRandao": "0x2971eefd1f71f3548728cad87c16cc91b979ef035054828c59a02e49ae300a84",
        "suggestedFeeRecipient": "0x8943545177806ed17b9f23f0a21ee5948ecaa776",
        "withdrawals": [],
        "parentBeaconBlockRoot": "0x4029a2342bb6d54db91457bc8e442be22b3481df8edea24cc721f9d0649f65be",
        "slotNumber": "0x10",
        "inclusionListTransactions": ["0xdeadbeef", "0x01020304"],
        "targetGasLimit": "0x2faf080"
    }"#;

    let attrs: PayloadAttributesV5 = serde_json::from_str(json).expect("V5 attributes deserialize");

    assert_eq!(attrs.timestamp, 0x6846fb2);
    assert_eq!(attrs.slot_number, 0x10);
    assert_eq!(
        attrs.suggested_fee_recipient,
        Address::from_slice(
            &hex::decode("8943545177806ed17b9f23f0a21ee5948ecaa776").expect("decode fee recipient")
        )
    );
    assert!(attrs.withdrawals.is_some());
    assert!(attrs.parent_beacon_block_root.is_some());
    assert_eq!(attrs.inclusion_list_transactions.len(), 2);
    assert_eq!(
        attrs.inclusion_list_transactions[0].as_ref(),
        &[0xde, 0xad, 0xbe, 0xef][..]
    );
    assert_eq!(
        attrs.inclusion_list_transactions[1].as_ref(),
        &[0x01, 0x02, 0x03, 0x04][..]
    );

    let serialized = serde_json::to_string(&attrs).expect("V5 attributes serialize");
    assert!(serialized.contains("\"inclusionListTransactions\":[\"0xdeadbeef\",\"0x01020304\"]"));
    assert!(serialized.contains("\"slotNumber\":\"0x10\""));

    let reparsed: PayloadAttributesV5 =
        serde_json::from_str(&serialized).expect("V5 attributes round-trip");
    assert_eq!(
        reparsed.inclusion_list_transactions,
        attrs.inclusion_list_transactions
    );
    assert_eq!(reparsed.timestamp, attrs.timestamp);
    assert_eq!(reparsed.slot_number, attrs.slot_number);
}

#[test]
fn payload_attributes_v5_accepts_empty_inclusion_list() {
    let json = r#"{
        "timestamp": "0x6846fb2",
        "prevRandao": "0x2971eefd1f71f3548728cad87c16cc91b979ef035054828c59a02e49ae300a84",
        "suggestedFeeRecipient": "0x8943545177806ed17b9f23f0a21ee5948ecaa776",
        "withdrawals": [],
        "parentBeaconBlockRoot": "0x4029a2342bb6d54db91457bc8e442be22b3481df8edea24cc721f9d0649f65be",
        "slotNumber": "0x10",
        "inclusionListTransactions": [],
        "targetGasLimit": "0x2faf080"
    }"#;

    let attrs: PayloadAttributesV5 = serde_json::from_str(json).expect("V5 attributes deserialize");
    assert!(attrs.inclusion_list_transactions.is_empty());

    let serialized = serde_json::to_string(&attrs).expect("V5 attributes serialize");
    assert!(serialized.contains("\"inclusionListTransactions\":[]"));
}

#[test]
fn payload_attributes_v5_to_v4_propagates_gas_limit_and_drops_il() {
    let attrs_v5 = PayloadAttributesV5 {
        timestamp: 0x6846fb2,
        prev_randao: H256::zero(),
        suggested_fee_recipient: Address::zero(),
        withdrawals: Some(vec![]),
        parent_beacon_block_root: Some(H256::zero()),
        slot_number: 0x10,
        inclusion_list_transactions: vec![Bytes::from_static(&[0xde, 0xad, 0xbe, 0xef])],
        target_gas_limit: 50_000_000,
    };

    let attrs_v4: PayloadAttributesV4 = (&attrs_v5).into();
    assert_eq!(attrs_v4.timestamp, attrs_v5.timestamp);
    assert_eq!(attrs_v4.slot_number, attrs_v5.slot_number);
    assert_eq!(attrs_v4.prev_randao, attrs_v5.prev_randao);
    assert_eq!(
        attrs_v4.suggested_fee_recipient,
        attrs_v5.suggested_fee_recipient
    );
    assert_eq!(attrs_v4.withdrawals, attrs_v5.withdrawals);
    assert_eq!(
        attrs_v4.parent_beacon_block_root,
        attrs_v5.parent_beacon_block_root
    );
    // execution-apis#796: the gas target must survive the V5->V4 downgrade.
    assert_eq!(attrs_v4.target_gas_limit, attrs_v5.target_gas_limit);
}

#[test]
fn payload_attributes_v5_parses_target_gas_limit() {
    let base = |extra: &str| {
        format!(
            r#"{{
                "timestamp": "0x65",
                "prevRandao": "0x0000000000000000000000000000000000000000000000000000000000000001",
                "suggestedFeeRecipient": "0x0000000000000000000000000000000000000002",
                "withdrawals": [],
                "parentBeaconBlockRoot": "0x0000000000000000000000000000000000000000000000000000000000000003",
                "slotNumber": "0x10",
                "inclusionListTransactions": []{extra}
            }}"#
        )
    };

    // present -> value
    let attrs: PayloadAttributesV5 =
        serde_json::from_str(&base(r#", "targetGasLimit": "0x2faf080""#)).unwrap();
    assert_eq!(attrs.target_gas_limit, 50_000_000);

    // execution-apis#796: targetGasLimit is required on V5; an absent field
    // fails deserialization (the FCUv5 request is rejected as invalid).
    assert!(serde_json::from_str::<PayloadAttributesV5>(&base("")).is_err());
}

#[test]
fn payload_attributes_v5_default_constructible() {
    let attrs = PayloadAttributesV5::default();
    assert_eq!(attrs.timestamp, 0);
    assert_eq!(attrs.slot_number, 0);
    assert!(attrs.inclusion_list_transactions.is_empty());
}

// ── eth/72 (EIP-8070) custodyColumns: parse_v4 / parse_custody_columns /
// apply_custody_update ───────────────────────────────────────────────────────
//
// Moved from crates/networking/rpc/engine/fork_choice.rs. These exercise the
// crate-private parse/apply internals through `test_utils` feature-gated shims.
use ethrex_rpc::test_utils::{apply_custody_update, parse_custody_columns, parse_v4};
use serde_json::json;

fn minimal_fcs_json() -> serde_json::Value {
    json!({
        "headBlockHash": H256::zero(),
        "safeBlockHash": H256::zero(),
        "finalizedBlockHash": H256::zero(),
    })
}

#[test]
fn parse_v4_custody_absent() {
    // 1 param — no custodyColumns
    let params = Some(vec![minimal_fcs_json()]);
    let (_, _, cc) = parse_v4(&params).unwrap();
    assert_eq!(cc, None);
}

#[test]
fn parse_v4_custody_null() {
    // 3 params, third is JSON null
    let params = Some(vec![minimal_fcs_json(), json!(null), json!(null)]);
    let (_, _, cc) = parse_v4(&params).unwrap();
    assert_eq!(cc, None);
}

#[test]
fn parse_v4_custody_valid_16_bytes() {
    // Little-endian: column 0 (bit 0) => byte[0] = 0x01 => u128 = 1.
    let params = Some(vec![
        minimal_fcs_json(),
        json!(null),
        json!("0x01000000000000000000000000000000"),
    ]);
    let (_, _, cc) = parse_v4(&params).unwrap();
    assert_eq!(cc, Some(1u128));
}

#[test]
fn parse_v4_custody_wrong_length_rejected() {
    // Only 8 bytes — must reject
    let params = Some(vec![
        minimal_fcs_json(),
        json!(null),
        json!("0x0000000000000001"),
    ]);
    let err = parse_v4(&params).unwrap_err();
    assert_eq!(RpcErrorMetadata::from(err).code, -32602);
}

#[test]
fn parse_custody_columns_null_returns_none() {
    assert_eq!(parse_custody_columns(&json!(null)).unwrap(), None);
}

#[test]
fn parse_custody_columns_16_byte_roundtrip() {
    let mask: u128 = 0xDEAD_BEEF_1234_5678_9ABC_DEF0_1234_5678;
    let hex = format!("0x{}", hex::encode(mask.to_le_bytes()));
    let result = parse_custody_columns(&json!(hex)).unwrap();
    assert_eq!(result, Some(mask));
}

#[test]
fn parse_custody_columns_wrong_length() {
    let err = parse_custody_columns(&json!("0xdeadbeef")).unwrap_err();
    assert_eq!(RpcErrorMetadata::from(err).code, -32602);
}

async fn fresh_context() -> RpcApiContext {
    let storage = Store::new("test", EngineType::InMemory).expect("store");
    default_context_with_storage(storage).await
}

#[tokio::test]
async fn apply_custody_update_null_is_noop() {
    let ctx = fresh_context().await;
    ctx.blockchain.mempool.set_custody_columns(0xFF).unwrap();
    apply_custody_update(&ctx, None);
    assert_eq!(ctx.blockchain.mempool.get_custody_columns().unwrap(), 0xFF);
}

#[tokio::test]
async fn apply_custody_update_identical_is_noop() {
    let ctx = fresh_context().await;
    ctx.blockchain.mempool.set_custody_columns(0b1010).unwrap();
    apply_custody_update(&ctx, Some(0b1010));
    assert_eq!(
        ctx.blockchain.mempool.get_custody_columns().unwrap(),
        0b1010
    );
}

#[tokio::test]
async fn apply_custody_update_expansion_sets_columns() {
    let ctx = fresh_context().await;
    ctx.blockchain.mempool.set_custody_columns(0b0001).unwrap();
    apply_custody_update(&ctx, Some(0b0011)); // add column 1
    assert_eq!(
        ctx.blockchain.mempool.get_custody_columns().unwrap(),
        0b0011
    );
}

#[tokio::test]
async fn apply_custody_update_contraction_sets_and_retains_cells() {
    let ctx = fresh_context().await;
    ctx.blockchain.mempool.set_custody_columns(0b1111).unwrap();
    let tx_hash = H256::from_low_u64_be(42);
    ctx.blockchain
        .mempool
        .store_cells(tx_hash, 1, vec![])
        .unwrap();
    let before = ctx.blockchain.mempool.get_cells_mask(tx_hash).unwrap();

    apply_custody_update(&ctx, Some(0b0011)); // remove columns 2,3

    assert_eq!(
        ctx.blockchain.mempool.get_custody_columns().unwrap(),
        0b0011
    );
    // Pruning dropped columns is optional (execution-apis amsterdam.md,
    // engine_forkchoiceUpdatedV4 §3.3.2); cells are retained so peers that
    // already sampled us can still be served.
    assert_eq!(
        ctx.blockchain.mempool.get_cells_mask(tx_hash).unwrap(),
        before
    );
}
