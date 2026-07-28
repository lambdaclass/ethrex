//! Regression tests for the `fee-token-l1-tx` finding (mempool-ingress side).
//!
//! `SendRawTransactionRequest::parse` is shared by the L1 *and* L2
//! `eth_sendRawTransaction` routes, and `FeeToken` (0x7d) is a valid L2 type the
//! L2 SDK submits via raw RPC — so the parser must NOT reject it. The L1-only
//! rejection of L2-only types lives in the mempool's `validate_transaction`
//! (gated on `BlockchainType::L1`); see the `blockchain` test domain.
use ethrex_common::types::{
    BlobsBundle, EIP1559Transaction, FeeTokenTransaction, Frame, FrameTransaction, Transaction,
    TxType, WrappedFrameTransaction,
};
use ethrex_common::{Address, H256, U256};
use ethrex_rlp::encode::RLPEncode;
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

/// A blob-carrying EIP-8141 frame transaction, submitted in the EIP-7594 wrapped
/// form per EIP-8141 §Networking. Without the wrapped variant it decodes as a
/// bare frame transaction, loses its sidecar, and is then rejected by the
/// blob-admission guard — so the feature would be unreachable over RPC.
#[test]
fn send_raw_transaction_accepts_a_wrapped_blob_frame_transaction() {
    let (tx, blobs_bundle) = blob_frame_tx();
    let wrapped = WrappedFrameTransaction {
        tx: tx.clone(),
        wrapper_version: Some(1),
        blobs_bundle: blobs_bundle.clone(),
    };
    let mut raw = vec![TxType::Frame as u8];
    wrapped.encode(&mut raw);

    let parsed = SendRawTransactionRequest::parse(&raw_params(&raw)).expect("must parse");
    let SendRawTransactionRequest::FrameWithBlobs(ref got) = parsed else {
        panic!("a wrapped frame transaction must parse to FrameWithBlobs, got {parsed:?}");
    };
    assert_eq!(got.blobs_bundle, blobs_bundle);
    assert_eq!(
        parsed.to_transaction(),
        Transaction::FrameTransaction(tx),
        "the pooled transaction is the inner one; the sidecar rides alongside"
    );
}

/// The converse of the p2p rule, on the RPC path: a frame transaction that
/// declares blobs but arrives unwrapped has irrecoverably lost its sidecar.
#[test]
fn send_raw_transaction_rejects_an_unwrapped_blob_frame_transaction() {
    let (tx, _) = blob_frame_tx();
    let res = SendRawTransactionRequest::parse(&raw_tx_params(&Transaction::FrameTransaction(tx)));
    assert!(
        res.is_err(),
        "a blob-carrying frame tx sent without its sidecar must be rejected (got {res:?})"
    );
}

/// A blobless frame transaction keeps the plain payload and the plain variant.
#[test]
fn send_raw_transaction_accepts_a_blobless_frame_transaction() {
    let tx = Transaction::FrameTransaction(FrameTransaction {
        frames: vec![Frame::default()],
        sender: Address::from_low_u64_be(0xABCD),
        ..Default::default()
    });
    let parsed = SendRawTransactionRequest::parse(&raw_tx_params(&tx)).expect("must parse");
    assert!(
        matches!(parsed, SendRawTransactionRequest::Frame(_)),
        "a blobless frame transaction must parse to the plain variant, got {parsed:?}"
    );
}

fn raw_params(raw: &[u8]) -> Option<Vec<Value>> {
    Some(vec![json!(format!("0x{}", hex::encode(raw)))])
}

/// A frame transaction that declares one blob, with a sidecar of the right shape.
/// Nothing here verifies KZG proofs, so the bundle's contents do not matter.
fn blob_frame_tx() -> (FrameTransaction, BlobsBundle) {
    let tx = FrameTransaction {
        frames: vec![Frame::default()],
        sender: Address::from_low_u64_be(0xABCD),
        max_fee_per_blob_gas: U256::from(7u64),
        // 0x01 is the EIP-4844 KZG version byte, which static validation requires.
        blob_versioned_hashes: vec![H256([
            0x01, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11,
            0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11,
            0x11, 0x11, 0x11, 0x11,
        ])],
        ..Default::default()
    };
    let bundle = BlobsBundle {
        blobs: vec![],
        commitments: vec![],
        proofs: vec![],
        version: 1,
    };
    (tx, bundle)
}
