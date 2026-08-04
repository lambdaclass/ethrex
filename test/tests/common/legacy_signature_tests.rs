//! Regression tests for legacy transaction `v` validation.
//!
//! EIP-155: a legacy signature's `v` must be 27/28 (pre-155) or
//! `{35,36} + chain_id * 2`. Any other value is malformed. ethrex used to
//! coerce an out-of-range `v` into a parity bit via `saturating_sub`, which
//! recovered a bogus sender (then failing later with an unrelated error such
//! as "insufficient account funds") instead of rejecting the signature.
use ethrex_common::U256;
use ethrex_common::types::{LegacyTransaction, Transaction};
use ethrex_crypto::{CryptoError, NativeCrypto};
use hex_literal::hex;

/// A real pre-EIP-155 legacy signature (v = 27). Only `v` is varied per case.
fn tx_with_v(v: u64) -> Transaction {
    Transaction::LegacyTransaction(LegacyTransaction {
        nonce: 0,
        gas_price: U256::from(0x0au64),
        gas: 0x05f5e100,
        value: 0.into(),
        v: U256::from(v),
        r: U256::from_big_endian(&hex!(
            "7e09e26678ed4fac08a249ebe8ed680bf9051a5e14ad223e4b2b9d26e0208f37"
        )),
        s: U256::from_big_endian(&hex!(
            "5f6e3f188e3e6eab7d7d3b6568f5eac7d687b08d307d3154ccd8c87b4630509b"
        )),
        ..Default::default()
    })
}

#[test]
fn valid_legacy_v_recovers_a_sender() {
    // 27/28 (pre-155) and {35,36}+chain_id*2 (EIP-155) are all well-formed and
    // must recover successfully.
    for v in [27u64, 28, 35, 36, 37, 38] {
        assert!(
            tx_with_v(v).sender(&NativeCrypto).is_ok(),
            "v={v} is a valid legacy signature and should recover a sender",
        );
    }
}

#[test]
fn out_of_range_legacy_v_is_rejected() {
    // Everything below 35 that isn't 27/28 is a malformed pre-155 `v` and must
    // be rejected as an invalid signature, not coerced into a parity bit.
    for v in [0u64, 1, 26, 29, 30, 34] {
        assert!(
            matches!(
                tx_with_v(v).sender(&NativeCrypto),
                Err(CryptoError::InvalidSignature)
            ),
            "v={v} is not a valid legacy signature and must be rejected",
        );
    }
}
