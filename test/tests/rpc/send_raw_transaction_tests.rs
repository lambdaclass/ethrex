//! Regression tests for the `fee-token-l1-tx` finding (mempool-ingress side).
//!
//! `SendRawTransactionRequest::parse` is shared by the L1 *and* L2
//! `eth_sendRawTransaction` routes, and `FeeToken` (0x7d) is a valid L2 type the
//! L2 SDK submits via raw RPC — so the parser must NOT reject it. The L1-only
//! rejection of L2-only types lives in the mempool's `validate_transaction`
//! (gated on `BlockchainType::L1`); see the `blockchain` test domain.
use ethrex_common::types::{EIP1559Transaction, FeeTokenTransaction, Transaction};
use ethrex_rpc::rpc::RpcHandler;
use ethrex_rpc::types::transaction::SendRawTransactionRequest;
use serde_json::{Value, json};

fn raw_tx_params(tx: &Transaction) -> Option<Vec<Value>> {
    let raw = tx.encode_canonical_to_vec();
    Some(vec![json!(format!("0x{}", hex::encode(raw)))])
}

/// The shared parser must accept `FeeToken` so the L2 `eth_sendRawTransaction`
/// route (which reuses this parser) keeps working. Rejecting it here would
/// break valid L2 ingress — guards against re-introducing that bug.
#[test]
fn send_raw_transaction_parse_accepts_fee_token() {
    let tx = Transaction::FeeTokenTransaction(FeeTokenTransaction::default());
    let res = SendRawTransactionRequest::parse(&raw_tx_params(&tx));
    assert!(
        res.is_ok(),
        "the shared parser must accept FeeToken (0x7d) so L2 ingress works; \
         the L1-only rejection belongs in validate_transaction (got {res:?})"
    );
}

/// Control: a normal L1 tx (EIP-1559) parses fine.
#[test]
fn send_raw_transaction_accepts_eip1559() {
    let tx = Transaction::EIP1559Transaction(EIP1559Transaction::default());
    let res = SendRawTransactionRequest::parse(&raw_tx_params(&tx));
    assert!(
        res.is_ok(),
        "a normal EIP-1559 tx must parse at RPC admission (got {res:?})"
    );
}
