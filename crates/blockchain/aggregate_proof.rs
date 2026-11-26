//! The aggregate proof of a block (EIP-8141 §Aggregation): one proof that the
//! claim of every stripped proof frame of the block has a valid leanSPHINCS
//! signature.
//!
//! A pending transaction carries its signatures inside it, as the proofs of its
//! proof frames. From IStar the builder of a block strips every proof the
//! protocol may strip from the transactions it includes and aggregates the
//! stripped signatures into one proof, which the block carries and its header
//! commits to. A stripped frame keeps its claim and the length of its proof, the
//! form the signature hash already gives it, so neither what was signed nor what
//! the payer is charged changes. A node that imports the block rebuilds the claims
//! from the block's stripped frames and checks the proof against exactly them.

use bytes::Bytes;
use ethrex_common::{
    H256,
    errors::InvalidBlockError,
    types::{
        Block, ChainConfig, LEANSPHINCS_VERIFIER_PROOF_LENGTH, MAX_AGGREGATE_PROOF_BYTES,
        Transaction, validate_aggregate_proof_commitment,
    },
};
use ethrex_crypto::{
    NativeCrypto,
    leansphincs::LEANSPHINCS_SIGNATURE_SIZE,
    leansphincs_aggregation::{
        LeanSphincsAggregationError, LeanSphincsClaim, leansphincs_aggregate,
        leansphincs_aggregate_verify, leansphincs_aggregation_setup,
    },
};

// A stripped proof is one signature of the scheme the aggregation proves.
const _: () = assert!(LEANSPHINCS_VERIFIER_PROOF_LENGTH as usize == LEANSPHINCS_SIGNATURE_SIZE);

/// Why [`strip_and_aggregate`] returned no proof.
#[derive(Debug, thiserror::Error)]
pub enum AggregateProofError {
    #[error("transaction {0} already has a stripped proof frame, whose signature is not at hand")]
    AlreadyStripped(usize),
    #[error(transparent)]
    Aggregation(#[from] LeanSphincsAggregationError),
    #[error("the proof has {0} bytes, more than MAX_AGGREGATE_PROOF_BYTES")]
    TooLarge(usize),
}

/// Builds the prover's program and starts its threads, which the first proof
/// does otherwise. A node calls this when it starts to build a payload from
/// IStar, so the first payload it has to prove does not wait for it.
pub fn prepare_prover() {
    leansphincs_aggregation_setup();
}

/// Whether the proof of any proof frame of `tx` was stripped.
fn has_stripped_proofs(tx: &Transaction) -> bool {
    matches!(tx, Transaction::FrameTransaction(frame_tx) if frame_tx.has_stripped_proofs())
}

/// Aggregates leanSPHINCS signatures into one aggregate proof. Each signature
/// comes with the `(key, msg)` claim it proves, and the claims may repeat. Without
/// signatures the proof is empty.
///
/// Proving takes from a tenth of a second to several seconds and runs one proof
/// at a time in the process.
pub fn aggregate_signatures(
    signatures: &[((H256, H256), Bytes)],
) -> Result<Bytes, AggregateProofError> {
    if signatures.is_empty() {
        return Ok(Bytes::new());
    }
    let mut signatures: Vec<(LeanSphincsClaim, &[u8])> = signatures
        .iter()
        .map(|((key, msg), signature)| ((key.0, msg.0), signature.as_ref()))
        .collect();
    // A claim that several frames make is proven once, by the first of its
    // signatures: the proof is checked against the claims as a set.
    signatures.sort_by_key(|(claim, _)| *claim);
    signatures.dedup_by_key(|(claim, _)| *claim);
    let proof = leansphincs_aggregate(&signatures)?;
    if proof.len() > MAX_AGGREGATE_PROOF_BYTES {
        return Err(AggregateProofError::TooLarge(proof.len()));
    }
    Ok(Bytes::from(proof))
}

/// Strips every proof the protocol may strip from `transactions` and aggregates
/// the stripped signatures. Returns the transactions as the block carries them
/// and the block's aggregate proof, which is empty when no proof was stripped.
///
/// A builder calls this once, for the payload it hands out, and holds no lock
/// while it runs.
pub fn strip_and_aggregate(
    transactions: &[Transaction],
) -> Result<(Vec<Transaction>, Bytes), AggregateProofError> {
    if let Some(index) = transactions.iter().position(has_stripped_proofs) {
        return Err(AggregateProofError::AlreadyStripped(index));
    }
    let mut transactions = transactions.to_vec();
    let mut stripped = Vec::new();
    for tx in &mut transactions {
        if let Transaction::FrameTransaction(frame_tx) = tx {
            stripped.extend(frame_tx.strip_proofs());
        }
    }
    let proof = aggregate_signatures(&stripped)?;
    Ok((transactions, proof))
}

/// Block validity for the aggregate proof (EIP-8141 §Aggregation), checked
/// before the block executes. From IStar:
///
/// 1. the block carries an aggregate proof of at most
///    `MAX_AGGREGATE_PROOF_BYTES` bytes;
/// 2. the header's `aggregate_proof_hash` is the hash of the proof;
/// 3. every stripped proof frame is one the protocol may strip;
/// 4. the proof is empty when the block has no stripped proof frame, and
///    otherwise it proves exactly the claims of the block's stripped frames.
///
/// Before IStar a block carries no aggregate proof and no stripped proof frame.
/// The rule is keyed on the activation time of IStar, as the header's fields
/// are: a chain that schedules LStar without IStar never has an aggregate proof.
/// Proof frames that keep their proof are not looked at here: executing them
/// verifies them, at any fork.
pub fn validate_aggregate_proof(
    block: &Block,
    chain_config: &ChainConfig,
) -> Result<(), InvalidBlockError> {
    let transactions = &block.body.transactions;
    if !chain_config.is_istar_activated(block.header.timestamp) {
        if block.body.aggregate_proof.is_some() {
            return Err(InvalidBlockError::AggregateProofPresent);
        }
        if let Some(index) = transactions.iter().position(has_stripped_proofs) {
            return Err(InvalidBlockError::StrippedProofFramePresent(index));
        }
        return Ok(());
    }
    let Some(proof) = &block.body.aggregate_proof else {
        return Err(InvalidBlockError::AggregateProofNotPresent);
    };
    validate_aggregate_proof_commitment(&block.header, &block.body, &NativeCrypto)
        .map_err(InvalidBlockError::InvalidBody)?;

    let mut claims: Vec<LeanSphincsClaim> = Vec::new();
    for (index, tx) in transactions.iter().enumerate() {
        let Transaction::FrameTransaction(frame_tx) = tx else {
            continue;
        };
        frame_tx
            .validate_stripped_proofs()
            .map_err(|reason| InvalidBlockError::InvalidStrippedProofFrame(index, reason))?;
        claims.extend(
            frame_tx
                .stripped_claims()
                .into_iter()
                .map(|(key, msg)| (key.0, msg.0)),
        );
    }
    if claims.is_empty() {
        if !proof.is_empty() {
            return Err(InvalidBlockError::AggregateProofNotEmpty(proof.len()));
        }
        return Ok(());
    }
    // The verifier sorts and deduplicates the claims, and accepts the proof only
    // for exactly that set: a claim fewer, a claim more or another claim fails.
    if !leansphincs_aggregate_verify(&claims, proof) {
        return Err(InvalidBlockError::InvalidAggregateProof(claims.len()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethrex_common::types::{
        BlockBody, BlockHeader, InvalidBlockBodyError, compute_aggregate_proof_hash,
    };

    const HEGOTA: u64 = 1_000;
    const ISTAR: u64 = 2_000;
    const LSTAR: u64 = 3_000;

    fn config() -> ChainConfig {
        ChainConfig {
            hegota_time: Some(HEGOTA),
            istar_time: Some(ISTAR),
            ..Default::default()
        }
    }

    /// A block without transactions at `timestamp`, with the given aggregate
    /// proof and the header committing to it.
    fn block(timestamp: u64, proof: Option<Bytes>) -> Block {
        Block::new(
            BlockHeader {
                timestamp,
                aggregate_proof_hash: proof
                    .as_ref()
                    .map(|proof| compute_aggregate_proof_hash(proof, &NativeCrypto)),
                ..Default::default()
            },
            BlockBody {
                aggregate_proof: proof,
                ..BlockBody::empty()
            },
        )
    }

    #[test]
    fn a_block_without_stripped_frames_is_proven_by_the_empty_proof() {
        assert!(validate_aggregate_proof(&block(ISTAR, Some(Bytes::new())), &config()).is_ok());
        let result =
            validate_aggregate_proof(&block(ISTAR, Some(Bytes::from(vec![1u8; 7]))), &config());
        assert!(
            matches!(result, Err(InvalidBlockError::AggregateProofNotEmpty(7))),
            "got {result:?}"
        );
    }

    #[test]
    fn an_istar_block_must_carry_its_aggregate_proof_and_commit_to_it() {
        let result = validate_aggregate_proof(&block(ISTAR, None), &config());
        assert!(
            matches!(result, Err(InvalidBlockError::AggregateProofNotPresent)),
            "got {result:?}"
        );
        // The proof without the header's commitment, and with a wrong one.
        let mut uncommitted = block(ISTAR, Some(Bytes::new()));
        uncommitted.header.aggregate_proof_hash = None;
        let mut miscommitted = block(ISTAR, Some(Bytes::new()));
        miscommitted.header.aggregate_proof_hash = Some(H256::repeat_byte(0xab));
        for invalid in [uncommitted, miscommitted] {
            let result = validate_aggregate_proof(&invalid, &config());
            assert!(
                matches!(
                    result,
                    Err(InvalidBlockError::InvalidBody(
                        InvalidBlockBodyError::AggregateProofHashNotMatch
                    ))
                ),
                "got {result:?}"
            );
        }
        // A proof over the bound, before anything reads it.
        let oversized = Bytes::from(vec![0u8; MAX_AGGREGATE_PROOF_BYTES + 1]);
        let result = validate_aggregate_proof(&block(ISTAR, Some(oversized)), &config());
        assert!(
            matches!(
                result,
                Err(InvalidBlockError::InvalidBody(
                    InvalidBlockBodyError::AggregateProofTooLarge(_)
                ))
            ),
            "got {result:?}"
        );
    }

    #[test]
    fn a_block_before_istar_carries_no_aggregate_proof() {
        for timestamp in [HEGOTA, ISTAR - 1] {
            assert!(validate_aggregate_proof(&block(timestamp, None), &config()).is_ok());
            let result = validate_aggregate_proof(&block(timestamp, Some(Bytes::new())), &config());
            assert!(
                matches!(result, Err(InvalidBlockError::AggregateProofPresent)),
                "got {result:?}"
            );
        }
        // Nor on a chain that never schedules IStar.
        let without_istar = ChainConfig {
            hegota_time: Some(HEGOTA),
            ..Default::default()
        };
        let result = validate_aggregate_proof(&block(ISTAR, Some(Bytes::new())), &without_istar);
        assert!(
            matches!(result, Err(InvalidBlockError::AggregateProofPresent)),
            "got {result:?}"
        );
    }

    #[test]
    fn an_lstar_block_carries_its_aggregate_proof_too() {
        // LStar follows IStar: its blocks keep the aggregate proof.
        let config = ChainConfig {
            lstar_time: Some(LSTAR),
            ..config()
        };
        for timestamp in [ISTAR, LSTAR - 1, LSTAR, LSTAR + 1] {
            assert!(
                validate_aggregate_proof(&block(timestamp, Some(Bytes::new())), &config).is_ok()
            );
            let result = validate_aggregate_proof(&block(timestamp, None), &config);
            assert!(
                matches!(result, Err(InvalidBlockError::AggregateProofNotPresent)),
                "got {result:?}"
            );
        }
    }

    #[test]
    fn a_chain_that_schedules_lstar_without_istar_carries_no_aggregate_proof() {
        // The aggregate proof belongs to IStar. A chain that goes from Hegota to
        // LStar has LStar's own rules and no aggregate proof at any time.
        let config = ChainConfig {
            hegota_time: Some(HEGOTA),
            lstar_time: Some(LSTAR),
            ..Default::default()
        };
        for timestamp in [HEGOTA, LSTAR - 1, LSTAR, LSTAR + 1] {
            assert!(validate_aggregate_proof(&block(timestamp, None), &config).is_ok());
            let result = validate_aggregate_proof(&block(timestamp, Some(Bytes::new())), &config);
            assert!(
                matches!(result, Err(InvalidBlockError::AggregateProofPresent)),
                "got {result:?}"
            );
        }
    }

    #[test]
    fn a_chain_that_schedules_istar_without_hegota_still_requires_the_proof() {
        // The rule is keyed on the activation time of IStar alone: with IStar
        // scheduled, a block at its timestamp is an IStar block whether or not
        // Hegota has a timestamp of its own.
        let config = ChainConfig {
            istar_time: Some(ISTAR),
            ..Default::default()
        };
        assert!(validate_aggregate_proof(&block(ISTAR, Some(Bytes::new())), &config).is_ok());
        let result = validate_aggregate_proof(&block(ISTAR, None), &config);
        assert!(
            matches!(result, Err(InvalidBlockError::AggregateProofNotPresent)),
            "got {result:?}"
        );
    }

    #[test]
    fn nothing_to_strip_gives_the_empty_proof() {
        let (transactions, proof) = strip_and_aggregate(&[]).expect("nothing to prove");
        assert!(transactions.is_empty());
        assert!(proof.is_empty());
        assert!(
            aggregate_signatures(&[])
                .expect("nothing to prove")
                .is_empty()
        );
    }
}
