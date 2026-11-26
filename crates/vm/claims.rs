//! EIP-8141 claim witnesses: the checks a node runs on the witnesses of a
//! block's claims before it executes the block, and on those of a single
//! dependency when it admits a transaction or builds a block.

use std::sync::Arc;

use bytes::Bytes;
use ethrex_common::{
    H256, InvalidBlockError,
    types::{
        Block, BlockHeader, Dependency, Fork, Transaction, block_dependencies,
        decode_claim_witnesses,
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
/// What the byte string looks like for a given version byte is decided in one
/// place, [`decode_claim_witnesses`].
///
/// Before any verifier runs, the gas of the block's claims is compared with the
/// block's gas limit, which bounds the work of rule 3 by one block's gas.
///
/// Frame transactions exist from Hegota on. Before it a block has no claims and
/// no `claim_witnesses`, since the header rules reject its commitment, so there
/// is nothing to check. Hegota is resolved through the fork ordinal, exactly as
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
    if chain_config.fork(block.header.timestamp) < Fork::Hegota {
        return Ok(());
    }
    let claim_witnesses: &[u8] = block.body.claim_witnesses.as_deref().unwrap_or_default();
    // Rule 1, and the form of the byte string.
    let witnesses: Vec<Bytes> = decode_claim_witnesses(claim_witnesses)
        .map_err(|error| InvalidBlockError::InvalidClaimWitnesses(error.to_string()))?;
    // Rule 2. The body check compares the same commitment where a caller runs
    // it, but not every import path does, and a block must not reach execution
    // with witnesses its header does not commit to.
    if block.header.claim_witnesses_hash != Some(H256(crypto.keccak256(claim_witnesses))) {
        return Err(InvalidBlockError::ClaimWitnessesHashMismatch.into());
    }
    // Rule 3.
    let dependencies =
        block_dependencies(&block.body.transactions).ok_or(InvalidBlockError::MalformedClaim)?;
    if witnesses.len() != dependencies.len() {
        return Err(InvalidBlockError::ClaimWitnessesCountMismatch {
            expected: dependencies.len(),
            found: witnesses.len(),
        }
        .into());
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
    for (index, (dependency, witness)) in dependencies.iter().zip(&witnesses).enumerate() {
        let satisfied = ethrex_levm::claims::verify_dependency(
            dependency, witness, &mut db, config, vm_type, crypto,
        )
        .map_err(EvmError::from)?;
        if !satisfied {
            return Err(InvalidBlockError::ClaimNotSatisfied(index).into());
        }
    }
    Ok(())
}
