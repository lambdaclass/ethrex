//! Tests for `Crypto::verify_signature`, the EIP-8025 sender-hint primitive.
//!
//! These exercise the native (`secp256k1`) backend, which is what the default
//! build and the SP1 guest compile. The `k256` backend variant of
//! `verify_signature` (which compares `R'.x` to `r` as field bytes to reject
//! the rare nonce-aliasing case `x = r + n`) is only compiled with
//! `--no-default-features` on `ethrex-crypto`, so it is not reachable from
//! this crate's secp256k1 build and is not covered here.

use ethereum_types::U256;
use ethrex_crypto::{Crypto, NativeCrypto};
use secp256k1::{Message as SecpMessage, PublicKey, SECP256K1, SecretKey};

const SECP256K1_N: [u8; 32] =
    hex_literal::hex!("fffffffffffffffffffffffffffffffebaaedce6af48a03bbfd25e8cd0364141");

/// Sign `msg` with `sk`, returning the 65-byte `r||s||v` signature and the
/// 65-byte uncompressed (0x04) public key.
fn sign(sk: &SecretKey, msg: &[u8; 32]) -> ([u8; 65], [u8; 65]) {
    let message = SecpMessage::from_digest(*msg);
    let (recovery_id, compact) = SECP256K1
        .sign_ecdsa_recoverable(&message, sk)
        .serialize_compact();

    let mut sig = [0u8; 65];
    sig[..64].copy_from_slice(&compact);
    sig[64] = Into::<i32>::into(recovery_id) as u8;

    let pk = PublicKey::from_secret_key(SECP256K1, sk).serialize_uncompressed();
    (sig, pk)
}

fn test_key() -> SecretKey {
    SecretKey::from_slice(&[7u8; 32]).unwrap()
}

#[test]
fn verify_signature_accepts_valid() {
    let msg = [0x11u8; 32];
    let (sig, pk) = sign(&test_key(), &msg);
    assert!(NativeCrypto.verify_signature(&sig, &msg, &pk));
}

/// Flipping the recovery id must make verification fail. This is the property
/// that forces the implementation to recover-and-compare rather than use a
/// plain ECDSA verify (which would accept either parity candidate).
#[test]
fn verify_signature_binds_recovery_id() {
    let msg = [0x11u8; 32];
    let (mut sig, pk) = sign(&test_key(), &msg);
    assert!(NativeCrypto.verify_signature(&sig, &msg, &pk));

    sig[64] ^= 1; // swap parity 0 <-> 1
    assert!(!NativeCrypto.verify_signature(&sig, &msg, &pk));
}

#[test]
fn verify_signature_rejects_wrong_public_key() {
    let msg = [0x11u8; 32];
    let (sig, _) = sign(&test_key(), &msg);

    let other = SecretKey::from_slice(&[9u8; 32]).unwrap();
    let other_pk = PublicKey::from_secret_key(SECP256K1, &other).serialize_uncompressed();
    assert!(!NativeCrypto.verify_signature(&sig, &msg, &other_pk));
}

/// EIP-2: signatures with `s > n/2` must be rejected.
#[test]
fn verify_signature_rejects_high_s() {
    let msg = [0x11u8; 32];
    let (mut sig, pk) = sign(&test_key(), &msg);

    // sign_ecdsa_recoverable normalizes to low-s; n - s is the high-s counterpart.
    let n = U256::from_big_endian(&SECP256K1_N);
    let s = U256::from_big_endian(&sig[32..64]);
    sig[32..64].copy_from_slice(&(n - s).to_big_endian());
    assert!(!NativeCrypto.verify_signature(&sig, &msg, &pk));
}

/// The recovery byte must be 0 or 1 (canonical y-parity).
#[test]
fn verify_signature_rejects_out_of_range_recovery_byte() {
    let msg = [0x11u8; 32];
    let (mut sig, pk) = sign(&test_key(), &msg);
    sig[64] = 2;
    assert!(!NativeCrypto.verify_signature(&sig, &msg, &pk));
}

/// The native secp256k1 backend parses hybrid (0x06/0x07) SEC1 keys, so a hybrid
/// hint passes `verify_signature`. This is exactly why the EIP-8025 caller
/// `Transaction::compute_sender_with_hint` must reject non-0x04 prefixes
/// explicitly — without that guard the native backend (lenient) and the k256
/// backend (which rejects hybrid) would diverge on the same input.
#[test]
fn verify_signature_accepts_hybrid_encoded_key() {
    let msg = [0x11u8; 32];
    let (sig, mut pk) = sign(&test_key(), &msg);
    assert!(NativeCrypto.verify_signature(&sig, &msg, &pk));

    // Re-tag as hybrid matching Y's parity; the underlying C parser accepts it.
    pk[0] = if pk[64] & 1 == 1 { 0x07 } else { 0x06 };
    assert!(NativeCrypto.verify_signature(&sig, &msg, &pk));
}
