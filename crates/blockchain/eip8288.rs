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
/// A no-op before J*. From J* the header entry is mandatory, so its absence
/// is already an invalid header by the time this runs; the `None` arm is defensive.
///
/// A block that declares no dependencies still carries the entry, and its proof is
/// still checked -- a node must not be able to smuggle an unverifiable aggregate in
/// by declaring nothing, and a backend is free to accept an empty proof for an empty
/// set if that is what its encoding means.
pub fn validate_recursive_stark(
    block: &Block,
    chain_config: &ChainConfig,
    aggregator: &dyn DependencyAggregator,
) -> Result<(), ChainError> {
    if !chain_config.is_jstar_activated(block.header.timestamp) {
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

    aggregator.verify(&entry.proof, &expected).map_err(|e| {
        // A node with no backend has learned nothing about this block, so it must
        // not report it as invalid: that would have it tell its consensus client
        // that a chain everyone else follows is bad. It still refuses the block --
        // it cannot verify it -- but as a local incapacity.
        if matches!(e, AggregateError::NoBackend) {
            ChainError::RecursiveStarkUnverifiable(e.to_string())
        } else {
            ChainError::RecursiveStarkInvalid(e.to_string())
        }
    })
}
