//! FOCIL Profile 2 omission replay at the VM boundary: the validation surface,
//! the per-list code budget and the three-way verdict, driven through
//! [`LEVM::replay_profile2_validation_prefix`] over signature-less frame
//! transactions whose senders approve through their own code (an empty
//! signature list validates trivially, so these stay crypto-free while running
//! the real dispatch loop, handlers and observer hooks).
//!
//! Case numbers refer to the Test Cases table of `docs/eip-focil-frametx.md`.

use std::sync::Arc;

use bytes::Bytes;
use ethrex_common::types::{
    Account, AccountState, BlockHeader, ChainConfig, Code, CodeMetadata, Frame, FrameMode,
    FrameTransaction, Transaction,
};
use ethrex_common::{Address, H256, U256};
use ethrex_crypto::NativeCrypto;
use ethrex_levm::db::{Database, gen_db::GeneralizedDatabase};
use ethrex_levm::errors::DatabaseError;
use ethrex_levm::vm::VMType;
use ethrex_vm::backends::levm::LEVM;
use ethrex_vm::{CodeBudget, Profile2Replay};
use rustc_hash::FxHashMap;

/// `AA_VOPS_SLOT_COUNT`, the value the spec's constants table pins.
const SLOT_COUNT: u64 = 4;
/// A comfortable per-list code budget: the spec's 16 bodies at the Amsterdam
/// `MAX_CODE_SIZE`.
fn roomy_budget() -> CodeBudget {
    CodeBudget::new(16, 16 * 0x10000)
}

struct Store {
    chain_config: ChainConfig,
    accounts: FxHashMap<Address, Account>,
    /// A code hash the store refuses to serve, standing in for a body the
    /// evaluator does not have.
    missing_code: Option<H256>,
}

impl Database for Store {
    fn get_account_state(&self, address: Address) -> Result<AccountState, DatabaseError> {
        Ok(self
            .accounts
            .get(&address)
            .map(|acc| AccountState {
                nonce: acc.info.nonce,
                balance: acc.info.balance,
                storage_root: *ethrex_common::constants::EMPTY_TRIE_HASH,
                code_hash: acc.info.code_hash,
            })
            .unwrap_or_default())
    }
    fn get_storage_value(&self, address: Address, key: H256) -> Result<U256, DatabaseError> {
        Ok(self
            .accounts
            .get(&address)
            .and_then(|acc| acc.storage.get(&key).copied())
            .unwrap_or_default())
    }
    fn get_block_hash(&self, _: u64) -> Result<H256, DatabaseError> {
        Ok(H256::zero())
    }
    fn get_chain_config(&self) -> Result<ChainConfig, DatabaseError> {
        Ok(self.chain_config)
    }
    fn get_account_code(&self, code_hash: H256) -> Result<Code, DatabaseError> {
        if self.missing_code == Some(code_hash) {
            return Err(DatabaseError::Custom(format!(
                "code body {code_hash:#x} is not available"
            )));
        }
        for acc in self.accounts.values() {
            if acc.info.code_hash == code_hash {
                return Ok(acc.code.clone());
            }
        }
        Ok(Code::default())
    }
    fn get_code_metadata(&self, code_hash: H256) -> Result<CodeMetadata, DatabaseError> {
        for acc in self.accounts.values() {
            if acc.info.code_hash == code_hash {
                return Ok(CodeMetadata {
                    length: acc.code.len() as u64,
                });
            }
        }
        Ok(CodeMetadata { length: 0 })
    }
}

fn hegota_chain_config() -> ChainConfig {
    ChainConfig {
        shanghai_time: Some(0),
        cancun_time: Some(0),
        prague_time: Some(0),
        osaka_time: Some(0),
        amsterdam_time: Some(0),
        hegota_time: Some(0),
        ..Default::default()
    }
}

fn addr(x: u64) -> Address {
    Address::from_low_u64_be(x)
}

fn account(balance: u64, code: Bytes) -> Account {
    Account::new(
        U256::from(balance),
        Code::from_bytecode(code, &NativeCrypto),
        0,
        FxHashMap::default(),
    )
}

/// The accounts land in the VM's cache; the store behind it is empty.
fn db_with(accounts: Vec<(Address, Account)>) -> GeneralizedDatabase {
    let map: FxHashMap<Address, Account> = accounts.into_iter().collect();
    GeneralizedDatabase::new_with_account_state(
        Arc::new(Store {
            chain_config: hegota_chain_config(),
            accounts: FxHashMap::default(),
            missing_code: None,
        }),
        map,
    )
}

/// `PUSH1 scope, PUSH1 0, PUSH1 0, APPROVE, STOP`.
fn approve_code(scope: u8) -> Bytes {
    Bytes::from(vec![0x60, scope, 0x60, 0x00, 0x60, 0x00, 0xAA, 0x00])
}

/// `<push slot>, SLOAD, POP`, then `APPROVE(scope)`; `push_slot` is a complete
/// PUSH instruction (opcode plus immediate).
fn sload_then_approve_code(push_slot: &[u8], scope: u8) -> Bytes {
    let mut code = push_slot.to_vec();
    code.extend_from_slice(&[0x54, 0x50]);
    code.extend_from_slice(&approve_code(scope));
    Bytes::from(code)
}

/// A zero-argument call to `target` through `call_opcode` (`GAS` right before it,
/// as the validation trace rules allow), result discarded, then `APPROVE(scope)`.
fn call_then_approve_code(call_opcode: u8, target: Address, scope: u8) -> Bytes {
    let mut code = vec![0x60, 0x00, 0x60, 0x00, 0x60, 0x00, 0x60, 0x00, 0x73];
    code.extend_from_slice(target.as_bytes());
    code.extend_from_slice(&[0x5A, call_opcode, 0x50]);
    code.extend_from_slice(&approve_code(scope));
    Bytes::from(code)
}

/// A helper that reads its slot 0 and stops: `PUSH1 0, SLOAD, POP, STOP`.
fn sload_slot_zero_code() -> Bytes {
    Bytes::from(vec![0x60, 0x00, 0x54, 0x50, 0x00])
}

fn verify_frame(flags: u8, target: Address, gas_limit: u64) -> Frame {
    Frame {
        mode: FrameMode::Verify as u8,
        flags,
        target: Some(target),
        gas_limit,
        state_gas_limit: 0,
        value: U256::zero(),
        data: Bytes::new(),
    }
}

fn frame_tx(sender: Address, frames: Vec<Frame>) -> Transaction {
    Transaction::FrameTransaction(FrameTransaction {
        chain_id: 0,
        nonce_keys: vec![U256::zero()],
        nonce_seq: 0,
        sender,
        frames,
        signatures: Vec::new(),
        max_priority_fee_per_gas: U256::zero(),
        max_fee_per_gas: U256::zero(),
        max_fee_per_blob_gas: U256::zero(),
        blob_versioned_hashes: Vec::new(),
        ..Default::default()
    })
}

fn self_verify_tx(sender: Address) -> Transaction {
    frame_tx(sender, vec![verify_frame(0x03, sender, 100_000)])
}

fn header() -> BlockHeader {
    BlockHeader {
        timestamp: 0,
        gas_limit: 30_000_000,
        ..Default::default()
    }
}

fn replay(
    tx: &Transaction,
    db: &mut GeneralizedDatabase,
    payer: Address,
    budget: &mut CodeBudget,
) -> Profile2Replay {
    let Transaction::FrameTransaction(frame_tx) = tx else {
        unreachable!("tests build frame transactions")
    };
    let prefix = frame_tx
        .validation_prefix()
        .expect("test transactions match an admitted shape");
    LEVM::replay_profile2_validation_prefix(
        tx,
        &header(),
        db,
        VMType::L1,
        &NativeCrypto,
        &prefix,
        payer,
        SLOT_COUNT,
        budget,
    )
    .expect("the replay runs")
}

fn assert_ineligible_because(verdict: &Profile2Replay, needle: &str) {
    match verdict {
        Profile2Replay::Ineligible(reason) if reason.contains(needle) => {}
        other => panic!("expected Ineligible mentioning {needle:?}, got {other:?}"),
    }
}

/// A `self_verify` prefix reading a sender slot below `AA_VOPS_SLOT_COUNT` is
/// inside the surface and eligible.
#[test]
fn sender_slot_below_slot_count_is_inside_the_surface() {
    let sender = addr(0x5E01);
    let mut db = db_with(vec![(
        sender,
        account(0, sload_then_approve_code(&[0x60, 0x03], 0x03)),
    )]);
    let tx = self_verify_tx(sender);
    let verdict = replay(&tx, &mut db, sender, &mut roomy_budget());
    assert_eq!(verdict, Profile2Replay::Eligible);
}

/// Case 7: the prefix reads storage slot `AA_VOPS_SLOT_COUNT` of `sender`;
/// the omission is justified. The same read passes the mempool's sender-only
/// rule, which is exactly why the two rules must not be intersected.
#[test]
fn sender_slot_at_slot_count_is_outside_the_surface() {
    let sender = addr(0x5E02);
    let mut db = db_with(vec![(
        sender,
        account(0, sload_then_approve_code(&[0x60, 0x04], 0x03)),
    )]);
    let tx = self_verify_tx(sender);
    let verdict = replay(&tx, &mut db, sender, &mut roomy_budget());
    assert_ineligible_because(&verdict, "StorageReadOutsideSurface");
}

/// The `pay` frame's sponsor may read its own low slots: `payer` is part of the
/// surface, which the mempool's sender-only rule would refuse.
#[test]
fn payer_slot_below_slot_count_is_inside_the_surface() {
    let sender = addr(0x5E03);
    let paymaster = addr(0xFA03);
    let mut db = db_with(vec![
        (sender, account(0, approve_code(0x02))),
        (
            paymaster,
            account(1_000_000, sload_then_approve_code(&[0x60, 0x00], 0x01)),
        ),
    ]);
    let tx = frame_tx(
        sender,
        vec![
            verify_frame(0x02, sender, 100_000),
            verify_frame(0x01, paymaster, 100_000),
        ],
    );
    let verdict = replay(&tx, &mut db, paymaster, &mut roomy_budget());
    assert_eq!(verdict, Profile2Replay::Eligible);
}

/// Case 8 and case 10: the `pay` frame reads a `keccak256`-derived slot of
/// `payer`, the location a paymaster's reservation ledger lives at. Outside the
/// surface whatever code the paymaster runs; there is no canonical exemption
/// on this path.
#[test]
fn keccak_derived_payer_slot_is_outside_the_surface_even_for_a_paymaster() {
    let sender = addr(0x5E04);
    let paymaster = addr(0xFA04);
    // PUSH32 of a hash-shaped slot.
    let mut push_slot = vec![0x7F];
    push_slot.extend_from_slice(&[0xAB; 32]);
    let mut db = db_with(vec![
        (sender, account(0, approve_code(0x02))),
        (
            paymaster,
            account(1_000_000, sload_then_approve_code(&push_slot, 0x01)),
        ),
    ]);
    let tx = frame_tx(
        sender,
        vec![
            verify_frame(0x02, sender, 100_000),
            verify_frame(0x01, paymaster, 100_000),
        ],
    );
    let verdict = replay(&tx, &mut db, paymaster, &mut roomy_budget());
    assert_ineligible_because(&verdict, "StorageReadOutsideSurface");
}

/// Case 9, as it can actually happen: a helper reached by `STATICCALL` reads its
/// own storage, a third account outside the surface. The restriction applies
/// transitively through every reachable frame.
#[test]
fn third_account_storage_reached_by_a_call_is_outside_the_surface() {
    let sender = addr(0x5E05);
    let helper = addr(0x4E05);
    let mut db = db_with(vec![
        (
            sender,
            account(0, call_then_approve_code(0xFA, helper, 0x03)),
        ),
        (helper, account(0, sload_slot_zero_code())),
    ]);
    let tx = self_verify_tx(sender);
    let verdict = replay(&tx, &mut db, sender, &mut roomy_budget());
    assert_ineligible_because(&verdict, "StorageReadOutsideSurface");
}

/// The case 9 wording says `DELEGATECALL`, but a delegatecalled library runs in
/// the sender's storage context: its slot 0 read is a read of `sender`'s slot
/// 0, inside the surface, and the transaction is eligible. Pinned so the
/// reading of the surface as "storage owner, not code owner" is explicit.
#[test]
fn delegatecalled_library_reads_the_senders_storage() {
    let sender = addr(0x5E06);
    let library = addr(0x4E06);
    let mut db = db_with(vec![
        (
            sender,
            account(0, call_then_approve_code(0xF4, library, 0x03)),
        ),
        (library, account(0, sload_slot_zero_code())),
    ]);
    let tx = self_verify_tx(sender);
    let verdict = replay(&tx, &mut db, sender, &mut roomy_budget());
    assert_eq!(verdict, Profile2Replay::Eligible);
}

/// Code accounting: each distinct `codeHash` is charged once per list and is
/// free to later replays; a replay that would load one body past
/// `MAX_VALIDATION_CODE_BODIES` is ineligible (case 22) and the budget records
/// that it was exceeded.
#[test]
fn code_budget_charges_each_distinct_body_once_per_list() {
    let sender_a = addr(0x5E07);
    let sender_b = addr(0x5E08);
    let sender_c = addr(0x5E09);
    let helper = addr(0x4E09);
    let mut budget = CodeBudget::new(2, 16 * 0x10000);

    // One body: sender A's approve code.
    let mut db = db_with(vec![(sender_a, account(0, approve_code(0x03)))]);
    assert_eq!(
        replay(&self_verify_tx(sender_a), &mut db, sender_a, &mut budget),
        Profile2Replay::Eligible
    );
    assert_eq!(budget.bodies_loaded, 1);
    assert_eq!(budget.bytes_loaded, 8);

    // The same code under another sender is the same body: charged nothing.
    let mut db = db_with(vec![(sender_b, account(0, approve_code(0x03)))]);
    assert_eq!(
        replay(&self_verify_tx(sender_b), &mut db, sender_b, &mut budget),
        Profile2Replay::Eligible
    );
    assert_eq!(budget.bodies_loaded, 1);

    // A new sender body (2) that then reaches a helper (3): the third body does
    // not fit a budget of two, so the replay does not proceed.
    let mut db = db_with(vec![
        (
            sender_c,
            account(0, call_then_approve_code(0xFA, helper, 0x03)),
        ),
        (helper, account(0, approve_code(0x00))),
    ]);
    let verdict = replay(&self_verify_tx(sender_c), &mut db, sender_c, &mut budget);
    assert_ineligible_because(&verdict, "CodeBudgetExceeded");
    assert!(budget.exceeded);
    assert_eq!(
        budget.bodies_loaded, 2,
        "the sender body was charged before the helper overran the budget"
    );
}

/// A refused charge stops the replay that overran from loading more, and only
/// that replay: the next replay of the same list starts with the flag cleared
/// and every earlier charge kept, so a transaction whose bodies the list has
/// already paid for is still eligible after another transaction overran.
#[test]
fn an_overrun_stops_charging_for_that_replay_only() {
    let sender_a = addr(0x5E20);
    let sender_b = addr(0x5E21);
    let sender_c = addr(0x5E22);
    let helper = addr(0x4E20);
    let stop = Bytes::from(vec![0x00]);
    let mut budget = CodeBudget::new(2, 16 * 0x10000);

    // Two bodies, the sender's and the helper's: the budget is now full.
    let mut db = db_with(vec![
        (
            sender_a,
            account(0, call_then_approve_code(0xFA, helper, 0x03)),
        ),
        (helper, account(0, stop.clone())),
    ]);
    assert_eq!(
        replay(&self_verify_tx(sender_a), &mut db, sender_a, &mut budget),
        Profile2Replay::Eligible
    );
    assert_eq!(budget.bodies_loaded, 2);

    // A third distinct body does not fit: this replay overran.
    let mut db = db_with(vec![(sender_b, account(0, approve_code(0x03)))]);
    let verdict = replay(&self_verify_tx(sender_b), &mut db, sender_b, &mut budget);
    assert_ineligible_because(&verdict, "CodeBudgetExceeded");
    assert_eq!(budget.bodies_loaded, 2, "the refused load is not charged");

    // The same two bodies again, under another sender: already paid for by this
    // list, so nothing is charged and the overrun before does not carry over.
    let mut db = db_with(vec![
        (
            sender_c,
            account(0, call_then_approve_code(0xFA, helper, 0x03)),
        ),
        (helper, account(0, stop)),
    ]);
    assert_eq!(
        replay(&self_verify_tx(sender_c), &mut db, sender_c, &mut budget),
        Profile2Replay::Eligible
    );
    assert_eq!(budget.bodies_loaded, 2);
}

/// The byte bound is separate from the body bound: a single body larger than
/// what remains of `MAX_VALIDATION_CODE_BYTES` is refused at frame entry.
#[test]
fn code_budget_bounds_bytes_as_well_as_bodies() {
    let sender = addr(0x5E0A);
    let mut db = db_with(vec![(sender, account(0, approve_code(0x03)))]);
    // The approve code is eight bytes.
    let mut budget = CodeBudget::new(16, 7);
    let verdict = replay(&self_verify_tx(sender), &mut db, sender, &mut budget);
    assert_ineligible_because(&verdict, "CodeBudgetExceeded");
    assert!(budget.exceeded);
}

/// Charges survive the verdict: a replay that loaded a body and then failed
/// still hands the budget back charged.
#[test]
fn code_budget_charges_survive_an_ineligible_verdict() {
    let sender = addr(0x5E0B);
    let mut db = db_with(vec![(
        sender,
        account(0, sload_then_approve_code(&[0x60, 0x04], 0x03)),
    )]);
    let mut budget = roomy_budget();
    let verdict = replay(&self_verify_tx(sender), &mut db, sender, &mut budget);
    assert_ineligible_because(&verdict, "StorageReadOutsideSurface");
    assert_eq!(budget.bodies_loaded, 1);
    assert!(!budget.exceeded);
}

/// A pre-frame check that refuses the transaction is a computed verdict:
/// `nonce_seq` disagreeing with the sender's nonce makes it ineligible, not
/// undecided, and costs no replay (no body is charged).
#[test]
fn keyed_nonce_mismatch_is_ineligible_and_loads_no_code() {
    let sender = addr(0x5E0C);
    let mut db = db_with(vec![(sender, account(0, approve_code(0x03)))]);
    let Transaction::FrameTransaction(mut inner) = self_verify_tx(sender) else {
        unreachable!()
    };
    inner.nonce_seq = 1;
    let tx = Transaction::FrameTransaction(inner);
    let mut budget = roomy_budget();
    let verdict = replay(&tx, &mut db, sender, &mut budget);
    assert_ineligible_because(&verdict, "Nonce mismatch");
    assert_eq!(budget.bodies_loaded, 0);
}

/// Cases 27 and 28: a code body the evaluator does not hold is neither evidence
/// for nor against. The verdict is `Undecided`, not `Ineligible`.
#[test]
fn missing_code_body_is_undecided() {
    let sender = addr(0x5E0D);
    let sender_account = account(0, approve_code(0x03));
    let code_hash = sender_account.info.code_hash;
    // The account is known to the store, its code is not, and nothing is cached.
    let mut db = GeneralizedDatabase::new(Arc::new(Store {
        chain_config: hegota_chain_config(),
        accounts: [(sender, sender_account)].into_iter().collect(),
        missing_code: Some(code_hash),
    }));
    let verdict = replay(
        &self_verify_tx(sender),
        &mut db,
        sender,
        &mut roomy_budget(),
    );
    assert!(
        matches!(verdict, Profile2Replay::Undecided(_)),
        "a missing code body must leave the verdict undecided, got {verdict:?}"
    );
}

/// A prefix frame that reverts is a computed verdict against the transaction.
#[test]
fn reverting_prefix_frame_is_ineligible() {
    let sender = addr(0x5E0E);
    // PUSH1 0, PUSH1 0, REVERT.
    let mut db = db_with(vec![(
        sender,
        account(0, Bytes::from(vec![0x60, 0x00, 0x60, 0x00, 0xFD])),
    )]);
    let verdict = replay(
        &self_verify_tx(sender),
        &mut db,
        sender,
        &mut roomy_budget(),
    );
    assert_ineligible_because(&verdict, "reverted");
}
