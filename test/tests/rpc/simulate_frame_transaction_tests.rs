use ethrex_common::types::{EIP1559Transaction, FrameTransaction, Transaction};
use ethrex_rpc::ethrex::SimulateFrameTransactionRequest;
use ethrex_rpc::rpc::RpcHandler;
use ethrex_rpc::utils::RpcErr;
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
