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
//! `DependencyAggregator::verify` takes the expected set for that reason.

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

    // Nothing to discharge and nothing offered to discharge it with: the rule holds
    // by arithmetic rather than by cryptography, and a node needs no backend to know
    // it.
    //
    // This matters more than it looks. The header entry is mandatory from J*, so
    // without this arm every J* block -- including every block on a chain where
    // nobody has ever used the feature -- reaches a backend. A build without one
    // would then refuse the entire chain rather than the transactions it cannot
    // check, and sit in SYNCING forever. Failing closed is right; failing closed on
    // *any* J* block instead of on *unproven dependencies* is the wrong
    // granularity.
    //
    // Narrow on purpose: a block declaring nothing but carrying a proof anyway is
    // not waved through here, it is checked like any other. The shortcut is only for
    // the case where there is provably nothing to check.
    if expected.is_empty() && entry.proof.is_empty() {
        return Ok(());
    }

    aggregator.verify(&entry.proof, &expected).map_err(|e| {
        // Two of these say nothing about the block, only about this node: it has no
        // backend at all, or the backend it has cannot express one of the schemes
        // the block uses. Reporting either as INVALID would have the node tell its
        // consensus client that a chain everyone else follows is bad. It still
        // refuses the block -- it cannot verify it -- but as a local incapacity.
        //
        // An unsupported scheme belongs on this side even though leanSTARK looks
        // permanently unprovable here: whether some other backend can discharge it
        // is not a fact this node has. Every other variant is a judgement about the
        // proof itself, and those are real invalidity.
        match e {
            AggregateError::NoBackend | AggregateError::SchemeUnsupported { .. } => {
                ChainError::RecursiveStarkUnverifiable(e.to_string())
            }
            _ => ChainError::RecursiveStarkInvalid(e.to_string()),
        }
    })
}
