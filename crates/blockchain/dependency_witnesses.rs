//! EIP-8504 dependency witnesses: the signatures that satisfy the dependencies a
//! block's transactions declare in their proof frames.
//!
//! A dependency says that a 32-byte message is signed under a leanSPHINCS public
//! key. The signature is not part of the transaction: it travels beside it in the
//! mempool, and a block carries one for each of its distinct dependencies. Nothing
//! here executes code; the checks need only the block and the signatures.

use ethrex_common::{
    errors::InvalidBlockError,
    types::{Block, ChainConfig, Dependency, LEANSPHINCS_SCHEME, block_dependencies},
};
use ethrex_crypto::leansphincs::{LEANSPHINCS_SIGNATURE_SIZE, leansphincs_verify};
use ethrex_metrics::metrics;

#[cfg(feature = "metrics")]
use ethrex_metrics::eip8504::METRICS_EIP8504;

// The witness size the transaction types price and the one the verifier accepts
// are the same consensus constant, declared in two crates.
const _: () =
    assert!(LEANSPHINCS_SIGNATURE_SIZE == ethrex_common::types::LEANSPHINCS_SIGNATURE_SIZE);

/// EIP-8504 `satisfies`: whether `witness` satisfies `dependency`, that is,
/// whether it is a valid leanSPHINCS signature of the dependency's message under
/// its public key. A dependency of any other scheme is never satisfied.
pub fn satisfies(dependency: &Dependency, witness: &[u8]) -> bool {
    if dependency.scheme != LEANSPHINCS_SCHEME {
        return false;
    }
    leansphincs_verify(
        dependency.verification_key_hash.as_fixed_bytes(),
        dependency.data_hash.as_fixed_bytes(),
        witness,
    )
}

/// Checks the dependency witnesses of a block before it executes: the block
/// carries exactly one witness for each entry of `block_dependencies`, in the
/// same order, and each witness satisfies its dependency.
///
/// EIP-8504 is part of Hegota. Before it a block has no frame transactions,
/// hence no dependencies, and no witness list, because the header rules reject
/// its commitment; there is nothing to check.
pub fn validate_dependency_witnesses(
    block: &Block,
    chain_config: &ChainConfig,
) -> Result<(), InvalidBlockError> {
    if !chain_config.is_hegota_activated(block.header.timestamp) {
        return Ok(());
    }
    #[cfg(feature = "metrics")]
    let started = std::time::Instant::now();
    let dependencies = block_dependencies(&block.body.transactions);
    let witnesses = block
        .body
        .dependency_witnesses
        .as_deref()
        .unwrap_or_default();
    if witnesses.len() != dependencies.len() {
        return Err(InvalidBlockError::DependencyWitnessesCountMismatch {
            expected: dependencies.len(),
            found: witnesses.len(),
        });
    }
    for (index, (dependency, witness)) in dependencies.iter().zip(witnesses).enumerate() {
        if !satisfies(dependency, witness) {
            return Err(InvalidBlockError::DependencyNotSatisfied(index));
        }
    }
    metrics!({
        let count = witnesses.len() as u64;
        let bytes = witnesses
            .iter()
            .map(|witness| witness.len() as u64)
            .sum::<u64>();
        METRICS_EIP8504.blocks_total.inc();
        METRICS_EIP8504.block_witnesses.set(count as i64);
        METRICS_EIP8504.block_witness_bytes.set(bytes as i64);
        METRICS_EIP8504.witnesses_total.inc_by(count);
        METRICS_EIP8504.witness_bytes_total.inc_by(bytes);
        METRICS_EIP8504
            .block_witness_verification_seconds
            .observe(started.elapsed().as_secs_f64());
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use ethrex_common::{
        Address, H256, U256,
        types::{
            BlockBody, BlockHeader, Frame, FrameMode, FrameTransaction,
            LEANSPHINCS_VERIFICATION_GAS, PROOF_FRAME_MODE, Transaction,
        },
    };

    const ACTIVATION: u64 = 1_000;

    fn active_config() -> ChainConfig {
        ChainConfig {
            hegota_time: Some(ACTIVATION),
            ..Default::default()
        }
    }

    struct Key {
        secret: [u8; 32],
        public: H256,
    }

    fn key(seed: u8) -> Key {
        let secret = [seed; 32];
        Key {
            secret,
            public: H256(lean_sphincs::key_gen(&secret).to_bytes()),
        }
    }

    fn sign(key: &Key, message: H256) -> Bytes {
        Bytes::from(
            lean_sphincs::sign(&key.secret, message.as_fixed_bytes())
                .to_bytes()
                .to_vec(),
        )
    }

    /// A frame transaction whose proof frame declares the given
    /// `(data_hash, public key)` pairs.
    fn dependency_tx(nonce: u64, declarations: &[(H256, H256)]) -> FrameTransaction {
        let mut data = Vec::new();
        for (data_hash, public_key) in declarations {
            data.extend_from_slice(&[0u8; 31]);
            data.push(LEANSPHINCS_SCHEME);
            data.extend_from_slice(data_hash.as_bytes());
            data.extend_from_slice(public_key.as_bytes());
        }
        FrameTransaction {
            chain_id: 1,
            nonce_seq: nonce,
            sender: Address::repeat_byte(0xAA),
            frames: vec![
                Frame {
                    mode: FrameMode::Verify as u8,
                    flags: 0x03,
                    target: None,
                    gas_limit: 50_000,
                    state_gas_limit: 0,
                    value: U256::zero(),
                    data: Bytes::new(),
                },
                Frame {
                    mode: PROOF_FRAME_MODE,
                    flags: 0,
                    target: None,
                    gas_limit: declarations.len() as u64 * LEANSPHINCS_VERIFICATION_GAS,
                    state_gas_limit: 0,
                    value: U256::zero(),
                    data: Bytes::from(data),
                },
            ],
            ..Default::default()
        }
    }

    fn block(transactions: Vec<FrameTransaction>, witnesses: Option<Vec<Bytes>>) -> Block {
        Block::new(
            BlockHeader {
                timestamp: ACTIVATION,
                ..Default::default()
            },
            BlockBody {
                transactions: transactions
                    .into_iter()
                    .map(Transaction::FrameTransaction)
                    .collect(),
                ommers: Vec::new(),
                withdrawals: Some(Vec::new()),
                dependency_witnesses: witnesses,
            },
        )
    }

    /// Two transactions from two keys. Each declares its own signature hash under
    /// its key, and both declare the same explicit message under the first key,
    /// so the block has three dependencies for four declarations.
    fn two_transactions() -> (Vec<FrameTransaction>, Vec<Bytes>) {
        let first = key(1);
        let second = key(2);
        let shared = H256::repeat_byte(0x07);
        let tx_a = dependency_tx(0, &[(H256::zero(), first.public), (shared, first.public)]);
        let tx_b = dependency_tx(1, &[(shared, first.public), (H256::zero(), second.public)]);

        let transactions = vec![tx_a, tx_b];
        let dependencies = block_dependencies(
            &transactions
                .iter()
                .cloned()
                .map(Transaction::FrameTransaction)
                .collect::<Vec<_>>(),
        );
        assert_eq!(dependencies.len(), 3);
        let witnesses = dependencies
            .iter()
            .map(|dependency| {
                let signer = if dependency.verification_key_hash == first.public {
                    &first
                } else {
                    &second
                };
                sign(signer, dependency.data_hash)
            })
            .collect();
        (transactions, witnesses)
    }

    #[test]
    fn a_block_with_one_valid_witness_per_dependency_is_accepted() {
        let (transactions, witnesses) = two_transactions();
        validate_dependency_witnesses(&block(transactions, Some(witnesses)), &active_config())
            .expect("every dependency is satisfied");
    }

    #[test]
    fn a_block_without_dependencies_carries_no_witnesses() {
        let empty = block(Vec::new(), Some(Vec::new()));
        validate_dependency_witnesses(&empty, &active_config()).expect("nothing to satisfy");

        let stray = block(Vec::new(), Some(vec![Bytes::from(vec![0u8; 7740])]));
        assert!(matches!(
            validate_dependency_witnesses(&stray, &active_config()),
            Err(InvalidBlockError::DependencyWitnessesCountMismatch {
                expected: 0,
                found: 1
            })
        ));
    }

    #[test]
    fn a_missing_or_extra_witness_invalidates_the_block() {
        let (transactions, witnesses) = two_transactions();

        let mut missing = witnesses.clone();
        missing.pop();
        assert!(matches!(
            validate_dependency_witnesses(
                &block(transactions.clone(), Some(missing)),
                &active_config()
            ),
            Err(InvalidBlockError::DependencyWitnessesCountMismatch {
                expected: 3,
                found: 2
            })
        ));

        let mut extra = witnesses.clone();
        extra.push(witnesses[0].clone());
        assert!(matches!(
            validate_dependency_witnesses(
                &block(transactions.clone(), Some(extra)),
                &active_config()
            ),
            Err(InvalidBlockError::DependencyWitnessesCountMismatch {
                expected: 3,
                found: 4
            })
        ));

        assert!(matches!(
            validate_dependency_witnesses(&block(transactions, None), &active_config()),
            Err(InvalidBlockError::DependencyWitnessesCountMismatch {
                expected: 3,
                found: 0
            })
        ));
    }

    #[test]
    fn witnesses_out_of_order_or_tampered_invalidate_the_block() {
        let (transactions, witnesses) = two_transactions();

        let mut swapped = witnesses.clone();
        swapped.swap(0, 2);
        assert!(matches!(
            validate_dependency_witnesses(
                &block(transactions.clone(), Some(swapped)),
                &active_config()
            ),
            Err(InvalidBlockError::DependencyNotSatisfied(0))
        ));

        let mut tampered = witnesses.clone();
        let mut bytes = tampered[1].to_vec();
        bytes[100] ^= 1;
        tampered[1] = Bytes::from(bytes);
        assert!(matches!(
            validate_dependency_witnesses(
                &block(transactions.clone(), Some(tampered)),
                &active_config()
            ),
            Err(InvalidBlockError::DependencyNotSatisfied(1))
        ));

        let mut truncated = witnesses;
        truncated[2] = truncated[2].slice(..7739);
        assert!(matches!(
            validate_dependency_witnesses(&block(transactions, Some(truncated)), &active_config()),
            Err(InvalidBlockError::DependencyNotSatisfied(2))
        ));
    }

    #[test]
    fn a_dependency_of_another_scheme_is_never_satisfied() {
        let signer = key(3);
        let message = H256::repeat_byte(0x09);
        let witness = sign(&signer, message);
        let mut dependency = Dependency {
            scheme: LEANSPHINCS_SCHEME,
            data_hash: message,
            verification_key_hash: signer.public,
        };
        assert!(satisfies(&dependency, &witness));
        dependency.scheme = 0x11;
        assert!(!satisfies(&dependency, &witness));
    }

    #[test]
    fn nothing_is_checked_before_hegota() {
        let (transactions, _) = two_transactions();
        let mut early = block(transactions, None);
        early.header.timestamp = ACTIVATION - 1;
        validate_dependency_witnesses(&early, &active_config())
            .expect("the header rules cover a chain before Hegota");
    }
}
