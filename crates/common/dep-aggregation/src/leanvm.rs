//! The leanVM-backed aggregator.
//!
//! Built on the `rec_aggregation` and `lean_sphincs` crates of lean-bindings
//! `8d54d3165d`, which sit on leanVM `f7ba4999` (the F128 field). That is the
//! stack the restarted Daisugi network pins, so a proof verifies here exactly when
//! it verifies there. Everything leanVM-shaped lives in this file, so a rename
//! upstream costs one file rather than a sweep.
//!
//! # What a block proof proves
//!
//! The proof's public input is the block's dependency digest under
//! `AGGREGATED_VK`, not a list of claims: the verifier holds only the digest and
//! the proof bytes. So checking a block is computing the digest of what the block
//! declares and asking whether the proof attests to that digest. A proof for any
//! other set fails, because its digest differs.
//!
//! # Mapping EIP-8288 dependencies onto the crate's claims
//!
//! A dependency is `(scheme, data_hash, key)`. For leanSPHINCS (`0x10`) the key is
//! the raw 32-byte public key, `pkSeed || pkRoot`; for leanSTARK (`0x11`) it is the
//! program's verification key. The crate keeps one sorted, deduplicated list per
//! scheme, and the digest is the leanSPHINCS list's alone when there is no
//! leanSTARK dependency, else both lists' digests joined under a domain string.

use ethereum_types::H256;
use ethrex_common::types::{
    DEPENDENCY_SCHEME_LEANSPHINCS, DEPENDENCY_SCHEME_LEANSTARK, DependencyTriple,
    deduplicate_and_sort_dependencies,
};
use lean_sphincs::{LeanSphincsPublicKey, LeanSphincsSignature};
use rec_aggregation::{
    AggregateVerifyError, EthereumClaims, GenericClaim, LeanSphincsClaim, RawGeneric,
};
use tracing::debug;

use crate::{AggregateError, DependencyAggregator, DependencyWitness, check_proof_length};

/// The lean-bindings revision this backend is built against.
pub const LEAN_BINDINGS_REVISION: &str = "8d54d3165d453f340420cf39d8da174e497c50bc";

/// The network's `AGGREGATED_VK`: the Fiat-Shamir seed of the compiled aggregation
/// guest. It is computed from the guest, not stored in the crate, so any change to
/// the guest or to the compiler that builds it changes this value. A build that
/// computes another key verifies a different statement, and must refuse rather
/// than disagree with the network.
pub const AGGREGATED_VK: [u8; 32] = [
    0xe0, 0x9e, 0x5a, 0xcf, 0xcf, 0xe8, 0x78, 0x32, 0xf3, 0x37, 0xeb, 0x42, 0xac, 0xac, 0x98, 0x24,
    0x6d, 0x16, 0x0b, 0xb9, 0x15, 0xb5, 0x5c, 0x55, 0xfb, 0xd8, 0x11, 0x8b, 0x20, 0x3c, 0xbb, 0x81,
];

/// leanVM's rate parameter. 1 is what its own aggregation benchmarks use.
const DEFAULT_LOG_INV_RATE: usize = 1;

/// Length of a leanSPHINCS public key on the wire and in a dependency.
const LEANSPHINCS_KEY_LEN: usize = lean_sphincs::PUB_KEY_SIZE;

/// Refuse to judge any proof if the compiled guest is not the network's.
fn check_backend() -> Result<(), AggregateError> {
    if rec_aggregation::aggregated_vk() != AGGREGATED_VK {
        return Err(AggregateError::BackendMismatch(
            "the compiled aggregation guest's key differs from the network's AGGREGATED_VK".into(),
        ));
    }
    Ok(())
}

/// Split `dependencies` into the crate's two claim lists, each sorted and
/// deduplicated as the digest requires.
fn claims_of(dependencies: &[DependencyTriple]) -> Result<EthereumClaims, AggregateError> {
    let mut lean_sphincs: Vec<LeanSphincsClaim> = Vec::new();
    let mut generic: Vec<GenericClaim> = Vec::new();
    for dependency in dependencies {
        match dependency.scheme {
            DEPENDENCY_SCHEME_LEANSPHINCS => lean_sphincs.push((
                dependency.data_hash.0,
                LeanSphincsPublicKey::from_bytes(&dependency.verification_key_hash.0),
            )),
            DEPENDENCY_SCHEME_LEANSTARK => {
                generic.push((dependency.data_hash.0, dependency.verification_key_hash.0))
            }
            scheme => return Err(AggregateError::SchemeUnsupported { scheme }),
        }
    }
    lean_sphincs.sort();
    lean_sphincs.dedup();
    generic.sort();
    generic.dedup();
    Ok(EthereumClaims {
        generic,
        lean_sphincs,
        da_commitment: None,
    })
}

/// The network's dependency digest of `dependencies`: Keccak-256 over the
/// leanSPHINCS entries, joined with the leanSTARK entries' digest under the
/// crate's domain string when there are any.
pub fn deps_hash(dependencies: &[DependencyTriple]) -> Result<H256, AggregateError> {
    let claims = claims_of(dependencies)?;
    Ok(H256(rec_aggregation::deps_hash(
        &claims.lean_sphincs,
        &claims.generic,
    )))
}

fn classify_verify_error(error: AggregateVerifyError) -> AggregateError {
    match error {
        AggregateVerifyError::MalformedEncoding => AggregateError::ProofMalformed,
        other => AggregateError::ProofInvalid(format!("{other:?}")),
    }
}

#[derive(Debug, Default)]
pub struct LeanVmAggregator;

impl LeanVmAggregator {
    /// Build the aggregator, paying the guest's one-off compilation up front.
    ///
    /// The crate compiles the aggregation guest on first use and caches it. Without
    /// a warm-up the first block to carry a proof pays for that inside block
    /// validation.
    pub fn new() -> Self {
        debug!("warming the leanVM aggregation guest");
        rec_aggregation::warm_up();
        Self
    }
}

impl DependencyAggregator for LeanVmAggregator {
    fn verify(&self, proof: &[u8], expected: &[DependencyTriple]) -> Result<(), AggregateError> {
        check_proof_length(proof)?;
        check_backend()?;
        let digest = deps_hash(expected)?;
        // The crate can panic on adversarial input; that is a rejection, not a crash.
        match std::panic::catch_unwind(|| rec_aggregation::verify_deps_hash(&digest.0, proof)) {
            Ok(result) => result.map_err(classify_verify_error),
            Err(_) => Err(AggregateError::ProofInvalid(
                "the proof verifier panicked".into(),
            )),
        }
    }

    fn aggregate(
        &self,
        raw: &[DependencyWitness],
        children: &[&[u8]],
        declare: Option<&[DependencyTriple]>,
    ) -> Result<Vec<u8>, AggregateError> {
        // On the wire a proof carries no claims; a receiver supplies them. Absorbing
        // a child therefore needs the set it proves, which this interface does not
        // pass.
        if !children.is_empty() {
            return Err(AggregateError::NotImplemented(
                "absorbing child proofs without their dependency sets",
            ));
        }
        check_backend()?;

        let mut raw_lean_sphincs = Vec::new();
        let mut raw_generic: Vec<RawGeneric> = Vec::new();
        for witness in raw {
            match witness.triple.scheme {
                DEPENDENCY_SCHEME_LEANSPHINCS => {
                    raw_lean_sphincs.push(decode_lean_sphincs_witness(witness)?)
                }
                DEPENDENCY_SCHEME_LEANSTARK => raw_generic.push((
                    witness.triple.data_hash.0,
                    witness.triple.verification_key_hash.0,
                    witness.witness.clone(),
                )),
                scheme => return Err(AggregateError::SchemeUnsupported { scheme }),
            }
        }

        // Narrowing must not quietly produce a proof covering less than asked: a
        // builder that got one would publish a block failing its own rule 2.
        let declared = match declare {
            None => None,
            Some(wanted) => {
                let wanted = deduplicate_and_sort_dependencies(wanted.to_vec());
                let established = deduplicate_and_sort_dependencies(
                    raw.iter().map(|witness| witness.triple).collect(),
                );
                let missing = wanted
                    .iter()
                    .filter(|triple| established.binary_search(triple).is_err())
                    .count();
                if missing > 0 {
                    return Err(AggregateError::ClaimsMismatch(format!(
                        "asked to declare {} dependencies but the witnesses establish only {} of them",
                        wanted.len(),
                        wanted.len() - missing
                    )));
                }
                Some(claims_of(&wanted)?)
            }
        };
        let proof = rec_aggregation::aggregate(
            &[],
            raw_generic,
            raw_lean_sphincs,
            &[],
            declared.as_ref(),
            DEFAULT_LOG_INV_RATE,
        )
        .map_err(|e| AggregateError::ProofInvalid(format!("{e:?}")))?;

        let bytes = proof.proof_bytes();
        check_proof_length(&bytes)?;
        Ok(bytes)
    }

    fn verify_witness(&self, witness: &DependencyWitness) -> Result<(), AggregateError> {
        match witness.triple.scheme {
            DEPENDENCY_SCHEME_LEANSPHINCS => {
                let (key, message, signature) = decode_lean_sphincs_witness(witness)?;
                lean_sphincs::verify(&key, &message, &signature)
                    .map_err(|e| AggregateError::ProofInvalid(format!("{e:?}")))
            }
            DEPENDENCY_SCHEME_LEANSTARK => {
                let accepted = std::panic::catch_unwind(|| {
                    rec_aggregation::verify_generic(
                        &witness.triple.data_hash.0,
                        &witness.triple.verification_key_hash.0,
                        &witness.witness,
                    )
                })
                .unwrap_or(false);
                if accepted {
                    Ok(())
                } else {
                    Err(AggregateError::ProofInvalid(
                        "the leanSTARK witness does not prove its data hash under its key".into(),
                    ))
                }
            }
            scheme => Err(AggregateError::SchemeUnsupported { scheme }),
        }
    }

    fn supports_scheme(&self, scheme: u8) -> bool {
        scheme == DEPENDENCY_SCHEME_LEANSPHINCS || scheme == DEPENDENCY_SCHEME_LEANSTARK
    }

    fn aggregated_vk(&self) -> H256 {
        H256(rec_aggregation::aggregated_vk())
    }

    fn name(&self) -> &'static str {
        "leanvm"
    }
}

/// A leanSPHINCS witness: the 32-byte public key, then the 6,176-byte signature.
/// The key must be the dependency's own, or the witness would discharge a claim
/// the transaction never declared.
fn decode_lean_sphincs_witness(
    witness: &DependencyWitness,
) -> Result<
    (
        LeanSphincsPublicKey,
        lean_sphincs::Message,
        LeanSphincsSignature,
    ),
    AggregateError,
> {
    if witness.witness.len() != LEANSPHINCS_KEY_LEN + lean_sphincs::SIG_SIZE {
        return Err(AggregateError::ProofMalformed);
    }
    let (key_bytes, signature_bytes) = witness.witness.split_at(LEANSPHINCS_KEY_LEN);
    if key_bytes != witness.triple.verification_key_hash.as_bytes() {
        return Err(AggregateError::ClaimsMismatch(
            "witness public key is not the dependency's key".into(),
        ));
    }
    let key_bytes: &[u8; LEANSPHINCS_KEY_LEN] = key_bytes
        .try_into()
        .map_err(|_| AggregateError::ProofMalformed)?;
    let signature_bytes: &[u8; lean_sphincs::SIG_SIZE] = signature_bytes
        .try_into()
        .map_err(|_| AggregateError::ProofMalformed)?;
    Ok((
        LeanSphincsPublicKey::from_bytes(key_bytes),
        witness.triple.data_hash.0,
        LeanSphincsSignature::from_bytes(signature_bytes),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_compiled_guest_has_the_network_key() {
        assert_eq!(rec_aggregation::aggregated_vk(), AGGREGATED_VK);
    }

    #[test]
    fn an_empty_set_needs_the_empty_proof() {
        let aggregator = LeanVmAggregator;
        assert!(aggregator.verify(&[], &[]).is_ok());
        assert_eq!(deps_hash(&[]).unwrap(), ethrex_common::utils::keccak([]));
    }
}
