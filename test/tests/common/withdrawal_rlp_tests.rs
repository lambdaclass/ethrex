//! Regression test for the `withdrawal-byte-normalization-root` finding: RLP decoding
//! must reject a non-canonical scalar encoding of a withdrawal's `amount`.
//!
//! RLP encodes a single byte in `0x00..=0x7f` as itself, so the scalar `1` is canonically
//! `0x01`. The non-canonical form `0x81 0x01` (a 1-byte *string* holding `0x01`) decodes
//! to the same value but is not minimal. ethrex's lenient decoder accepts it and
//! re-encodes it canonically, so `validate_block_body` then matches it against a canonical
//! `withdrawals_root` and the block is accepted — while geth rejects the raw bytes at
//! decode (`ErrCanonSize`). Two clients disagreeing on the same bytes is a consensus split.
use ethrex_common::Address;
use ethrex_common::types::Withdrawal;
use ethrex_rlp::decode::RLPDecode;
use ethrex_rlp::structs::Encoder;

/// Canonical RLP of a withdrawal with the given `amount`.
fn canonical_withdrawal_rlp(amount: u64) -> Vec<u8> {
    let mut buf = Vec::new();
    Encoder::new(&mut buf)
        .encode_field(&0u64) // index
        .encode_field(&0u64) // validator_index
        .encode_field(&Address::zero()) // address
        .encode_field(&amount) // amount
        .finish();
    buf
}

#[test]
fn rlp_accepts_canonical_withdrawal_amount() {
    // Control: the canonical encoding of amount = 1 decodes fine.
    let decoded =
        Withdrawal::decode(&canonical_withdrawal_rlp(1)).expect("canonical withdrawal must decode");
    assert_eq!(decoded.amount, 1);
}

#[test]
fn rlp_rejects_noncanonical_withdrawal_amount() {
    // Hand-build the withdrawal list with `amount` as the non-canonical 1-byte string
    // `0x81 0x01` instead of the canonical `0x01`. List payload:
    //   index            -> 0x80   (u64 0)
    //   validator_index  -> 0x80   (u64 0)
    //   address          -> 0x94 ‖ 20 zero bytes
    //   amount           -> 0x81 0x01   (NON-canonical encoding of 1)
    // payload len = 1 + 1 + 21 + 2 = 25 (0x19) -> list header 0xc0 + 25 = 0xd9.
    let mut rlp = vec![0xd9, 0x80, 0x80, 0x94];
    rlp.extend_from_slice(&[0u8; 20]);
    rlp.extend_from_slice(&[0x81, 0x01]);

    assert!(
        Withdrawal::decode(&rlp).is_err(),
        "a withdrawal whose amount uses a non-canonical RLP scalar encoding (0x8101) \
         must be rejected, matching geth's ErrCanonSize"
    );
}

#[test]
fn rlp_rejects_noncanonical_withdrawal_amount_zero() {
    // The same rule for 0: canonical is the empty string `0x80`; the 1-byte string
    // `0x81 0x00` is a non-minimal encoding of 0 and must be rejected.
    let mut rlp = vec![0xd9, 0x80, 0x80, 0x94];
    rlp.extend_from_slice(&[0u8; 20]);
    rlp.extend_from_slice(&[0x81, 0x00]);

    assert!(
        Withdrawal::decode(&rlp).is_err(),
        "a withdrawal whose amount uses a non-canonical encoding of zero (0x8100) must be rejected"
    );
}
