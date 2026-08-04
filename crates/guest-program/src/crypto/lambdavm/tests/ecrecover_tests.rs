//! Known-answer tests for the full `ecsm_ecrecover` path (r/s parse,
//! decompress + parity, z-reduction, u1/u2, final keccak(X‖Y) address).
//!
//! On host, `ecsm_lincomb2` returns `None`, so these exercise the recovery
//! wiring through the pure-Rust `ProjectivePoint::lincomb` fallback.

use super::super::*;

/// Build a valid ECDSA/secp256k1 signature from (d, kk, msg) using only the
/// k256 primitives already imported and return `(sig, recid, expected_addr)`.
///
/// `expected_addr` = keccak(X‖Y) of the uncompressed public key, exactly as
/// `ecsm_ecrecover` computes it.
fn make_ecdsa_fixture(d: Scalar, kk: Scalar, msg: [u8; 32]) -> ([u8; 64], u8, [u8; 32]) {
    assert!(!bool::from(d.is_zero()), "private key must be nonzero");
    assert!(!bool::from(kk.is_zero()), "nonce must be nonzero");

    // Public key Q = d·G.
    let q = (ProjectivePoint::GENERATOR * d).to_affine();
    let q_uncompressed = q.to_encoded_point(false);
    let expected = keccak_hash(&q_uncompressed.as_bytes()[1..65]);

    // R = kk·G; r = reduce(Rx); assert r ≠ 0.
    let r_point = (ProjectivePoint::GENERATOR * kk).to_affine();
    let (rx, ry) = affine_xy(&r_point).expect("R is not identity");
    let r = <Scalar as Reduce<U256>>::reduce_bytes(&rx.to_bytes());
    assert!(!bool::from(r.is_zero()), "r must be nonzero");
    // rx is in Fp; since n < p, rx >= n with probability ~2^{-128}. When that
    // happens r = rx-n and the signature requires the high-x recovery bit
    // (recid >= 2, meaning R.x = r+n) which ecsm_ecrecover does not handle.
    // Assert no reduction occurred so the low-x path is valid.
    assert_eq!(
        r.to_bytes(),
        rx.to_bytes(),
        "rx >= n: this kk needs high-x recovery (recid >= 2) — pick a different nonce"
    );

    // recid parity: low bit of Ry (big-endian, byte 31).
    let recid = ry.normalize().to_bytes()[31] & 1;

    // z = reduce(msg).
    let z = <Scalar as Reduce<U256>>::reduce_bytes(&FieldBytes::from(msg));

    // s = kk⁻¹ · (z + r·d).
    let s = kk.invert_vartime().expect("kk is nonzero") * (z + r * d);
    assert!(!bool::from(s.is_zero()), "s must be nonzero");

    // sig = r (BE, 32 bytes) ‖ s (BE, 32 bytes).
    let mut sig = [0u8; 64];
    sig[..32].copy_from_slice(&r.to_bytes());
    sig[32..].copy_from_slice(&s.to_bytes());

    (sig, recid, expected)
}

#[test]
fn ecrecover_known_answer_three_tuples() {
    // Three distinct (d, kk, msg) tuples — deterministic, no RNG.
    let tuples: &[(u64, u64, [u8; 32])] = &[
        (0x0000_0000_0000_0001u64, 0x0000_0000_dead_beefu64, {
            let mut m = [0u8; 32];
            m[31] = 0x42;
            m
        }),
        (0x00c0_ffee_dead_beef_u64, 0x0123_4567_89ab_cdef_u64, {
            let mut m = [0u8; 32];
            m[0] = 0xff;
            m[31] = 0x01;
            m
        }),
        (0x0bad_f00d_1337_cafe, 0xfeed_face_0000_0001, {
            let mut m = [0u8; 32];
            for (i, b) in m.iter_mut().enumerate() {
                *b = i as u8;
            }
            m
        }),
    ];

    for &(d_u64, kk_u64, msg) in tuples {
        let d = Scalar::from(d_u64);
        let kk = Scalar::from(kk_u64);
        let (sig, recid, expected) = make_ecdsa_fixture(d, kk, msg);
        let crypto = LambdaVmCrypto;
        match crypto.secp256k1_ecrecover(&sig, recid, &msg) {
            Ok(got) => assert_eq!(
                got, expected,
                "ecrecover returned wrong address for d={d_u64:#x} kk={kk_u64:#x}"
            ),
            Err(e) => panic!("ecrecover failed for d={d_u64:#x} kk={kk_u64:#x}: {e:?}"),
        }
    }
}

#[test]
fn ecrecover_rejects_zero_s() {
    // sig = valid r ‖ 0x00..00 (s = 0) must return InvalidSignature.
    let mut sig = [0u8; 64];
    // r = 1 (nonzero, but s = 0 in the second half).
    sig[31] = 0x01;
    let msg = [0u8; 32];
    let crypto = LambdaVmCrypto;
    assert!(
        matches!(
            crypto.secp256k1_ecrecover(&sig, 0, &msg),
            Err(CryptoError::InvalidSignature)
        ),
        "expected InvalidSignature for zero s"
    );
}

#[test]
fn ecrecover_rejects_zero_r() {
    // sig = 0x00..00 ‖ valid s must return InvalidSignature.
    let mut sig = [0u8; 64];
    sig[63] = 0x01; // s = 1, r = 0
    let msg = [0u8; 32];
    let crypto = LambdaVmCrypto;
    assert!(
        matches!(
            crypto.secp256k1_ecrecover(&sig, 0, &msg),
            Err(CryptoError::InvalidSignature)
        ),
        "expected InvalidSignature for zero r"
    );
}
