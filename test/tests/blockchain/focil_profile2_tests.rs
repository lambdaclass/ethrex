//! `InclusionListSatisfactionValidator::check_with_profile_2`: EIP-8369
//! Profile 2 (FOCIL AA-VOPS) omissions. Only `Eligible` makes a list
//! unsatisfied; `Ineligible` and `Undecided` leave the payload verdict alone.
//!
//! Two layers:
//! - Unit-level (`FakeEvaluator`): pins the wiring contract — which fill
//!   outcomes reach the evaluator, and which of the three verdicts lands in
//!   which report bucket — without any EVM or `Store`.
//! - End-to-end (real `Store`/`Blockchain`): drives
//!   [`BlockchainProfile2Evaluator`] against real block state to prove the
//!   three verdicts are actually reachable through validation-prefix replay,
//!   and that only `Eligible` moves the payload verdict.
//!
//! The end-to-end layer also pins the pre-execution gates that
//! `run_frame_validation_prefix` runs ahead of the replay — the EIP-8250 keyed
//! nonce and the EIP-8141 outer signatures — because a transaction that was
//! never includable must be excused, not reported unjustified.

use std::cell::RefCell;
use std::collections::HashSet;

use bytes::Bytes;
use ethrex_blockchain::Blockchain;
use ethrex_blockchain::error::ChainError;
use ethrex_blockchain::focil_eligibility::max_verify_gas_per_tx;
use ethrex_blockchain::focil_profile2::BlockchainProfile2Evaluator;
use ethrex_blockchain::inclusion_list_builder::{
    AccountStateView, IlStateProvider, IlStateProviderError,
};
use ethrex_blockchain::inclusion_list_validator::{
    IlProfile2Evaluator, IlUnsatisfied, InclusionListSatisfactionValidator, Profile2Eligibility,
    StoreIlStateProvider,
};
use ethrex_common::validation::BlockValidationContext;
use ethrex_common::{
    Address, H256, U256,
    types::{
        APPROVE_EXECUTION_AND_PAYMENT, BlockHeader, ChainConfig, Frame, FrameMode,
        FrameTransaction, Genesis, GenesisAccount, Transaction,
    },
};
use ethrex_crypto::NativeCrypto;
use ethrex_storage::{EngineType, Store};
use rustc_hash::FxHashMap;

// ─────────────────────────────────────────────────────────────────────────────
// Unit-level: `check_with_profile_2` wiring, against a `FakeEvaluator`.
// ─────────────────────────────────────────────────────────────────────────────

/// An `IlStateProvider` that treats every address as an empty account. The
/// Profile 2 path never consults the per-sender tracker, so this is enough to
/// satisfy `InclusionListSatisfactionValidator::new`'s state-read contract.
struct EmptyState;

impl IlStateProvider for EmptyState {
    fn get_account(
        &self,
        _address: Address,
    ) -> Result<Option<AccountStateView>, IlStateProviderError> {
        Ok(None)
    }

    fn classify_code(
        &self,
        _code_hash: H256,
    ) -> Result<ethrex_blockchain::focil_eligibility::SenderCode, IlStateProviderError> {
        Ok(ethrex_blockchain::focil_eligibility::SenderCode::Eoa)
    }
}

/// A fixed verdict per sender address, plus a call log. Panics if `evaluate`
/// is called for a sender with no registered verdict — used to prove a
/// non-`Admitted` fill outcome never reaches the evaluator at all.
#[derive(Default)]
struct FakeEvaluator {
    verdict: FxHashMap<Address, Profile2Eligibility>,
    calls: RefCell<Vec<Address>>,
}

impl FakeEvaluator {
    fn with(verdict: FxHashMap<Address, Profile2Eligibility>) -> Self {
        Self {
            verdict,
            calls: RefCell::new(Vec::new()),
        }
    }
}

impl IlProfile2Evaluator for FakeEvaluator {
    fn evaluate(&self, tx: &FrameTransaction) -> Profile2Eligibility {
        self.calls.borrow_mut().push(tx.sender);
        self.verdict.get(&tx.sender).cloned().unwrap_or_else(|| {
            panic!(
                "evaluate() must not be called for sender {:#x} — its fill outcome was not Admitted",
                tx.sender
            )
        })
    }
}

fn header() -> BlockHeader {
    BlockHeader::default()
}

fn config() -> ChainConfig {
    ChainConfig::default()
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

/// A statically-valid `self_verify` frame transaction (the simplest Profile 2
/// candidate shape) for `sender`.
fn self_verify_tx(sender: Address, gas_limit: u64) -> FrameTransaction {
    FrameTransaction {
        chain_id: 1,
        nonce_keys: vec![U256::zero()],
        nonce_seq: 0,
        sender,
        frames: vec![verify_frame(
            Some(sender),
            APPROVE_EXECUTION_AND_PAYMENT,
            gas_limit,
        )],
        // Empty, not a placeholder signature: EIP-8141 signature validation
        // passes vacuously on an empty list, and a garbage-but-present entry
        // would fail `validate_frame_signatures` and abort the prefix replay
        // before it ever reaches the VERIFY frame.
        signatures: vec![],
        // Comfortably above the execution-api fixture genesis's 1 gwei
        // `baseFeePerGas`, so `fee_valid` holds in both the unit-level tests
        // (default header, base fee 0) and the end-to-end tests (a real
        // chain's base fee).
        max_priority_fee_per_gas: 1,
        max_fee_per_gas: 10_000_000_000,
        max_fee_per_blob_gas: U256::zero(),
        blob_versioned_hashes: vec![],
        ..Default::default()
    }
}

fn frame_tx_transaction(tx: FrameTransaction) -> Transaction {
    Transaction::FrameTransaction(tx)
}

/// An `Eligible` omission lands in `profile_2_unjustified`; `unsatisfied`
/// stays `None` regardless.
#[test]
fn eligible_profile2_omission_lands_in_unjustified_bucket() {
    let crypto = NativeCrypto;
    let sender = Address::repeat_byte(0x11);
    let tx = self_verify_tx(sender, 50_000);
    let il = vec![frame_tx_transaction(tx)];

    let state = EmptyState;
    let validator =
        InclusionListSatisfactionValidator::new(&il, &state, &crypto).expect("construct");

    let mut verdicts = FxHashMap::default();
    verdicts.insert(sender, Profile2Eligibility::Eligible);
    let evaluator = FakeEvaluator::with(verdicts);

    let block_txs: HashSet<H256> = HashSet::new();
    let report = validator.check_with_profile_2(
        &il,
        &block_txs,
        30_000_000,
        &header(),
        &config(),
        &crypto,
        Some(&evaluator),
        60000000,
    );

    assert_eq!(
        report.unsatisfied,
        Some(IlUnsatisfied {
            tx_hash: il[0].hash(&crypto)
        }),
        "an eligible Profile 2 omission is unjustified, so the list is unsatisfied"
    );
    assert_eq!(report.profile_2_unjustified, vec![il[0].hash(&crypto)]);
    assert!(report.profile_2_undecided.is_empty());
    assert_eq!(evaluator.calls.borrow().as_slice(), &[sender]);
}

/// `Ineligible` and `Undecided` omissions never populate `profile_2_unjustified`
/// (only `Eligible` does); `Undecided` populates `profile_2_undecided`,
/// `Ineligible` populates neither. `unsatisfied` stays `None` throughout: a
/// frame transaction is never Profile 1.
#[test]
fn ineligible_and_undecided_omissions_are_excluded_from_unjustified() {
    let crypto = NativeCrypto;
    let ineligible_sender = Address::repeat_byte(0x22);
    let undecided_sender = Address::repeat_byte(0x33);

    let ineligible_tx = self_verify_tx(ineligible_sender, 50_000);
    let undecided_tx = self_verify_tx(undecided_sender, 60_000);
    let il = vec![
        frame_tx_transaction(ineligible_tx),
        frame_tx_transaction(undecided_tx),
    ];

    let state = EmptyState;
    let validator =
        InclusionListSatisfactionValidator::new(&il, &state, &crypto).expect("construct");

    let mut verdicts = FxHashMap::default();
    verdicts.insert(
        ineligible_sender,
        Profile2Eligibility::Ineligible("over budget".to_string()),
    );
    verdicts.insert(
        undecided_sender,
        Profile2Eligibility::Undecided("carries a UTXO frame".to_string()),
    );
    let evaluator = FakeEvaluator::with(verdicts);

    let block_txs: HashSet<H256> = HashSet::new();
    let report = validator.check_with_profile_2(
        &il,
        &block_txs,
        30_000_000,
        &header(),
        &config(),
        &crypto,
        Some(&evaluator),
        60000000,
    );

    assert!(report.unsatisfied.is_none());
    assert!(
        report.profile_2_unjustified.is_empty(),
        "neither Ineligible nor Undecided may land in profile_2_unjustified"
    );
    assert_eq!(report.profile_2_undecided, vec![il[1].hash(&crypto)]);
    // Both were evaluated (both were Admitted candidates).
    let mut calls = evaluator.calls.borrow().clone();
    calls.sort();
    let mut expected = vec![ineligible_sender, undecided_sender];
    expected.sort();
    assert_eq!(calls, expected);
}

/// A frame tx whose `fill_il_budget` outcome is `Ignored` (over the
/// per-transaction VERIFY budget cap) or `ChargedNotAdmitted` (priceable but
/// structurally invalid) is never handed to the evaluator at all. The
/// `FakeEvaluator` has no verdict registered for either sender, so a stray
/// `evaluate()` call panics the test.
#[test]
fn ignored_and_charged_not_admitted_never_reach_the_evaluator() {
    let crypto = NativeCrypto;
    let ignored_sender = Address::repeat_byte(0x44);
    let charged_not_admitted_sender = Address::repeat_byte(0x55);

    // Over max_verify_gas_per_tx(60000000): priced, then rejected by the per-tx cap →
    // `FillOutcome::Ignored`.
    let ignored_tx = self_verify_tx(ignored_sender, max_verify_gas_per_tx(60000000) + 1);

    // Priceable (a valid prefix shape) but statically invalid (empty
    // `nonce_keys`, which EIP-8250 forbids for a non-vault sender) →
    // `FillOutcome::ChargedNotAdmitted`.
    let mut charged_not_admitted_tx = self_verify_tx(charged_not_admitted_sender, 50_000);
    charged_not_admitted_tx.nonce_keys = vec![];

    let il = vec![
        frame_tx_transaction(ignored_tx),
        frame_tx_transaction(charged_not_admitted_tx),
    ];

    let state = EmptyState;
    let validator =
        InclusionListSatisfactionValidator::new(&il, &state, &crypto).expect("construct");

    // No verdicts registered — any `evaluate()` call panics.
    let evaluator = FakeEvaluator::with(FxHashMap::default());

    let block_txs: HashSet<H256> = HashSet::new();
    let report = validator.check_with_profile_2(
        &il,
        &block_txs,
        30_000_000,
        &header(),
        &config(),
        &crypto,
        Some(&evaluator),
        60000000,
    );

    assert!(report.unsatisfied.is_none());
    assert!(report.profile_2_unjustified.is_empty());
    assert!(report.profile_2_undecided.is_empty());
    assert!(
        evaluator.calls.borrow().is_empty(),
        "neither Ignored nor ChargedNotAdmitted may reach the evaluator"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// End-to-end: `BlockchainProfile2Evaluator` against a real `Store`/`Blockchain`.
// ─────────────────────────────────────────────────────────────────────────────

const AA_VOPS_SLOT_COUNT: u64 = 4;

/// APPROVE(scope) then STOP: `PUSH1 scope; PUSH1 0; PUSH1 0; APPROVE; STOP`.
/// A `self_verify` VERIFY frame targeting an address running this code
/// establishes that address as payer without touching any storage.
fn approve_code(scope: u8) -> Bytes {
    Bytes::from(vec![0x60, scope, 0x60, 0x00, 0x60, 0x00, 0xAA, 0x00])
}

/// `SLOAD` of `slot` (discarded), then the same APPROVE sequence. Used to put
/// a storage read at or above `AA_VOPS_SLOT_COUNT` inside the validation
/// prefix, which the Profile 2 surface must reject.
fn oob_sload_then_approve_code(slot: u8, scope: u8) -> Bytes {
    let mut code = vec![0x60, slot, 0x54, 0x50]; // PUSH1 slot; SLOAD; POP
    code.extend_from_slice(&[0x60, scope, 0x60, 0x00, 0x60, 0x00, 0xAA, 0x00]);
    Bytes::from(code)
}

/// A store with Hegotá active from genesis and one contract account per
/// `(address, code)` pair, funded with enough state to be a plausible chain.
///
/// Starts from the execution-api fixture genesis (rather than
/// `Genesis::default()`) so the EIP-1559 base-fee fields are the ones real
/// blocks already build against; only Hegotá activation and the extra
/// contract accounts are added.
async fn setup_profile2_store(accounts: &[(Address, Bytes)]) -> (Store, Blockchain, BlockHeader) {
    setup_profile2_store_at_nonce(accounts, 0, "focil-profile2-store.db").await
}

/// As [`setup_profile2_store`], with the seeded accounts at `nonce`. Nonce key
/// `0` is the account's linear nonce, so this is what a keyed-nonce sequence
/// looks like before and after a predecessor consumes it.
async fn setup_profile2_store_at_nonce(
    accounts: &[(Address, Bytes)],
    nonce: u64,
    db_name: &str,
) -> (Store, Blockchain, BlockHeader) {
    setup_profile2_store_funded(
        accounts,
        nonce,
        U256::from(10u64).pow(U256::from(18u64)),
        db_name,
    )
    .await
}

/// As [`setup_profile2_store_at_nonce`], with the seeded accounts holding
/// `balance`. `self_verify_tx` self-pays, so this is the payer's balance as far
/// as eligibility replay is concerned.
async fn setup_profile2_store_funded(
    accounts: &[(Address, Bytes)],
    nonce: u64,
    balance: U256,
    db_name: &str,
) -> (Store, Blockchain, BlockHeader) {
    let file = std::fs::File::open(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("fixtures/genesis/execution-api.json"),
    )
    .expect("open execution-api genesis fixture");
    let mut genesis: Genesis =
        serde_json::from_reader(std::io::BufReader::new(file)).expect("parse genesis fixture");
    genesis.config.hegota_time = Some(0);
    genesis.config.amsterdam_time = Some(0);
    for (address, code) in accounts {
        genesis.alloc.insert(
            *address,
            GenesisAccount {
                code: code.clone(),
                storage: Default::default(),
                // `self_verify_tx`'s APPROVE_EXECUTION_AND_PAYMENT self-pays:
                // it debits `max_fee_per_gas * total_gas_limit` from this same
                // account inside the VERIFY frame, which reverts on an
                // underfunded sender before any surface/UTXO check runs.
                balance,
                nonce,
            },
        );
    }
    let mut store = Store::new(db_name, EngineType::InMemory).expect("in-memory store");
    store
        .add_initial_state(genesis)
        .await
        .expect("add genesis state");
    let blockchain = Blockchain::default_with_store(store.clone());
    let genesis_header = store.get_block_header(0).unwrap().unwrap();
    (store, blockchain, genesis_header)
}

/// Import one empty block on top of `parent` via the IL-aware pipeline, with
/// `il` listed but never included (an "external proposer omitted it"
/// scenario). Returns the imported (NOT canonical) block's header.
///
/// Asserts the payload verdict (`add_block_pipeline_with_il`'s `Result`) is
/// Returns the imported header alongside the pipeline's verdict, so a caller can
/// assert what Profile 2 enforcement did to it end to end.
async fn import_block_omitting_il(
    store: &Store,
    blockchain: &Blockchain,
    parent: &BlockHeader,
    il: Vec<Transaction>,
) -> (BlockHeader, Result<(), ChainError>) {
    use ethrex_blockchain::payload::{BuildPayloadArgs, create_payload};
    use ethrex_common::types::{DEFAULT_BUILDER_GAS_CEIL, ELASTICITY_MULTIPLIER};

    let args = BuildPayloadArgs {
        parent: parent.hash(),
        timestamp: parent.timestamp + 12,
        fee_recipient: ethrex_common::H160::zero(),
        random: H256::zero(),
        withdrawals: Some(Vec::new()),
        beacon_root: Some(H256::zero()),
        slot_number: Some(1),
        version: 5,
        elasticity_multiplier: ELASTICITY_MULTIPLIER,
        gas_ceil: DEFAULT_BUILDER_GAS_CEIL,
        inclusion_list_transactions: None,
    };
    let block = create_payload(&args, store, Bytes::new()).unwrap();
    let block = blockchain.build_payload(block).unwrap().payload;
    assert!(
        block.body.transactions.is_empty(),
        "the IL tx must be omitted, not included"
    );
    let header = block.header.clone();

    let context = BlockValidationContext::with_inclusion_list(il);
    let verdict = blockchain.add_block_pipeline_with_il(block, None, &context);

    (header, verdict)
}

/// A `self_verify` frame tx listed and omitted from a block whose post-state
/// leaves it valid: `evaluate()` reports `Eligible`, and the payload verdict
/// (block import) is unaffected.
#[tokio::test]
async fn self_verify_frame_tx_that_would_pass_replay_is_eligible() {
    let sender = Address::from_low_u64_be(0xE11);
    let (store, blockchain, genesis) =
        setup_profile2_store(&[(sender, approve_code(APPROVE_EXECUTION_AND_PAYMENT))]).await;

    let tx = self_verify_tx(sender, 50_000);
    let il = vec![frame_tx_transaction(tx.clone())];

    let (header, verdict) =
        import_block_omitting_il(&store, &blockchain, &genesis, il.clone()).await;

    let gas_left = header.gas_limit.saturating_sub(header.gas_used);
    let pre_state = StoreIlStateProvider {
        store: &store,
        state_root: genesis.state_root,
    };
    let post_state = StoreIlStateProvider {
        store: &store,
        state_root: header.state_root,
    };
    let crypto = NativeCrypto;
    let mut validator =
        InclusionListSatisfactionValidator::new(&il, &pre_state, &crypto).expect("construct");
    validator
        .refresh_all_from(&post_state, &crypto)
        .expect("refresh");

    let evaluator =
        BlockchainProfile2Evaluator::new(&blockchain, &header, header.state_root, gas_left);
    let report = validator.check_with_profile_2(
        &il,
        &HashSet::new(),
        gas_left,
        &header,
        &store.get_chain_config(),
        &crypto,
        Some(&evaluator),
        60000000,
    );

    assert_eq!(
        report.unsatisfied,
        Some(IlUnsatisfied {
            tx_hash: il[0].hash(&crypto)
        })
    );
    assert_eq!(report.profile_2_unjustified, vec![il[0].hash(&crypto)]);
    assert!(report.profile_2_undecided.is_empty());

    // End to end: the pipeline reaches the same verdict, so a builder that drops
    // an includable listed frame transaction has its block reported unsatisfied.
    match verdict {
        Err(ChainError::IlUnsatisfied { tx_hash }) => {
            assert_eq!(tx_hash, il[0].hash(&crypto))
        }
        other => panic!("expected IlUnsatisfied from the pipeline, got {other:?}"),
    }
}

/// The same shape, but the sender's code reads storage slot
/// `AA_VOPS_SLOT_COUNT` (the first slot outside the Profile 2 surface):
/// `evaluate()` reports `Ineligible` with a `StorageOutsideVopsSurface`
/// violation, and the payload verdict is unaffected.
#[tokio::test]
async fn self_verify_frame_tx_reading_outside_the_surface_is_ineligible() {
    let sender = Address::from_low_u64_be(0xE22);
    let (store, blockchain, genesis) = setup_profile2_store(&[(
        sender,
        oob_sload_then_approve_code(AA_VOPS_SLOT_COUNT as u8, APPROVE_EXECUTION_AND_PAYMENT),
    )])
    .await;

    let tx = self_verify_tx(sender, 50_000);
    let il = vec![frame_tx_transaction(tx.clone())];

    let (header, verdict) =
        import_block_omitting_il(&store, &blockchain, &genesis, il.clone()).await;

    let gas_left = header.gas_limit.saturating_sub(header.gas_used);
    let pre_state = StoreIlStateProvider {
        store: &store,
        state_root: genesis.state_root,
    };
    let post_state = StoreIlStateProvider {
        store: &store,
        state_root: header.state_root,
    };
    let crypto = NativeCrypto;
    let mut validator =
        InclusionListSatisfactionValidator::new(&il, &pre_state, &crypto).expect("construct");
    validator
        .refresh_all_from(&post_state, &crypto)
        .expect("refresh");

    let evaluator =
        BlockchainProfile2Evaluator::new(&blockchain, &header, header.state_root, gas_left);
    let report = validator.check_with_profile_2(
        &il,
        &HashSet::new(),
        gas_left,
        &header,
        &store.get_chain_config(),
        &crypto,
        Some(&evaluator),
        60000000,
    );

    assert!(
        report.unsatisfied.is_none(),
        "payload verdict must be unchanged"
    );
    assert!(
        report.profile_2_unjustified.is_empty(),
        "an out-of-surface read must not be reported unjustified"
    );
    assert!(report.profile_2_undecided.is_empty());

    // Confirm the specific violation directly through the evaluator too.
    match evaluator.evaluate(&tx) {
        Profile2Eligibility::Ineligible(violation) => {
            assert!(
                violation.contains("StorageOutsideVopsSurface"),
                "expected a StorageOutsideVopsSurface violation, got: {violation}"
            );
        }
        other => panic!("expected Ineligible, got {other:?}"),
    }

    verdict.expect("an out-of-surface read must not fail the block");
}

/// A frame tx carrying an EIP-8312 UTXO frame is `Undecided`: EIP-8369 does
/// not model UTXO frames, so replaying only the validation prefix cannot
/// account for a spend that could invalidate it after the prefix runs. The
/// payload verdict is unaffected.
#[tokio::test]
async fn frame_tx_with_a_utxo_frame_is_undecided() {
    let sender = Address::from_low_u64_be(0xE33);
    let (store, blockchain, genesis) =
        setup_profile2_store(&[(sender, approve_code(APPROVE_EXECUTION_AND_PAYMENT))]).await;

    let mut tx = self_verify_tx(sender, 50_000);
    // The UTXO frame's data is never decoded: `evaluate()` returns `Undecided`
    // on frame mode alone, before validation-prefix derivation.
    tx.frames.push(Frame {
        mode: FrameMode::Utxo as u8,
        flags: 0,
        target: None,
        gas_limit: 0,
        value: U256::zero(),
        data: Default::default(),
    });
    let il = vec![frame_tx_transaction(tx.clone())];

    let (header, verdict) =
        import_block_omitting_il(&store, &blockchain, &genesis, il.clone()).await;

    let gas_left = header.gas_limit.saturating_sub(header.gas_used);
    let evaluator =
        BlockchainProfile2Evaluator::new(&blockchain, &header, header.state_root, gas_left);
    match evaluator.evaluate(&tx) {
        Profile2Eligibility::Undecided(_) => {}
        other => panic!("expected Undecided, got {other:?}"),
    }

    // Undecided is excused, so the block stands.
    verdict.expect("an undecidable omission must not fail the block");
}

/// Eligibility is not constant across a payload, which is what makes a
/// builder-chosen evaluation index a censorship vector rather than a detail.
///
/// The same queued frame transaction (`nonce_seq == 1`, so it sits behind one
/// predecessor) is judged against the two states a payload's endpoints resolve
/// to. Before the predecessor consumes the sequence it is `Ineligible`; after,
/// it is `Eligible`.
///
/// EIP-8369 judges an omission at an index the builder claims, falling back to
/// end-of-payload. End-of-payload is one of the claimable indices, so the set of
/// omissions a free claim excuses strictly contains the set end-of-payload
/// excuses. This transaction is in the difference: enforced under the fallback,
/// excused by a builder that claims the earlier index. EIP-8369's
/// `a600eba447` puts exactly this shape outside the position-stable class.
#[tokio::test]
async fn queued_frame_tx_eligibility_flips_across_the_payload() {
    let sender = Address::from_low_u64_be(0xE66);
    let code = approve_code(APPROVE_EXECUTION_AND_PAYMENT);

    // Nonce key 0 is the sender's linear nonce. `nonce_seq == 1` needs a
    // predecessor to have consumed sequence 0 first.
    let mut tx = self_verify_tx(sender, 50_000);
    tx.nonce_seq = 1;

    // Start of payload: the predecessor has not run.
    let (_s0, chain_before, header_before) =
        setup_profile2_store_at_nonce(&[(sender, code.clone())], 0, "focil-queued-before.db").await;
    let gas_left = header_before.gas_limit;
    let before = BlockchainProfile2Evaluator::new(
        &chain_before,
        &header_before,
        header_before.state_root,
        gas_left,
    )
    .evaluate(&tx);

    // End of payload: the predecessor has consumed sequence 0.
    let (_s1, chain_after, header_after) =
        setup_profile2_store_at_nonce(&[(sender, code)], 1, "focil-queued-after.db").await;
    let after = BlockchainProfile2Evaluator::new(
        &chain_after,
        &header_after,
        header_after.state_root,
        gas_left,
    )
    .evaluate(&tx);

    match (&before, &after) {
        (Profile2Eligibility::Ineligible(violation), Profile2Eligibility::Eligible) => {
            // The flip must be the keyed nonce reading 0 where the transaction
            // wants 1, and nothing incidental, otherwise this passes for the
            // wrong reason.
            assert!(
                violation.contains("Nonce mismatch: expected 0, got 1"),
                "expected the early verdict to fail on the keyed nonce, got: {violation}"
            );
        }
        _ => panic!(
            "expected the verdict to flip Ineligible -> Eligible across the payload, \
             got before={before:?} after={after:?}"
        ),
    }
}

/// A transaction can be eligible at both ends of a payload and ineligible in
/// between, which is the shape a builder-claimed evaluation index excuses and
/// no endpoint rule does.
///
/// The payer's balance is not monotonic across a payload: a sponsored
/// transaction draws it down, a top-up restores it. The same listed frame
/// transaction is therefore eligible at the start, ineligible after the draw,
/// and eligible again at the end.
///
/// This shape matters because it survives the objection that kills the queued
/// case. An includer builds its list against the head, which is the
/// start-of-payload state, and here the transaction is eligible there, so an
/// honest includer does list it. Under EIP-7805's end-of-payload rule the
/// omission is enforced, because the transaction is eligible at the end. Under
/// a builder-claimed index the builder names the middle and is excused, at the
/// cost of two transactions it was free to order as it liked.
#[tokio::test]
async fn frame_tx_eligible_at_both_endpoints_can_be_ineligible_between_them() {
    let sender = Address::from_low_u64_be(0xE77);
    let code = approve_code(APPROVE_EXECUTION_AND_PAYMENT);
    let tx = self_verify_tx(sender, 50_000);

    // `self_verify_tx` self-pays `max_fee_per_gas * total_gas_limit`, so this is
    // comfortably above the cost, and the drawn-down figure below it.
    let funded = U256::from(10u64).pow(U256::from(18u64));
    let drawn_down = U256::from(1_000u64);

    async fn verdict_at(
        sender: Address,
        code: Bytes,
        tx: &FrameTransaction,
        balance: U256,
        db: &str,
    ) -> Profile2Eligibility {
        let (_store, chain, header) =
            setup_profile2_store_funded(&[(sender, code)], 0, balance, db).await;
        let gas_left = header.gas_limit;
        BlockchainProfile2Evaluator::new(&chain, &header, header.state_root, gas_left).evaluate(tx)
    }

    let start = verdict_at(sender, code.clone(), &tx, funded, "focil-sandwich-start.db").await;
    let middle = verdict_at(
        sender,
        code.clone(),
        &tx,
        drawn_down,
        "focil-sandwich-middle.db",
    )
    .await;
    let end = verdict_at(sender, code, &tx, funded, "focil-sandwich-end.db").await;

    assert!(
        matches!(start, Profile2Eligibility::Eligible),
        "must be eligible at the start, or an honest includer would never list it: {start:?}"
    );
    assert!(
        matches!(end, Profile2Eligibility::Eligible),
        "must be eligible at the end, or the end-of-payload rule would excuse the omission by \
         itself and the claimed index would not be what did it: {end:?}"
    );
    match &middle {
        Profile2Eligibility::Ineligible(_) => {}
        other => panic!("expected the drawn-down payer to be ineligible, got {other:?}"),
    }
}

/// EIP-8250: a listed frame tx whose `nonce_seq` does not match the current
/// sequence for a selected key could never have been included, so its omission
/// is justified. The keyed-nonce gate runs inside `run_frame_validation_prefix`,
/// ahead of the prefix replay, and a stale sequence must surface as `Ineligible`
/// rather than `Eligible` — the latter would withhold an attestation from an
/// honest block.
#[tokio::test]
async fn frame_tx_with_a_stale_keyed_nonce_is_ineligible() {
    let sender = Address::from_low_u64_be(0xE44);
    let (store, blockchain, genesis) =
        setup_profile2_store(&[(sender, approve_code(APPROVE_EXECUTION_AND_PAYMENT))]).await;

    let mut tx = self_verify_tx(sender, 50_000);
    // Key 0 is the sender's linear account nonce, which genesis sets to 0.
    tx.nonce_seq = 7;
    let il = vec![frame_tx_transaction(tx.clone())];

    let (header, verdict) =
        import_block_omitting_il(&store, &blockchain, &genesis, il.clone()).await;

    let gas_left = header.gas_limit.saturating_sub(header.gas_used);
    let evaluator =
        BlockchainProfile2Evaluator::new(&blockchain, &header, header.state_root, gas_left);
    match evaluator.evaluate(&tx) {
        Profile2Eligibility::Ineligible(violation) => {
            assert!(
                violation.contains("Nonce") || violation.contains("nonce"),
                "expected a nonce-mismatch violation, got: {violation}"
            );
        }
        other => panic!("expected Ineligible, got {other:?}"),
    }

    verdict.expect("a stale keyed nonce must not fail the block");
}

/// EIP-8141: a listed frame tx carrying a signature that does not verify could
/// never have been included. `validate_frame_signatures` runs inside
/// `run_frame_validation_prefix`, so a bad signature must surface as
/// `Ineligible`.
///
/// An empty signature list passes vacuously and is legitimate: a smart-account
/// sender is authenticated by its own VERIFY frame calling `APPROVE`, not by an
/// outer signature. This pins the case where a signature IS present and does
/// not verify.
#[tokio::test]
async fn frame_tx_with_an_unverifiable_signature_is_ineligible() {
    use ethrex_common::types::{FRAME_SIG_SCHEME_SECP256K1, FrameSignature};

    let sender = Address::from_low_u64_be(0xE55);
    let (store, blockchain, genesis) =
        setup_profile2_store(&[(sender, approve_code(APPROVE_EXECUTION_AND_PAYMENT))]).await;

    let mut tx = self_verify_tx(sender, 50_000);
    // Well-formed length and a bare recovery id, so the entry is rejected for
    // failing to recover `sender` rather than for being malformed.
    let mut signature = vec![0x01u8; 65];
    signature[0] = 0;
    tx.signatures = vec![FrameSignature {
        scheme: FRAME_SIG_SCHEME_SECP256K1,
        msg: Default::default(),
        signature: signature.into(),
        ..Default::default()
    }];
    let il = vec![frame_tx_transaction(tx.clone())];

    let (header, verdict) =
        import_block_omitting_il(&store, &blockchain, &genesis, il.clone()).await;

    let gas_left = header.gas_limit.saturating_sub(header.gas_used);
    let evaluator =
        BlockchainProfile2Evaluator::new(&blockchain, &header, header.state_root, gas_left);
    match evaluator.evaluate(&tx) {
        Profile2Eligibility::Ineligible(_) => {}
        other => panic!("expected Ineligible, got {other:?}"),
    }

    verdict.expect("an unverifiable signature must not fail the block");
}

// ─────────────────────────────────────────────────────────────────────────────
// Per-inclusion-list code-body budget.
// ─────────────────────────────────────────────────────────────────────────────
//
// EVM gas does not bound how much code a Profile 2 replay makes an attester
// read: at a few thousand gas per cold account, one candidate's VERIFY budget
// admits hundreds of cold accesses, each able to pull a maximum-size body. The
// budget is per inclusion **list** rather than per transaction, because the
// motivating shape is many transactions validating against one shared verifier
// contract whose bytes an attester loads once.

/// `EXTCODESIZE` each of `targets` (result discarded), then APPROVE both
/// scopes. Each cold `EXTCODESIZE` charges the target's code body against the
/// list's allowance.
fn extcodesize_then_approve_code(targets: &[Address], scope: u8) -> Bytes {
    let mut code = Vec::new();
    for target in targets {
        code.push(0x73); // PUSH20
        code.extend_from_slice(target.as_bytes());
        code.push(0x3B); // EXTCODESIZE
        code.push(0x50); // POP
    }
    code.extend_from_slice(&[0x60, scope, 0x60, 0x00, 0x60, 0x00, 0xAA, 0x00]);
    Bytes::from(code)
}

/// `count` filler contracts, each with a distinct one-off body so each has its
/// own code hash. Identical bodies would share a hash and be charged once,
/// which is the opposite of what these tests need.
fn filler_contracts(count: u8) -> Vec<(Address, Bytes)> {
    (0..count)
        .map(|i| {
            (
                Address::from_low_u64_be(0xF000 + u64::from(i)),
                Bytes::from(vec![0x60, i, 0x00]),
            )
        })
        .collect()
}

/// A candidate reading `n` filler bodies on top of its own.
fn reader_tx(sender: Address) -> FrameTransaction {
    self_verify_tx(sender, 400_000)
}

#[tokio::test]
async fn a_prefix_over_the_code_body_bound_is_ineligible() {
    // MAX_VALIDATION_CODE_BODIES is 16 and the prefix's own code counts, so 16
    // fillers put the candidate one body over.
    let sender = Address::from_low_u64_be(0xC0DE);
    let fillers = filler_contracts(16);
    let targets: Vec<Address> = fillers.iter().map(|(a, _)| *a).collect();

    let mut accounts = vec![(
        sender,
        extcodesize_then_approve_code(&targets, APPROVE_EXECUTION_AND_PAYMENT),
    )];
    accounts.extend(fillers);

    let (_store, chain, header) = setup_profile2_store_funded(
        &accounts,
        0,
        U256::from(10u64).pow(U256::from(18u64)),
        "focil-code-budget-over.db",
    )
    .await;
    let evaluator =
        BlockchainProfile2Evaluator::new(&chain, &header, header.state_root, header.gas_limit);

    // Both endpoints replay and both run out, so the verdict is the union's
    // combined message rather than the bare violation.
    match evaluator.evaluate(&reader_tx(sender)) {
        Profile2Eligibility::Ineligible(why) => {
            assert!(
                why.contains("ValidationCodeBudgetExceeded"),
                "expected the budget violation, got {why}"
            );
        }
        other => panic!("expected the budget to reject the replay, got {other:?}"),
    }
}

#[tokio::test]
async fn a_prefix_within_the_code_body_bound_is_eligible() {
    // The negative control for the test above: 15 fillers plus the prefix's own
    // code is exactly MAX_VALIDATION_CODE_BODIES, so nothing is rejected and the
    // bound above is a bound rather than a blanket refusal.
    let sender = Address::from_low_u64_be(0xC0DF);
    let fillers = filler_contracts(15);
    let targets: Vec<Address> = fillers.iter().map(|(a, _)| *a).collect();

    let mut accounts = vec![(
        sender,
        extcodesize_then_approve_code(&targets, APPROVE_EXECUTION_AND_PAYMENT),
    )];
    accounts.extend(fillers);

    let (_store, chain, header) = setup_profile2_store_funded(
        &accounts,
        0,
        U256::from(10u64).pow(U256::from(18u64)),
        "focil-code-budget-under.db",
    )
    .await;
    let evaluator =
        BlockchainProfile2Evaluator::new(&chain, &header, header.state_root, header.gas_limit);

    assert_eq!(
        evaluator.evaluate(&reader_tx(sender)),
        Profile2Eligibility::Eligible
    );
}

#[tokio::test]
async fn the_code_budget_is_shared_across_the_list() {
    // One evaluator judges one inclusion list. The first candidate fills the
    // allowance to the brim; the second asks for one body more and is refused,
    // even though it would pass on its own. Without a shared ledger a list could
    // multiply an attester's read work by its own length.
    let first = Address::from_low_u64_be(0xA001);
    let second = Address::from_low_u64_be(0xA002);
    let fillers = filler_contracts(14);
    let targets: Vec<Address> = fillers.iter().map(|(a, _)| *a).collect();

    // first: own code + 14 fillers = 15 bodies. second: own code + the same 14
    // = one body more, the 16th, which fits. A third distinct body would not.
    let mut accounts = vec![
        (
            first,
            extcodesize_then_approve_code(&targets, APPROVE_EXECUTION_AND_PAYMENT),
        ),
        (
            second,
            extcodesize_then_approve_code(&targets, APPROVE_EXECUTION_AND_PAYMENT),
        ),
    ];
    accounts.extend(fillers.clone());
    // A 17th body only the third candidate reads.
    let extra = Address::from_low_u64_be(0xBEEF);
    accounts.push((extra, Bytes::from(vec![0x60, 0xEE, 0x00])));
    let third = Address::from_low_u64_be(0xA003);
    let mut third_targets = targets.clone();
    third_targets.push(extra);
    accounts.push((
        third,
        extcodesize_then_approve_code(&third_targets, APPROVE_EXECUTION_AND_PAYMENT),
    ));

    let (_store, chain, header) = setup_profile2_store_funded(
        &accounts,
        0,
        U256::from(10u64).pow(U256::from(18u64)),
        "focil-code-budget-shared.db",
    )
    .await;
    let evaluator =
        BlockchainProfile2Evaluator::new(&chain, &header, header.state_root, header.gas_limit);

    // 15 bodies charged.
    assert_eq!(
        evaluator.evaluate(&reader_tx(first)),
        Profile2Eligibility::Eligible,
        "the first candidate must fit"
    );
    // Its own code is the 16th; the 14 fillers are already charged and free.
    assert_eq!(
        evaluator.evaluate(&reader_tx(second)),
        Profile2Eligibility::Eligible,
        "a candidate reusing charged bodies must still fit"
    );
    // Own code (17th) is one too many.
    match evaluator.evaluate(&reader_tx(third)) {
        Profile2Eligibility::Ineligible(why) => {
            assert!(
                why.contains("ValidationCodeBudgetExceeded"),
                "expected the budget violation, got {why}"
            );
        }
        other => panic!("expected the list's allowance to be spent, got {other:?}"),
    }

    // The same candidate against a fresh list passes, so the refusal above is
    // the shared ledger and not something about the transaction itself.
    let fresh =
        BlockchainProfile2Evaluator::new(&chain, &header, header.state_root, header.gas_limit);
    assert_eq!(
        fresh.evaluate(&reader_tx(third)),
        Profile2Eligibility::Eligible
    );
}
