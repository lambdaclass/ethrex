//! Regression tests for `eip7702-auth-y-parity-bound` (Option B).
//!
//! EIP-7702 bounds an authorization tuple's `y_parity` to `< 2**8`. We enforce that
//! with a range check at RLP decode while keeping the `U256` representation, so the
//! JSON-RPC and rkyv wire formats are unchanged. These tests pin the two surfaces a
//! type change would have silently broken (RLP and JSON), plus the canonical-RLP
//! behavior `U256` already provides.
use ethrex_common::types::AuthorizationTuple;
use ethrex_common::{Address, U256};
use ethrex_rlp::decode::RLPDecode;
use ethrex_rlp::structs::Encoder;

/// Canonical RLP of an authorization tuple with the given `y_parity`.
fn tuple_rlp(y_parity: U256) -> Vec<u8> {
    let mut buf = Vec::new();
    Encoder::new(&mut buf)
        .encode_field(&U256::zero()) // chain_id
        .encode_field(&Address::zero()) // address
        .encode_field(&0u64) // nonce
        .encode_field(&y_parity) // y_parity
        .encode_field(&U256::one()) // r
        .encode_field(&U256::one()) // s
        .finish();
    buf
}

#[test]
fn rlp_rejects_y_parity_at_or_above_256() {
    // geth models y_parity as a u8 and rejects >= 2**8 at decode. ethrex must too,
    // otherwise an L1 block carrying such a type-4 tx is accepted here but rejected
    // by other clients (consensus split).
    let rlp = tuple_rlp(U256::from(256u64));
    assert!(
        AuthorizationTuple::decode(&rlp).is_err(),
        "y_parity >= 2**8 must be rejected at RLP decode"
    );
}

#[test]
fn rlp_accepts_in_range_y_parity() {
    for v in [0u64, 1, 255] {
        let decoded = AuthorizationTuple::decode(&tuple_rlp(U256::from(v)))
            .unwrap_or_else(|e| panic!("y_parity={v} should decode, got {e:?}"));
        assert_eq!(decoded.y_parity, U256::from(v));
    }
}

#[test]
fn rlp_rejects_noncanonical_y_parity() {
    // y_parity encoded as the 1-byte string [0x00] (RLP `0x81 0x00`) is a non-minimal
    // encoding of 0 — the canonical encoding is the empty string (`0x80`). The U256
    // decoder rejects leading-zero scalars, matching geth.
    let mut rlp = vec![0xdb, 0x80, 0x94];
    rlp.extend_from_slice(&[0u8; 20]); // address
    rlp.extend_from_slice(&[0x80, 0x81, 0x00, 0x01, 0x01]); // nonce, y_parity(non-canon), r, s
    assert!(
        AuthorizationTuple::decode(&rlp).is_err(),
        "non-canonical y_parity encoding (0x8100) must be rejected"
    );
}

#[test]
fn json_deserializes_y_parity_as_hex_quantity() {
    // The `ethrex_sendTransaction` path deserializes `AuthorizationTuple` directly from
    // JSON, where y_parity is a hex quantity (e.g. "0x1"). Keeping the field `U256`
    // preserves this; a bare `u8` field would reject the hex string.
    let json = r#"{
        "chainId": "0x1",
        "address": "0x000000000000000000000000000000000000aaaa",
        "nonce": "0x0",
        "yParity": "0x1",
        "r": "0x1",
        "s": "0x1"
    }"#;
    let tuple: AuthorizationTuple =
        serde_json::from_str(json).expect("hex y_parity must deserialize");
    assert_eq!(tuple.y_parity, U256::one());
}
