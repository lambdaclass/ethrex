//! The leanVM-backed aggregator: the real thing EIP-8288 points at.
//!
//! Pinned to leanVM `7f9777da6a`. Everything leanVM-shaped lives in this file, so a
//! rename upstream costs one file rather than a sweep.
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
//! We hash with BLAKE3-256, matching `block_deps_hash`, and raise the gap as item
//! 18 of `NOTES-FOR-8288-AUTHOR.md`.

use ethereum_types::H256;
use ethrex_common::types::{
    DEPENDENCY_SCHEME_LEANSPHINCS, DependencyTriple, deduplicate_and_sort_dependencies,
};
use rec_aggregation::{EthereumProof, SphincsClaim};
use tracing::debug;

use crate::{AggregateError, DependencyAggregator, DependencyWitness, check_proof_length};

/// The leanVM revision this backend is built against. Part of `aggregated_vk`
/// because leanVM does not expose a digest of its own circuit; see
/// [`LeanVmAggregator::aggregated_vk`].
pub const LEANVM_REVISION: &str = "7f9777da6ab3d7bb8d10c3f5c7edce2554fcfc03";

/// EIP-8288's `verification_key_hash` for a leanSPHINCS public key.
///
/// BLAKE3-256 over the 32-byte flattened key, chosen to match `block_deps_hash`
/// rather than because the EIP says so -- it does not say anything (item 17).
pub fn leansphincs_verification_key_hash(key: &sphincs::SphincsPublicKey) -> H256 {
    H256::from_slice(blake3::hash(&key.flatten()).as_bytes())
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

        let proof = rec_aggregation::aggregate(
            &child_proofs,
            Vec::new(),
            raw_sphincs,
            &[],
            None,
            DEFAULT_LOG_INV_RATE,
        )
        .map_err(|e| AggregateError::ProofInvalid(format!("{e:?}")))?;

        let bytes = proof.to_bytes();
        check_proof_length(&bytes)?;
        Ok(bytes)
    }

    fn verify_witness(&self, witness: &DependencyWitness) -> Result<(), AggregateError> {
        // No circuit and no aggregate: a leanSPHINCS dependency's witness is a
        // public key and a signature, and checking it is an ordinary signature
        // verification. This is the cheap path mode 0 exists for.
        let (key, message, signature) = decode_sphincs_witness(witness)?;
        sphincs::verify(&key, &message, &signature)
            .map_err(|e| AggregateError::ProofInvalid(format!("{e:?}")))
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
    // EIP-8288's Motivation puts hash-based signatures at "~2-3 kB", which is off
    // by roughly a factor of two against the scheme its own tooling implements;
    // raised as item 18.
    let sig_bytes: &[u8; sphincs::SIG_SIZE] = sig_bytes
        .try_into()
        .map_err(|_| AggregateError::ProofMalformed)?;
    let signature = sphincs::SphincsSignature::from_bytes(sig_bytes);

    Ok((key, message, signature))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DependencyWitness;
    use ethrex_common::types::DEPENDENCY_SCHEME_LEANSTARK;

    /// One leanSPHINCS dependency, with the witness that discharges it.
    ///
    /// Deterministic from `seed` so a failure is reproducible; leanVM exposes
    /// `key_gen_from_seed` for exactly that.
    fn dependency(seed: u8, message: [u8; 32]) -> (DependencyTriple, DependencyWitness) {
        let (sk, pk) = sphincs::key_gen_from_seed([seed; 32]);
        // Seeded rather than from the OS: signing samples randomness, and a test
        // that fails intermittently on a proving system is not worth having.
        let mut rng = <rand::rngs::StdRng as rand::SeedableRng>::seed_from_u64(seed as u64);
        let signature =
            sphincs::sign(&mut rng, &sk, &message).expect("signing with a fresh key must succeed");

        let triple = DependencyTriple {
            scheme: DEPENDENCY_SCHEME_LEANSPHINCS,
            data_hash: H256(message),
            verification_key_hash: leansphincs_verification_key_hash(&pk),
        };
        let mut witness = pk.flatten().to_vec();
        witness.extend_from_slice(&signature.to_bytes());
        (triple, DependencyWitness { triple, witness })
    }

    /// The premise of this whole PoC: a real recursive aggregate, produced and then
    /// verified against the dependency set a block would declare.
    ///
    /// Ignored by default because proving peaks around 9-11 GiB of resident memory
    /// and takes about a second. Run it deliberately:
    /// `cargo test -p ethrex-dep-aggregation --features leanvm -- --ignored`
    #[test]
    #[ignore = "leanVM proving peaks at 9-11 GiB and takes ~1s"]
    fn a_real_aggregate_verifies_against_the_dependencies_it_proves() {
        let agg = LeanVmAggregator::new();

        let (t1, w1) = dependency(1, [0x11; 32]);
        let (t2, w2) = dependency(2, [0x22; 32]);
        let expected = ethrex_common::types::deduplicate_and_sort_dependencies(vec![t1, t2]);

        let proof = agg
            .aggregate(&[w1, w2], &[])
            .expect("aggregating two leanSPHINCS dependencies must succeed");

        agg.verify(&proof, &expected)
            .expect("the aggregate must discharge exactly the dependencies it proved");

        // The check EIP-8288's rule 2 omits. Without it any valid aggregate would
        // satisfy any block, since the proof does not bind the block's digest.
        let (other, _) = dependency(3, [0x33; 32]);
        assert!(
            agg.verify(&proof, &[other]).is_err(),
            "a proof must not satisfy a dependency set it does not cover"
        );
        assert!(
            agg.verify(&proof, &expected[..1]).is_err(),
            "nor a strict subset of what it proved"
        );
    }

    /// leanSTARK has no counterpart in leanVM, and the backend says so by name
    /// rather than failing with something that reads like a corrupt proof.
    #[test]
    fn a_leanstark_dependency_is_reported_as_unsupported() {
        let triple = DependencyTriple {
            scheme: DEPENDENCY_SCHEME_LEANSTARK,
            data_hash: H256::zero(),
            verification_key_hash: H256::zero(),
        };
        // The scheme check runs before any proving or verifying, so this needs no
        // warm-up and no circuit.
        let agg = LeanVmAggregator;
        assert_eq!(
            agg.verify(&[], &[triple]),
            Err(AggregateError::SchemeUnsupported {
                scheme: DEPENDENCY_SCHEME_LEANSTARK
            })
        );
    }

    #[test]
    /// Both block-validity rules against one block carrying a real aggregate.
    ///
    /// Rule 1 is the header's digest against the block's own transactions; rule 2 is
    /// the proof against that same set. Together they are what makes a dependency
    /// binding: rule 1 alone lets a block declare anything and prove nothing, rule 2
    /// alone lets the header disagree with the body.
    ///
    /// Ignored for the same memory reason as the round trip above.
    #[test]
    #[ignore = "leanVM proving peaks at 9-11 GiB and takes ~1s"]
    fn a_block_carrying_a_real_aggregate_satisfies_both_rules() {
        use ethrex_common::U256;
        use ethrex_common::types::{
            BlockBody, DEPENDENCY_SCHEME_LEANSPHINCS as SPHINCS, Frame, FrameMode,
            FrameTransaction, LEANSPHINCS_VERIFICATION_GAS, RecursiveStark, Transaction,
        };

        let agg = LeanVmAggregator::new();
        let (t1, w1) = dependency(10, [0xA1; 32]);
        let (t2, w2) = dependency(11, [0xB2; 32]);

        // A transaction declaring both dependencies in one frame.
        let mut data = t1.encode().to_vec();
        data.extend_from_slice(&t2.encode());
        let tx = FrameTransaction {
            frames: vec![Frame {
                mode: FrameMode::DepVerify as u8,
                flags: 0,
                target: None,
                gas_limit: 2 * LEANSPHINCS_VERIFICATION_GAS,
                state_gas_limit: 0,
                value: U256::zero(),
                data: data.into(),
            }],
            ..Default::default()
        };
        let body = BlockBody {
            transactions: vec![Transaction::FrameTransaction(tx)],
            ommers: Vec::new(),
            withdrawals: None,
        };

        let declared = body.dependencies();
        assert_eq!(declared.len(), 2, "the frame declares both dependencies");

        let proof = agg
            .aggregate(&[w1, w2], &[])
            .expect("the builder aggregates what the block declares");

        let header_entry = RecursiveStark {
            proof: proof.into(),
            block_deps_hash: body.block_deps_hash(),
        };

        // Rule 1.
        assert_eq!(header_entry.block_deps_hash, body.block_deps_hash());
        // Rule 2.
        agg.verify(&header_entry.proof, &declared)
            .expect("the aggregate discharges the block's dependencies");

        // A header that declares a different digest fails rule 1 while the proof
        // still verifies -- which is why rule 1 is not redundant.
        let tampered = RecursiveStark {
            proof: header_entry.proof.clone(),
            block_deps_hash: H256::from_low_u64_be(0xBAD),
        };
        assert_ne!(tampered.block_deps_hash, body.block_deps_hash());
        assert!(agg.verify(&tampered.proof, &declared).is_ok());
    }

    #[test]
    fn a_witness_must_match_the_dependency_it_is_filed_under() {
        let (_, w) = dependency(4, [0x44; 32]);
        let mut tampered = w.clone();
        tampered.triple.verification_key_hash = H256::from_low_u64_be(0xBAD);
        assert!(
            matches!(
                decode_sphincs_witness(&tampered),
                Err(AggregateError::ClaimsMismatch(_))
            ),
            "a witness whose key does not hash to the declared value must be refused, \
             or an aggregate could prove a claim the block never declared"
        );
    }
}
