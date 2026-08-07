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
use ethrex_blockchain::focil_eligibility::MAX_VERIFY_GAS_PER_TX;
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

    // Over MAX_VERIFY_GAS_PER_TX: priced, then rejected by the per-tx cap →
    // `FillOutcome::Ignored`.
    let ignored_tx = self_verify_tx(ignored_sender, MAX_VERIFY_GAS_PER_TX + 1);

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
    let file = std::fs::File::open(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("fixtures/genesis/execution-api.json"),
    )
    .expect("open execution-api genesis fixture");
    let mut genesis: Genesis =
        serde_json::from_reader(std::io::BufReader::new(file)).expect("parse genesis fixture");
    genesis.config.hegota_time = Some(0);
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
                balance: U256::from(10u64).pow(U256::from(18u64)),
                nonce: 0,
            },
        );
    }
    let mut store =
        Store::new("focil-profile2-store.db", EngineType::InMemory).expect("in-memory store");
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
        slot_number: None,
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

    let evaluator = BlockchainProfile2Evaluator::new(&blockchain, &header, gas_left);
    let report = validator.check_with_profile_2(
        &il,
        &HashSet::new(),
        gas_left,
        &header,
        &store.get_chain_config(),
        &crypto,
        Some(&evaluator),
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

    let evaluator = BlockchainProfile2Evaluator::new(&blockchain, &header, gas_left);
    let report = validator.check_with_profile_2(
        &il,
        &HashSet::new(),
        gas_left,
        &header,
        &store.get_chain_config(),
        &crypto,
        Some(&evaluator),
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
    let evaluator = BlockchainProfile2Evaluator::new(&blockchain, &header, gas_left);
    match evaluator.evaluate(&tx) {
        Profile2Eligibility::Undecided(_) => {}
        other => panic!("expected Undecided, got {other:?}"),
    }

    // Undecided is excused, so the block stands.
    verdict.expect("an undecidable omission must not fail the block");
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
    let evaluator = BlockchainProfile2Evaluator::new(&blockchain, &header, gas_left);
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
    let evaluator = BlockchainProfile2Evaluator::new(&blockchain, &header, gas_left);
    match evaluator.evaluate(&tx) {
        Profile2Eligibility::Ineligible(_) => {}
        other => panic!("expected Ineligible, got {other:?}"),
    }

    verdict.expect("an unverifiable signature must not fail the block");
}
