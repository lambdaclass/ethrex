//! EIP-8288 dependency proof: one proof, carried by a block, that every
//! dependency its transactions declare has a valid signature.
//!
//! From LStar a block does not carry the witnesses of its dependencies, the list
//! EIP-8504 gives it. Its builder aggregates them into one proof, and the header
//! commits to that proof and to the dependencies it proves. A pending transaction
//! is unchanged: it travels with its witnesses, the pool verifies each one, and
//! the builder takes them from the pool.

use bytes::Bytes;
use ethrex_common::{
    errors::InvalidBlockError,
    types::{
        Block, ChainConfig, Dependency, LEANSPHINCS_SCHEME, MAX_DEPENDENCY_PROOF_BYTES,
        block_dependencies, validate_dependency_proof_commitments,
    },
};
use ethrex_crypto::{
    NativeCrypto,
    leansphincs_aggregation::{
        LeanSphincsAggregationError, LeanSphincsClaim, leansphincs_aggregate,
        leansphincs_aggregate_verify, leansphincs_aggregation_setup,
    },
};

/// What a leanSPHINCS dependency claims: its `verification_key_hash` is the
/// public key, and its `data_hash` the message signed under it.
fn claim(dependency: &Dependency) -> LeanSphincsClaim {
    (dependency.verification_key_hash.0, dependency.data_hash.0)
}

/// Why [`prove_dependencies`] returned no proof.
#[derive(Debug, thiserror::Error)]
pub enum DependencyProofError {
    #[error("a dependency is not of the leanSPHINCS scheme")]
    UnsupportedScheme,
    #[error(transparent)]
    Aggregation(#[from] LeanSphincsAggregationError),
    #[error("the proof has {0} bytes, more than MAX_DEPENDENCY_PROOF_BYTES")]
    TooLarge(usize),
}

/// Builds the prover's program and starts its threads, which the first proof
/// does otherwise. A node calls this when it starts to build an LStar payload,
/// so the first payload it has to prove does not wait for it.
pub fn prepare_prover() {
    leansphincs_aggregation_setup();
}

/// Builds the dependency proof of a block from its dependencies, each with the
/// witness that satisfies it. Without dependencies the proof is empty.
///
/// Proving takes from a tenth of a second to several seconds and runs one proof
/// at a time in the process, so a builder calls this once, for the payload it
/// hands out, and holds no lock while it runs.
pub fn prove_dependencies(
    dependencies: &[(Dependency, Bytes)],
) -> Result<Bytes, DependencyProofError> {
    if dependencies.is_empty() {
        return Ok(Bytes::new());
    }
    if dependencies
        .iter()
        .any(|(dependency, _)| dependency.scheme != LEANSPHINCS_SCHEME)
    {
        return Err(DependencyProofError::UnsupportedScheme);
    }
    let signatures: Vec<_> = dependencies
        .iter()
        .map(|(dependency, witness)| (claim(dependency), witness.as_ref()))
        .collect();
    let proof = leansphincs_aggregate(&signatures)?;
    if proof.len() > MAX_DEPENDENCY_PROOF_BYTES {
        return Err(DependencyProofError::TooLarge(proof.len()));
    }
    Ok(Bytes::from(proof))
}

/// EIP-8288 block validity, checked before the block executes, where the
/// witnesses of EIP-8504 are checked before LStar:
///
/// 1. the header's `block_deps_hash` is the digest of `block_dependencies`;
/// 2. the dependency proof has at most `MAX_DEPENDENCY_PROOF_BYTES` bytes;
/// 3. the header's `dependency_proof_hash` is the hash of the proof;
/// 4. the proof is empty when the block has no dependencies, and otherwise it
///    proves exactly the block's dependencies.
///
/// From LStar the proof takes the place of the witness list, so a block that
/// still carries the list is invalid, as is a block that carries a proof before
/// LStar. LStar is resolved through the fork ordinal, as execution resolves it.
pub fn validate_dependency_proof(
    block: &Block,
    chain_config: &ChainConfig,
) -> Result<(), InvalidBlockError> {
    if !chain_config
        .fork(block.header.timestamp)
        .has_dependency_proof()
    {
        if block.body.dependency_proof.is_some() {
            return Err(InvalidBlockError::DependencyProofPresent);
        }
        return Ok(());
    }
    if block.body.dependency_witnesses.is_some() {
        return Err(InvalidBlockError::DependencyWitnessesPresent);
    }
    let Some(proof) = &block.body.dependency_proof else {
        return Err(InvalidBlockError::DependencyProofNotPresent);
    };
    let dependencies = block_dependencies(&block.body.transactions);
    validate_dependency_proof_commitments(&block.header, &block.body, &dependencies, &NativeCrypto)
        .map_err(InvalidBlockError::InvalidBody)?;
    if dependencies.is_empty() {
        if !proof.is_empty() {
            return Err(InvalidBlockError::DependencyProofNotEmpty(proof.len()));
        }
        return Ok(());
    }
    // The proof system proves leanSPHINCS claims, the only scheme a proof frame
    // may declare. A dependency of any other scheme is never proven.
    let claims: Vec<_> = dependencies.iter().map(claim).collect();
    let proven = dependencies
        .iter()
        .all(|dependency| dependency.scheme == LEANSPHINCS_SCHEME)
        && leansphincs_aggregate_verify(&claims, proof);
    if !proven {
        return Err(InvalidBlockError::InvalidDependencyProof(
            dependencies.len(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethrex_common::{
        Address, H256,
        types::{
            BlockBody, BlockHeader, Frame, FrameTransaction, InvalidBlockBodyError,
            LEANSPHINCS_VERIFICATION_GAS, PROOF_FRAME_MODE, Transaction,
            compute_dependency_proof_hash, compute_deps_hash,
        },
    };

    const HEGOTA: u64 = 1_000;
    const LSTAR: u64 = 2_000;

    fn config() -> ChainConfig {
        ChainConfig {
            hegota_time: Some(HEGOTA),
            lstar_time: Some(LSTAR),
            ..Default::default()
        }
    }

    /// A block without transactions at `timestamp`, with the given proof and
    /// the header committing to it and to the empty list of dependencies.
    fn block(timestamp: u64, proof: Option<Bytes>) -> Block {
        Block::new(
            BlockHeader {
                timestamp,
                dependency_proof_hash: proof
                    .as_ref()
                    .map(|proof| compute_dependency_proof_hash(proof, &NativeCrypto)),
                block_deps_hash: proof
                    .as_ref()
                    .map(|_| compute_deps_hash(&[], &NativeCrypto)),
                ..Default::default()
            },
            BlockBody {
                transactions: Vec::new(),
                ommers: Vec::new(),
                withdrawals: Some(Vec::new()),
                dependency_witnesses: None,
                dependency_proof: proof,
            },
        )
    }

    #[test]
    fn a_block_without_dependencies_is_proven_by_the_empty_proof() {
        assert_eq!(
            prove_dependencies(&[]).expect("nothing to prove"),
            Bytes::new()
        );
    }

    #[test]
    fn a_dependency_of_another_scheme_is_not_proven() {
        let dependency = Dependency {
            scheme: LEANSPHINCS_SCHEME + 1,
            data_hash: H256::repeat_byte(0x01),
            verification_key_hash: H256::repeat_byte(0x02),
        };
        let result = prove_dependencies(&[(dependency, Bytes::from(vec![0u8; 7740]))]);
        assert!(
            matches!(result, Err(DependencyProofError::UnsupportedScheme)),
            "got {result:?}"
        );
    }

    #[test]
    fn a_block_carries_a_dependency_proof_from_lstar_and_never_before() {
        let config = config();

        // Before LStar there is no proof to check, and a block that carries one
        // is invalid.
        for timestamp in [HEGOTA - 1, HEGOTA, LSTAR - 1] {
            validate_dependency_proof(&block(timestamp, None), &config).expect("no proof");
            let mut with_proof = block(timestamp, None);
            with_proof.body.dependency_proof = Some(Bytes::new());
            let result = validate_dependency_proof(&with_proof, &config);
            assert!(
                matches!(result, Err(InvalidBlockError::DependencyProofPresent)),
                "got {result:?}"
            );
        }

        // From LStar the block has a proof, empty when it has no dependencies.
        for timestamp in [LSTAR, LSTAR + 1] {
            validate_dependency_proof(&block(timestamp, Some(Bytes::new())), &config)
                .expect("the empty proof of a block without dependencies");

            let result = validate_dependency_proof(&block(timestamp, None), &config);
            assert!(
                matches!(result, Err(InvalidBlockError::DependencyProofNotPresent)),
                "got {result:?}"
            );

            let result = validate_dependency_proof(
                &block(timestamp, Some(Bytes::from(vec![0xEE; 300]))),
                &config,
            );
            assert!(
                matches!(result, Err(InvalidBlockError::DependencyProofNotEmpty(300))),
                "got {result:?}"
            );

            // The witness list of EIP-8504 is not part of the block any more.
            let mut with_witnesses = block(timestamp, Some(Bytes::new()));
            with_witnesses.body.dependency_witnesses = Some(Vec::new());
            let result = validate_dependency_proof(&with_witnesses, &config);
            assert!(
                matches!(result, Err(InvalidBlockError::DependencyWitnessesPresent)),
                "got {result:?}"
            );
        }
    }

    #[test]
    fn a_chain_that_schedules_lstar_without_hegota_still_requires_the_proof() {
        // Execution resolves the fork by its ordinal, so on this chain it accepts
        // proof frames from LStar's activation. The proof must be required by the
        // same test, or those frames would execute with no signature behind them.
        let lstar_only = ChainConfig {
            lstar_time: Some(LSTAR),
            ..Default::default()
        };
        assert!(lstar_only.hegota_time.is_none());

        // One transaction that depends on a signature of its own signature hash.
        let mut triple = vec![0u8; 31];
        triple.push(LEANSPHINCS_SCHEME);
        triple.extend_from_slice(H256::zero().as_bytes());
        triple.extend_from_slice(H256::repeat_byte(0x02).as_bytes());
        let transaction = Transaction::FrameTransaction(FrameTransaction {
            chain_id: 1,
            sender: Address::repeat_byte(0xAA),
            frames: vec![Frame {
                mode: PROOF_FRAME_MODE,
                gas_limit: LEANSPHINCS_VERIFICATION_GAS,
                data: Bytes::from(triple),
                ..Default::default()
            }],
            ..Default::default()
        });

        // The block commits to its dependency and to an empty proof.
        let mut unproven = block(LSTAR, Some(Bytes::new()));
        unproven.body.transactions = vec![transaction];
        let dependencies = block_dependencies(&unproven.body.transactions);
        assert_eq!(dependencies.len(), 1);
        unproven.header.block_deps_hash = Some(compute_deps_hash(&dependencies, &NativeCrypto));
        let result = validate_dependency_proof(&unproven, &lstar_only);
        assert!(
            matches!(result, Err(InvalidBlockError::InvalidDependencyProof(1))),
            "got {result:?}"
        );
    }

    #[test]
    fn the_header_commitments_are_checked_before_the_proof() {
        let config = config();

        let mut wrong_deps_hash = block(LSTAR, Some(Bytes::new()));
        wrong_deps_hash.header.block_deps_hash = Some(H256::repeat_byte(0x01));
        let result = validate_dependency_proof(&wrong_deps_hash, &config);
        assert!(
            matches!(
                result,
                Err(InvalidBlockError::InvalidBody(
                    InvalidBlockBodyError::BlockDepsHashNotMatch
                ))
            ),
            "got {result:?}"
        );

        let mut wrong_proof_hash = block(LSTAR, Some(Bytes::new()));
        wrong_proof_hash.header.dependency_proof_hash = Some(H256::repeat_byte(0x01));
        let result = validate_dependency_proof(&wrong_proof_hash, &config);
        assert!(
            matches!(
                result,
                Err(InvalidBlockError::InvalidBody(
                    InvalidBlockBodyError::DependencyProofHashNotMatch
                ))
            ),
            "got {result:?}"
        );

        // The size bound comes before the proof is hashed: a proof over it is
        // rejected for its size whatever the header commits to.
        let oversized = Bytes::from(vec![0u8; MAX_DEPENDENCY_PROOF_BYTES + 1]);
        let mut too_large = block(LSTAR, Some(Bytes::new()));
        too_large.body.dependency_proof = Some(oversized);
        let result = validate_dependency_proof(&too_large, &config);
        assert!(
            matches!(
                result,
                Err(InvalidBlockError::InvalidBody(
                    InvalidBlockBodyError::DependencyProofTooLarge(size)
                )) if size == MAX_DEPENDENCY_PROOF_BYTES + 1
            ),
            "got {result:?}"
        );
    }
}
