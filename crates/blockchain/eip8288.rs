//! EIP-8288 block-validity rule 2: the block's recursive proof discharges its
//! dependencies.
//!
//! Rule 1 -- that the header's `block_deps_hash` equals the digest of the
//! dependencies the block's transactions declare -- lives in
//! `ethrex_common::validation::validate_block_pre_execution`, because it needs only
//! the body. This one needs an aggregation backend, so it lives here.
//!
//! The proof's public input is the digest of the block's dependency set under
//! `AGGREGATED_VK`. `DependencyAggregator::verify` takes the set rather than the
//! header's digest so the backend computes the digest from what the block actually
//! declares; a proof of any other set then fails because its digest differs.

use std::sync::LazyLock;

use ethrex_common::types::{Block, ChainConfig};
use ethrex_dep_aggregation::{AggregateError, DependencyAggregator, default_aggregator};
use tracing::info;

use crate::error::ChainError;

/// The aggregation backend this build carries, built once per process: the leanVM
/// guest compiles on first use, so a node pays for it at startup (see
/// [`log_backend`]) rather than inside the first block to carry a proof. Chosen at
/// compile time rather than configured, so a node cannot be talked into accepting
/// proofs it has no code to check: without the `leanvm` feature this is the backend
/// that refuses.
static AGGREGATOR: LazyLock<Box<dyn DependencyAggregator>> = LazyLock::new(default_aggregator);

/// The EIP-8288 aggregation backend this build carries.
pub fn aggregator() -> &'static dyn DependencyAggregator {
    AGGREGATOR.as_ref()
}

/// Build the backend and report which one this node runs and the key it verifies
/// against. Called at startup on a chain that schedules EIP-8288.
pub fn log_backend() {
    let backend = aggregator();
    info!(
        backend = backend.name(),
        aggregated_vk = %backend.aggregated_vk(),
        "EIP-8288 proof backend ready"
    );
}

/// Check the block's recursive proof against the dependencies it declares.
///
/// A no-op before EIP-8288 activates. From then the header's commitment is
/// mandatory, so its absence is already an invalid header by the time this runs;
/// the check here is defensive.
///
/// A block that declares no dependencies must carry an empty proof, and one that
/// declares any must carry a non-empty proof of exactly that set.
pub fn validate_recursive_stark(
    block: &Block,
    chain_config: &ChainConfig,
) -> Result<(), ChainError> {
    if !chain_config.is_eip8288_active(block.header.timestamp) {
        return Ok(());
    }

    if block.header.recursive_stark.is_none() {
        return Err(ChainError::RecursiveStarkInvalid(
            "an EIP-8288 header carries no recursive stark commitment".to_string(),
        ));
    }
    // The body's proof, which body validation has already bound to the header's
    // commitment. A body without one carries the empty proof.
    let proof = block.body.stark_proof.as_deref().unwrap_or_default();

    // Rule 1 already established that this matches the header, so checking against
    // the body's own set keeps rule 2 meaningful whatever order the two run in.
    let expected = block.body.dependencies();

    // Decided without a backend, so a build without one still follows every block
    // that declares nothing.
    if expected.is_empty() {
        if proof.is_empty() {
            return Ok(());
        }
        return Err(ChainError::RecursiveStarkInvalid(
            "a block without dependencies must carry an empty proof".to_string(),
        ));
    }
    if proof.is_empty() {
        return Err(ChainError::RecursiveStarkInvalid(
            "a block with dependencies must carry a proof".to_string(),
        ));
    }

    aggregator()
        .verify(proof, &expected)
        .map_err(classify_aggregate_error)
}

fn classify_aggregate_error(e: AggregateError) -> ChainError {
    // Three of these say nothing about the block, only about this node: it has no
    // backend at all, the backend it has cannot express one of the schemes the
    // block uses, or the backend it has is not the one the network verifies with.
    // Reporting any of them as INVALID would have the node tell its consensus
    // client that a chain everyone else follows is bad. It still refuses the block
    // -- it cannot verify it -- but as a local incapacity. Every other variant is a
    // judgement about the proof itself, and those are real invalidity.
    match e {
        AggregateError::NoBackend
        | AggregateError::SchemeUnsupported { .. }
        | AggregateError::BackendMismatch(_) => {
            ChainError::RecursiveStarkUnverifiable(e.to_string())
        }
        _ => ChainError::RecursiveStarkInvalid(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mismatched_backend_is_unverifiable_not_invalid() {
        assert!(matches!(
            classify_aggregate_error(AggregateError::BackendMismatch("guest key".into())),
            ChainError::RecursiveStarkUnverifiable(_)
        ));
        assert!(matches!(
            classify_aggregate_error(AggregateError::NoBackend),
            ChainError::RecursiveStarkUnverifiable(_)
        ));
        assert!(matches!(
            classify_aggregate_error(AggregateError::ProofMalformed),
            ChainError::RecursiveStarkInvalid(_)
        ));
    }
}
