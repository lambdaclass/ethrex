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
