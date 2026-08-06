//! EIP-8369 VOPS profile classification and the per-inclusion-list budget fill.

use ethrex_blockchain::focil_eligibility::{
    FillOutcome, MAX_VERIFY_GAS_PER_IL, MAX_VERIFY_GAS_PER_TX, VopsProfile, classify, fee_valid,
    fill_il_budget, verify_budget_cost,
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
