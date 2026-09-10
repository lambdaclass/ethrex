//! Regression test for the `typed-zero-legacy-transaction` finding: a typed-transaction
//! envelope whose type byte is `0x00` (`0x00 || rlp(legacy-fields)`) is non-canonical per
//! EIP-2718 — type `0x00` is unassigned, and a legacy transaction must be a bare RLP list, not
//! a typed envelope. geth rejects it (`ErrTxTypeNotSupported`). ethrex instead decodes it as a
//! legacy transaction, then `compute_transactions_root` re-encodes it canonically (dropping the
//! `0x00` prefix), so ethrex commits `rlp(legacy)` for bytes that were `0x00 || rlp(legacy)` —
//! a cross-client transactions-root divergence. The decoders must reject the `0x00` type byte.
use ethrex_common::types::{
    Block, BlockBody, BlockHeader, EIP1559Transaction, LegacyTransaction, P2PTransaction,
    Transaction, TxKind,
};
use ethrex_common::{Address, Bytes, U256};
use ethrex_rlp::decode::RLPDecode;
use ethrex_rlp::encode::RLPEncode;
use ethrex_rlp::structs::Encoder;

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

#[test]
fn block_decode_rejects_body_with_typed_zero_transaction() {
    // End-to-end: a block whose body carries a 0x00-typed tx must fail to decode (this is the
    // path P2P block import takes: block RLP -> BlockBody -> Vec<Transaction>). We hand-assemble
    // the block RLP because the encoders can't *produce* the non-canonical tx — a hostile peer
    // would. `encode_raw` injects the pre-built transactions list containing the bad tx.
    let mut txs_list = Vec::new();
    Encoder::new(&mut txs_list)
        .encode_raw(&typed_zero_rlp_item())
        .finish();

    let mut block_rlp = Vec::new();
    Encoder::new(&mut block_rlp)
        .encode_field(&BlockHeader::default())
        .encode_raw(&txs_list) // transactions
        .encode_field(&Vec::<BlockHeader>::new()) // ommers
        .finish();

    let decoded = Block::decode(&block_rlp);
    assert!(
        decoded.is_err(),
        "a block whose body carries a 0x00-typed tx must fail to decode, got: {decoded:?}"
    );
}

#[test]
fn block_decode_still_accepts_a_body_with_a_legacy_transaction() {
    // Control: a well-formed block carrying a genuine legacy tx round-trips through decode.
    let block = Block::new(
        BlockHeader::default(),
        BlockBody {
            transactions: vec![sample_legacy()],
            ommers: Vec::new(),
            withdrawals: None,
        },
    );
    let encoded = block.encode_to_vec();
    let decoded = Block::decode(&encoded).expect("a valid block must still decode");
    assert_eq!(decoded.body.transactions.len(), 1);
    assert!(matches!(
        decoded.body.transactions[0],
        Transaction::LegacyTransaction(_)
    ));
}
