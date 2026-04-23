//! 0x66 — blsSignatureVerify
//!
//! BLS12-381 signature verification used by the BSC cross-chain light-client
//! infrastructure.  Supports both single-key verification and aggregated
//! (fast-aggregate) verification.
//!
//! Reference: `core/vm/contracts.go` in the BSC repository, `blsSignatureVerify`.
//!
//! Input layout:
//! | msg (32) | signature (96) | pubkey_0 (48) | … | pubkey_N-1 (48) |
//!
//! At least one public key must be present.
//!
//! Output (matches bsc-geth — `big.Int.Bytes()` of 0 / 1):
//! - `[0x01]` (1 byte) on successful verification.
//! - `[]` (0 bytes, empty) on failed verification.
//! - `Err(ExecutionReverted)` on invalid/malformed input.
//!
//! Gas:
//! - Base: 1 000 gas
//! - Per additional public key: 3 500 gas
//!
//! These constants match `params.BlsSignatureVerifyBaseGas` and
//! `params.BlsSignatureVerifyPerKeyGas` in the BSC reference.
//!
//! # BLS scheme
//!
//! BSC uses BLS signatures following the Ethereum beacon-chain convention
//! (Prysm / herumi / blst): public keys are G1 points (48 bytes compressed),
//! signatures are G2 points (96 bytes compressed).  The verification equation
//! is the standard pairing check.
//!
//! Single-key:    e(pk, H(msg)) == e(G1, sig)
//! Aggregate:     e(agg_pk, H(msg)) == e(G1, sig)   where agg_pk = Σ pk_i
//!
//! The message is hashed to a G2 point using the `BLS_SIG_DST` domain
//! separation tag `"BLS_SIG_BLS12381G2_XMD:SHA-256_SSWU_RO_POP_"` (proof of
//! possession scheme, matching Prysm's default — which is what bsc-geth's
//! blsSignatureVerify precompile calls into).

use super::PrecompileError;
use bls12_381::{G1Affine, G2Affine, G2Projective, Gt, multi_miller_loop};

/// Base gas cost.  Matches `params.BlsSignatureVerifyBaseGas`.
pub const BLS_VERIFY_BASE_GAS: u64 = 1_000;

/// Per-key gas cost.  Matches `params.BlsSignatureVerifyPerKeyGas`.
pub const BLS_VERIFY_PER_KEY_GAS: u64 = 3_500;

const MSG_LENGTH: usize = 32;
const SIGNATURE_LENGTH: usize = 96;
const PUBKEY_LENGTH: usize = 48;
const MSG_AND_SIG_LENGTH: usize = MSG_LENGTH + SIGNATURE_LENGTH;

/// Domain separation tag used by BSC / Prysm for BLS signature verification.
/// Matches prysm's `crypto/bls/blst/signature.go` DST (proof-of-possession
/// scheme), which is what bsc-geth's `blsSignatureVerify` precompile invokes.
const DST: &[u8] = b"BLS_SIG_BLS12381G2_XMD:SHA-256_SSWU_RO_POP_";

/// Compute the required gas for the given input.
pub fn required_gas(input: &[u8]) -> u64 {
    let input_len = input.len() as u64;
    if input_len <= MSG_AND_SIG_LENGTH as u64
        || !(input_len - MSG_AND_SIG_LENGTH as u64).is_multiple_of(PUBKEY_LENGTH as u64)
    {
        return BLS_VERIFY_BASE_GAS;
    }
    let pub_key_count = (input_len - MSG_AND_SIG_LENGTH as u64) / PUBKEY_LENGTH as u64;
    BLS_VERIFY_BASE_GAS + pub_key_count * BLS_VERIFY_PER_KEY_GAS
}

/// Run the blsSignatureVerify precompile.
pub fn run(input: &[u8], gas_limit: u64) -> Result<(u64, Vec<u8>), PrecompileError> {
    let gas_cost = required_gas(input);
    if gas_limit < gas_cost {
        return Err(PrecompileError::NotEnoughGas);
    }

    let input_len = input.len();

    // Validate: must have msg + sig + at least one pubkey, and pubkey section
    // must be a multiple of PUBKEY_LENGTH.
    if input_len <= MSG_AND_SIG_LENGTH
        || !(input_len - MSG_AND_SIG_LENGTH).is_multiple_of(PUBKEY_LENGTH)
    {
        return Err(PrecompileError::ExecutionReverted);
    }

    let msg = &input[..MSG_LENGTH];
    let sig_bytes: &[u8; SIGNATURE_LENGTH] = input[MSG_LENGTH..MSG_AND_SIG_LENGTH]
        .try_into()
        .expect("slice is exactly SIGNATURE_LENGTH bytes");

    let pub_key_count = (input_len - MSG_AND_SIG_LENGTH) / PUBKEY_LENGTH;

    // Parse signature (G2 compressed, 96 bytes).
    let sig_g2 = parse_g2_compressed(sig_bytes).ok_or(PrecompileError::ExecutionReverted)?;

    // Parse and (if multiple keys) aggregate public keys.
    let agg_pubkey: G1Affine = if pub_key_count == 1 {
        let pk_bytes: &[u8; PUBKEY_LENGTH] = input
            [MSG_AND_SIG_LENGTH..MSG_AND_SIG_LENGTH + PUBKEY_LENGTH]
            .try_into()
            .expect("slice is exactly PUBKEY_LENGTH bytes");
        parse_g1_compressed(pk_bytes).ok_or(PrecompileError::ExecutionReverted)?
    } else {
        let mut agg = G1Affine::identity();
        for i in 0..pub_key_count {
            let offset = MSG_AND_SIG_LENGTH + i * PUBKEY_LENGTH;
            let pk_bytes: &[u8; PUBKEY_LENGTH] = input[offset..offset + PUBKEY_LENGTH]
                .try_into()
                .expect("slice is exactly PUBKEY_LENGTH bytes");
            let pk = parse_g1_compressed(pk_bytes).ok_or(PrecompileError::ExecutionReverted)?;
            // Aggregate: agg += pk
            use bls12_381::G1Projective;
            #[allow(clippy::arithmetic_side_effects)]
            let sum = G1Projective::from(agg) + G1Projective::from(pk);
            agg = G1Affine::from(sum);
        }
        agg
    };

    // Hash message to G2 using hash_to_curve.
    let msg_g2 = hash_msg_to_g2(msg);

    // Verify: e(agg_pk, H(msg)) == e(G1_generator, sig)
    //
    // Equivalent check using the pairing:
    //   e(-G1_gen, sig) * e(agg_pk, H(msg)) == Gt::identity()
    //
    // We negate the generator to avoid computing two full pairings separately.
    let g1_neg = G1Affine::from(-bls12_381::G1Projective::generator());
    let gt = multi_miller_loop(&[
        (&g1_neg, &bls12_381::G2Prepared::from(sig_g2)),
        (&agg_pubkey, &bls12_381::G2Prepared::from(msg_g2)),
    ])
    .final_exponentiation();

    // bsc-geth returns `common.Big{0,1}.Bytes()` directly, i.e. an empty
    // slice on failure and `[0x01]` on success. Match that exactly so
    // callers observing `returndatasize()` see the same value.
    let result = if gt == Gt::identity() {
        vec![0x01]
    } else {
        Vec::new()
    };

    Ok((gas_cost, result))
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Parse a BLS12-381 G1 point from 48-byte compressed form.
/// Returns `None` if the bytes are not a valid, on-curve, torsion-free point.
fn parse_g1_compressed(bytes: &[u8; PUBKEY_LENGTH]) -> Option<G1Affine> {
    let ct_opt = G1Affine::from_compressed(bytes);
    if ct_opt.is_some().into() {
        let point = ct_opt.unwrap();
        // Subgroup check (torsion-free).
        if bool::from(point.is_torsion_free()) {
            Some(point)
        } else {
            None
        }
    } else {
        None
    }
}

/// Parse a BLS12-381 G2 point from 96-byte compressed form.
/// Returns `None` if the bytes are not a valid, on-curve, torsion-free point.
fn parse_g2_compressed(bytes: &[u8; SIGNATURE_LENGTH]) -> Option<G2Affine> {
    let ct_opt = G2Affine::from_compressed(bytes);
    if ct_opt.is_some().into() {
        let point = ct_opt.unwrap();
        if bool::from(point.is_torsion_free()) {
            Some(point)
        } else {
            None
        }
    } else {
        None
    }
}

/// Hash a message to a G2 point using the `BLS_SIG_DST` domain separation tag.
///
/// This uses the `hash_to_curve` algorithm (`XMD:SHA-256, SSWU, RO`) as
/// specified in RFC 9380 and as implemented by the `bls12_381` crate.
fn hash_msg_to_g2(msg: &[u8]) -> G2Affine {
    use bls12_381::hash_to_curve::{ExpandMsgXmd, HashToCurve};
    let pt = <G2Projective as HashToCurve<ExpandMsgXmd<sha2::Sha256>>>::hash_to_curve([msg], DST);
    G2Affine::from(pt)
}
