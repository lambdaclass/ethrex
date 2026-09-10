//! Accelerated-provider vs host parity.
//!
//! The zkVM crypto providers reimplement EVM precompiles on top of zkVM intrinsics. If
//! one of them disagrees with the host implementation the guest computes a different
//! post-state than the node did, and the failure is silent: a valid-looking proof of the
//! wrong state transition.
//!
//! Nothing else covers this. The deleted `platform_parity.rs` built `NativeCrypto` on
//! *both* sides, so it compared native to native; `host_fixtures.rs` is also NativeCrypto
//! and is skipped unless `ETHREX_STATELESS_FIXTURES` is set; and the tag-time
//! `build-stateless-validator-guest` conformance step executes one fixture, exercising
//! only keccak256 and sha256. bn254, BLS12-381 and modexp had no equivalence check on any
//! path, which is exactly where the unreduced-field-element findings live.
//!
//! Runs on the host: the openvm crates build for non-RISC-V targets using their portable
//! fallbacks, so this compares the two code paths rather than two hardware backends. It
//! therefore catches interface and encoding divergence, not intrinsic-vs-fallback
//! divergence, which needs a real guest execution.
#![cfg(all(feature = "host", feature = "openvm"))]

use std::sync::Arc;

use ethrex_crypto::{Crypto, NativeCrypto};

/// BN254 generator, and 2G, as the 64-byte uncompressed encoding the precompiles use.
const BN_G: [u8; 64] = {
    let mut out = [0u8; 64];
    out[31] = 1;
    out[63] = 2;
    out
};

/// The accelerated provider, obtained through the same selector the guest uses, so this
/// also covers the feature-to-provider wiring rather than only the implementation.
fn providers() -> (NativeCrypto, Arc<dyn Crypto>) {
    (NativeCrypto, ethrex_stateless_validator::crypto::crypto())
}

/// Compare `Result<T, _>` shape and value, treating both errors as equivalent: the
/// providers legitimately word their errors differently, but must agree on accept/reject.
fn assert_agrees<T: PartialEq + core::fmt::Debug, E, E2>(
    what: &str,
    native: Result<T, E>,
    accel: Result<T, E2>,
) {
    match (native, accel) {
        (Ok(a), Ok(b)) => assert_eq!(a, b, "{what}: providers returned different values"),
        (Err(_), Err(_)) => {}
        (Ok(a), Err(_)) => panic!("{what}: native accepted (=> {a:?}) but openvm rejected"),
        (Err(_), Ok(b)) => panic!("{what}: native rejected but openvm accepted (=> {b:?})"),
    }
}

#[test]
fn keccak256_agrees() {
    let (native, accel) = providers();
    for input in [b"".as_slice(), b"abc".as_slice(), &[0xff; 200]] {
        assert_eq!(
            native.keccak256(input),
            accel.keccak256(input),
            "keccak256 diverged on a {}-byte input",
            input.len()
        );
    }
}

#[test]
fn sha256_agrees() {
    let (native, accel) = providers();
    for input in [b"".as_slice(), b"abc".as_slice(), &[0x5a; 200]] {
        assert_eq!(native.sha256(input), accel.sha256(input));
    }
}

#[test]
fn bn254_g1_add_agrees() {
    let (native, accel) = providers();
    // G+G, G+0, 0+0, and a point that is not on the curve.
    let zero = [0u8; 64];
    let mut bad = [0u8; 64];
    bad[31] = 1;
    bad[63] = 1; // (1, 1) is not on y^2 = x^3 + 3

    for (name, a, b) in [
        ("G+G", BN_G, BN_G),
        ("G+0", BN_G, zero),
        ("0+0", zero, zero),
        ("G+offcurve", BN_G, bad),
    ] {
        assert_agrees(
            name,
            native.bn254_g1_add(&a, &b),
            accel.bn254_g1_add(&a, &b),
        );
    }
}

#[test]
fn bn254_g1_mul_agrees() {
    let (native, accel) = providers();
    let mut scalar = [0u8; 32];

    for k in [0u8, 1, 2, 7, 255] {
        scalar[31] = k;
        assert_agrees(
            &format!("G*{k}"),
            native.bn254_g1_mul(&BN_G, &scalar),
            accel.bn254_g1_mul(&BN_G, &scalar),
        );
    }

    // A scalar at/over the group order must be handled identically.
    let big = [0xffu8; 32];
    assert_agrees(
        "G*0xff..ff",
        native.bn254_g1_mul(&BN_G, &big),
        accel.bn254_g1_mul(&BN_G, &big),
    );
}

#[test]
fn modexp_agrees() {
    let (native, accel) = providers();

    // The BN254-Fr modulus takes the accelerated path in the openvm provider; the
    // others fall through to the generic one. Both must match the host.
    let bn254_fr: [u8; 32] = [
        0x30, 0x64, 0x4e, 0x72, 0xe1, 0x31, 0xa0, 0x29, 0xb8, 0x50, 0x45, 0xb6, 0x81, 0x81, 0x58,
        0x5d, 0x28, 0x33, 0xe8, 0x48, 0x79, 0xb9, 0x70, 0x91, 0x43, 0xe1, 0xf5, 0x93, 0xf0, 0x00,
        0x00, 0x01,
    ];

    /// (label, base, exponent, modulus)
    type ModexpCase<'a> = (&'a str, &'a [u8], &'a [u8], &'a [u8]);

    let cases: &[ModexpCase] = &[
        ("3^2 mod 8", &[3], &[2], &[8]),
        ("0^0 mod 7", &[0], &[0], &[7]),
        ("5^0 mod 7", &[5], &[0], &[7]),
        ("2^256 mod bn254fr", &[2], &[1, 0], &bn254_fr),
        ("big base mod bn254fr", &[0xff; 32], &[0x03], &bn254_fr),
        // Exercises the padding branch: base shorter than the field width.
        ("short base mod bn254fr", &[7], &[5], &bn254_fr),
    ];

    for (name, base, exp, modulus) in cases {
        assert_agrees(
            name,
            native.modexp(base, exp, modulus),
            accel.modexp(base, exp, modulus),
        );
    }
}

#[test]
fn bls12_381_g1_add_agrees() {
    let (native, accel) = providers();
    // The 48-byte-per-coordinate encoding; all-zero is the identity.
    let zero = ([0u8; 48], [0u8; 48]);
    assert_agrees(
        "bls g1 0+0",
        native.bls12_381_g1_add(zero, zero),
        accel.bls12_381_g1_add(zero, zero),
    );
}

#[test]
fn bls12_381_fp_to_g1_agrees() {
    let (native, accel) = providers();
    for seed in [0u8, 1, 0x5a] {
        let fp = [seed; 48];
        assert_agrees(
            &format!("fp_to_g1({seed:#x})"),
            native.bls12_381_fp_to_g1(&fp),
            accel.bls12_381_fp_to_g1(&fp),
        );
    }
}

#[test]
fn secp256r1_verify_agrees() {
    let (native, accel) = providers();
    let msg = [0x11u8; 32];
    let sig = [0x22u8; 64];
    let pk = [0x33u8; 64];
    assert_eq!(
        native.secp256r1_verify(&msg, &sig, &pk),
        accel.secp256r1_verify(&msg, &sig, &pk),
        "secp256r1_verify disagreed on a malformed input"
    );
}

#[test]
fn secp256k1_ecrecover_agrees() {
    let (native, accel) = providers();
    let msg = [0x01u8; 32];
    // Malformed signature: both providers must reject, and reject identically.
    let sig = [0u8; 64];
    for recid in [0u8, 1, 2, 3] {
        assert_agrees(
            &format!("ecrecover(zero sig, recid={recid})"),
            native.secp256k1_ecrecover(&sig, recid, &msg),
            accel.secp256k1_ecrecover(&sig, recid, &msg),
        );
    }
}
