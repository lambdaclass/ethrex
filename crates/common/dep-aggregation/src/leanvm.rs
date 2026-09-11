//! The leanVM-backed aggregator.
//!
//! Pinned to leanVM `7f9777da6a`. Everything leanVM-shaped lives in this file, so a
//! rename upstream costs one file rather than a sweep.
//!
//! # This is a prototype, and the distinction matters
//!
//! The cryptography is real: signatures are signed and verified, aggregates are
//! produced and checked, and a mutated proof or a dropped dependency fails. What it
//! is **not** is the circuit EIP-8288 specifies. The EIP asks for a proof verifying
//! "against `block_deps_hash` and the fixed protocol-level `AGGREGATED_VK`" -- one
//! digest as a public input, under a key the EIP lists as `TBD`. This verifies
//! leanVM's claim-list statement and compares the recovered triples on the host
//! instead, because that is the shape the tooling actually has.
//!
//! So a proof produced here discharges the dependencies it names, and another
//! client's proof of the same dependencies would not be interchangeable with it.
//! Cross-client compatibility is not established and cannot be until the EIP names
//! the circuit. Nothing here should be read as evidence that it has been.
//!
//! # Mapping EIP-8288's triples onto leanVM's claims
//!
//! EIP-8288 describes a dependency as `(scheme, data_hash, verification_key_hash)`.
//! leanVM describes one as `SphincsClaim = (SphincsPublicKey, sphincs::Message)`.
//! Lining those up turned up two things the EIP does not say:
//!
//! - **`data_hash` maps cleanly.** `sphincs::Message` is `[u8; 32]`, the same width
//!   as `data_hash`, and is the message the signature is over.
//! - **`verification_key_hash` does not.** leanVM's claim carries the *key*, not a
//!   hash of it -- and `SphincsPublicKey::flatten()` is already exactly 32 bytes
//!   (a 16-byte root and a 16-byte public parameter). So for leanSPHINCS the EIP's
//!   hash indirection compresses nothing, while introducing a hash function that
//!   consensus depends on and that the EIP never names. Two clients that pick
//!   differently will disagree about whether a proof covers a block's dependencies,
//!   and both will pass their own tests.
//!
//! We hash with BLAKE3-256, matching `block_deps_hash`.

use ethereum_types::H256;
use ethrex_common::types::{
    DEPENDENCY_SCHEME_LEANSPHINCS, DependencyTriple, deduplicate_and_sort_dependencies,
};
use rec_aggregation::{ClaimSelection, EthereumProof, SignatureClaims, SphincsClaim};
use tracing::debug;

use crate::{AggregateError, DependencyAggregator, DependencyWitness, check_proof_length};

/// The leanVM revision this backend is built against. Part of `aggregated_vk`
/// because leanVM does not expose a digest of its own circuit; see
/// [`LeanVmAggregator::aggregated_vk`].
pub const LEANVM_REVISION: &str = "7f9777da6ab3d7bb8d10c3f5c7edce2554fcfc03";

/// EIP-8288's `verification_key_hash` for a leanSPHINCS public key.
///
/// BLAKE3-256 over the 32-byte flattened key, chosen to match `block_deps_hash`
/// rather than because the EIP says so -- it does not say anything.
pub fn leansphincs_verification_key_hash(key: &sphincs::SphincsPublicKey) -> H256 {
    H256::from_slice(blake3::hash(&key.flatten()).as_bytes())
}

/// Narrow the claims an aggregate will publish down to exactly `wanted`.
///
/// The candidates are everything the inputs establish: the witnesses proved
/// directly, plus every claim each child proof already carries. A triple in
/// `wanted` that no candidate establishes is an error, not a silent omission --
/// otherwise a builder would get a proof that quietly covers less than the block it
/// is building declares, and the block would fail its own rule 2 on import.
fn select_claims(
    wanted: &[DependencyTriple],
    raw: &[(
        sphincs::SphincsPublicKey,
        sphincs::Message,
        sphincs::SphincsSignature,
    )],
    children: &[EthereumProof],
) -> Result<SignatureClaims, AggregateError> {
    let wanted = deduplicate_and_sort_dependencies(wanted.to_vec());
    for triple in &wanted {
        if !triple.is_leansphincs() {
            return Err(AggregateError::SchemeUnsupported {
                scheme: triple.scheme,
            });
        }
    }

    let mut candidates: Vec<SphincsClaim> = raw
        .iter()
        .map(|(key, message, _)| (*key, *message))
        .chain(
            children
                .iter()
                .flat_map(|child| child.sphincs_signers().iter().copied()),
        )
        .collect();
    candidates.sort();
    candidates.dedup();

    let selected: Vec<SphincsClaim> = candidates
        .into_iter()
        .filter(|claim| wanted.binary_search(&triple_of(claim)).is_ok())
        .collect();

    if selected.len() != wanted.len() {
        return Err(AggregateError::ClaimsMismatch(format!(
            "asked to declare {} dependencies but the witnesses and children \
             establish only {} of them",
            wanted.len(),
            selected.len()
        )));
    }

    Ok(SignatureClaims {
        xmss: Vec::new(),
        sphincs: selected,
    })
}

/// Turn one leanVM claim into the EIP-8288 triple that names it.
fn triple_of(claim: &SphincsClaim) -> DependencyTriple {
    let (key, message) = claim;
    DependencyTriple {
        scheme: DEPENDENCY_SCHEME_LEANSPHINCS,
        data_hash: H256::from_slice(message),
        verification_key_hash: leansphincs_verification_key_hash(key),
    }
}

#[derive(Debug, Default)]
pub struct LeanVmAggregator;

impl LeanVmAggregator {
    /// Build the aggregator, paying leanVM's one-off circuit build up front.
    ///
    /// `EthereumProof::verify` builds the aggregation guest on first use and caches
    /// it in a `OnceLock`. leanVM's published ~4 ms verification figure is the
    /// steady state *after* that; without a warm-up the first block to carry a
    /// proof pays it instead, inside block validation.
    pub fn new() -> Self {
        debug!("warming the leanVM aggregation circuit");
        rec_aggregation::warm_up();
        Self
    }
}

impl DependencyAggregator for LeanVmAggregator {
    fn verify(&self, proof: &[u8], expected: &[DependencyTriple]) -> Result<(), AggregateError> {
        check_proof_length(proof)?;

        // Finding: leanVM has XMSS, SPHINCS and data-availability claims, and no
        // way to express "this arbitrary STARK verified against this vk". A
        // leanSTARK dependency is therefore unprovable with the tooling EIP-8288
        // names, and saying so is more useful than pretending otherwise.
        if let Some(unsupported) = expected.iter().find(|t| !t.is_leansphincs()) {
            return Err(AggregateError::SchemeUnsupported {
                scheme: unsupported.scheme,
            });
        }

        let aggregate =
            EthereumProof::from_bytes(proof).map_err(|_| AggregateError::ProofMalformed)?;

        aggregate
            .verify()
            .map_err(|e| AggregateError::ProofInvalid(format!("{e:?}")))?;

        // The second half of the check, which EIP-8288's rule 2 omits entirely. A
        // verified aggregate says "these keys signed these messages" -- with the
        // keys and messages chosen by whoever built it. leanVM's own docs are
        // explicit that a caller expecting particular pairs has to compare them.
        // Without this, any valid aggregate over any keys satisfies any block.
        let proven = deduplicate_and_sort_dependencies(
            aggregate.sphincs_signers().iter().map(triple_of).collect(),
        );

        if !aggregate.xmss_signers().is_empty() || !aggregate.da_commitments().is_empty() {
            return Err(AggregateError::ClaimsMismatch(format!(
                "aggregate carries {} XMSS groups and {} DA roots, which no EIP-8288 \
                 dependency can declare",
                aggregate.xmss_signers().len(),
                aggregate.da_commitments().len()
            )));
        }

        if proven != expected {
            return Err(AggregateError::ClaimsMismatch(format!(
                "proof establishes {} dependencies, block declares {}",
                proven.len(),
                expected.len()
            )));
        }

        Ok(())
    }

    fn aggregate(
        &self,
        raw: &[DependencyWitness],
        children: &[&[u8]],
        declare: Option<&[DependencyTriple]>,
    ) -> Result<Vec<u8>, AggregateError> {
        let mut child_proofs = Vec::with_capacity(children.len());
        for child in children {
            check_proof_length(child)?;
            child_proofs.push(
                EthereumProof::from_bytes(child).map_err(|_| AggregateError::ProofMalformed)?,
            );
        }

        let mut raw_sphincs = Vec::with_capacity(raw.len());
        for witness in raw {
            if !witness.triple.is_leansphincs() {
                return Err(AggregateError::SchemeUnsupported {
                    scheme: witness.triple.scheme,
                });
            }
            raw_sphincs.push(decode_sphincs_witness(witness)?);
        }

        let selection = match declare {
            None => None,
            Some(wanted) => Some(select_claims(wanted, &raw_sphincs, &child_proofs)?),
        };
        let declared = selection.as_ref().map(|signatures| ClaimSelection {
            signatures,
            da_commitments: &[],
        });

        let proof = rec_aggregation::aggregate(
            &child_proofs,
            Vec::new(),
            raw_sphincs,
            &[],
            declared,
            DEFAULT_LOG_INV_RATE,
        )
        .map_err(|e| AggregateError::ProofInvalid(format!("{e:?}")))?;

        let bytes = proof.to_bytes();
        check_proof_length(&bytes)?;
        Ok(bytes)
    }

    fn verify_witness(&self, witness: &DependencyWitness) -> Result<(), AggregateError> {
        // Dispatch on the scheme rather than assume leanSPHINCS: mode 0 carries a
        // leanSTARK dependency's own STARK, and this backend has no way to check one
        // at all. Reading those bytes as a signature would be the wrong question,
        // and would fail as `ProofMalformed` -- a statement about the proof rather
        // than about this backend's reach.
        if !self.supports_scheme(witness.triple.scheme) {
            return Err(AggregateError::SchemeUnsupported {
                scheme: witness.triple.scheme,
            });
        }

        // No circuit and no aggregate: a leanSPHINCS dependency's witness is a
        // public key and a signature, and checking it is an ordinary signature
        // verification. This is the cheap path mode 0 exists for.
        let (key, message, signature) = decode_sphincs_witness(witness)?;
        sphincs::verify(&key, &message, &signature)
            .map_err(|e| AggregateError::ProofInvalid(format!("{e:?}")))
    }

    fn supports_scheme(&self, scheme: u8) -> bool {
        // leanVM aggregates XMSS, SPHINCS and data-availability claims. It has no
        // way to express "this arbitrary STARK verified against this verification
        // key", so leanSTARK has no counterpart here.
        scheme == DEPENDENCY_SCHEME_LEANSPHINCS
    }

    fn aggregated_vk(&self) -> H256 {
        // EIP-8288 lists AGGREGATED_VK as TBD, and the natural value is the
        // identity of the aggregation circuit. leanVM builds that circuit
        // (`unified_guest`) but does not expose a digest of it, so we bind to the
        // revision instead and report the gap as item 15: the EIP can pin a real
        // value as soon as the tooling publishes one, and until then "TBD" is not
        // the only thing missing -- the circuit has no published identity either.
        H256::from_slice(
            blake3::hash(format!("ethrex/eip-8288/leanvm/{LEANVM_REVISION}").as_bytes()).as_bytes(),
        )
    }

    fn name(&self) -> &'static str {
        "leanvm"
    }
}

/// leanVM's rate parameter. 1 is what its own aggregation benchmarks use.
const DEFAULT_LOG_INV_RATE: usize = 1;

/// A leanSPHINCS witness on the wire: the flattened public key, then the
/// signature in leanVM's own serialization.
fn decode_sphincs_witness(
    witness: &DependencyWitness,
) -> Result<
    (
        sphincs::SphincsPublicKey,
        sphincs::Message,
        sphincs::SphincsSignature,
    ),
    AggregateError,
> {
    const KEY_LEN: usize = 32;
    if witness.witness.len() < KEY_LEN {
        return Err(AggregateError::ProofMalformed);
    }
    let (key_bytes, sig_bytes) = witness.witness.split_at(KEY_LEN);

    let mut root = [0u8; sphincs::N];
    let mut public_param = [0u8; sphincs::PUBLIC_PARAM_LEN];
    root.copy_from_slice(&key_bytes[..sphincs::N]);
    public_param.copy_from_slice(&key_bytes[sphincs::N..]);
    let key = sphincs::SphincsPublicKey { root, public_param };

    // The witness must actually discharge the dependency it is filed under, or the
    // aggregate would prove a claim the block never declared.
    if leansphincs_verification_key_hash(&key) != witness.triple.verification_key_hash {
        return Err(AggregateError::ClaimsMismatch(
            "witness public key does not hash to the declared verification_key_hash".into(),
        ));
    }

    let message: sphincs::Message = witness.triple.data_hash.0;

    // A leanSPHINCS signature is a fixed 4,924 bytes. Worth recording because
    // EIP-8288's Motivation puts hash-based signatures at "~2-3 kB", which is off by
    // roughly a factor of two against the scheme its own tooling implements -- so the
    // EIP's bandwidth estimates should not be taken as a bound here.
    let sig_bytes: &[u8; sphincs::SIG_SIZE] = sig_bytes
        .try_into()
        .map_err(|_| AggregateError::ProofMalformed)?;
    let signature = sphincs::SphincsSignature::from_bytes(sig_bytes);

    Ok((key, message, signature))
}
