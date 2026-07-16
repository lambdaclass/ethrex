//! Regression test for the `typed-zero-legacy-transaction` finding: a typed-transaction
//! envelope whose type byte is `0x00` (`0x00 || rlp(legacy-fields)`) is non-canonical per
//! EIP-2718 — type `0x00` is unassigned, and a legacy transaction must be a bare RLP list, not
//! a typed envelope. geth rejects it (`ErrTxTypeNotSupported`). ethrex instead decodes it as a
//! legacy transaction, then `compute_transactions_root` re-encodes it canonically (dropping the
//! `0x00` prefix), so ethrex commits `rlp(legacy)` for bytes that were `0x00 || rlp(legacy)` —
//! a cross-client transactions-root divergence. The decoders must reject the `0x00` type byte.
use ethrex_common::types::{
    EIP1559Transaction, LegacyTransaction, P2PTransaction, Transaction, TxKind,
};
use ethrex_common::{Address, Bytes, U256};
use ethrex_rlp::decode::RLPDecode;
use ethrex_rlp::encode::RLPEncode;

fn sample_legacy() -> Transaction {
    Transaction::LegacyTransaction(LegacyTransaction {
        nonce: 0,
        gas_price: U256::from(1),
        gas: 21_000,
        to: TxKind::Call(Address::from_low_u64_be(0x1234)),
        value: U256::one(),
        data: Bytes::new(),
        v: U256::from(27),
        r: U256::one(),
        s: U256::one(),
        ..Default::default()
    })
}

/// `0x00 || rlp(legacy)`: a legacy body smuggled into a typed envelope with type byte 0x00.
fn typed_zero_canonical() -> Vec<u8> {
    let mut malicious = vec![0x00u8];
    malicious.extend_from_slice(&sample_legacy().encode_canonical_to_vec());
    malicious
}

/// The same non-canonical bytes wrapped as an RLP byte-string, i.e. how a tx appears inside a
/// block body / P2P list (what `decode_unfinished` sees).
fn typed_zero_rlp_item() -> Vec<u8> {
    let mut buf = Vec::new();
    typed_zero_canonical().encode(&mut buf);
    buf
}

#[test]
fn transaction_decode_canonical_rejects_typed_zero() {
    let decoded = Transaction::decode_canonical(&typed_zero_canonical());
    assert!(
        decoded.is_err(),
        "a 0x00-prefixed typed envelope must be rejected (EIP-2718), got: {decoded:?}"
    );
}

#[test]
fn transaction_decode_rejects_typed_zero() {
    let decoded = Transaction::decode(&typed_zero_rlp_item());
    assert!(decoded.is_err(), "got: {decoded:?}");
}

#[test]
fn p2p_transaction_decode_rejects_typed_zero() {
    let decoded = P2PTransaction::decode(&typed_zero_rlp_item());
    assert!(decoded.is_err(), "got: {decoded:?}");
}

#[test]
fn decode_canonical_still_accepts_a_genuine_legacy_transaction() {
    // A real legacy tx is a bare RLP list (first byte >= 0xc0), not a 0x00-typed envelope.
    let encoded = sample_legacy().encode_canonical_to_vec();
    let decoded = Transaction::decode_canonical(&encoded).expect("legacy must still decode");
    assert!(matches!(decoded, Transaction::LegacyTransaction(_)));
}

#[test]
fn decode_canonical_still_accepts_a_valid_typed_transaction() {
    // A well-formed typed tx (0x02 EIP-1559) must still decode after the change.
    let tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        chain_id: 1,
        nonce: 0,
        max_priority_fee_per_gas: 0,
        max_fee_per_gas: 1,
        gas_limit: 21_000,
        to: TxKind::Call(Address::from_low_u64_be(0x1234)),
        value: U256::one(),
        data: Bytes::new(),
        ..Default::default()
    });
    let encoded = tx.encode_canonical_to_vec();
    assert_eq!(encoded.first(), Some(&0x02u8));
    let decoded = Transaction::decode_canonical(&encoded).expect("valid typed tx must decode");
    assert!(matches!(decoded, Transaction::EIP1559Transaction(_)));
}
