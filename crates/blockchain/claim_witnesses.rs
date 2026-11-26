//! EIP-8141 claim witnesses: the bytes that satisfy the claims a block's frame
//! transactions declare.
//!
//! A claim says that a verifier attests that a key authorizes a message. The
//! bytes that prove it are not part of the transaction: they travel beside it in
//! the mempool, and a block carries one witness for each of its distinct
//! dependencies in its `claim_witnesses`. The rules themselves live in
//! `ethrex-vm`, which stateless execution shares; this module runs them on a
//! block about to be imported and reports what it saw.
//!
//! From IStar a block carries one aggregate proof in place of the witnesses of
//! its leanSPHINCS dependencies (EIP-8141 §Aggregation and EIP-8288
//! compatibility). A pending transaction is unchanged: it travels with its
//! witnesses and the pool verifies each one. The builder of a block makes the
//! proof from them, with the prover this module wraps.

use bytes::Bytes;
use ethrex_common::types::{Block, Dependency, LEANSPHINCS_VERIFIER, MAX_AGGREGATE_PROOF_BYTES};
use ethrex_crypto::leansphincs_aggregation::{
    LeanSphincsAggregationError, leansphincs_aggregate, leansphincs_aggregation_setup,
};
use ethrex_metrics::metrics;
use ethrex_vm::{Evm, claims::ClaimWitnessesError};

use crate::error::ChainError;

#[cfg(feature = "metrics")]
use ethrex_metrics::claims::METRICS_CLAIMS;

/// Checks the claim witnesses of a block before it executes (EIP-8141 §Block
/// Witnesses): the block's `claim_witnesses` holds exactly one witness for each
/// entry of `block_dependencies`, in the same order, and each witness satisfies
/// its dependency against the block's pre-state, the state `vm` was created over.
/// From IStar one aggregate proof satisfies the leanSPHINCS dependencies, and the
/// block holds a witness for each of the others.
///
/// It runs on every import path, including blocks assembled from an engine
/// payload, whose body commitments are derived and so say nothing about whether
/// the witnesses are valid.
pub fn validate_claim_witnesses(block: &Block, vm: &Evm) -> Result<(), ChainError> {
    #[cfg(feature = "metrics")]
    let started = std::time::Instant::now();
    vm.verify_claim_witnesses(block)
        .map_err(|error| match error {
            ClaimWitnessesError::InvalidBlock(error) => ChainError::InvalidBlock(error),
            ClaimWitnessesError::Evm(error) => ChainError::from(error),
        })?;
    metrics!({
        // The check passed, so the byte string decodes under the block's fork.
        // The witnesses counted are the raw ones; the bytes also count the
        // aggregate proof that stands for the others from IStar on.
        let claim_witnesses = block
            .body
            .claim_witnesses
            .as_deref()
            .and_then(|claim_witnesses| {
                let fork = vm
                    .db
                    .store
                    .get_chain_config()
                    .ok()?
                    .fork(block.header.timestamp);
                ethrex_common::types::decode_claim_witnesses_at(fork, claim_witnesses).ok()
            })
            .unwrap_or_default();
        let count = claim_witnesses.witnesses.len() as u64;
        let bytes = claim_witnesses.aggregate_proof.len() as u64
            + claim_witnesses
                .witnesses
                .iter()
                .map(|witness| witness.len() as u64)
                .sum::<u64>();
        METRICS_CLAIMS.blocks_total.inc();
        METRICS_CLAIMS.block_witnesses.set(count as i64);
        METRICS_CLAIMS.block_witness_bytes.set(bytes as i64);
        METRICS_CLAIMS.witnesses_total.inc_by(count);
        METRICS_CLAIMS.witness_bytes_total.inc_by(bytes);
        METRICS_CLAIMS
            .block_witness_verification_seconds
            .observe(started.elapsed().as_secs_f64());
    });
    Ok(())
}

/// Why [`aggregate_claim_witnesses`] returned no proof.
#[derive(Debug, thiserror::Error)]
pub enum ClaimAggregationError {
    #[error("a dependency to aggregate is not a leanSPHINCS one")]
    UnsupportedVerifier,
    #[error(transparent)]
    Aggregation(#[from] LeanSphincsAggregationError),
    #[error("the proof has {0} bytes, more than MAX_AGGREGATE_PROOF_BYTES")]
    TooLarge(usize),
}

/// Builds the prover's program and starts its threads, which the first proof
/// does otherwise. A node calls this when it starts to build a payload of IStar
/// or a later fork, so the first payload it has to prove does not wait for it.
pub fn prepare_claim_aggregation() {
    leansphincs_aggregation_setup();
}

/// Builds the aggregate proof a block carries from IStar for its leanSPHINCS
/// dependencies, from each of them with the witness that satisfies it. Without
/// dependencies the proof is empty.
///
/// Proving takes from a tenth of a second to several seconds and runs one proof
/// at a time in the process, so a builder calls this once, for the payload it
/// hands out, and holds no lock while it runs.
pub fn aggregate_claim_witnesses(
    witnesses: &[(Dependency, Bytes)],
) -> Result<Bytes, ClaimAggregationError> {
    if witnesses.is_empty() {
        return Ok(Bytes::new());
    }
    if witnesses
        .iter()
        .any(|(dependency, _)| dependency.verifier != LEANSPHINCS_VERIFIER)
    {
        return Err(ClaimAggregationError::UnsupportedVerifier);
    }
    let signatures: Vec<_> = witnesses
        .iter()
        .map(|(dependency, witness)| ((dependency.key.0, dependency.msg.0), witness.as_ref()))
        .collect();
    let proof = leansphincs_aggregate(&signatures)?;
    if proof.len() > MAX_AGGREGATE_PROOF_BYTES {
        return Err(ClaimAggregationError::TooLarge(proof.len()));
    }
    Ok(Bytes::from(proof))
}
