//! EIP-8288 §Mempool Wrapper Object.
//!
//! A wrapper bundles transactions with either their individual proofs (mode 0) or
//! one recursive aggregate covering all of them (mode 1). Nodes rebroadcast one
//! wrapper per `AGGREGATION_INTERVAL`, folding what they received into a single
//! aggregate, which is what keeps per-peer bandwidth flat as transaction count
//! grows.
//!
//! # What is implemented here, and what cannot be
//!
//! The object and its validity rules are implemented. **How it travels is not**,
//! because EIP-8288 does not say. It specifies the encoding and the checks a
//! receiver performs, and then says wrappers are "broadcast" -- no devp2p message,
//! no capability version, no announcement scheme, and no way to resolve the
//! transaction hashes it permits in place of full transactions. Inventing an `eth`
//! message here would be inventing protocol, so the transport is left out and
//! raised as item 21.

use ethrex_common::H256;
use ethrex_common::types::{
    DependencyTriple, FrameTransaction, deduplicate_and_sort_dependencies, dependencies_hash,
};
use ethrex_rlp::{
    decode::RLPDecode,
    encode::RLPEncode,
    error::RLPDecodeError,
    structs::{Decoder, Encoder},
};

use crate::{
    AggregateError, DependencyAggregator, DependencyWitness, MAX_RECURSIVE_STARK_PROOF_BYTES,
};

/// EIP-8288 `MAX_LEANSIG_DEPS_PER_WRAPPER`.
pub const MAX_LEANSIG_DEPS_PER_WRAPPER: usize = 16;
/// EIP-8288 `MAX_LEANSTARK_DEPS_PER_WRAPPER`.
pub const MAX_LEANSTARK_DEPS_PER_WRAPPER: usize = 1;
/// EIP-8288 `AGGREGATION_INTERVAL`, in milliseconds.
pub const AGGREGATION_INTERVAL_MS: u64 = 1_000;

const _: () = assert!(MAX_LEANSIG_DEPS_PER_WRAPPER == 16);
const _: () = assert!(MAX_LEANSTARK_DEPS_PER_WRAPPER == 1);
const _: () = assert!(AGGREGATION_INTERVAL_MS == 1_000);

/// An entry in a wrapper's transaction list.
///
/// The EIP allows a bare hash "for transactions that were already broadcast in a
/// previous wrapper", but gives no way to resolve one a receiver has never seen,
/// and a receiver holding only a hash cannot check that `deps` is the union of the
/// transactions' dependencies -- rule 1 of both modes. See item 22.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WrapperEntry {
    Full(Box<FrameTransaction>),
    Hash(H256),
}

/// Mode 0 carries one proof per dependency; mode 1 carries one aggregate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WrapperContent {
    /// `[deps, proofs]`. One proof per entry of `deps`, in the same order.
    Direct {
        deps: Vec<DependencyTriple>,
        proofs: Vec<Vec<u8>>,
    },
    /// `[deps, recursive_stark]`.
    Recursive {
        deps: Vec<DependencyTriple>,
        recursive_stark: Vec<u8>,
    },
}

impl WrapperContent {
    pub fn deps(&self) -> &[DependencyTriple] {
        match self {
            WrapperContent::Direct { deps, .. } | WrapperContent::Recursive { deps, .. } => deps,
        }
    }

    pub fn mode(&self) -> u8 {
        match self {
            WrapperContent::Direct { .. } => 0,
            WrapperContent::Recursive { .. } => 1,
        }
    }
}

/// `[transactions, mode, content]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MempoolWrapper {
    pub transactions: Vec<WrapperEntry>,
    pub content: WrapperContent,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum WrapperError {
    #[error("wrapper declares no transactions")]
    Empty,
    #[error("wrapper's deps are not the sorted, deduplicated union of its transactions' deps")]
    DepsNotUnion,
    #[error("wrapper holds {count} leanSPHINCS dependencies, over the limit of {limit}")]
    TooManyLeanSphincs { count: usize, limit: usize },
    #[error("wrapper holds {count} leanSTARK dependencies, over the limit of {limit}")]
    TooManyLeanStark { count: usize, limit: usize },
    #[error("mode 0 carries {proofs} proofs for {deps} dependencies")]
    ProofCountMismatch { proofs: usize, deps: usize },
    #[error(
        "wrapper's transaction list holds hashes, so its dependency union cannot be checked here"
    )]
    UnresolvedHashes,
    #[error(transparent)]
    Aggregate(#[from] AggregateError),
}

impl MempoolWrapper {
    /// The EIP's per-mode receive checks.
    ///
    /// Both modes share the two count limits and the union check; they differ in
    /// how the dependencies are proven. Rule 1 of both modes compares `deps`
    /// against the transactions' own dependencies, so it is only checkable when
    /// every entry is a full transaction.
    pub fn validate(&self, aggregator: &dyn DependencyAggregator) -> Result<(), WrapperError> {
        if self.transactions.is_empty() {
            return Err(WrapperError::Empty);
        }

        let deps = self.content.deps();
        let leansphincs = deps.iter().filter(|d| d.is_leansphincs()).count();
        if leansphincs > MAX_LEANSIG_DEPS_PER_WRAPPER {
            return Err(WrapperError::TooManyLeanSphincs {
                count: leansphincs,
                limit: MAX_LEANSIG_DEPS_PER_WRAPPER,
            });
        }
        let leanstark = deps.iter().filter(|d| d.is_leanstark()).count();
        if leanstark > MAX_LEANSTARK_DEPS_PER_WRAPPER {
            return Err(WrapperError::TooManyLeanStark {
                count: leanstark,
                limit: MAX_LEANSTARK_DEPS_PER_WRAPPER,
            });
        }

        self.check_deps_are_the_union()?;

        match &self.content {
            WrapperContent::Direct { deps, proofs } => {
                if proofs.len() != deps.len() {
                    return Err(WrapperError::ProofCountMismatch {
                        proofs: proofs.len(),
                        deps: deps.len(),
                    });
                }
                // "Each dependency has a corresponding proof that can be verified
                // individually."
                //
                // For leanSPHINCS that proof is the raw witness, not a STARK. The
                // EIP says so twice: mode 0 carries "one STARK per leanSTARK
                // dependency" -- not one per dependency -- and a node may "naively
                // concatenate the leanSPHINCS instead of proving them". Mode 0 is
                // also "intended to be used primarily by clients broadcasting their
                // transactions", and a user's first broadcast has no aggregate, so
                // demanding a recursive proof here would make the mode unusable for
                // the case it exists to serve.
                for (dep, proof) in deps.iter().zip(proofs) {
                    if dep.is_leansphincs() {
                        aggregator.verify_witness(&DependencyWitness {
                            triple: *dep,
                            witness: proof.clone(),
                        })?;
                    } else {
                        // A leanSTARK dependency does carry a STARK of its own.
                        aggregator.verify(proof, std::slice::from_ref(dep))?;
                    }
                }
                Ok(())
            }
            WrapperContent::Recursive {
                deps,
                recursive_stark,
            } => {
                if recursive_stark.len() > MAX_RECURSIVE_STARK_PROOF_BYTES {
                    return Err(WrapperError::Aggregate(AggregateError::ProofTooLarge {
                        len: recursive_stark.len(),
                        max: MAX_RECURSIVE_STARK_PROOF_BYTES,
                    }));
                }
                // The EIP's rule 1 here is that the proof's public input equals
                // `hash(deps)`. Our aggregator checks the stronger and more useful
                // property -- that the claims proven *are* `deps` -- for the reason
                // in item 14: a digest comparison is only equivalent if the proof
                // actually binds that digest, which the named tooling does not.
                aggregator.verify(recursive_stark, deps)?;
                Ok(())
            }
        }
    }

    /// Rule 1 of both modes: `deps` is the sorted, deduplicated union of the
    /// dependencies of every transaction in the wrapper.
    fn check_deps_are_the_union(&self) -> Result<(), WrapperError> {
        if self
            .transactions
            .iter()
            .any(|entry| matches!(entry, WrapperEntry::Hash(_)))
        {
            return Err(WrapperError::UnresolvedHashes);
        }

        let mut union = Vec::new();
        for entry in &self.transactions {
            if let WrapperEntry::Full(tx) = entry {
                union.extend(tx.dependencies());
            }
        }
        if deduplicate_and_sort_dependencies(union) != self.content.deps() {
            return Err(WrapperError::DepsNotUnion);
        }
        Ok(())
    }

    /// The digest the EIP names as the recursive proof's public input.
    pub fn deps_hash(&self) -> H256 {
        dependencies_hash(self.content.deps())
    }
}

impl RLPEncode for WrapperEntry {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        match self {
            // A full transaction encodes as its canonical payload, a hash as 32
            // bytes. The two are distinguishable by length, which is how a decoder
            // tells them apart without a tag -- the same structural disambiguation
            // `FramePayload` uses.
            WrapperEntry::Full(tx) => tx.encode(buf),
            WrapperEntry::Hash(hash) => hash.encode(buf),
        }
    }
}

/// `deps` on the wire: each triple as its canonical 96 bytes.
fn encoded_deps(deps: &[DependencyTriple]) -> Vec<Vec<u8>> {
    deps.iter().map(|d| d.encode().to_vec()).collect()
}

impl RLPEncode for MempoolWrapper {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        let encoder = Encoder::new(buf)
            .encode_field(&self.transactions)
            .encode_field(&(self.content.mode() as u64));
        // `content` is itself a two-element list, encoded as a tuple the way
        // EIP-8141's nested `limits` and `fees` lists are.
        match &self.content {
            WrapperContent::Direct { deps, proofs } => encoder
                .encode_field(&(encoded_deps(deps), proofs.clone()))
                .finish(),
            WrapperContent::Recursive {
                deps,
                recursive_stark,
            } => encoder
                .encode_field(&(encoded_deps(deps), recursive_stark.clone()))
                .finish(),
        }
    }
}

impl RLPDecode for MempoolWrapper {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (transactions, decoder) = decoder.decode_field::<Vec<WrapperEntry>>("transactions")?;
        let (mode, decoder) = decoder.decode_field::<u64>("mode")?;
        match mode {
            0 => {
                let (content, decoder) =
                    decoder.decode_field::<(Vec<Vec<u8>>, Vec<Vec<u8>>)>("content")?;
                let (raw_deps, proofs) = content;
                Ok((
                    MempoolWrapper {
                        transactions,
                        content: WrapperContent::Direct {
                            deps: decode_deps(&raw_deps)?,
                            proofs,
                        },
                    },
                    decoder.finish()?,
                ))
            }
            1 => {
                let (content, decoder) =
                    decoder.decode_field::<(Vec<Vec<u8>>, Vec<u8>)>("content")?;
                let (raw_deps, recursive_stark) = content;
                Ok((
                    MempoolWrapper {
                        transactions,
                        content: WrapperContent::Recursive {
                            deps: decode_deps(&raw_deps)?,
                            recursive_stark,
                        },
                    },
                    decoder.finish()?,
                ))
            }
            other => Err(RLPDecodeError::Custom(format!(
                "unknown wrapper mode {other}"
            ))),
        }
    }
}

impl RLPDecode for WrapperEntry {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        // Structural disambiguation, as `FramePayload` does it: a transaction
        // payload is a list, a hash is a 32-byte string.
        match FrameTransaction::decode_unfinished(rlp) {
            Ok((tx, rest)) => Ok((WrapperEntry::Full(Box::new(tx)), rest)),
            Err(_) => {
                let (hash, rest) = H256::decode_unfinished(rlp)?;
                Ok((WrapperEntry::Hash(hash), rest))
            }
        }
    }
}

/// Parse the wire `deps` list. A malformed triple is a decode failure, not a
/// silently dropped dependency.
fn decode_deps(raw: &[Vec<u8>]) -> Result<Vec<DependencyTriple>, RLPDecodeError> {
    raw.iter()
        .map(|encoded| {
            DependencyTriple::from_bytes(encoded)
                .ok_or_else(|| RLPDecodeError::Custom("malformed dependency triple".to_string()))
        })
        .collect()
}
