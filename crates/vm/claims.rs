//! EIP-8141 claim witnesses: the checks a node runs on the witnesses of a
//! block's claims before it executes the block, and on those of a single
//! dependency when it admits a transaction or builds a block.

use std::sync::Arc;

use ethrex_common::{
    H256, InvalidBlockError,
    types::{
        Block, BlockHeader, ClaimWitnesses, Dependency, Fork, Transaction, block_dependencies,
        decode_claim_witnesses_at,
    },
};
use ethrex_crypto::Crypto;
use ethrex_levm::{
    db::{Database as LevmDatabase, gen_db::GeneralizedDatabase},
    environment::EVMConfig,
    vm::VMType,
};

use crate::EvmError;

/// Verifies a witness against a dependency of a protocol verifier natively,
/// without reading any state. `None` for a dependency of any other verifier.
pub use ethrex_levm::claims::verify_protocol_dependency;

/// Why the check of a block's claim witnesses did not pass: the block breaks a
/// validity rule, or the check itself could not run.
#[derive(Debug, thiserror::Error)]
pub enum ClaimWitnessesError {
    #[error(transparent)]
    InvalidBlock(#[from] InvalidBlockError),
    #[error(transparent)]
    Evm(#[from] EvmError),
}

/// EIP-8141 `verify_dependency` at the block `header` belongs to: whether
/// `witness` satisfies `dependency`. A protocol verifier's dependency is checked
/// natively; any other verifier runs in the pure context against the state `db`
/// exposes, under the EVM configuration of that block.
pub fn verify_dependency(
    dependency: &Dependency,
    witness: &[u8],
    db: &mut GeneralizedDatabase,
    header: &BlockHeader,
    vm_type: VMType,
    crypto: &dyn Crypto,
) -> Result<bool, EvmError> {
    let chain_config = db.store.get_chain_config()?;
    let config = EVMConfig::new_from_chain_config(&chain_config, header);
    Ok(ethrex_levm::claims::verify_dependency(
        dependency, witness, db, config, vm_type, crypto,
    )?)
}

/// EIP-8141 §Block Witnesses: checks a block's `claim_witnesses` before the block
/// executes, under the three rules that make it valid:
///
/// 1. it is at most `MAX_BLOCK_WITNESS_BYTES` long;
/// 2. the header's `claim_witnesses_hash` is its Keccak-256 hash;
/// 3. it holds exactly one witness for each entry of `block_dependencies(block)`,
///    in the same order, and each witness satisfies its dependency against the
///    block's pre-state, which `store` exposes.
///
/// From IStar the byte string has the form the EIP reserves for aggregation
/// (§Block Witnesses, §Aggregation and EIP-8288 compatibility), and rule 3 reads:
/// one aggregate proof satisfies every dependency of `LEANSPHINCS_VERIFIER`, and
/// the byte string holds exactly one raw witness for each of the other entries
/// of `block_dependencies(block)`, in the same order, each satisfying its
/// dependency as before. The proof is empty when the block has no
/// `LEANSPHINCS_VERIFIER` dependency, and otherwise proves exactly those the
/// block has: a block whose proof misses one of them, proves another one in its
/// place or proves one more is invalid.
///
/// What the byte string looks like for a given version byte, and which version
/// byte a fork accepts, is decided in one place, [`decode_claim_witnesses_at`].
///
/// Before any verifier runs, the gas of the block's claims is compared with the
/// block's gas limit, which bounds the work of rule 3 by one block's gas.
///
/// Frame transactions exist from Hegota on. Before it a block has no claims and
/// no `claim_witnesses`, since the header rules reject its commitment, so there
/// is nothing to check. The fork is resolved through its ordinal, exactly as
/// execution resolves it: the rule that lets frames rely on claims and the rule
/// that requires their witnesses must come from the same test.
///
/// Verification reads state through a database of its own, so nothing it loads
/// reaches the one the block then executes on.
pub fn verify_claim_witnesses(
    block: &Block,
    store: Arc<dyn LevmDatabase>,
    vm_type: VMType,
    crypto: &dyn Crypto,
) -> Result<(), ClaimWitnessesError> {
    let chain_config = store.get_chain_config().map_err(EvmError::from)?;
    let fork = chain_config.fork(block.header.timestamp);
    if fork < Fork::Hegota {
        return Ok(());
    }
    let claim_witnesses: &[u8] = block.body.claim_witnesses.as_deref().unwrap_or_default();
    // Rule 1, and the form of the byte string at the block's fork.
    let ClaimWitnesses {
        aggregate_proof,
        witnesses,
    } = decode_claim_witnesses_at(fork, claim_witnesses)
        .map_err(|error| InvalidBlockError::InvalidClaimWitnesses(error.to_string()))?;
    // Rule 2. The body check compares the same commitment where a caller runs
    // it, but not every import path does, and a block must not reach execution
    // with witnesses its header does not commit to.
    if block.header.claim_witnesses_hash != Some(H256(crypto.keccak256(claim_witnesses))) {
        return Err(InvalidBlockError::ClaimWitnessesHashMismatch.into());
    }
    // Rule 3. The dependencies the aggregate proof stands for carry no witness
    // of their own; before IStar there are none, and every dependency has one.
    // A dependency keeps its index in `block_dependencies`, which is what an
    // unsatisfied claim is reported by.
    let dependencies =
        block_dependencies(&block.body.transactions).ok_or(InvalidBlockError::MalformedClaim)?;
    let (aggregated, raw): (Vec<_>, Vec<_>) = dependencies
        .iter()
        .enumerate()
        .partition(|(_, dependency)| dependency.is_aggregated(fork));
    if witnesses.len() != raw.len() {
        return Err(InvalidBlockError::ClaimWitnessesCountMismatch {
            expected: raw.len(),
            found: witnesses.len(),
        }
        .into());
    }
    if aggregated.is_empty() && !aggregate_proof.is_empty() {
        return Err(InvalidBlockError::UnexpectedAggregateProof(aggregate_proof.len()).into());
    }
    // A claim's gas is part of its transaction's intrinsic gas, so a valid block's
    // claims take at most the block's gas limit. Execution enforces that limit,
    // but only after the claims are verified, so it is compared here: without it
    // a block that execution would reject could make a node run verifiers for
    // more gas than any block holds.
    let claim_gas = block
        .body
        .transactions
        .iter()
        .fold(0u64, |total, tx| match tx {
            Transaction::FrameTransaction(tx) => total.saturating_add(tx.claim_verification_cost()),
            _ => total,
        });
    if claim_gas > block.header.gas_limit {
        return Err(InvalidBlockError::ClaimGasExceedsGasLimit {
            claim_gas,
            gas_limit: block.header.gas_limit,
        }
        .into());
    }
    let config = EVMConfig::new_from_chain_config(&chain_config, &block.header);
    let mut db = GeneralizedDatabase::new(store);
    for ((index, dependency), witness) in raw.into_iter().zip(&witnesses) {
        let satisfied = ethrex_levm::claims::verify_dependency(
            dependency, witness, &mut db, config, vm_type, crypto,
        )
        .map_err(EvmError::from)?;
        if !satisfied {
            return Err(InvalidBlockError::ClaimNotSatisfied(index).into());
        }
    }
    // The proof is checked against the dependencies this node derived from the
    // block's transactions, so it cannot stand for any other set of them. An
    // empty proof proves nothing.
    if !aggregated.is_empty() {
        let aggregated: Vec<Dependency> = aggregated
            .into_iter()
            .map(|(_, dependency)| *dependency)
            .collect();
        if !ethrex_levm::claims::verify_aggregated_dependencies(&aggregated, &aggregate_proof) {
            return Err(InvalidBlockError::InvalidAggregateProof(aggregated.len()).into());
        }
    }
    Ok(())
}
