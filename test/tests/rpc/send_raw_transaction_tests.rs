//! Regression test for the `fee-token-l1-tx` finding (RPC mempool-ingress side).
//! FeeToken (type 0x7d) and PrivilegedL2 (type 0x7e) are L2-only transaction
//! types unknown to other L1 clients. The block-import path was closed by #6752
//! (`is_l2_only()`); these tests guard the `eth_sendRawTransaction` admission
//! parser, which must reject both so neither enters the L1 mempool.
use ethrex_common::types::{EIP1559Transaction, FeeTokenTransaction, Transaction};
use ethrex_rpc::RpcErr;
use ethrex_rpc::rpc::RpcHandler;
use ethrex_rpc::types::transaction::SendRawTransactionRequest;
use serde_json::{Value, json};

fn raw_tx_params(tx: &Transaction) -> Option<Vec<Value>> {
    let raw = tx.encode_canonical_to_vec();
    Some(vec![json!(format!("0x{}", hex::encode(raw)))])
}

#[test]
fn send_raw_transaction_rejects_fee_token() {
    let tx = Transaction::FeeTokenTransaction(FeeTokenTransaction::default());
    let res = SendRawTransactionRequest::parse(&raw_tx_params(&tx));
    assert!(
        matches!(res, Err(RpcErr::BadParams(_))),
        "FeeToken (0x7d) tx must be rejected at RPC admission (got {res:?})"
    );
}

/// Control: a normal L1 tx (EIP-1559) must still be accepted by the parser, so
/// the FeeToken rejection isn't an over-broad reject of all typed txs.
#[test]
fn send_raw_transaction_accepts_eip1559() {
    let tx = Transaction::EIP1559Transaction(EIP1559Transaction::default());
    let res = SendRawTransactionRequest::parse(&raw_tx_params(&tx));
    assert!(
        res.is_ok(),
        "a normal EIP-1559 tx must still parse at RPC admission (got {res:?})"
    );
}
