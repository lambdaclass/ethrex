//! EIP-8369 VOPS profile classification and the per-inclusion-list budget fill.

use ethrex_blockchain::focil_eligibility::{
    FillOutcome, MAX_VERIFY_GAS_PER_IL, MAX_VERIFY_GAS_PER_TX, VopsProfile, classify,
    default_evaluation_index, evaluation_index, fee_valid, fill_il_budget, profile_2_payer,
    verify_budget_cost,
};
use ethrex_common::types::{
    APPROVE_EXECUTION, APPROVE_EXECUTION_AND_PAYMENT, APPROVE_PAYMENT, EIP1559Transaction,
    FRAME_SIG_SCHEME_SECP256K1, Frame, FrameMode, FrameSignature, FrameTransaction,
    LegacyTransaction, Transaction, TxKind, frame_tx_expiry_verifier,
};
use ethrex_common::{Address, U256};

fn sender() -> Address {
    Address::repeat_byte(0x11)
}

fn verify_frame(target: Option<Address>, scope: u8, gas_limit: u64) -> Frame {
    Frame {
        mode: FrameMode::Verify as u8,
        flags: scope,
        target,
        gas_limit,
        value: U256::zero(),
        data: Default::default(),
    }
}

/// A frame transaction that passes EIP-8141 static validation. `nonce_keys` must
/// carry 1..=16 entries for a non-vault sender, which the shorter helpers in the
/// inclusion-list tests do not bother with because they never validate.
fn frame_tx(frames: Vec<Frame>) -> FrameTransaction {
    FrameTransaction {
        chain_id: 1,
        nonce_keys: vec![U256::zero()],
        nonce_seq: 0,
        sender: sender(),
        frames,
        signatures: vec![FrameSignature {
            scheme: FRAME_SIG_SCHEME_SECP256K1,
            signer: Some(sender()),
            msg: Default::default(),
            signature: Default::default(),
        }],
        max_priority_fee_per_gas: 1,
        max_fee_per_gas: 1_000,
        max_fee_per_blob_gas: U256::zero(),
        blob_versioned_hashes: vec![],
        ..Default::default()
    }
}

/// `SelfVerify`: one VERIFY frame targeting the sender with scope
/// `APPROVE_EXECUTION_AND_PAYMENT`. The simplest of the four admitted shapes.
fn self_verify_tx(gas_limit: u64) -> FrameTransaction {
    frame_tx(vec![verify_frame(
        Some(sender()),
        APPROVE_EXECUTION_AND_PAYMENT,
        gas_limit,
    )])
}

fn legacy_tx() -> Transaction {
    Transaction::LegacyTransaction(LegacyTransaction {
        nonce: 0,
        gas_price: U256::from(1_000u64),
        gas: 21_000,
        to: TxKind::Call(Address::repeat_byte(0xaa)),
        value: U256::zero(),
        ..Default::default()
    })
}

fn eip1559_tx(max_fee: u64, priority: u64) -> Transaction {
    Transaction::EIP1559Transaction(EIP1559Transaction {
        chain_id: 1,
        nonce: 0,
        max_priority_fee_per_gas: priority,
        max_fee_per_gas: max_fee,
        gas_limit: 21_000,
        to: TxKind::Call(Address::repeat_byte(0xaa)),
        value: U256::zero(),
        ..Default::default()
    })
}

#[test]
fn regular_transactions_are_profile_1() {
    assert_eq!(classify(&legacy_tx(), false), VopsProfile::One);
    assert_eq!(classify(&eip1559_tx(1_000, 1), false), VopsProfile::One);
}

#[test]
fn a_recognized_prefix_within_budget_is_a_profile_2_candidate() {
    let tx = Transaction::FrameTransaction(self_verify_tx(50_000));
    assert_eq!(classify(&tx, false), VopsProfile::TwoCandidate);
}

/// EIP-8369 puts every blob-carrying transaction outside both profiles: "blob gas
/// has its own target and maximum, and this EIP defines no omission check over
/// that second budget". A frame transaction carrying blobs is covered by the same
/// sentence, so shape alone does not make it a candidate.
#[test]
fn blob_carrying_transactions_are_outside_both_profiles() {
    let mut tx = self_verify_tx(50_000);
    tx.blob_versioned_hashes = vec![Default::default()];
    assert_eq!(
        classify(&Transaction::FrameTransaction(tx), false),
        VopsProfile::Ineligible
    );
}

/// Condition 3 of the candidate test. A declared budget over the per-transaction
/// cap makes the transaction ineligible outright, not merely expensive.
#[test]
fn a_prefix_over_the_per_tx_cap_is_not_a_candidate() {
    let tx = self_verify_tx(MAX_VERIFY_GAS_PER_TX + 1);
    assert!(verify_budget_cost(&tx).is_some_and(|c| c > MAX_VERIFY_GAS_PER_TX));
    assert_eq!(
        classify(&Transaction::FrameTransaction(tx), false),
        VopsProfile::Ineligible
    );
}

/// A frame transaction whose frames match none of the four admitted shapes is
/// not a candidate. A lone SENDER-mode frame has no validation prefix at all.
#[test]
fn an_unrecognized_prefix_is_not_a_candidate() {
    let tx = frame_tx(vec![Frame {
        mode: FrameMode::Sender as u8,
        flags: 0,
        target: Some(Address::repeat_byte(0xaa)),
        gas_limit: 1_000,
        value: U256::zero(),
        data: Default::default(),
    }]);
    assert_eq!(
        classify(&Transaction::FrameTransaction(tx), false),
        VopsProfile::Ineligible
    );
}

/// EIP-8369: an expiry verifier frame "is ignored only when matching the four
/// allowed prefix shapes; its gas limit still counts". `ValidationPrefix`
/// deliberately omits expiry frames from `frame_indices`, so pricing the prefix
/// alone would undercount the budget and let a transaction declare unbounded
/// expiry-frame gas for free.
#[test]
fn an_expiry_verifier_frames_gas_counts_toward_the_budget() {
    let without = self_verify_tx(50_000);
    let base = verify_budget_cost(&without).expect("priceable");

    let mut with_expiry = without.clone();
    with_expiry.frames.insert(
        0,
        Frame {
            mode: FrameMode::Verify as u8,
            flags: 0,
            target: Some(frame_tx_expiry_verifier()),
            gas_limit: 7_000,
            value: U256::zero(),
            data: 0u64.to_be_bytes().to_vec().into(),
        },
    );

    let with = verify_budget_cost(&with_expiry).expect("priceable");
    assert_eq!(
        with,
        base + 7_000,
        "expiry verifier gas must be added to the prefix sum"
    );
}

/// The budget is signature-verification gas plus prefix frame gas, so adding a
/// signature raises the cost. SECP256K1 is 2800 gas per EIP-8141.
#[test]
fn signature_verification_gas_is_part_of_the_budget() {
    let one_sig = self_verify_tx(50_000);
    let base = verify_budget_cost(&one_sig).expect("priceable");

    let mut two_sigs = one_sig.clone();
    two_sigs.signatures.push(FrameSignature {
        scheme: FRAME_SIG_SCHEME_SECP256K1,
        signer: Some(sender()),
        msg: Default::default(),
        signature: Default::default(),
    });

    assert_eq!(
        verify_budget_cost(&two_sigs).expect("priceable"),
        base + 2_800
    );
}

#[test]
fn fee_valid_rejects_below_base_fee_and_inverted_priority() {
    assert!(fee_valid(&eip1559_tx(1_000, 1), 500));
    assert!(!fee_valid(&eip1559_tx(100, 1), 500), "below base fee");
    assert!(
        !fee_valid(&eip1559_tx(1_000, 2_000), 500),
        "priority above max fee"
    );
    // Legacy uses gas_price for both fields, so the second condition is trivial.
    assert!(fee_valid(&legacy_tx(), 500));
    assert!(!fee_valid(&legacy_tx(), 5_000));
}

/// Profile 1 transactions "do not use the IL VERIFY budget".
#[test]
fn profile_1_transactions_are_not_metered() {
    let il = vec![legacy_tx(), eip1559_tx(1_000, 1)];
    let outcomes = fill_il_budget(&il, false);
    assert!(outcomes.iter().all(|o| *o == FillOutcome::NotMetered));
}

/// The fill runs in list order against a single budget, so a transaction that
/// does not fit the remainder is ignored while earlier ones are already charged.
#[test]
fn the_fill_is_ordered_and_stops_at_the_list_budget() {
    let big = MAX_VERIFY_GAS_PER_IL / 2;
    let il = vec![
        Transaction::FrameTransaction(self_verify_tx(big)),
        Transaction::FrameTransaction(self_verify_tx(big)),
        Transaction::FrameTransaction(self_verify_tx(big)),
    ];
    let outcomes = fill_il_budget(&il, false);

    assert!(outcomes[0].is_admitted());
    assert_eq!(
        outcomes[2],
        FillOutcome::Ignored,
        "third occurrence cannot fit and must consume nothing"
    );
}

/// EIP-8369: "A failed candidate keeps the budget debit but is not admitted."
/// The debit lands before the candidate checks precisely so that structurally
/// valid but invalid transactions cannot force unbounded verification work.
#[test]
fn a_failed_candidate_keeps_its_budget_debit() {
    // Priceable from its shape, but static validation fails: nonce_keys is empty,
    // which EIP-8250 forbids for a non-vault sender.
    let mut invalid = self_verify_tx(MAX_VERIFY_GAS_PER_IL / 2);
    invalid.nonce_keys = vec![];

    let il = vec![
        Transaction::FrameTransaction(invalid),
        Transaction::FrameTransaction(self_verify_tx(MAX_VERIFY_GAS_PER_IL / 2 + 1)),
    ];
    let outcomes = fill_il_budget(&il, false);

    assert!(
        matches!(outcomes[0], FillOutcome::ChargedNotAdmitted { .. }),
        "charged but not admitted, got {:?}",
        outcomes[0]
    );
    assert_eq!(
        outcomes[1],
        FillOutcome::Ignored,
        "the debit from the failed candidate must still reduce the remaining budget"
    );
}

/// An occurrence over the per-transaction cap is ignored and "consumes nothing",
/// so a later occurrence that fits is still admitted.
#[test]
fn an_over_cap_occurrence_consumes_nothing() {
    let il = vec![
        Transaction::FrameTransaction(self_verify_tx(MAX_VERIFY_GAS_PER_TX + 1)),
        Transaction::FrameTransaction(self_verify_tx(1_000)),
    ];
    let outcomes = fill_il_budget(&il, false);

    assert_eq!(outcomes[0], FillOutcome::Ignored);
    assert!(
        outcomes[1].is_admitted(),
        "budget must be untouched by the ignored occurrence"
    );
}

/// The `only_verify | pay` shape: two VERIFY frames, the second of which may
/// target a sponsor rather than the sender.
#[test]
fn the_only_verify_pay_shape_is_a_candidate() {
    let tx = frame_tx(vec![
        verify_frame(Some(sender()), APPROVE_EXECUTION, 20_000),
        verify_frame(Some(Address::repeat_byte(0x22)), APPROVE_PAYMENT, 30_000),
    ]);
    assert_eq!(
        classify(&Transaction::FrameTransaction(tx.clone()), false),
        VopsProfile::TwoCandidate
    );
    // Both prefix frames are priced, plus the one signature.
    assert_eq!(verify_budget_cost(&tx), Some(20_000 + 30_000 + 2_800));
}

/// EIP-8369: "`payer` is `sender` for the `self_verify` shapes and the `pay`
/// frame's EIP-8141 `resolved_target` otherwise." Static resolution is what makes
/// the Profile 2 storage surface knowable before replay starts.
#[test]
fn the_payer_resolves_from_the_prefix_shape_alone() {
    assert_eq!(profile_2_payer(&self_verify_tx(10_000)), Some(sender()));

    let sponsor = Address::repeat_byte(0x22);
    let sponsored = frame_tx(vec![
        verify_frame(Some(sender()), APPROVE_EXECUTION, 10_000),
        verify_frame(Some(sponsor), APPROVE_PAYMENT, 10_000),
    ]);
    assert_eq!(profile_2_payer(&sponsored), Some(sponsor));
}

/// "A null `pay` target resolves to `sender`."
#[test]
fn a_null_pay_target_resolves_to_the_sender() {
    let tx = frame_tx(vec![
        verify_frame(Some(sender()), APPROVE_EXECUTION, 10_000),
        verify_frame(None, APPROVE_PAYMENT, 10_000),
    ]);
    assert_eq!(profile_2_payer(&tx), Some(sender()));
}

/// The Profile 2 surface both widens and narrows EIP-8141's mempool rule: the
/// payer becomes readable, but only slots below `AA_VOPS_SLOT_COUNT`, which is
/// what puts keccak-derived mapping slots outside the profile.
#[test]
fn the_vops_surface_admits_sender_and_payer_low_slots_only() {
    use ethrex_levm::validation_observer::{FocilVopsSurface, ValidationObserver};

    let payer = Address::repeat_byte(0x22);
    let mut obs = ValidationObserver::new(sender(), None, Address::zero());
    obs.focil_surface = Some(FocilVopsSurface {
        payer,
        slot_count: 4,
    });

    let slot = |n: u64| ethrex_common::H256::from_low_u64_be(n);

    assert!(obs.within_vops_surface(sender(), slot(0)));
    assert!(obs.within_vops_surface(sender(), slot(3)));
    assert!(obs.within_vops_surface(payer, slot(2)));

    assert!(
        !obs.within_vops_surface(sender(), slot(4)),
        "slot_count is exclusive"
    );
    assert!(
        !obs.within_vops_surface(Address::repeat_byte(0x33), slot(0)),
        "a third account is outside the surface"
    );

    // A keccak-derived mapping slot is numerically enormous, so the bound
    // excludes it without any special case.
    let mapping_slot = ethrex_common::H256::repeat_byte(0xab);
    assert!(!obs.within_vops_surface(sender(), mapping_slot));
}

/// Without a configured surface the predicate rejects, so a caller that reaches
/// it outside Profile 2 fails closed rather than silently permitting a read.
#[test]
fn the_surface_predicate_fails_closed_when_unconfigured() {
    use ethrex_levm::validation_observer::ValidationObserver;
    let obs = ValidationObserver::new(sender(), None, Address::zero());
    assert!(!obs.within_vops_surface(sender(), ethrex_common::H256::zero()));
}

/// EIP-8369 pins the fallback: "A missing, malformed, or out-of-range index
/// defaults to `len(block.transactions)`, the end of the payload."
#[test]
fn the_evaluation_index_falls_back_to_end_of_payload() {
    assert_eq!(default_evaluation_index(7), 7);
    // No claim, and an out-of-range claim, both fall back.
    assert_eq!(evaluation_index(None, 7), 7);
    assert_eq!(evaluation_index(Some(8), 7), 7);
    // In-range claims are honoured, including index 0 (before the first tx) and
    // exactly len(block.transactions).
    assert_eq!(evaluation_index(Some(0), 7), 0);
    assert_eq!(evaluation_index(Some(3), 7), 3);
    assert_eq!(evaluation_index(Some(7), 7), 7);
}
