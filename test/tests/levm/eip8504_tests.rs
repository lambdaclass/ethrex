//! leanSPHINCS dependencies declared in the EIP-8141 signature list, executed by
//! the VM the way a block executes them: a `LEANSPHINCS` entry carries no
//! signature, costs its verification gas as intrinsic gas, and account code
//! reads it through `SIGPARAM`.
//!
//! Whether the witnesses that satisfy those dependencies are valid is checked on
//! the block, before it executes; nothing here involves a signature.

use bytes::Bytes;
use ethrex_blockchain::vm::StoreVmDatabase;
use ethrex_common::types::{
    Account, BlockHeader, Code, FRAME_RECEIPT_STATUS_SUCCESS, FRAME_SIG_SCHEME_LEANSPHINCS, Fork,
    Frame, FrameMode, FrameSignature, FrameTransaction, LEANSPHINCS_VERIFICATION_GAS, Transaction,
};
use ethrex_common::{Address, H256, U256, constants::EMPTY_TRIE_HASH};
use ethrex_crypto::NativeCrypto;
use ethrex_levm::db::gen_db::GeneralizedDatabase;
use ethrex_levm::environment::{EVMConfig, Environment};
use ethrex_levm::errors::{ExecutionReport, VMError};
use ethrex_levm::tracing::LevmCallTracer;
use ethrex_levm::vm::{VM, VMType};
use ethrex_storage::Store;
use ethrex_vm::DynVmDatabase;
use rustc_hash::FxHashMap;
use std::sync::Arc;

const HARNESS_CHAIN_ID: u64 = 1;
const HARNESS_BASE_FEE: u64 = 1;
const SENDER: Address = Address::repeat_byte(0xAA);
const RECIPIENT: Address = Address::repeat_byte(0xBB);

/// Validation code of an account controlled by the leanSPHINCS key `public_key`.
///
/// The frame's data is one word, the index of an entry of the signature list.
/// The code approves execution and payment when that entry has the LEANSPHINCS
/// scheme, its signer is `public_key` and its `msg` is empty, that is, when the
/// transaction depends on a signature of its own signature hash under the
/// account's key. It reverts otherwise, and halts if the entry is of a scheme
/// whose signer is not a key.
pub(crate) fn account_code(public_key: H256) -> Vec<u8> {
    let mut code = vec![
        // SIGPARAM(scheme, index) == LEANSPHINCS
        0x60,
        0x01, // PUSH1 0x01 (param: scheme)
        0x5f,
        0x35, // PUSH0, CALLDATALOAD (entry index)
        0xb4, // SIGPARAM
        0x60,
        FRAME_SIG_SCHEME_LEANSPHINCS, // PUSH1 0x10
        0x14,                         // EQ
        // SIGPARAM(raw signer, index) == public_key
        0x60,
        0x04, // PUSH1 0x04 (param: raw 32-byte signer)
        0x5f,
        0x35, // PUSH0, CALLDATALOAD (entry index)
        0xb4, // SIGPARAM
        0x7f, // PUSH32 public_key
    ];
    code.extend_from_slice(public_key.as_bytes());
    code.extend_from_slice(&[0x14, 0x16]); // EQ, AND
    code.extend_from_slice(&[
        // SIGPARAM(msg, index) == 0: the message is this transaction's signature hash
        0x60, 0x02, // PUSH1 0x02 (param: msg)
        0x5f, 0x35, // PUSH0, CALLDATALOAD (entry index)
        0xb4, // SIGPARAM
        0x15, 0x16, // ISZERO, AND
    ]);
    // JUMPI to the approval, 6 bytes ahead: PUSH1 dest, JUMPI, PUSH0, PUSH0, REVERT.
    let approve_at = u8::try_from(code.len() + 6).expect("the code is shorter than 256 bytes");
    code.extend_from_slice(&[0x60, approve_at, 0x57, 0x5f, 0x5f, 0xfd]);
    // JUMPDEST; APPROVE(offset 0, length 0, scope 3)
    code.extend_from_slice(&[0x5b, 0x60, 0x03, 0x5f, 0x5f, 0xaa]);
    code
}

fn seeded_db(sender_code: Vec<u8>) -> GeneralizedDatabase {
    let in_memory_db = Store::new("", ethrex_storage::EngineType::InMemory).unwrap();
    let header = BlockHeader {
        state_root: *EMPTY_TRIE_HASH,
        ..Default::default()
    };
    let store: DynVmDatabase = Box::new(StoreVmDatabase::new(in_memory_db, header).unwrap());

    let mut cache: FxHashMap<Address, Account> = FxHashMap::default();
    cache.insert(
        SENDER,
        Account::new(
            U256::from(10u64).pow(U256::from(18u64)),
            Code::from_bytecode(Bytes::from(sender_code), &NativeCrypto),
            0,
            FxHashMap::default(),
        ),
    );
    GeneralizedDatabase::new_with_account_state(Arc::new(store), cache)
}

fn frame_tx_env(tx: &FrameTransaction) -> Environment {
    Environment {
        origin: tx.sender,
        gas_limit: tx.max_gas(),
        block_gas_limit: (i64::MAX - 1) as u64,
        config: EVMConfig::new(Fork::Hegota, EVMConfig::canonical_values(Fork::Hegota)),
        chain_id: U256::from(HARNESS_CHAIN_ID),
        base_fee_per_gas: U256::from(HARNESS_BASE_FEE),
        gas_price: tx
            .fee_budget(U256::from(HARNESS_BASE_FEE), U256::zero())
            .map_or(U256::from(HARNESS_BASE_FEE), |budget| budget.gas_price),
        tx_nonce: tx.nonce_seq,
        ..Default::default()
    }
}

/// Signature list entries declaring the given `(message, public key)` pairs. A
/// zero message stands for the transaction's signature hash and is encoded as an
/// empty `msg`. The entry's `signature` is empty: the signature that satisfies
/// the dependency never enters the transaction.
pub(crate) fn dependency_entries(declarations: &[(H256, H256)]) -> Vec<FrameSignature> {
    declarations
        .iter()
        .map(|(message, public_key)| FrameSignature {
            scheme: FRAME_SIG_SCHEME_LEANSPHINCS,
            signer: None,
            signer_key: Some(*public_key),
            msg: if message.is_zero() {
                Bytes::new()
            } else {
                Bytes::copy_from_slice(message.as_bytes())
            },
            signature: Bytes::new(),
        })
        .collect()
}

/// `[VERIFY, SENDER]` with `entries` as the signature list: the account's
/// validation frame pointed at entry `entry_index`, and a call to `RECIPIENT`.
fn transfer(entries: Vec<FrameSignature>, entry_index: u64) -> FrameTransaction {
    let mut verify_data = [0u8; 32];
    verify_data[24..].copy_from_slice(&entry_index.to_be_bytes());
    FrameTransaction {
        chain_id: HARNESS_CHAIN_ID,
        nonce_keys: vec![],
        nonce_seq: 0,
        sender: SENDER,
        frames: vec![
            Frame {
                mode: u8::from(FrameMode::Verify),
                flags: 0x03,
                target: Some(SENDER),
                gas_limit: 100_000,
                state_gas_limit: 0,
                value: U256::zero(),
                data: Bytes::from(verify_data.to_vec()),
            },
            Frame {
                mode: u8::from(FrameMode::Sender),
                flags: 0,
                target: Some(RECIPIENT),
                gas_limit: 50_000,
                state_gas_limit: 0,
                value: U256::zero(),
                data: Bytes::new(),
            },
        ],
        signatures: entries,
        max_fee: U256::from(1_000_000_000_000_000_000u64),
        max_priority_fee_per_gas: U256::from(1),
        blob_versioned_hashes: Vec::new(),
        inner_hash: Default::default(),
        cached_canonical: Default::default(),
    }
}

fn run(sender_code: Vec<u8>, tx: FrameTransaction) -> Result<ExecutionReport, VMError> {
    let mut db = seeded_db(sender_code);
    let env = frame_tx_env(&tx);
    let transaction = Transaction::FrameTransaction(tx);
    let mut vm = VM::new(
        env,
        &mut db,
        &transaction,
        LevmCallTracer::disabled(),
        VMType::L1,
        &NativeCrypto,
        None,
    )
    .expect("VM::new should succeed for a frame tx");
    vm.execute()
}

#[test]
fn a_leansphincs_entry_runs_no_code_and_adds_no_frame() {
    let key = H256::repeat_byte(0x5A);
    // Two entries: an explicit message under another key, then the account's own.
    let entries = dependency_entries(&[
        (H256::repeat_byte(0x01), H256::repeat_byte(0x77)),
        (H256::zero(), key),
    ]);
    let tx = transfer(entries, 1);
    assert_eq!(tx.dependencies().len(), 2);
    let report = run(account_code(key), tx).expect("the transaction is valid");
    assert!(report.is_success(), "{report:?}");

    // The declarations are not frames: only the validation frame and the call
    // yield receipts.
    let frames = report.frame_results.expect("frame results present");
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[0].0, FRAME_RECEIPT_STATUS_SUCCESS);
    assert_eq!(frames[1].0, FRAME_RECEIPT_STATUS_SUCCESS);
    assert_eq!(report.payer_address, Some(SENDER));
}

#[test]
fn a_leansphincs_entry_is_charged_as_intrinsic_gas() {
    let key = H256::repeat_byte(0x5A);
    let other = H256::repeat_byte(0x5B);
    let one = transfer(dependency_entries(&[(H256::zero(), key)]), 0);
    let two = transfer(
        dependency_entries(&[(H256::zero(), key), (H256::zero(), other)]),
        0,
    );
    // Each entry costs its verification gas among the mandatory costs, on top of
    // the witness its dependency adds.
    assert_eq!(
        (two.mandatory_gas() - two.dependency_witness_gas())
            - (one.mandatory_gas() - one.dependency_witness_gas()),
        LEANSPHINCS_VERIFICATION_GAS
    );
    assert_eq!(
        two.signature_verification_cost(),
        2 * LEANSPHINCS_VERIFICATION_GAS
    );

    let report_one = run(account_code(key), one).expect("valid");
    let report_two = run(account_code(key), two).expect("valid");
    let frames_one = report_one.frame_results.expect("frame results present");
    let frames_two = report_two.frame_results.expect("frame results present");
    // The validation frame does the same work in both: the charge is not frame gas.
    assert_eq!(frames_one[0].1, frames_two[0].1);
    assert!(report_two.gas_used > report_one.gas_used);
}

#[test]
fn account_code_rejects_an_entry_that_names_another_key() {
    let key = H256::repeat_byte(0x5A);
    let other = H256::repeat_byte(0x5B);
    let result = run(
        account_code(key),
        transfer(dependency_entries(&[(H256::zero(), other)]), 0),
    );
    assert!(
        result.is_err(),
        "a reverted VERIFY frame invalidates the transaction: {result:?}"
    );
}

#[test]
fn account_code_rejects_an_explicit_message_as_its_own_authorization() {
    let key = H256::repeat_byte(0x5A);
    // The account requires an empty msg, which stands for this transaction's
    // signature hash. A signature of some other message under its key does not
    // authorize this transaction.
    let result = run(
        account_code(key),
        transfer(dependency_entries(&[(H256::repeat_byte(0x01), key)]), 0),
    );
    assert!(result.is_err(), "{result:?}");
}

#[test]
fn account_code_halts_on_an_entry_whose_signer_is_not_a_key() {
    let key = H256::repeat_byte(0x5A);
    // Entry 0 is an ARBITRARY entry: it has no raw 32-byte signer to read.
    let mut entries = vec![FrameSignature {
        scheme: 0,
        signer: None,
        signer_key: None,
        msg: Bytes::new(),
        signature: Bytes::from_static(&[0xAA; 4]),
    }];
    entries.extend(dependency_entries(&[(H256::zero(), key)]));
    assert!(run(account_code(key), transfer(entries.clone(), 0)).is_err());
    // Pointed at its own entry, the same transaction is valid.
    assert!(run(account_code(key), transfer(entries, 1)).is_ok());
}
