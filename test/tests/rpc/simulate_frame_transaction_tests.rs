use std::{fs::File, io::BufReader, path::PathBuf};

use ethrex_common::types::{
    APPROVE_EXECUTION_AND_PAYMENT, EIP1559Transaction, FRAME_RECEIPT_STATUS_SUCCESS,
    FRAME_SIG_SCHEME_SECP256K1, Frame, FrameEncoding, FrameLimits, FrameMode, FrameReceipt,
    FrameSignature, FrameTransaction, Transaction,
};
use ethrex_common::{Address, U256};
use ethrex_rpc::ethrex::SimulateFrameTransactionRequest;
use ethrex_rpc::rpc::{RpcApiContext, RpcHandler};
use ethrex_rpc::test_utils::default_context_with_storage;
use ethrex_rpc::types::receipt::RpcFrameReceipt;
use ethrex_rpc::utils::RpcErr;
use ethrex_storage::{EngineType, Store};
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

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

async fn context() -> RpcApiContext {
    let file = File::open(workspace_root().join("fixtures/genesis/execution-api.json"))
        .expect("open genesis");
    let genesis = serde_json::from_reader(BufReader::new(file)).expect("parse genesis");
    let mut store = Store::new("store.db", EngineType::InMemory).expect("build store");
    store
        .add_initial_state(genesis)
        .await
        .expect("genesis state");
    default_context_with_storage(store).await
}

fn sender() -> Address {
    Address::repeat_byte(0x11)
}

/// A frame tx whose prefix is one `SelfVerify` frame — the simplest of the four
/// admitted shapes — so anything reported invalid comes from the gate under test.
fn self_verify_tx() -> FrameTransaction {
    FrameTransaction {
        chain_id: 1,
        nonce_keys: vec![U256::zero()],
        nonce_seq: 0,
        sender: sender(),
        frames: vec![Frame {
            mode: FrameMode::Verify as u8,
            flags: APPROVE_EXECUTION_AND_PAYMENT,
            target: Some(sender()),
            limits: FrameLimits {
                execution: 21_000,
                state: 21_000,
            },
            value: U256::zero(),
            data: Default::default(),
            encoding: FrameEncoding::Limits,
        }],
        signatures: vec![FrameSignature {
            scheme: FRAME_SIG_SCHEME_SECP256K1,
            signer: Some(sender()),
            msg: Default::default(),
            signature: Default::default(),
        }],
        max_priority_fee_per_gas: 1,
        max_fee_per_gas: 1_000,
        ..Default::default()
    }
}

async fn simulate(tx: FrameTransaction) -> serde_json::Value {
    let params = Some(vec![json!(raw_hex(&Transaction::FrameTransaction(tx)))]);
    SimulateFrameTransactionRequest::parse(&params)
        .expect("parse")
        .handle(context().await)
        .await
        .expect("handle")
}

#[tokio::test]
async fn simulate_rejects_nonce_keys_that_are_not_strictly_increasing() {
    // EIP-8250 static rule. Before the admission gates were wired in, a prefix
    // that simulated cleanly reported `valid: true` for a transaction the
    // mempool would refuse outright.
    let mut tx = self_verify_tx();
    tx.nonce_keys = vec![U256::from(5u64), U256::from(5u64)];

    let result = simulate(tx).await;

    assert_eq!(result["valid"], json!(false));
    let violation = result["violation"].as_str().expect("violation");
    assert!(
        violation.contains("nonce_keys"),
        "expected a nonce-key violation, got: {violation}"
    );
}

#[tokio::test]
async fn simulate_rejects_an_unauthenticated_sender() {
    // EIP-8141: `sender` is an unauthenticated field until the signature list
    // recovers to it. An empty SECP256K1 signature can never do that.
    let result = simulate(self_verify_tx()).await;

    assert_eq!(result["valid"], json!(false));
    let violation = result["violation"].as_str().expect("violation");
    assert!(
        violation.contains("signature"),
        "expected a signature violation, got: {violation}"
    );
}

#[tokio::test]
async fn simulate_reports_max_cost_even_when_a_gate_rejects() {
    // `maxCost` is a pure function of the transaction fields, so a caller still
    // learns what the transaction would have cost.
    let mut tx = self_verify_tx();
    tx.nonce_keys = vec![];

    let result = simulate(tx).await;

    assert_eq!(result["valid"], json!(false));
    assert!(
        result["maxCost"]
            .as_str()
            .is_some_and(|c| c.starts_with("0x")),
        "maxCost must be reported on every path"
    );
}

/// Both per-frame gas dimensions must reach JSON-RPC.
///
/// EIP-8141 made `gas_used` two-dimensional, and the two pools never mix. A
/// frame that only moves value does no EVM work, so its execution figure is
/// zero and the state figure is the whole of its cost — a receipt reporting
/// only `gasUsed` shows `0x0` and reads as "this frame was free". The consensus
/// receipt carried `state_gas_used` while both RPC views dropped it, which is
/// exactly the shape of bug that survives because the number it hides is
/// usually small.
#[test]
fn rpc_frame_receipt_reports_both_gas_dimensions() {
    let receipt = FrameReceipt {
        status: FRAME_RECEIPT_STATUS_SUCCESS,
        gas_used: 0,
        state_gas_used: 183_600,
        logs: vec![],
        encoding: FrameEncoding::Limits,
    };
    let rpc: RpcFrameReceipt = receipt.into();
    let json = serde_json::to_value(&rpc).expect("must serialize");

    assert_eq!(
        json.get("stateGasUsed").and_then(|v| v.as_str()),
        Some("0x2cd30"),
        "the state dimension must be present and hex-encoded: {json}"
    );
    assert_eq!(
        json.get("gasUsed").and_then(|v| v.as_str()),
        Some("0x0"),
        "the execution dimension is genuinely zero here, which is why the \
         state figure is the only one that describes this frame"
    );
}
