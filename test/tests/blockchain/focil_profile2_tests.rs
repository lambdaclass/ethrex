//! FOCIL Profile 2, the stateless half: the consensus constants, candidacy
//! (decided from the transaction bytes alone), `verify_budget_cost`, and the
//! per-list two-stage budget fill. Nothing here opens state or runs the EVM.
//!
//! Case numbers refer to the Test Cases table of `docs/eip-focil-frametx.md`.

use bytes::Bytes;
use ethrex_blockchain::focil_profile2::{
    MAX_VALIDATION_CODE_BODIES, MAX_VERIFY_GAS_PER_IL, MAX_VERIFY_GAS_PER_TX, NotProfile2Candidate,
    budget_fill, max_validation_code_bytes, prefix_and_verifier_frame_cost, profile2_candidate,
    verify_budget_cost,
};
use ethrex_common::types::{
    ChainConfig, EIP1559Transaction, FRAME_SIG_SCHEME_SECP256K1, FRAME_TX_RECENT_ROOT_TUPLE_BYTES,
    Fork, Frame, FrameMode, FrameSignature, FrameTransaction, Transaction, TxKind,
    frame_tx_expiry_verifier, frame_tx_recent_root,
};
use ethrex_common::{Address, U256};
use ethrex_crypto::NativeCrypto;

const SENDER: Address = Address::repeat_byte(0x5E);
const PAYMASTER: Address = Address::repeat_byte(0xFA);
/// EIP-8141 prices one SECP256K1 signature at this.
const SECP256K1_SIGNATURE_GAS: u64 = 2800;

fn frame(mode: FrameMode, flags: u8, target: Option<Address>, gas_limit: u64) -> Frame {
    Frame {
        mode: mode as u8,
        flags,
        target,
        gas_limit,
        state_gas_limit: 0,
        value: U256::zero(),
        data: Bytes::new(),
    }
}

fn self_verify(gas_limit: u64) -> Frame {
    frame(FrameMode::Verify, 0x03, Some(SENDER), gas_limit)
}

fn only_verify(gas_limit: u64) -> Frame {
    frame(FrameMode::Verify, 0x02, Some(SENDER), gas_limit)
}

fn pay(target: Option<Address>, gas_limit: u64) -> Frame {
    frame(FrameMode::Verify, 0x01, target, gas_limit)
}

fn deploy(gas_limit: u64) -> Frame {
    frame(
        FrameMode::Default,
        0x00,
        Some(Address::repeat_byte(0xDE)),
        gas_limit,
    )
}

fn body(gas_limit: u64) -> Frame {
    frame(
        FrameMode::Sender,
        0x00,
        Some(Address::repeat_byte(0xB0)),
        gas_limit,
    )
}

fn expiry_frame(gas_limit: u64) -> Frame {
    Frame {
        data: Bytes::from(vec![0xFF; 8]),
        ..frame(
            FrameMode::Verify,
            0x00,
            Some(frame_tx_expiry_verifier()),
            gas_limit,
        )
    }
}

fn recent_root_frame(gas_limit: u64) -> Frame {
    Frame {
        data: Bytes::from(vec![0x11; FRAME_TX_RECENT_ROOT_TUPLE_BYTES]),
        ..frame(
            FrameMode::Verify,
            0x00,
            Some(frame_tx_recent_root()),
            gas_limit,
        )
    }
}

fn frame_tx(frames: Vec<Frame>) -> FrameTransaction {
    FrameTransaction {
        chain_id: 1,
        nonce_keys: vec![U256::zero()],
        nonce_seq: 0,
        sender: SENDER,
        frames,
        signatures: Vec::new(),
        max_priority_fee_per_gas: U256::zero(),
        max_fee_per_gas: U256::from(1u64),
        ..Default::default()
    }
}

/// A structurally valid SECP256K1 signature entry that does not verify.
fn bad_signature() -> FrameSignature {
    FrameSignature {
        scheme: FRAME_SIG_SCHEME_SECP256K1,
        signer: Some(SENDER),
        msg: Bytes::new(),
        signature: Bytes::from(vec![0u8; 65]),
    }
}

fn listed(tx: FrameTransaction) -> Transaction {
    Transaction::FrameTransaction(tx)
}

fn amsterdam_config() -> ChainConfig {
    ChainConfig {
        amsterdam_time: Some(0),
        hegota_time: Some(0),
        ..Default::default()
    }
}

/// The constants table, pinned. Consensus values, never node configuration.
#[test]
fn constants_match_the_spec_table() {
    assert_eq!(MAX_VERIFY_GAS_PER_IL, 1 << 20);
    assert_eq!(MAX_VERIFY_GAS_PER_TX, MAX_VERIFY_GAS_PER_IL);
    assert_eq!(MAX_VALIDATION_CODE_BODIES, 16);
    // MAX_VALIDATION_CODE_BYTES = MAX_VALIDATION_CODE_BODIES * MAX_CODE_SIZE, with
    // the code size of the active fork.
    assert_eq!(
        max_validation_code_bytes(&amsterdam_config(), 0),
        16 * 0x10000
    );
    assert_eq!(
        max_validation_code_bytes(&ChainConfig::default(), 0),
        16 * 0x6000
    );
}

#[test]
fn self_verify_is_a_candidate_paying_for_itself() {
    let candidate = profile2_candidate(&frame_tx(vec![self_verify(50_000), body(10_000)]))
        .expect("self_verify is a candidate");
    assert_eq!(candidate.payer, SENDER);
    assert_eq!(candidate.verify_budget_cost, 50_000);
    assert_eq!(candidate.prefix.frame_indices, vec![0]);
}

#[test]
fn pay_frame_target_is_the_payer_and_a_null_target_is_the_sender() {
    let sponsored = profile2_candidate(&frame_tx(vec![
        only_verify(40_000),
        pay(Some(PAYMASTER), 30_000),
    ]))
    .expect("only_verify | pay is a candidate");
    assert_eq!(sponsored.payer, PAYMASTER);
    assert_eq!(sponsored.verify_budget_cost, 70_000);

    let self_paid = profile2_candidate(&frame_tx(vec![only_verify(40_000), pay(None, 30_000)]))
        .expect("a null pay target is admitted");
    assert_eq!(self_paid.payer, SENDER);
}

/// Cases 15 and 16: the protocol verifier frames are disregarded by shape
/// matching but their declared gas counts, because replay executes them. A
/// transaction that fits the cap without them and not with them is not a
/// candidate.
#[test]
fn verify_budget_cost_counts_the_expiry_and_recent_root_frames() {
    let with_expiry = frame_tx(vec![expiry_frame(20_000), self_verify(30_000)]);
    assert_eq!(prefix_and_verifier_frame_cost(&with_expiry), Some(50_000));
    assert_eq!(verify_budget_cost(&with_expiry), Some(50_000));

    let with_both = frame_tx(vec![
        expiry_frame(20_000),
        recent_root_frame(25_000),
        self_verify(30_000),
    ]);
    assert_eq!(verify_budget_cost(&with_both), Some(75_000));

    let just_fits = frame_tx(vec![self_verify(MAX_VERIFY_GAS_PER_TX)]);
    assert!(profile2_candidate(&just_fits).is_ok());

    // Case 15: over the cap only once the expiry frame's gas is counted.
    let over_with_expiry = frame_tx(vec![
        expiry_frame(20_000),
        self_verify(MAX_VERIFY_GAS_PER_TX - 10_000),
    ]);
    assert_eq!(
        profile2_candidate(&over_with_expiry),
        Err(NotProfile2Candidate::BudgetCostExceeded {
            cost: MAX_VERIFY_GAS_PER_TX + 10_000,
            limit: MAX_VERIFY_GAS_PER_TX,
        })
    );
    // Case 16: the same, for the recent-root verifier frame.
    let over_with_recent_root = frame_tx(vec![
        recent_root_frame(20_000),
        self_verify(MAX_VERIFY_GAS_PER_TX - 10_000),
    ]);
    assert!(matches!(
        profile2_candidate(&over_with_recent_root),
        Err(NotProfile2Candidate::BudgetCostExceeded { .. })
    ));
}

/// The signature cost is intrinsic and part of the budget.
#[test]
fn verify_budget_cost_includes_the_signature_verification_cost() {
    let mut tx = frame_tx(vec![self_verify(30_000)]);
    tx.signatures = vec![bad_signature()];
    assert_eq!(
        verify_budget_cost(&tx),
        Some(30_000 + SECP256K1_SIGNATURE_GAS)
    );
}

/// Case 12: a frame transaction carrying blobs is outside enforcement.
#[test]
fn blob_carrying_frame_tx_is_not_a_candidate() {
    let mut tx = frame_tx(vec![self_verify(30_000)]);
    tx.blob_versioned_hashes = vec![ethrex_common::H256::from([0x01; 32])];
    tx.max_fee_per_blob_gas = U256::from(1u64);
    assert_eq!(
        profile2_candidate(&tx),
        Err(NotProfile2Candidate::CarriesBlobs)
    );
}

/// Case 13: `ATOMIC_BATCH_FLAG` on a validation prefix frame. EIP-8141's static
/// constraints already forbid the flag on a VERIFY frame and on any frame
/// followed by one, so every prefix shape trips condition 1 before condition 5.
#[test]
fn atomic_batch_flag_on_a_prefix_frame_is_not_a_candidate() {
    let mut batched_deploy = deploy(20_000);
    batched_deploy.flags = 0x04;
    let tx = frame_tx(vec![batched_deploy, self_verify(30_000)]);
    assert!(matches!(
        profile2_candidate(&tx),
        Err(NotProfile2Candidate::StaticallyInvalid(_))
    ));
}

/// Case 14: a VERIFY-mode body frame. Replay never observes it, and its
/// failure would invalidate a transaction that replayed cleanly.
#[test]
fn verify_mode_body_frame_is_not_a_candidate() {
    let tx = frame_tx(vec![
        self_verify(30_000),
        body(10_000),
        frame(
            FrameMode::Verify,
            0x00,
            Some(Address::repeat_byte(0xB1)),
            5_000,
        ),
    ]);
    assert_eq!(
        profile2_candidate(&tx),
        Err(NotProfile2Candidate::VerifyBodyFrame { frame_index: 2 })
    );
}

/// Case 11: a frame mode EIP-8141 does not define. Static validity (condition
/// 1) rejects it before condition 7 is reached.
#[test]
fn undefined_frame_mode_is_not_a_candidate() {
    let mut reserved = body(10_000);
    reserved.mode = 3;
    let tx = frame_tx(vec![self_verify(30_000), reserved]);
    assert!(matches!(
        profile2_candidate(&tx),
        Err(NotProfile2Candidate::StaticallyInvalid(_))
    ));
}

/// Case 25: a recent-root verifier frame present but not in the leading
/// position, and an expiry verifier frame not first.
#[test]
fn misplaced_protocol_verifier_frames_are_not_candidates() {
    let trailing_recent_root = frame_tx(vec![self_verify(30_000), recent_root_frame(20_000)]);
    assert_eq!(
        profile2_candidate(&trailing_recent_root),
        Err(NotProfile2Candidate::ProtocolVerifierMisplaced { frame_index: 1 })
    );
    let trailing_expiry = frame_tx(vec![self_verify(30_000), expiry_frame(20_000)]);
    assert_eq!(
        profile2_candidate(&trailing_expiry),
        Err(NotProfile2Candidate::ProtocolVerifierMisplaced { frame_index: 1 })
    );
    // Reversed protocol verifiers: the recent-root frame must follow the expiry
    // frame, not precede it.
    let reversed = frame_tx(vec![
        recent_root_frame(20_000),
        expiry_frame(20_000),
        self_verify(30_000),
    ]);
    assert_eq!(
        profile2_candidate(&reversed),
        Err(NotProfile2Candidate::ProtocolVerifierMisplaced { frame_index: 1 })
    );
    // In position, both are admitted and the prefix is shape-matched around them.
    let in_position = frame_tx(vec![
        expiry_frame(20_000),
        recent_root_frame(20_000),
        self_verify(30_000),
    ]);
    let candidate = profile2_candidate(&in_position).expect("in position");
    assert_eq!(candidate.prefix.frame_indices, vec![2]);
    assert_eq!(candidate.prefix.recent_root_index, Some(1));
}

/// A prefix that is not one of the four shapes is unpriceable and not a
/// candidate: here a lone SENDER frame, and a `pay` frame with no preceding
/// `only_verify`.
#[test]
fn unrecognised_shapes_are_not_candidates_and_have_no_price() {
    let lone_sender = frame_tx(vec![body(10_000)]);
    assert_eq!(prefix_and_verifier_frame_cost(&lone_sender), None);
    assert_eq!(
        profile2_candidate(&lone_sender),
        Err(NotProfile2Candidate::UnrecognisedShape)
    );
    let pay_only = frame_tx(vec![pay(Some(PAYMASTER), 10_000)]);
    assert_eq!(
        profile2_candidate(&pay_only),
        Err(NotProfile2Candidate::UnrecognisedShape)
    );
}

/// A `self_verify` frame naming another account is statically invalid under
/// EIP-8141 (`APPROVE_EXECUTION` requires the sender as target), so the shape's
/// target condition is met by condition 1 before it is checked.
#[test]
fn verify_frame_targeting_another_account_is_not_a_candidate() {
    let tx = frame_tx(vec![frame(
        FrameMode::Verify,
        0x03,
        Some(Address::repeat_byte(0x99)),
        30_000,
    )]);
    assert!(matches!(
        profile2_candidate(&tx),
        Err(NotProfile2Candidate::StaticallyInvalid(_))
    ));
}

/// Case 17: the second occurrence in a list whose first consumed all of
/// `MAX_VERIFY_GAS_PER_IL` is not admitted.
#[test]
fn budget_fill_admits_nothing_once_the_list_budget_is_spent() {
    let first = listed(frame_tx(vec![self_verify(MAX_VERIFY_GAS_PER_IL)]));
    let mut second_inner = frame_tx(vec![self_verify(MAX_VERIFY_GAS_PER_IL)]);
    second_inner.nonce_seq = 1;
    let second = listed(second_inner);
    let fill = budget_fill(
        &[first.clone(), second.clone()],
        Fork::Hegota,
        &NativeCrypto,
    );
    assert!(fill.admitted.contains(&first.hash(&NativeCrypto)));
    assert!(!fill.admitted.contains(&second.hash(&NativeCrypto)));
    assert_eq!(fill.remaining_gas, 0);
}

/// Case 6: a structurally valid transaction whose signature does not verify is
/// not admitted, and only the signature half of its cost is debited.
#[test]
fn budget_fill_debits_only_the_signature_half_for_a_bad_signature() {
    let mut inner = frame_tx(vec![self_verify(100_000)]);
    inner.signatures = vec![bad_signature()];
    let tx = listed(inner);
    let fill = budget_fill(std::slice::from_ref(&tx), Fork::Hegota, &NativeCrypto);
    assert!(fill.admitted.is_empty());
    assert_eq!(
        fill.remaining_gas,
        MAX_VERIFY_GAS_PER_IL - SECP256K1_SIGNATURE_GAS
    );
}

/// Case 18: two occurrences at half the list budget each, the first with a bad
/// signature. The first debits only its signature half, so the second still
/// fits and is admitted.
#[test]
fn budget_fill_two_stage_debit_leaves_room_for_the_second_occurrence() {
    let half = MAX_VERIFY_GAS_PER_IL / 2;
    let mut first_inner = frame_tx(vec![self_verify(half - SECP256K1_SIGNATURE_GAS)]);
    first_inner.signatures = vec![bad_signature()];
    let first = listed(first_inner);
    let mut second_inner = frame_tx(vec![self_verify(half)]);
    second_inner.nonce_seq = 1;
    let second = listed(second_inner);
    let fill = budget_fill(
        &[first.clone(), second.clone()],
        Fork::Hegota,
        &NativeCrypto,
    );
    assert!(!fill.admitted.contains(&first.hash(&NativeCrypto)));
    assert!(fill.admitted.contains(&second.hash(&NativeCrypto)));
    assert_eq!(
        fill.remaining_gas,
        MAX_VERIFY_GAS_PER_IL - SECP256K1_SIGNATURE_GAS - half
    );
}

/// Unpriceable and unmetered occurrences leave the budget untouched; a priced
/// occurrence that fails candidacy after the debit keeps the debit.
#[test]
fn budget_fill_debits_exactly_what_the_spec_meters() {
    // Not a frame transaction: not metered.
    let profile1 = Transaction::EIP1559Transaction(EIP1559Transaction {
        chain_id: 1,
        gas_limit: 21_000,
        to: TxKind::Call(Address::repeat_byte(0xAA)),
        ..Default::default()
    });
    // Blobs: not metered.
    let mut blob_inner = frame_tx(vec![self_verify(30_000)]);
    blob_inner.blob_versioned_hashes = vec![ethrex_common::H256::from([0x01; 32])];
    blob_inner.max_fee_per_blob_gas = U256::from(1u64);
    // No recognised shape: unpriceable, no debit.
    let shapeless = listed(frame_tx(vec![body(10_000)]));
    // Priced, signature-less, but a VERIFY body frame: charged, not admitted.
    let verify_body = listed(frame_tx(vec![
        self_verify(40_000),
        frame(
            FrameMode::Verify,
            0x00,
            Some(Address::repeat_byte(0xB1)),
            5_000,
        ),
    ]));
    // Does not fit the per-transaction cap: ignored, no debit.
    let too_expensive = listed(frame_tx(vec![self_verify(MAX_VERIFY_GAS_PER_TX + 1)]));

    let fill = budget_fill(
        &[
            profile1,
            listed(blob_inner),
            shapeless,
            verify_body,
            too_expensive,
        ],
        Fork::Hegota,
        &NativeCrypto,
    );
    assert!(fill.admitted.is_empty());
    assert_eq!(fill.remaining_gas, MAX_VERIFY_GAS_PER_IL - 40_000);
}

/// Duplicate occurrences are metered per occurrence, in list order.
#[test]
fn budget_fill_meters_every_occurrence_of_the_same_transaction() {
    let tx = listed(frame_tx(vec![self_verify(300_000)]));
    let fill = budget_fill(
        &[tx.clone(), tx.clone(), tx.clone()],
        Fork::Hegota,
        &NativeCrypto,
    );
    assert!(fill.admitted.contains(&tx.hash(&NativeCrypto)));
    assert_eq!(fill.remaining_gas, MAX_VERIFY_GAS_PER_IL - 900_000);
}
