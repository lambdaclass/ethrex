//! EIP-8141 claim witnesses: the bytes that satisfy the claims a block's frame
//! transactions declare.
//!
//! A claim says that a verifier attests that a key authorizes a message. The
//! bytes that prove it are not part of the transaction: they travel beside it in
//! the mempool, and a block carries one witness for each of its distinct
//! dependencies in its `claim_witnesses`. The rules themselves live in
//! `ethrex-vm`, which stateless execution shares; this module runs them on a
//! block about to be imported and reports what it saw.

use ethrex_common::types::Block;
use ethrex_metrics::metrics;
use ethrex_vm::{Evm, claims::ClaimWitnessesError};

use crate::error::ChainError;

#[cfg(feature = "metrics")]
use ethrex_metrics::claims::METRICS_CLAIMS;

/// Checks the claim witnesses of a block before it executes (EIP-8141 §Block
/// Witnesses): the block's `claim_witnesses` holds exactly one witness for each
/// entry of `block_dependencies`, in the same order, and each witness satisfies
/// its dependency against the block's pre-state, the state `vm` was created over.
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
        // The check passed, so the byte string decodes.
        let witnesses = block
            .body
            .claim_witnesses
            .as_deref()
            .and_then(|claim_witnesses| {
                ethrex_common::types::decode_claim_witnesses(claim_witnesses).ok()
            })
            .unwrap_or_default();
        let count = witnesses.len() as u64;
        let bytes = witnesses
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
