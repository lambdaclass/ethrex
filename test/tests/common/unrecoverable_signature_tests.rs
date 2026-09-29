//! A signature whose components are each in range but from which no public key
//! can be recovered (EEST `test_unrecoverable_signature`): `r = 5` is the
//! smallest value that is not the x-coordinate of any secp256k1 point, so there
//! is no ephemeral point R and no sender, with no range rule violated.
//!
//! The spec rejects it as an invalid signature, the same failure as an
//! out-of-range `r` or `s` (EELS raises `InvalidSignatureError` in both cases),
//! and the rejection message must say so.
use ethrex_common::U256;
use ethrex_common::types::{
    EIP1559Transaction, EIP2930Transaction, LegacyTransaction, Transaction, TxKind,
};
use ethrex_crypto::{CryptoError, NativeCrypto};

const UNRECOVERABLE_R: u64 = 5;

fn unrecoverable_txs() -> [(&'static str, Transaction); 3] {
    let to = TxKind::Call(ethrex_common::Address::from_low_u64_be(0xdead_beee));
    let r = U256::from(UNRECOVERABLE_R);
    let s = U256::one();
    [
        (
            "legacy",
            Transaction::LegacyTransaction(LegacyTransaction {
                gas: 21_000,
                to: to.clone(),
                value: U256::one(),
                v: U256::from(27),
                r,
                s,
                ..Default::default()
            }),
        ),
        (
            "eip2930",
            Transaction::EIP2930Transaction(EIP2930Transaction {
                chain_id: 1,
                gas_limit: 21_000,
                to: to.clone(),
                value: U256::one(),
                signature_y_parity: false,
                signature_r: r,
                signature_s: s,
                ..Default::default()
            }),
        ),
        (
            "eip1559",
            Transaction::EIP1559Transaction(EIP1559Transaction {
                chain_id: 1,
                gas_limit: 21_000,
                to,
                value: U256::one(),
                signature_y_parity: false,
                signature_r: r,
                signature_s: s,
                ..Default::default()
            }),
        ),
    ]
}

#[test]
fn unrecoverable_signature_recovers_no_sender() {
    for (ty, tx) in unrecoverable_txs() {
        assert!(
            matches!(tx.sender(&NativeCrypto), Err(CryptoError::RecoveryFailed)),
            "{ty}: r = {UNRECOVERABLE_R} must fail public-key recovery",
        );
    }
}

#[test]
fn unrecoverable_signature_is_reported_as_an_invalid_signature() {
    for (ty, tx) in unrecoverable_txs() {
        let err = tx
            .sender(&NativeCrypto)
            .expect_err("an unrecoverable signature has no sender");
        assert!(
            err.to_string().starts_with("invalid signature"),
            "{ty}: the rejection must name the invalid-signature category, got {err:?} ({err})",
        );
    }
}
