//! EIP-8288 dependency aggregation: the seam between block validity and a
//! recursive-proof backend.
//!
//! EIP-8288 lets a transaction declare dependencies -- post-quantum signatures and
//! STARKs it needs proven -- without carrying their proofs. Instead the block
//! carries one recursive proof in its header that discharges every dependency
//! declared by every transaction in it. Verifying that proof is what this crate
//! abstracts.
//!
//! # Why a seam rather than a direct dependency
//!
//! The EIP names Lean Ethereum tooling, which lives in a pre-1.0 repository that
//! restructures its crate layout between revisions. Everything leanVM-shaped is
//! therefore confined to the `leanvm` module, behind an off-by-default feature, and
//! nothing outside this crate names a leanVM type.
//!
//! # What the EIP gets wrong here, and what we do instead
//!
//! EIP-8288's block-validity rule 2 says the proof "verifies successfully against
//! `block_deps_hash` and the fixed protocol-level `AGGREGATED_VK`", as though the
//! dependency set were a single public input the proof commits to. The tooling it
//! names does not work that way: a leanVM aggregate *publishes claim lists*, and
//! leanVM's own documentation says a caller "that expects particular pairs has to
//! check the two lists against them". Verification is therefore two steps, not one:
//! check the proof, then check that what it proved is what the block declared.
//!
//! [`DependencyAggregator::verify`] takes both the proof and the expected
//! dependency set for exactly that reason.

use std::fmt::Debug;

use ethereum_types::H256;
use ethrex_common::types::DependencyTriple;

pub mod unavailable;
pub mod wrapper;
pub use unavailable::UnavailableAggregator;
pub use wrapper::{MempoolWrapper, WrapperContent, WrapperEntry, WrapperError};

#[cfg(feature = "leanvm")]
pub mod leanvm;

#[cfg(feature = "leanvm")]
pub mod test_support;

/// Whether this build carries a real aggregation backend.
///
/// Mirrors `ethrex_crypto::NATIVE_P256_BACKEND`: a runtime-observable constant so a
/// test can assert which backend it is exercising and fail loudly rather than
/// silently conclude that a stub "verified" a proof.
pub const LEANVM_AGGREGATOR: bool = cfg!(feature = "leanvm");

/// The largest `recursive_stark` proof a header may carry.
///
/// EIP-8288 sets no bound. It needs one: the proof rides in the block header, the
/// header is inside `Block::encode`, and EIP-7934 caps the encoded block at
/// `MAX_RLP_BLOCK_SIZE` (8 MiB). An unbounded field lets a proof crowd out the
/// transactions it exists to serve, and a malformed length is cheaper to reject
/// here than after allocation. leanVM's aggregates measure about 300 KiB, so this
/// leaves generous room for recursion depth while staying an eighth of the block
/// budget.
pub const MAX_RECURSIVE_STARK_PROOF_BYTES: usize = 1 << 20;

// The header's RLP decode applies the same bound, so an oversized proof is refused
// before it is allocated rather than only when a backend looks at it. The two
// constants cannot be one item -- `ethrex-common` cannot depend on this crate --
// so they are asserted equal instead.
const _: () = assert!(
    MAX_RECURSIVE_STARK_PROOF_BYTES == ethrex_common::types::MAX_RECURSIVE_STARK_PROOF_BYTES
);

/// One dependency together with the secret material that discharges it.
///
/// The triple is what the transaction declared and what the block commits to; the
/// witness is what the aggregator needs to prove it and never appears on chain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DependencyWitness {
    pub triple: DependencyTriple,
    /// Scheme-specific. For `LEANSPHINCS_SCHEME`, the public key and signature
    /// whose hashes `triple` names.
    pub witness: Vec<u8>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AggregateError {
    #[error(
        "no dependency-aggregation backend is compiled in; rebuild with the `leanvm` feature to verify EIP-8288 proofs"
    )]
    NoBackend,
    #[error(
        "dependency scheme {scheme:#04x} has no counterpart in the aggregation backend, so it cannot be proven"
    )]
    SchemeUnsupported { scheme: u8 },
    #[error("recursive stark proof is {len} bytes, over the {max}-byte limit")]
    ProofTooLarge { len: usize, max: usize },
    #[error("recursive stark proof is not a valid encoding")]
    ProofMalformed,
    #[error("recursive stark proof did not verify: {0}")]
    ProofInvalid(String),
    /// The proof verified, but it does not prove what the block declared. This is
    /// the check EIP-8288's rule 2 omits.
    #[error("proof verified but its claims do not match the block's dependencies: {0}")]
    ClaimsMismatch(String),
    #[error("{0} is not implemented by this backend")]
    NotImplemented(&'static str),
}

/// Verifies, and optionally produces, the recursive proof that discharges a
/// block's EIP-8288 dependencies.
pub trait DependencyAggregator: Send + Sync + Debug {
    /// Check `proof` against `expected`.
    ///
    /// Must fail unless **both** hold: the proof itself verifies, and the set of
    /// claims it proves is exactly `expected`. `expected` is already deduplicated
    /// and sorted -- it comes from `BlockBody::dependencies`.
    ///
    /// An implementation that cannot check something must return an error. There is
    /// no "unknown" outcome: a block whose proof cannot be checked is invalid, not
    /// provisionally valid.
    fn verify(&self, proof: &[u8], expected: &[DependencyTriple]) -> Result<(), AggregateError>;

    /// Produce a proof discharging `raw`, absorbing `children` (proofs from a
    /// previous aggregation round or from peers) so the result covers their claims
    /// too. This is the recursion EIP-8288 is built on.
    ///
    /// `declare` is what makes the recursion usable rather than only possible. A
    /// child proof covers the dependencies of whoever built it, and the next round
    /// rarely wants all of them: a transaction was included, or dropped, or expired,
    /// so the set moves. `Some(set)` publishes exactly `set` and discards the rest;
    /// `None` publishes everything the inputs establish.
    ///
    /// Without it a builder absorbing a child covering `{A, B}` could not produce a
    /// proof for a block containing only `A` -- and [`DependencyAggregator::verify`]
    /// would correctly reject the `{A, B}` proof for that block, so the mismatch
    /// surfaces as an unbuildable block rather than as a bad one.
    ///
    /// An implementation must fail rather than silently narrow: every triple in
    /// `declare` has to be established by `raw` or by a child.
    fn aggregate(
        &self,
        raw: &[DependencyWitness],
        children: &[&[u8]],
        declare: Option<&[DependencyTriple]>,
    ) -> Result<Vec<u8>, AggregateError>;

    /// Verify one dependency directly from its own proof material, with no
    /// aggregation.
    ///
    /// This is what EIP-8288's mode-0 wrapper carries, and what "each dependency has
    /// a corresponding proof that can be verified individually" means. The material
    /// differs by scheme, which is why this is one scheme-dispatching entry point
    /// rather than two: §Mempool Wrapper Object says mode 0 holds "one STARK per
    /// leanSTARK dependency" -- not per dependency -- and that a node may "naively
    /// concatenate the leanSPHINCS instead of proving them". So for leanSPHINCS the
    /// material is the raw signature, and for leanSTARK it is that dependency's own
    /// STARK, checked against its own `verification_key_hash`.
    ///
    /// Neither is an aggregate, so neither goes to [`DependencyAggregator::verify`],
    /// whose contract is the protocol aggregation circuit over a whole expected set.
    /// Mode 0 is also what a user's very first broadcast uses, and a user has no
    /// aggregate yet, so requiring a recursive proof here would make the mode
    /// unusable for its stated purpose.
    fn verify_witness(&self, witness: &DependencyWitness) -> Result<(), AggregateError>;

    /// Whether this backend can prove and verify a dependency of `scheme`.
    ///
    /// Admission needs this. A scheme the backend cannot discharge makes every
    /// block carrying it invalid, so admitting such a transaction fills a pool slot
    /// with something that can never be included -- a free griefing vector, since
    /// the transaction is statically valid and costs nothing to construct. The
    /// mempool refuses them instead.
    fn supports_scheme(&self, scheme: u8) -> bool;

    /// The protocol-level verification key this backend verifies against.
    ///
    /// EIP-8288 lists `AGGREGATED_VK` as `TBD`. It need not be: it is the identity
    /// of the aggregation circuit, so a backend can derive it rather than having a
    /// value assigned.
    fn aggregated_vk(&self) -> H256;

    /// A short name for logs and error messages.
    fn name(&self) -> &'static str;
}

/// Reject a proof whose length is implausible before anything tries to decode it.
pub fn check_proof_length(proof: &[u8]) -> Result<(), AggregateError> {
    if proof.len() > MAX_RECURSIVE_STARK_PROOF_BYTES {
        return Err(AggregateError::ProofTooLarge {
            len: proof.len(),
            max: MAX_RECURSIVE_STARK_PROOF_BYTES,
        });
    }
    Ok(())
}

/// The aggregator this build uses.
///
/// Returns the leanVM backend when compiled in, and otherwise one that fails
/// closed. Deliberately not a runtime choice: a node cannot be talked into
/// accepting proofs it has no code to check.
pub fn default_aggregator() -> Box<dyn DependencyAggregator> {
    #[cfg(feature = "leanvm")]
    {
        Box::new(leanvm::LeanVmAggregator::new())
    }
    #[cfg(not(feature = "leanvm"))]
    {
        Box::new(UnavailableAggregator)
    }
}
