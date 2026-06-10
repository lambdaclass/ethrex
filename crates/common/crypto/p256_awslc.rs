//! aws-lc-rs-backed P-256 (secp256r1) signature verification used by
//! [`crate::NativeCrypto`].
//!
//! The portable default on the [`crate::Crypto`] trait uses RustCrypto `p256`,
//! whose verify path performs two full constant-time scalar multiplications
//! with no Shamir/Strauss trick and no precomputed basepoint table. Every input
//! to the P256VERIFY precompile (message hash, signature, public key) is public
//! on-chain data, so constant-time execution buys nothing; AWS-LC's
//! assembly-backed P-256 (p256-nistz) is roughly 5x faster. We route the native
//! (non-zkVM) path through it here, leaving the pure-Rust trait default in place
//! for zkVM guests.
//!
//! Wire format follows EIP-7951: `msg` is the 32-byte prehashed message, `sig`
//! is the fixed 64-byte `r || s`, and `pk` is the uncompressed public key
//! `x || y` (64 bytes, no leading tag).

use aws_lc_rs::digest::{Digest, SHA256};
use aws_lc_rs::signature::{ECDSA_P256_SHA256_FIXED, UnparsedPublicKey};

/// The secp256r1 group order `n`, big-endian.
const P256_N: [u8; 32] =
    hex_literal::hex!("ffffffff00000000ffffffffffffffffbce6faada7179e84f3b9cac2fc632551");

/// Verify a P-256 ECDSA signature over a 32-byte prehashed message.
///
/// Returns `true` only for a valid signature. Matches the P256VERIFY
/// (EIP-7951) reject conditions: `r`/`s` outside `(0, n)`, public key not on
/// the curve, point at infinity, or a failed verification all return `false`.
/// High-`s` signatures are accepted (EIP-7951 imposes no malleability rule).
pub fn secp256r1_verify(msg: &[u8; 32], sig: &[u8; 64], pk: &[u8; 64]) -> bool {
    // EIP-7951: reject r, s == 0 or >= n up front. Cheap and keeps the scalar
    // range semantics locally auditable rather than implicit in the backend.
    if !scalar_in_range(&sig[..32]) || !scalar_in_range(&sig[32..]) {
        return false;
    }

    // SEC1 uncompressed public key: 0x04 || x || y. AWS-LC rejects coordinates
    // >= p and off-curve points during parsing; (0, 0) is off-curve and the
    // point at infinity is not encodable in this fixed 65-byte form.
    let mut sec1 = [0u8; 65];
    sec1[0] = 0x04;
    sec1[1..].copy_from_slice(pk);

    // The message is already hashed; import it as a SHA-256-sized digest and
    // verify directly rather than re-hashing.
    let Ok(digest) = Digest::import_less_safe(msg, &SHA256) else {
        return false;
    };
    let key = UnparsedPublicKey::new(&ECDSA_P256_SHA256_FIXED, &sec1[..]);
    key.verify_digest(&digest, sig).is_ok()
}

/// True iff `bytes` (32-byte big-endian) encodes a scalar strictly in `(0, n)`.
/// For equal-length big-endian slices, lexicographic order matches numeric
/// order, so the slice comparison against `n` is an exact `< n` test.
fn scalar_in_range(bytes: &[u8]) -> bool {
    if bytes.iter().all(|&b| b == 0) {
        return false;
    }
    bytes < &P256_N[..]
}

#[cfg(test)]
mod tests {
    use super::secp256r1_verify as awslc_verify;
    use crate::provider::Crypto;
    use p256::ecdsa::{SigningKey, signature::hazmat::PrehashSigner};

    /// Pure-Rust reference verifier: a `Crypto` impl with no overrides, so
    /// `secp256r1_verify` resolves to the portable `p256` trait default. This
    /// is the backend aws-lc-rs must agree with byte-for-byte.
    #[derive(Debug)]
    struct PureRust;
    impl Crypto for PureRust {}

    fn ref_verify(msg: &[u8; 32], sig: &[u8; 64], pk: &[u8; 64]) -> bool {
        PureRust.secp256r1_verify(msg, sig, pk)
    }

    /// Deterministic pseudo-random byte stream (no RNG dependency in tests).
    fn pseudo(seed: u64, n: usize) -> Vec<u8> {
        let mut s = seed;
        (0..n)
            .map(|_| {
                s = s
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                (s >> 33) as u8
            })
            .collect()
    }

    fn pk_bytes(key: &SigningKey) -> [u8; 64] {
        let ep = key.verifying_key().to_encoded_point(false);
        let mut pk = [0u8; 64];
        pk.copy_from_slice(&ep.as_bytes()[1..65]);
        pk
    }

    /// Sign `msg` with a key derived deterministically from `seed`.
    fn sample(seed: u64, msg: &[u8; 32]) -> ([u8; 64], [u8; 64]) {
        // Derive a valid signing scalar; retry on the negligible-probability
        // reject (>= n or zero).
        let mut k = seed;
        let key = loop {
            let bytes = pseudo(k, 32);
            if let Ok(key) = SigningKey::from_slice(&bytes) {
                break key;
            }
            k = k.wrapping_add(1);
        };
        let sig: p256::ecdsa::Signature = key.sign_prehash(msg).expect("sign");
        let mut sig_bytes = [0u8; 64];
        sig_bytes.copy_from_slice(sig.to_bytes().as_slice());
        (sig_bytes, pk_bytes(&key))
    }

    /// Valid signatures must verify under both backends.
    #[test]
    fn valid_signatures_agree() {
        for i in 0..64u64 {
            let msg: [u8; 32] = pseudo(0xA11CE ^ i, 32).try_into().unwrap();
            let (sig, pk) = sample(0xB0B ^ i, &msg);
            assert!(
                awslc_verify(&msg, &sig, &pk),
                "aws-lc rejected valid sig #{i}"
            );
            assert!(ref_verify(&msg, &sig, &pk), "p256 rejected valid sig #{i}");
        }
    }

    /// Tampering with the message, signature, or key must be rejected by both,
    /// and the two backends must agree on every mutation.
    #[test]
    fn tampered_inputs_agree() {
        for i in 0..32u64 {
            let mut msg: [u8; 32] = pseudo(0xDEAD ^ i, 32).try_into().unwrap();
            let (mut sig, mut pk) = sample(0xBEEF ^ i, &msg);

            // Baseline valid.
            assert_eq!(awslc_verify(&msg, &sig, &pk), ref_verify(&msg, &sig, &pk));

            msg[0] ^= 0x01;
            assert!(!awslc_verify(&msg, &sig, &pk));
            assert_eq!(awslc_verify(&msg, &sig, &pk), ref_verify(&msg, &sig, &pk));
            msg[0] ^= 0x01;

            sig[0] ^= 0x01;
            assert!(!awslc_verify(&msg, &sig, &pk));
            assert_eq!(awslc_verify(&msg, &sig, &pk), ref_verify(&msg, &sig, &pk));
            sig[0] ^= 0x01;

            pk[0] ^= 0x01;
            assert_eq!(awslc_verify(&msg, &sig, &pk), ref_verify(&msg, &sig, &pk));
        }
    }

    /// Structural edge cases (scalars out of range, degenerate keys). The exact
    /// reject reason differs across backends, but the boolean verdict must match.
    #[test]
    fn adversarial_cases_agree() {
        let msg: [u8; 32] = pseudo(0xF00D, 32).try_into().unwrap();
        let (good_sig, good_pk) = sample(0x1234, &msg);

        let n = super::P256_N;
        let mut n_minus_1 = n;
        n_minus_1[31] -= 1;
        let p_minus_1: [u8; 32] = {
            // secp256r1 field prime p, minus 1.
            let mut p = hex_literal::hex!(
                "ffffffff00000001000000000000000000000000ffffffffffffffffffffffff"
            );
            p[31] -= 1;
            p
        };

        let mut cases: Vec<([u8; 64], [u8; 64])> = Vec::new();
        // r = 0
        let mut s = good_sig;
        s[..32].fill(0);
        cases.push((s, good_pk));
        // s = 0
        let mut s = good_sig;
        s[32..].fill(0);
        cases.push((s, good_pk));
        // r = n
        let mut s = good_sig;
        s[..32].copy_from_slice(&n);
        cases.push((s, good_pk));
        // s = n
        let mut s = good_sig;
        s[32..].copy_from_slice(&n);
        cases.push((s, good_pk));
        // r = s = n-1
        let mut s = good_sig;
        s[..32].copy_from_slice(&n_minus_1);
        s[32..].copy_from_slice(&n_minus_1);
        cases.push((s, good_pk));
        // pk = (0, 0): point at infinity / off-curve
        cases.push((good_sig, [0u8; 64]));
        // pk x = p-1 (coordinate at field boundary, off-curve)
        let mut bad_pk = good_pk;
        bad_pk[..32].copy_from_slice(&p_minus_1);
        cases.push((good_sig, bad_pk));

        for (i, (sig, pk)) in cases.iter().enumerate() {
            assert_eq!(
                awslc_verify(&msg, sig, pk),
                ref_verify(&msg, sig, pk),
                "backends disagree on adversarial case #{i}"
            );
        }
    }
}
