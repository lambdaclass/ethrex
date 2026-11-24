//! EIP-8504 proof frames, executed by the VM the way a block executes them: the
//! frame runs no code, consumes its whole execution limit and yields a successful
//! receipt, and account code reads the dependencies it declares through the
//! frame introspection instructions.
//!
//! Whether the witnesses that satisfy those dependencies are valid is checked on
//! the block, before it executes; nothing here involves a signature.

use bytes::Bytes;
use ethrex_blockchain::vm::StoreVmDatabase;
use ethrex_common::types::{
    Account, BlockHeader, Code, FRAME_RECEIPT_STATUS_SUCCESS, Fork, Frame, FrameMode,
    FrameTransaction, LEANSPHINCS_SCHEME, LEANSPHINCS_VERIFICATION_GAS, PROOF_FRAME_MODE,
    Transaction,
};
use ethrex_common::{Address, H256, U256, constants::EMPTY_TRIE_HASH};
use ethrex_crypto::NativeCrypto;
use ethrex_levm::db::gen_db::GeneralizedDatabase;
use ethrex_levm::environment::{EVMConfig, Environment};
use ethrex_levm::errors::{ExecutionReport, TxValidationError, VMError};
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
/// The frame's data is two words: the index of a proof frame and the byte offset
/// of a triple in it. The code approves execution and payment when that frame is
/// a proof frame and the triple is `(LEANSPHINCS_SCHEME, 0, public_key)`, that
/// is, when the transaction depends on a signature of its own signature hash
/// under the account's key. It reverts otherwise.
fn account_code(public_key: H256) -> Vec<u8> {
    let mut code = vec![
        // FRAMEPARAM(mode, index) == PROOF_FRAME_MODE
        0x60,
        0x02, // PUSH1 0x02 (param: mode)
        0x5f,
        0x35, // PUSH0, CALLDATALOAD (frame index)
        0xb3, // FRAMEPARAM
        0x60,
        PROOF_FRAME_MODE, // PUSH1 4
        0x14,             // EQ
        // FRAMEDATACOPY(memOffset 0, dataOffset, length 96, frame index)
        0x5f,
        0x35, // PUSH0, CALLDATALOAD (frame index)
        0x60,
        0x60, // PUSH1 96 (length)
        0x60,
        0x20,
        0x35, // PUSH1 32, CALLDATALOAD (offset of the triple)
        0x5f, // PUSH0 (memOffset)
        0xb2, // FRAMEDATACOPY
        // scheme word == LEANSPHINCS_SCHEME
        0x5f,
        0x51, // PUSH0, MLOAD
        0x60,
        LEANSPHINCS_SCHEME, // PUSH1 0x10
        0x14,
        0x16, // EQ, AND
        // data_hash == 0: the message is this transaction's signature hash
        0x60,
        0x20,
        0x51, // PUSH1 32, MLOAD
        0x15,
        0x16, // ISZERO, AND
        // verification_key_hash == public_key
        0x60,
        0x40,
        0x51, // PUSH1 64, MLOAD
        0x7f, // PUSH32 public_key
    ];
    code.extend_from_slice(public_key.as_bytes());
    code.extend_from_slice(&[0x14, 0x16]); // EQ, AND
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

fn frame_tx_env(tx: &FrameTransaction, proof_frames: bool) -> Environment {
    Environment {
        origin: tx.sender,
        gas_limit: tx.max_gas(),
        block_gas_limit: (i64::MAX - 1) as u64,
        config: EVMConfig::new(Fork::Hegota, EVMConfig::canonical_values(Fork::Hegota))
            .with_proof_frames(proof_frames),
        chain_id: U256::from(HARNESS_CHAIN_ID),
        base_fee_per_gas: U256::from(HARNESS_BASE_FEE),
        gas_price: tx
            .fee_budget(U256::from(HARNESS_BASE_FEE), U256::zero())
            .map_or(U256::from(HARNESS_BASE_FEE), |budget| budget.gas_price),
        tx_nonce: tx.nonce_seq,
        ..Default::default()
    }
}

/// A proof frame declaring the given `(data_hash, public key)` pairs.
fn proof_frame(declarations: &[(H256, H256)]) -> Frame {
    let mut data = Vec::new();
    for (data_hash, public_key) in declarations {
        data.extend_from_slice(&[0u8; 31]);
        data.push(LEANSPHINCS_SCHEME);
        data.extend_from_slice(data_hash.as_bytes());
        data.extend_from_slice(public_key.as_bytes());
    }
    Frame {
        mode: PROOF_FRAME_MODE,
        flags: 0,
        target: None,
        gas_limit: declarations.len() as u64 * LEANSPHINCS_VERIFICATION_GAS,
        state_gas_limit: 0,
        value: U256::zero(),
        data: Bytes::from(data),
    }
}

/// `[VERIFY, PROOF, SENDER]`: the account's validation frame pointed at the
/// triple at `triple_offset` of the proof frame, the proof frame, and a call to
/// `RECIPIENT`.
fn transfer(proof: Frame, triple_offset: u64) -> FrameTransaction {
    let mut verify_data = [0u8; 64];
    verify_data[31] = 1; // the proof frame is frame 1
    verify_data[56..64].copy_from_slice(&triple_offset.to_be_bytes());
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
            proof,
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
        signatures: Vec::new(),
        max_fee: U256::from(1_000_000_000_000_000_000u64),
        max_priority_fee_per_gas: U256::from(1),
        blob_versioned_hashes: Vec::new(),
        inner_hash: Default::default(),
        cached_canonical: Default::default(),
    }
}

fn run(
    sender_code: Vec<u8>,
    tx: FrameTransaction,
    proof_frames: bool,
) -> Result<ExecutionReport, VMError> {
    let mut db = seeded_db(sender_code);
    let env = frame_tx_env(&tx, proof_frames);
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
fn proof_frame_runs_no_code_and_consumes_its_whole_limit() {
    let key = H256::repeat_byte(0x5A);
    // Two triples: an explicit message under another key, then the account's own.
    let proof = proof_frame(&[
        (H256::repeat_byte(0x01), H256::repeat_byte(0x77)),
        (H256::zero(), key),
    ]);
    let report =
        run(account_code(key), transfer(proof, 96), true).expect("the transaction is valid");
    assert!(report.is_success(), "{report:?}");

    let frames = report.frame_results.expect("frame results present");
    assert_eq!(frames.len(), 3);
    assert_eq!(
        frames[1],
        (
            FRAME_RECEIPT_STATUS_SUCCESS,
            2 * LEANSPHINCS_VERIFICATION_GAS,
            0,
            Vec::new()
        ),
        "the proof frame reports status 1, its whole limit, no state gas and no logs"
    );
    assert_eq!(frames[0].0, FRAME_RECEIPT_STATUS_SUCCESS);
    assert_eq!(frames[2].0, FRAME_RECEIPT_STATUS_SUCCESS);
    assert_eq!(report.payer_address, Some(SENDER));
}

#[test]
fn proof_frame_gas_is_part_of_what_the_transaction_uses() {
    let key = H256::repeat_byte(0x5A);
    let one = run(
        account_code(key),
        transfer(proof_frame(&[(H256::zero(), key)]), 0),
        true,
    )
    .expect("valid");
    let two = run(
        account_code(key),
        transfer(proof_frame(&[(H256::zero(), key), (H256::zero(), key)]), 0),
        true,
    )
    .expect("valid");
    let frames_one = one.frame_results.expect("frame results present");
    let frames_two = two.frame_results.expect("frame results present");
    // The validation frame does the same work in both, so the only difference in
    // frame gas is the second triple's.
    assert_eq!(frames_one[0].1, frames_two[0].1);
    assert_eq!(
        frames_two[1].1 - frames_one[1].1,
        LEANSPHINCS_VERIFICATION_GAS
    );
    assert!(two.gas_used > one.gas_used);
}

#[test]
fn account_code_rejects_a_proof_frame_that_names_another_key() {
    let key = H256::repeat_byte(0x5A);
    let other = H256::repeat_byte(0x5B);
    let result = run(
        account_code(key),
        transfer(proof_frame(&[(H256::zero(), other)]), 0),
        true,
    );
    assert!(
        result.is_err(),
        "a reverted VERIFY frame invalidates the transaction: {result:?}"
    );
}

#[test]
fn account_code_rejects_an_explicit_message_as_its_own_authorization() {
    let key = H256::repeat_byte(0x5A);
    // The account requires a zero data_hash, the placeholder for this
    // transaction's signature hash. A signature of some other message under its
    // key does not authorize this transaction.
    let result = run(
        account_code(key),
        transfer(proof_frame(&[(H256::repeat_byte(0x01), key)]), 0),
        true,
    );
    assert!(result.is_err(), "{result:?}");
}

#[test]
fn proof_frame_is_a_reserved_mode_while_eip8504_is_inactive() {
    let key = H256::repeat_byte(0x5A);
    let result = run(
        account_code(key),
        transfer(proof_frame(&[(H256::zero(), key)]), 0),
        false,
    );
    match result {
        Err(VMError::TxValidation(TxValidationError::InvalidFrameTransactionFormat(reason))) => {
            assert!(reason.contains("reserved execution mode 4"), "{reason}");
        }
        other => panic!("expected a frame format rejection, got {other:?}"),
    }
}
