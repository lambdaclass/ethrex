use std::{fs::File, io::BufReader, path::PathBuf};

use ethrex_blockchain::matcha::{MatchaConfig, admission_gas, charge_for};
use ethrex_blockchain::mempool::KeyedConcurrency;
use ethrex_common::types::MempoolTransaction;
use ethrex_common::types::{
    APPROVE_EXECUTION_AND_PAYMENT, EIP1559Transaction, FRAME_SIG_SCHEME_SECP256K1, Frame,
    FrameMode, FrameSignature, FrameTransaction, Transaction,
};
use ethrex_common::{Address, U256};
use ethrex_crypto::NativeCrypto;
use ethrex_rpc::ethrex::{MatchaWidthRequest, SimulateFrameTransactionRequest};
use ethrex_rpc::rpc::{RpcApiContext, RpcHandler};
use ethrex_rpc::test_utils::default_context_with_storage;
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
            gas_limit: 21_000,
            state_gas_limit: 0,
            value: U256::zero(),
            data: Default::default(),
        }],
        signatures: vec![FrameSignature {
            scheme: FRAME_SIG_SCHEME_SECP256K1,
            signer: Some(sender()),
            msg: Default::default(),
            signature: Default::default(),
        }],
        max_priority_fee_per_gas: U256::from(1),
        max_fee_per_gas: U256::from(1_000),
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

/// The fixture with no signature at all: EIP-8141 lets a contract sender's code be the
/// authorization, so an empty list authenticates and the prefix gates run.
fn unsigned_self_verify_tx(key: u64) -> FrameTransaction {
    let mut tx = self_verify_tx();
    tx.nonce_keys = vec![U256::from(key)];
    tx.signatures = vec![];
    // Above the genesis base fee (1 gwei), so the simulation reaches the prefix instead
    // of stopping at the fee check.
    tx.max_fee_per_gas = U256::from(2_000_000_000u64);
    tx
}

async fn simulate_in(context: RpcApiContext, tx: FrameTransaction) -> serde_json::Value {
    let params = Some(vec![json!(raw_hex(&Transaction::FrameTransaction(tx)))]);
    SimulateFrameTransactionRequest::parse(&params)
        .expect("parse")
        .handle(context)
        .await
        .expect("handle")
}

#[tokio::test]
async fn simulate_quotes_the_matcha_charge_admission_would_take() {
    // A fresh sender has nothing pending, so the transaction would be its free baseline,
    // and the quoted charge is the figure admission itself computes from the prefix.
    let tx = unsigned_self_verify_tx(1);
    let expected = charge_for(&MatchaConfig::default(), admission_gas(&tx, 21_000));

    let result = simulate(tx).await;

    assert_eq!(result["matchaCharge"], json!(format!("0x{expected:x}")));
    assert_eq!(result["matchaAdmissible"], json!(true));
    assert_eq!(result["matchaRefusal"], json!(null));
}

#[tokio::test]
async fn simulate_reports_the_refusal_a_pending_sender_with_no_width_would_get() {
    let context = context().await;
    let pending = unsigned_self_verify_tx(1);
    let pending_hash = Transaction::FrameTransaction(pending.clone()).hash(&NativeCrypto);
    context
        .blockchain
        .mempool
        .add_transaction(
            pending_hash,
            sender(),
            MempoolTransaction::new(Transaction::FrameTransaction(pending), sender()),
            None,
            None,
            KeyedConcurrency::Allowed,
            None,
        )
        .expect("baseline admitted");

    let result = simulate_in(context, unsigned_self_verify_tx(2)).await;

    assert_eq!(result["matchaAdmissible"], json!(false));
    let refusal = result["matchaRefusal"].as_str().expect("refusal");
    assert!(
        refusal.contains("has 0 MATCHA width"),
        "expected the width refusal admission returns, got: {refusal}"
    );
    assert!(
        result["matchaCharge"]
            .as_str()
            .is_some_and(|c| c.starts_with("0x"))
    );
}

#[tokio::test]
async fn simulate_leaves_the_matcha_fields_null_when_the_prefix_is_structurally_invalid() {
    let mut tx = self_verify_tx();
    tx.nonce_keys = vec![];

    let result = simulate(tx).await;

    assert_eq!(result["matchaCharge"], json!(null));
    assert_eq!(result["matchaAdmissible"], json!(null));
}

#[test]
fn matcha_width_parse_takes_exactly_one_address() {
    let ok = MatchaWidthRequest::parse(&Some(vec![json!(format!("{:#x}", sender()))]))
        .expect("address accepted");
    assert_eq!(ok.sender, sender());
    assert!(matches!(
        MatchaWidthRequest::parse(&Some(vec![])),
        Err(RpcErr::BadParams(_))
    ));
    assert!(matches!(
        MatchaWidthRequest::parse(&Some(vec![json!("0x12")])),
        Err(RpcErr::WrongParam(_))
    ));
    assert!(matches!(
        MatchaWidthRequest::parse(&Some(vec![json!(format!("{:#x}", sender())), json!(1)])),
        Err(RpcErr::BadParams(_))
    ));
}

#[tokio::test]
async fn matcha_width_reports_the_ledger_and_the_policy() {
    let context = context().await;
    let request = MatchaWidthRequest { sender: sender() };

    let fresh = request.handle(context.clone()).await.expect("handle");
    assert_eq!(fresh["enabled"], json!(true));
    assert_eq!(fresh["width"], json!("0x0"));
    assert_eq!(fresh["widthCap"], json!("0x1c9c380"));
    assert_eq!(fresh["lastCreditedBlock"], json!(null));
    assert_eq!(fresh["pendingFrameTxs"], json!(0));
    assert_eq!(fresh["safetyFactorNum"], json!(3));
    assert_eq!(fresh["safetyFactorDen"], json!(2));

    let mut gas = rustc_hash::FxHashMap::default();
    gas.insert(sender(), 1_000_000u64);
    context
        .blockchain
        .mempool
        .credit_finalized_block(7, &gas)
        .expect("credit");

    let credited = request.handle(context).await.expect("handle");
    assert_eq!(credited["width"], json!("0xf4240"));
    assert_eq!(credited["lastCreditedBlock"], json!("0x7"));
}
