//! EIP-8288 block-validity rule 2: the recursive proof discharges the block's
//! dependencies.
//!
//! Rule 1 -- that the header's `block_deps_hash` equals the digest of the
//! dependencies the block's transactions declare -- lives in
//! `ethrex_common::validation::validate_block_pre_execution`, because it needs only
//! the body. This one needs an aggregation backend, so it lives here.
//!
//! # The rule as the EIP states it, and as it has to be implemented
//!
//! §Block Validity Rules says a block is valid iff, among other things, "the
//! recursive STARK proof verifies successfully against `block_deps_hash` and the
//! fixed protocol-level `AGGREGATED_VK`". That reads as one check with the digest
//! as a public input.
//!
//! The tooling the EIP names does not work that way. A leanVM aggregate publishes
//! the claims it proved, and leanVM's own documentation says a caller expecting
//! particular claims has to compare them itself. So verification is two steps, and
//! skipping the second means any valid aggregate over any keys satisfies any block.
//! `DependencyAggregator::verify` takes the expected set for that reason; see item
//! 14 of `scripts/hegota-testnet/NOTES-FOR-8288-AUTHOR.md`.

use ethrex_common::types::{Block, ChainConfig};
use ethrex_dep_aggregation::{AggregateError, DependencyAggregator};

use crate::error::ChainError;

/// Check the block's recursive proof against the dependencies it declares.
///
/// A no-op before J*, resolved by ordinal so that this and the VM's frame-mode
/// gate cannot disagree (see [`ChainConfig::is_jstar_or_later`]). From J* the
/// header entry is mandatory, so its absence is already an invalid header by the
/// time this runs; the `None` arm is defensive.
///
/// A block that declares no dependencies has nothing for a proof to discharge, and
/// is required to carry an empty one; see the comment on that arm below.
pub fn validate_recursive_stark(
    block: &Block,
    chain_config: &ChainConfig,
    aggregator: &dyn DependencyAggregator,
) -> Result<(), ChainError> {
    if !chain_config.is_jstar_or_later(block.header.timestamp) {
        return Ok(());
    }

    let Some(entry) = block.header.recursive_stark.as_ref() else {
        return Err(ChainError::RecursiveStarkInvalid(
            "J* header carries no recursive stark entry".to_string(),
        ));
    };

    // Rule 1 already established that this matches the body, so passing the body's
    // own set here would be checking the proof against a value the header agrees
    // with either way. Use the body's, so that a future reordering of the two rules
    // cannot make rule 2 vacuous.
    let expected = block.body.dependencies();

    // A block that declares no dependencies has nothing for a proof to discharge,
    // so rule 2 is satisfied by arithmetic rather than by cryptography, and a node
    // needs no backend to know it.
    //
    // This matters more than it looks. The header entry is mandatory from J*, so
    // without this arm every J* block -- including every block on a chain where
    // nobody has ever used the feature -- reaches a backend. A build without one
    // would then refuse the entire chain rather than the transactions it cannot
    // check, and sit in SYNCING forever. Failing closed is right; failing closed on
    // *any* J* block instead of on *unproven dependencies* is the wrong
    // granularity.
    //
    // The proof must still be empty. Nothing here would read it, so a non-empty one
    // is unaccounted bytes riding in a header every node stores and hashes.
    if expected.is_empty() {
        if entry.proof.is_empty() {
            return Ok(());
        }
        return Err(ChainError::RecursiveStarkInvalid(
            "block declares no dependencies but carries a non-empty recursive stark proof"
                .to_string(),
        ));
    }

    aggregator.verify(&entry.proof, &expected).map_err(|e| {
        // Two of these say nothing about the block, only about this node: it has no
        // backend at all, or the backend it has cannot express one of the schemes
        // the block uses. Reporting either as INVALID would have the node tell its
        // consensus client that a chain everyone else follows is bad. It still
        // refuses the block -- it cannot verify it -- but as a local incapacity.
        //
        // `SchemeUnsupported` belongs on this side even though leanSTARK looks
        // permanently unprovable to us (item 29): whether some other backend can
        // discharge it is not a fact this node has. Every other variant is a
        // judgement about the proof itself, and those are real invalidity.
        match e {
            AggregateError::NoBackend | AggregateError::SchemeUnsupported { .. } => {
                ChainError::RecursiveStarkUnverifiable(e.to_string())
            }
            _ => ChainError::RecursiveStarkInvalid(e.to_string()),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use ethrex_common::H256;
    use ethrex_common::types::{
        BlockBody, BlockHeader, DEPENDENCY_SCHEME_LEANSPHINCS, DependencyTriple, Frame, FrameMode,
        FrameTransaction, RecursiveStark, Transaction,
    };
    use ethrex_dep_aggregation::UnavailableAggregator;

    fn jstar_config() -> ChainConfig {
        ChainConfig {
            jstar_time: Some(0),
            ..Default::default()
        }
    }

    /// A block whose single transaction declares one leanSPHINCS dependency.
    fn block_with_a_dependency(proof: Bytes) -> Block {
        let triple = DependencyTriple {
            scheme: DEPENDENCY_SCHEME_LEANSPHINCS,
            data_hash: H256::from_low_u64_be(1),
            verification_key_hash: H256::from_low_u64_be(2),
        };
        let tx = FrameTransaction {
            frames: vec![Frame {
                mode: FrameMode::DepVerify as u8,
                data: Bytes::from(triple.encode().to_vec()),
                ..Default::default()
            }],
            ..Default::default()
        };
        let body = BlockBody {
            transactions: vec![Transaction::FrameTransaction(tx)],
            ..Default::default()
        };
        let header = BlockHeader {
            recursive_stark: Some(RecursiveStark {
                proof,
                block_deps_hash: body.block_deps_hash(),
            }),
            ..Default::default()
        };
        Block::new(header, body)
    }

    fn empty_block(proof: Bytes) -> Block {
        let body = BlockBody::default();
        let header = BlockHeader {
            recursive_stark: Some(RecursiveStark {
                proof,
                block_deps_hash: body.block_deps_hash(),
            }),
            ..Default::default()
        };
        Block::new(header, body)
    }

    /// The arm that decides whether a default build can follow a J* chain at all.
    /// Every block carries the header entry from J* on, so without this a node with
    /// no backend would refuse the whole chain rather than the transactions it
    /// cannot check.
    #[test]
    fn a_block_with_no_dependencies_needs_no_backend() {
        validate_recursive_stark(
            &empty_block(Bytes::new()),
            &jstar_config(),
            &UnavailableAggregator,
        )
        .expect("nothing to discharge means nothing to verify");
    }

    /// Nothing reads the proof in that case, so a non-empty one is unaccounted bytes
    /// in a header every node stores and hashes.
    #[test]
    fn a_block_with_no_dependencies_must_carry_no_proof() {
        assert!(matches!(
            validate_recursive_stark(
                &empty_block(Bytes::from_static(b"padding")),
                &jstar_config(),
                &UnavailableAggregator,
            ),
            Err(ChainError::RecursiveStarkInvalid(_))
        ));
    }

    /// Notes item 19: a node with no verifier has learned nothing about the block.
    /// It must refuse it, but as a local incapacity -- the two errors map to
    /// different fork-choice outcomes.
    #[test]
    fn a_dependency_without_a_backend_is_unverifiable_not_invalid() {
        assert!(matches!(
            validate_recursive_stark(
                &block_with_a_dependency(Bytes::from_static(b"an aggregate")),
                &jstar_config(),
                &UnavailableAggregator,
            ),
            Err(ChainError::RecursiveStarkUnverifiable(_))
        ));
    }

    /// The rule is fork-gated, so a pre-J* block never reaches a backend.
    #[test]
    fn the_rule_is_silent_before_jstar() {
        validate_recursive_stark(
            &block_with_a_dependency(Bytes::from_static(b"an aggregate")),
            &ChainConfig::default(),
            &UnavailableAggregator,
        )
        .expect("rule 2 does not apply before J*");
    }

    /// A schedule naming LStar and no `jstarTime` resolves to `Fork::LStar`, which
    /// the VM's frame-mode gate reads as "mode 3 is legal". Rule 2 has to read it
    /// the same way, or that chain runs dependency frames nothing ever discharges.
    #[test]
    fn a_schedule_past_jstar_without_a_jstar_time_still_gets_the_rule() {
        let lstar_only = ChainConfig {
            lstar_time: Some(0),
            ..Default::default()
        };
        assert!(
            !lstar_only.is_jstar_activated(0),
            "the timestamp predicate says no, which is the trap this guards"
        );
        assert!(lstar_only.is_jstar_or_later(0), "the ordinal says yes");

        assert!(matches!(
            validate_recursive_stark(
                &block_with_a_dependency(Bytes::from_static(b"an aggregate")),
                &lstar_only,
                &UnavailableAggregator,
            ),
            Err(ChainError::RecursiveStarkUnverifiable(_))
        ));
    }

    /// A backend that has the machinery but not this scheme. leanSTARK (`0x11`) is
    /// the real case: it is statically valid and no backend can express it.
    #[derive(Debug)]
    struct SchemeBlindAggregator;

    impl DependencyAggregator for SchemeBlindAggregator {
        fn verify(
            &self,
            _proof: &[u8],
            expected: &[DependencyTriple],
        ) -> Result<(), AggregateError> {
            Err(AggregateError::SchemeUnsupported {
                scheme: expected[0].scheme,
            })
        }

        fn aggregate(
            &self,
            _raw: &[ethrex_dep_aggregation::DependencyWitness],
            _children: &[&[u8]],
        ) -> Result<Vec<u8>, AggregateError> {
            unimplemented!("not exercised")
        }

        fn verify_witness(
            &self,
            _witness: &ethrex_dep_aggregation::DependencyWitness,
        ) -> Result<(), AggregateError> {
            unimplemented!("not exercised")
        }

        fn supports_scheme(&self, _scheme: u8) -> bool {
            false
        }

        fn aggregated_vk(&self) -> H256 {
            H256::zero()
        }

        fn name(&self) -> &'static str {
            "scheme-blind"
        }
    }

    /// The same reasoning as `NoBackend`, and easy to get wrong: an unsupported
    /// scheme is a fact about this node's backend, not about the block. A node that
    /// reported INVALID here would tell its consensus client that a chain the rest
    /// of the network follows is bad.
    #[test]
    fn an_unsupported_scheme_is_unverifiable_not_invalid() {
        assert!(matches!(
            validate_recursive_stark(
                &block_with_a_dependency(Bytes::from_static(b"an aggregate")),
                &jstar_config(),
                &SchemeBlindAggregator,
            ),
            Err(ChainError::RecursiveStarkUnverifiable(_))
        ));
    }

    /// Defensive: header validation rejects this first, but rule 2 must not read a
    /// missing entry as "no dependencies to discharge".
    #[test]
    fn a_jstar_block_missing_the_entry_is_invalid() {
        let mut block = block_with_a_dependency(Bytes::new());
        block.header.recursive_stark = None;
        assert!(matches!(
            validate_recursive_stark(&block, &jstar_config(), &UnavailableAggregator),
            Err(ChainError::RecursiveStarkInvalid(_))
        ));
    }
}
