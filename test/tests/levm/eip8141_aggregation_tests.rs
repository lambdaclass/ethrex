//! EIP-8141 claims across IStar, in the VM and in the verification of the
//! aggregate proof.
//!
//! From IStar a block carries one aggregate proof in place of the witnesses of
//! its leanSPHINCS dependencies (EIP-8141 §Aggregation and EIP-8288
//! compatibility). The transaction does not change, so neither does what it is
//! charged: every charge of a claim follows from the transaction body. The
//! proof holds for exactly the dependencies it was made for.

use std::sync::Arc;

use bytes::Bytes;
use ethrex_blockchain::claim_witnesses::{ClaimAggregationError, aggregate_claim_witnesses};
use ethrex_common::types::{
    Account, Claim, Code, Dependency, FRAME_RECEIPT_STATUS_SUCCESS, Fork, Frame, FrameMode,
    FrameTransaction, LEANSPHINCS_SIGNATURE_SIZE, LEANSPHINCS_VERIFICATION_GAS,
    LEANSPHINCS_VERIFIER, SECP256K1_VERIFIER, Transaction, WITNESS_BYTE_GAS,
};
use ethrex_common::{Address, H256, U256};
use ethrex_crypto::NativeCrypto;
use ethrex_levm::claims::verify_aggregated_dependencies;
use ethrex_levm::db::gen_db::GeneralizedDatabase;
use ethrex_levm::environment::{EVMConfig, Environment};
use ethrex_levm::tracing::LevmCallTracer;
use ethrex_levm::vm::{VM, VMType};
use rustc_hash::FxHashMap;

use super::test_db::TestDatabase;

const CHAIN_ID: u64 = 1;
const BASE_FEE: u64 = 1;
const SENDER: Address = Address::repeat_byte(0xA1);
const BURNER: Address = Address::repeat_byte(0xD4);

/// `APPROVE(scope 3)`: approves execution and payment whatever the transaction.
const APPROVE_BOTH_CODE: &[u8] = &[0x60, 0x03, 0x5f, 0x5f, 0xaa];

fn account(balance: U256, code: &[u8]) -> Account {
    Account::new(
        balance,
        Code::from_bytecode(Bytes::copy_from_slice(code), &NativeCrypto),
        0,
        FxHashMap::default(),
    )
}

/// The gas a transaction with `claims` is charged at `fork`, over a sender that
/// approves anything and a frame that burns enough execution gas for the
/// calldata floor not to bind.
fn gas_used(fork: Fork, claims: Vec<Claim>) -> u64 {
    let mut burner_code = vec![0x5b; 20_000];
    burner_code.push(0x00);
    let tx = FrameTransaction {
        chain_id: CHAIN_ID,
        sender: SENDER,
        claims,
        frames: vec![
            Frame {
                mode: u8::from(FrameMode::Verify),
                flags: 0x03,
                target: None,
                gas_limit: 50_000,
                ..Default::default()
            },
            Frame {
                mode: u8::from(FrameMode::Sender),
                target: Some(BURNER),
                gas_limit: 200_000,
                state_gas_limit: 1_000_000,
                ..Default::default()
            },
        ],
        max_fee: U256::from(10u64).pow(U256::from(17u64)),
        max_priority_fee_per_gas: U256::one(),
        ..Default::default()
    };
    assert!(
        tx.calldata_floor_total() < tx.mandatory_gas() + tx.data_cost() + 20_000,
        "the floor must not bind for the difference to be the claim's cost"
    );
    let mut db = GeneralizedDatabase::new_with_account_state(
        Arc::new(TestDatabase::new()),
        [
            (
                SENDER,
                account(U256::from(10u64).pow(U256::from(18u64)), APPROVE_BOTH_CODE),
            ),
            (BURNER, account(U256::zero(), &burner_code)),
        ]
        .into_iter()
        .collect(),
    );
    let env = Environment {
        origin: tx.sender,
        gas_limit: tx.max_gas(),
        block_gas_limit: (i64::MAX - 1) as u64,
        config: EVMConfig::new(fork, EVMConfig::canonical_values(fork)),
        chain_id: U256::from(CHAIN_ID),
        base_fee_per_gas: U256::from(BASE_FEE),
        gas_price: tx
            .fee_budget(U256::from(BASE_FEE), U256::zero())
            .map_or(U256::from(BASE_FEE), |budget| budget.gas_price),
        tx_nonce: tx.nonce_seq,
        ..Default::default()
    };
    let transaction = Transaction::FrameTransaction(tx);
    let report = VM::new(
        env,
        &mut db,
        &transaction,
        LevmCallTracer::disabled(),
        VMType::L1,
        &NativeCrypto,
        None,
    )
    .expect("VM::new should succeed for a frame transaction")
    .execute()
    .expect("valid");
    let frames = report.frame_results.as_ref().expect("frame results");
    assert_eq!(frames[1].0, FRAME_RECEIPT_STATUS_SUCCESS);
    report.gas_used
}

// EIP-8141 derives every charge of a claim from the transaction body, and names
// a later discount for aggregated witnesses without defining it. So a
// leanSPHINCS claim pays in an IStar block, where one proof stands for its
// witness, what it pays in a Hegota block, which carries the witness: its
// verification, its witness bytes and its metadata. LStar, the fork after
// IStar, charges the same.
#[test]
fn a_leansphincs_claim_is_charged_the_same_gas_before_and_after_istar() {
    let leansphincs = Claim {
        verifier: LEANSPHINCS_VERIFIER,
        key: Bytes::from(vec![0x06; 32]),
        ..Default::default()
    };
    // The verifier's address as calldata, 19 zero bytes and one that is not,
    // and the 32 bytes of the key.
    let metadata = 19 * 4 + 16 + 32 * 16;
    let claim_gas =
        LEANSPHINCS_VERIFICATION_GAS + WITNESS_BYTE_GAS * LEANSPHINCS_SIGNATURE_SIZE as u64;
    assert_eq!(claim_gas + metadata, 44_764);

    let with_claim = |fork: Fork| gas_used(fork, vec![leansphincs.clone()]);
    assert_eq!(with_claim(Fork::Hegota), with_claim(Fork::IStar));
    assert_eq!(with_claim(Fork::IStar), with_claim(Fork::LStar));
    for fork in [Fork::Hegota, Fork::IStar, Fork::LStar] {
        assert_eq!(
            with_claim(fork) - gas_used(fork, Vec::new()),
            claim_gas + metadata
        );
    }
}

/// A leanSPHINCS dependency for a signature of `msg` under the key derived from
/// `seed`, with that signature.
fn signed_dependency(seed: u8, msg: H256) -> (Dependency, Bytes) {
    let secret = [seed; 32];
    let dependency = Dependency {
        verifier: LEANSPHINCS_VERIFIER,
        key: H256(lean_sphincs::key_gen(&secret).to_bytes()),
        msg,
        gas_limit: LEANSPHINCS_VERIFICATION_GAS,
        witness_len: LEANSPHINCS_SIGNATURE_SIZE as u64,
    };
    let witness = lean_sphincs::sign(&secret, msg.as_fixed_bytes())
        .to_bytes()
        .to_vec();
    (dependency, Bytes::from(witness))
}

#[test]
fn an_aggregate_proof_satisfies_exactly_the_dependencies_it_was_made_for() {
    let signed: Vec<_> = (1..=3u8)
        .map(|seed| signed_dependency(seed, H256::repeat_byte(0x10 + seed)))
        .collect();
    let dependencies: Vec<Dependency> = signed.iter().map(|(dependency, _)| *dependency).collect();
    let proof = aggregate_claim_witnesses(&signed).expect("valid signatures aggregate");
    assert!(!proof.is_empty());

    assert!(verify_aggregated_dependencies(&dependencies, &proof));
    // The order of the dependencies is not part of what the proof states.
    let reversed: Vec<Dependency> = dependencies.iter().rev().copied().collect();
    assert!(verify_aggregated_dependencies(&reversed, &proof));

    // One dependency fewer, and one more.
    assert!(!verify_aggregated_dependencies(&dependencies[..2], &proof));
    let mut extra = dependencies.clone();
    extra.push(signed_dependency(4, H256::repeat_byte(0x14)).0);
    assert!(!verify_aggregated_dependencies(&extra, &proof));
    // Another message under the same key, and the same message under another key.
    let mut other_message = dependencies.clone();
    other_message[0].msg = H256::repeat_byte(0x99);
    assert!(!verify_aggregated_dependencies(&other_message, &proof));
    let mut other_key = dependencies.clone();
    other_key[0].key = signed_dependency(4, other_key[0].msg).0.key;
    assert!(!verify_aggregated_dependencies(&other_key, &proof));

    // Nothing proves an empty list, and no bytes but the proof's prove the list.
    assert!(!verify_aggregated_dependencies(&[], &proof));
    assert!(!verify_aggregated_dependencies(&dependencies, &[]));
    assert!(!verify_aggregated_dependencies(
        &dependencies,
        &proof[..proof.len() / 2]
    ));
    let mut tampered = proof.to_vec();
    let last = tampered.len() - 1;
    tampered[last] ^= 0x01;
    assert!(!verify_aggregated_dependencies(&dependencies, &tampered));
}

// The proof stands for leanSPHINCS signatures and nothing else: a dependency of
// any other verifier is neither aggregated nor satisfied by it.
#[test]
fn only_leansphincs_dependencies_are_aggregated() {
    let signed = signed_dependency(5, H256::repeat_byte(0x15));
    let proof =
        aggregate_claim_witnesses(std::slice::from_ref(&signed)).expect("a valid signature");
    assert!(verify_aggregated_dependencies(&[signed.0], &proof));

    // The same key and message under the secp256k1 verifier.
    let secp256k1 = Dependency {
        verifier: SECP256K1_VERIFIER,
        ..signed.0
    };
    assert!(!verify_aggregated_dependencies(&[secp256k1], &proof));
    assert!(!verify_aggregated_dependencies(
        &[signed.0, secp256k1],
        &proof
    ));
    let result = aggregate_claim_witnesses(&[(secp256k1, signed.1.clone())]);
    assert!(
        matches!(result, Err(ClaimAggregationError::UnsupportedVerifier)),
        "got {result:?}"
    );

    // A signature that is not valid has no proof, and no dependencies have the
    // empty one.
    let mut invalid = signed.1.to_vec();
    invalid[40] ^= 0x01;
    let result = aggregate_claim_witnesses(&[(signed.0, Bytes::from(invalid))]);
    assert!(
        matches!(result, Err(ClaimAggregationError::Aggregation(_))),
        "got {result:?}"
    );
    assert_eq!(
        aggregate_claim_witnesses(&[]).expect("nothing to prove"),
        Bytes::new()
    );
}
