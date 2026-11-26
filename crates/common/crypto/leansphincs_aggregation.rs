//! Aggregation of leanSPHINCS signatures into one proof.
//!
//! A block that aggregates its leanSPHINCS signatures carries one proof in their
//! place: a proof that every `(public key, message)` claim of a list had a valid
//! signature. The proof system is leanVM's recursive aggregation, the Lean Ethereum
//! tooling the verifier of `leansphincs` comes from, at the same revision.
//!
//! Proving builds a tree. Its leaves prove at most
//! [`AGGREGATION_LEAF_SIGNATURES`] signatures each, and a node above them proves
//! at most [`AGGREGATION_FAN_IN`] proofs of the level below, until one proof is
//! left. The prover runs one proof at a time in a process, so the tree is built
//! sequentially.

use lean_vm::lean_sphincs::{LeanSphincsPublicKey, LeanSphincsSignature};
use lean_vm::{AggregateClaims, AggregateProof, MAX_RECURSIONS, OutputClaims, aggregate, setup};
use std::sync::{Mutex, Once};

use crate::leansphincs::{
    LEANSPHINCS_MESSAGE_SIZE, LEANSPHINCS_PUBLIC_KEY_SIZE, LEANSPHINCS_SIGNATURE_SIZE,
};

/// Signatures a leaf of the aggregation tree proves at most.
pub const AGGREGATION_LEAF_SIGNATURES: usize = 512;
/// Proofs a node of the aggregation tree aggregates at most.
pub const AGGREGATION_FAN_IN: usize = MAX_RECURSIONS;
/// Logarithm of the inverse code rate every node of the tree is proven at. A
/// higher value gives a smaller proof and slower proving.
pub const AGGREGATION_LOG_INV_RATE: usize = 1;

/// What a signature states: the public key, then the message it signed.
pub type LeanSphincsClaim = (
    [u8; LEANSPHINCS_PUBLIC_KEY_SIZE],
    [u8; LEANSPHINCS_MESSAGE_SIZE],
);

/// Why [`leansphincs_aggregate`] returned no proof.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LeanSphincsAggregationError {
    #[error("there is no signature to aggregate")]
    NoSignatures,
    #[error("a signature is not {LEANSPHINCS_SIGNATURE_SIZE} bytes long")]
    InvalidSignatureLength,
    #[error("the prover returned no proof: {0}")]
    Proving(String),
}

/// The prover allows one proof at a time in a process.
static PROVER: Mutex<()> = Mutex::new(());
static SETUP: Once = Once::new();

/// Builds the aggregation program and starts the prover's threads. Both happen
/// on first use otherwise, so a builder calls this ahead of its first block.
pub fn leansphincs_aggregation_setup() {
    SETUP.call_once(setup);
}

fn sorted_claims(claims: impl Iterator<Item = LeanSphincsClaim>) -> AggregateClaims {
    let mut lean_sphincs: Vec<_> = claims
        .map(|(public_key, message)| (LeanSphincsPublicKey::from_bytes(&public_key), message))
        .collect();
    lean_sphincs.sort();
    lean_sphincs.dedup();
    AggregateClaims {
        lean_xmss: Vec::new(),
        lean_sphincs,
        da_commitment: None,
    }
}

/// Proves that every claim of `signatures` has a valid signature, the one given
/// with it, and returns the serialized proof. The claims may come in any order.
///
/// Every signature must be valid: the prover fails on one that is not.
pub fn leansphincs_aggregate(
    signatures: &[(LeanSphincsClaim, &[u8])],
) -> Result<Vec<u8>, LeanSphincsAggregationError> {
    if signatures.is_empty() {
        return Err(LeanSphincsAggregationError::NoSignatures);
    }
    let mut signers = Vec::with_capacity(signatures.len());
    for ((public_key, message), signature) in signatures {
        let signature = <&[u8; LEANSPHINCS_SIGNATURE_SIZE]>::try_from(*signature)
            .map_err(|_| LeanSphincsAggregationError::InvalidSignatureLength)?;
        signers.push((
            LeanSphincsPublicKey::from_bytes(public_key),
            *message,
            LeanSphincsSignature::from_bytes(signature),
        ));
    }
    leansphincs_aggregation_setup();
    let _one_proof_at_a_time = PROVER
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let proving = |error| LeanSphincsAggregationError::Proving(format!("{error}"));

    let mut level = Vec::with_capacity(signers.len().div_ceil(AGGREGATION_LEAF_SIGNATURES));
    for leaf in signers.chunks(AGGREGATION_LEAF_SIGNATURES) {
        level.push(
            aggregate(
                &[],
                Vec::new(),
                leaf.to_vec(),
                None,
                OutputClaims::All,
                AGGREGATION_LOG_INV_RATE,
            )
            .map_err(proving)?,
        );
    }
    while level.len() > 1 {
        let mut next = Vec::with_capacity(level.len().div_ceil(AGGREGATION_FAN_IN));
        for children in level.chunks(AGGREGATION_FAN_IN) {
            next.push(
                aggregate(
                    children,
                    Vec::new(),
                    Vec::new(),
                    None,
                    OutputClaims::All,
                    AGGREGATION_LOG_INV_RATE,
                )
                .map_err(proving)?,
            );
        }
        level = next;
    }
    level
        .pop()
        .map(|(_, proof)| proof.to_bytes())
        .ok_or(LeanSphincsAggregationError::NoSignatures)
}

/// Returns whether `proof` proves exactly `claims`: that each of them has a
/// valid signature, and nothing about any other claim. The claims may come in
/// any order, and a repeated claim counts once.
///
/// Returns `false`, without failing, when `proof` does not decode.
pub fn leansphincs_aggregate_verify(claims: &[LeanSphincsClaim], proof: &[u8]) -> bool {
    if claims.is_empty() {
        return false;
    }
    let Some(proof) = AggregateProof::from_bytes(proof) else {
        return false;
    };
    proof.verify(&sorted_claims(claims.iter().copied()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use lean_vm::lean_sphincs::{key_gen, sign};

    fn signer(index: u8) -> (LeanSphincsClaim, Vec<u8>) {
        let secret_key = [index; 32];
        let message = [index.wrapping_add(1); 32];
        let public_key = key_gen(&secret_key).to_bytes();
        let signature = sign(&secret_key, &message).to_bytes().to_vec();
        ((public_key, message), signature)
    }

    #[test]
    fn a_proof_holds_for_its_claims_and_for_no_others() {
        let signers: Vec<_> = (0..3).map(signer).collect();
        let signatures: Vec<_> = signers
            .iter()
            .map(|(claim, signature)| (*claim, signature.as_slice()))
            .collect();
        let claims: Vec<_> = signers.iter().map(|(claim, _)| *claim).collect();
        let proof = leansphincs_aggregate(&signatures).expect("valid signatures aggregate");

        assert!(leansphincs_aggregate_verify(&claims, &proof));
        // The order of the claims is not part of the statement.
        let reversed: Vec<_> = claims.iter().rev().copied().collect();
        assert!(leansphincs_aggregate_verify(&reversed, &proof));

        // A claim the proof does not cover.
        let mut substituted = claims.clone();
        substituted[0].1[0] ^= 1;
        assert!(!leansphincs_aggregate_verify(&substituted, &proof));
        // A claim more, and a claim fewer.
        let mut extra = claims.clone();
        extra.push(signer(9).0);
        assert!(!leansphincs_aggregate_verify(&extra, &proof));
        assert!(!leansphincs_aggregate_verify(&claims[..2], &proof));
        assert!(!leansphincs_aggregate_verify(&[], &proof));

        // Bytes that are not the proof.
        let mut tampered = proof.clone();
        let last = tampered.len() - 1;
        tampered[last] ^= 1;
        assert!(!leansphincs_aggregate_verify(&claims, &tampered));
        assert!(!leansphincs_aggregate_verify(
            &claims,
            &proof[..proof.len() / 2]
        ));
        assert!(!leansphincs_aggregate_verify(&claims, &[]));
    }

    #[test]
    fn an_invalid_signature_gives_no_proof() {
        let (claim, mut signature) = signer(0);
        assert_eq!(
            leansphincs_aggregate(&[(claim, &signature[..100])]),
            Err(LeanSphincsAggregationError::InvalidSignatureLength)
        );
        signature[40] ^= 1;
        assert!(matches!(
            leansphincs_aggregate(&[(claim, signature.as_slice())]),
            Err(LeanSphincsAggregationError::Proving(_))
        ));
        assert_eq!(
            leansphincs_aggregate(&[]),
            Err(LeanSphincsAggregationError::NoSignatures)
        );
    }
}
