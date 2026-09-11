//! The backend a node without `leanvm` gets: one that refuses.

use ethereum_types::H256;
use ethrex_common::types::DependencyTriple;
use tracing::warn;

use crate::{AggregateError, DependencyAggregator, DependencyWitness};

/// Fails closed on everything.
///
/// A node built without an aggregation backend can still enforce EIP-8288's first
/// block-validity rule, because `block_deps_hash` is computed from the block's own
/// transactions and needs no proof. What it cannot do is check rule 2, so it must
/// reject any block that carries a proof rather than wave it through.
///
/// That asymmetry is deliberate and is the whole reason this type exists instead of
/// an `Option<Box<dyn DependencyAggregator>>` that callers might treat as "skip the
/// check". Compare `ExecBackend` in `crates/prover/src/backend/exec.rs`, whose
/// `verify` returns `Ok` and warns -- correct there, because it is a developer
/// convenience for a proof nothing consensus-critical depends on, and wrong here.
#[derive(Debug, Clone, Copy, Default)]
pub struct UnavailableAggregator;

impl DependencyAggregator for UnavailableAggregator {
    fn verify(&self, _proof: &[u8], _expected: &[DependencyTriple]) -> Result<(), AggregateError> {
        warn!(
            "rejecting a block carrying an EIP-8288 recursive proof: this build has no \
             aggregation backend compiled in. Rebuild with the `leanvm` feature to verify them."
        );
        Err(AggregateError::NoBackend)
    }

    fn aggregate(
        &self,
        _raw: &[DependencyWitness],
        _children: &[&[u8]],
    ) -> Result<Vec<u8>, AggregateError> {
        Err(AggregateError::NoBackend)
    }

    fn verify_witness(&self, _witness: &DependencyWitness) -> Result<(), AggregateError> {
        Err(AggregateError::NoBackend)
    }

    fn aggregated_vk(&self) -> H256 {
        // Not a real key and must never be treated as one. A node with no backend
        // verifies nothing, so there is no circuit whose identity this could name.
        H256::zero()
    }

    fn name(&self) -> &'static str {
        "unavailable"
    }
}
