//! EIP-8288 gas of a dependency transaction. From IStar a block carries one proof
//! for the dependencies of its transactions, and a transaction pays
//! `LEANSTARK_VERIFICATION_GAS` for every LEANSPHINCS entry of its signature
//! list, in place of the witness gas charged for each dependency before. The
//! entry's own verification cost is charged under both forks.
//!
//! The charge is part of the intrinsic gas, so it is fixed here on the
//! transaction alone and then on what the VM reports having used.

use bytes::Bytes;
use ethrex_blockchain::vm::StoreVmDatabase;
use ethrex_common::types::{
    Account, BlockHeader, Code, FRAME_RECEIPT_STATUS_SUCCESS, FRAME_SIG_SCHEME_ARBITRARY, Fork,
    Frame, FrameMode, FrameSignature, FrameTransaction, LEANSPHINCS_VERIFICATION_GAS,
    LEANSPHINCS_WITNESS_GAS, LEANSTARK_VERIFICATION_GAS, Transaction,
};
use ethrex_common::{Address, H256, U256, constants::EMPTY_TRIE_HASH};
use ethrex_crypto::NativeCrypto;
use ethrex_levm::db::gen_db::GeneralizedDatabase;
use ethrex_levm::environment::{EVMConfig, Environment};
use ethrex_levm::errors::ExecutionReport;
use ethrex_levm::tracing::LevmCallTracer;
use ethrex_levm::vm::{VM, VMType};
use ethrex_storage::Store;
use ethrex_vm::DynVmDatabase;
use rustc_hash::FxHashMap;
use std::sync::Arc;

use super::eip8504_tests::{account_code, dependency_entries};

const CHAIN_ID: u64 = 1;
const BASE_FEE: u64 = 1;
const SENDER: Address = Address::repeat_byte(0xAA);
const RECIPIENT: Address = Address::repeat_byte(0xBB);

/// `[VERIFY, SENDER]` from `SENDER` with `entries` as the signature list: its
/// validation frame pointed at entry 0 of the list, and a call to `RECIPIENT`.
fn transfer(entries: Vec<FrameSignature>) -> FrameTransaction {
    let verify_data = [0u8; 32];
    FrameTransaction {
        chain_id: CHAIN_ID,
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

/// Executes `tx` in a block of `fork`, from an account whose validation code
/// approves a signature under `key`.
fn run(fork: Fork, key: H256, tx: FrameTransaction) -> ExecutionReport {
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
            Code::from_bytecode(Bytes::from(account_code(key)), &NativeCrypto),
            0,
            FxHashMap::default(),
        ),
    );
    let mut db = GeneralizedDatabase::new_with_account_state(Arc::new(store), cache);

    let env = Environment {
        origin: tx.sender,
        gas_limit: tx.max_gas_at(fork),
        block_gas_limit: (i64::MAX - 1) as u64,
        config: EVMConfig::new(fork, EVMConfig::canonical_values(fork)),
        chain_id: U256::from(CHAIN_ID),
        base_fee_per_gas: U256::from(BASE_FEE),
        gas_price: tx
            .fee_budget_at(fork, U256::from(BASE_FEE), U256::zero())
            .map_or(U256::from(BASE_FEE), |budget| budget.gas_price),
        tx_nonce: tx.nonce_seq,
        ..Default::default()
    };
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
    vm.execute().expect("the transaction is valid")
}

#[test]
fn intrinsic_gas_of_a_dependency_transaction_before_and_after_istar() {
    let key = H256::repeat_byte(0x5A);
    let tx = transfer(dependency_entries(&[(H256::zero(), key)]));
    assert_eq!(tx.dependencies().len(), 1);

    // What does not depend on the fork: the transaction's fixed cost, its two
    // frames and the verification cost of its one entry, and the calldata of
    // the frames, of the entry's signer and of the nonce fields.
    assert_eq!(LEANSPHINCS_VERIFICATION_GAS, 3_000);
    let fixed = 12_000 + 2 * 475 + 3_000;
    let data = tx.data_cost();
    assert_eq!(data, 672);

    // Before IStar the transaction pays for the witness the block carries for its
    // dependency: 7,740 bytes at the calldata floor of 64 gas per byte.
    assert_eq!(LEANSPHINCS_WITNESS_GAS, 495_360);
    assert_eq!(tx.dependency_gas(Fork::Hegota), 495_360);
    assert_eq!(tx.mandatory_gas_at(Fork::Hegota), fixed + 495_360);
    assert_eq!(tx.mandatory_gas_at(Fork::Hegota) + data, 511_982);

    // From IStar it pays its share of the block's dependency proof instead.
    assert_eq!(LEANSTARK_VERIFICATION_GAS, 30_000);
    assert_eq!(tx.dependency_gas(Fork::IStar), 30_000);
    assert_eq!(tx.mandatory_gas_at(Fork::IStar), fixed + 30_000);
    assert_eq!(tx.mandatory_gas_at(Fork::IStar) + data, 46_622);

    // Both sides of `max_gas` move by the same amount, so `max_gas` does too.
    for (at_hegota, at_istar) in [
        (
            tx.standard_gas_limit_at(Fork::Hegota),
            tx.standard_gas_limit_at(Fork::IStar),
        ),
        (
            tx.calldata_floor_total_at(Fork::Hegota),
            tx.calldata_floor_total_at(Fork::IStar),
        ),
        (tx.max_gas_at(Fork::Hegota), tx.max_gas_at(Fork::IStar)),
    ] {
        assert_eq!(at_hegota - at_istar, 495_360 - 30_000);
    }

    // LStar keeps the quantities of IStar.
    assert_eq!(tx.dependency_gas(Fork::LStar), 30_000);
    assert_eq!(
        tx.mandatory_gas_at(Fork::LStar),
        tx.mandatory_gas_at(Fork::IStar)
    );
    assert_eq!(
        tx.standard_gas_limit_at(Fork::LStar),
        tx.standard_gas_limit_at(Fork::IStar)
    );
    assert_eq!(
        tx.calldata_floor_total_at(Fork::LStar),
        tx.calldata_floor_total_at(Fork::IStar)
    );
    assert_eq!(tx.max_gas_at(Fork::LStar), tx.max_gas_at(Fork::IStar));

    // The quantities without a fork are those of Hegota, where frame
    // transactions start.
    assert_eq!(tx.mandatory_gas(), tx.mandatory_gas_at(Fork::Hegota));
    assert_eq!(tx.max_gas(), tx.max_gas_at(Fork::Hegota));
    assert_eq!(
        Transaction::FrameTransaction(tx.clone()).gas_limit_at(Fork::IStar),
        tx.max_gas_at(Fork::IStar)
    );
}

#[test]
fn execution_charges_the_proof_gas_in_place_of_the_witness_gas_from_istar() {
    let key = H256::repeat_byte(0x5A);
    let tx = transfer(dependency_entries(&[(H256::zero(), key)]));
    // The entry's verification cost is the same under every fork: it is a
    // function of the signature list alone.
    assert_eq!(
        tx.signature_verification_cost(),
        LEANSPHINCS_VERIFICATION_GAS
    );

    let at_hegota = run(Fork::Hegota, key, tx.clone());
    let at_istar = run(Fork::IStar, key, tx.clone());
    let at_lstar = run(Fork::LStar, key, tx);
    assert!(at_hegota.is_success(), "{at_hegota:?}");
    assert!(at_istar.is_success(), "{at_istar:?}");
    assert!(at_lstar.is_success(), "{at_lstar:?}");

    // The frames do the same work under the three forks, so what the transaction
    // uses differs by the two charges alone, and not at all from IStar to LStar.
    assert_eq!(
        at_hegota.gas_used - at_istar.gas_used,
        LEANSPHINCS_WITNESS_GAS - LEANSTARK_VERIFICATION_GAS
    );
    assert_eq!(at_istar.gas_used, at_lstar.gas_used);

    // An entry is not a frame: under every fork the transaction has the receipts
    // of its two frames, and each frame uses the same gas.
    let frames_at_hegota = at_hegota.frame_results.expect("frame results present");
    let frames_at_istar = at_istar.frame_results.expect("frame results present");
    let frames_at_lstar = at_lstar.frame_results.expect("frame results present");
    assert_eq!(frames_at_hegota.len(), 2);
    assert!(
        frames_at_hegota
            .iter()
            .all(|frame| frame.0 == FRAME_RECEIPT_STATUS_SUCCESS)
    );
    assert_eq!(frames_at_hegota, frames_at_istar);
    assert_eq!(frames_at_istar, frames_at_lstar);
}

#[test]
fn proof_gas_is_charged_for_every_leansphincs_entry() {
    let key = H256::repeat_byte(0x5A);
    let other_key = H256::repeat_byte(0x5B);

    // The same entry twice is one dependency: one witness before IStar, two
    // declarations from it.
    let twice = transfer(dependency_entries(&[
        (H256::zero(), key),
        (H256::zero(), key),
    ]));
    assert_eq!(twice.dependencies().len(), 1);
    assert_eq!(twice.dependency_gas(Fork::Hegota), LEANSPHINCS_WITNESS_GAS);
    assert_eq!(
        twice.dependency_gas(Fork::IStar),
        2 * LEANSTARK_VERIFICATION_GAS
    );

    // Declarations are counted over the whole signature list, whatever their
    // message and key.
    let three = transfer(dependency_entries(&[
        (H256::zero(), key),
        (H256::zero(), key),
        (H256::repeat_byte(0x01), other_key),
    ]));
    assert_eq!(three.dependencies().len(), 2);
    assert_eq!(
        three.dependency_gas(Fork::Hegota),
        2 * LEANSPHINCS_WITNESS_GAS
    );
    assert_eq!(
        three.dependency_gas(Fork::IStar),
        3 * LEANSTARK_VERIFICATION_GAS
    );

    // Only LEANSPHINCS entries are declarations: an entry of another scheme
    // beside one adds nothing to the charge.
    let mut mixed = transfer(dependency_entries(&[(H256::zero(), key)]));
    mixed.signatures.push(FrameSignature {
        scheme: FRAME_SIG_SCHEME_ARBITRARY,
        signer: None,
        signer_key: None,
        msg: Bytes::new(),
        signature: Bytes::from_static(&[0xAA; 4]),
    });
    assert_eq!(mixed.dependency_gas(Fork::Hegota), LEANSPHINCS_WITNESS_GAS);
    assert_eq!(
        mixed.dependency_gas(Fork::IStar),
        LEANSTARK_VERIFICATION_GAS
    );

    // A transaction without LEANSPHINCS entries pays neither.
    let mut plain = mixed.clone();
    plain.signatures.remove(0);
    assert!(!plain.has_dependencies());
    assert_eq!(plain.dependency_gas(Fork::Hegota), 0);
    assert_eq!(plain.dependency_gas(Fork::IStar), 0);

    // The VM charges what the transaction declares. These transactions do little
    // work, so each one uses its calldata floor: the mandatory costs, the proof gas
    // among them, and 64 gas for each byte of data.
    let once = transfer(dependency_entries(&[(H256::zero(), key)]));
    let used_once = run(Fork::IStar, key, once.clone());
    let used_twice = run(Fork::IStar, key, twice.clone());
    assert!(used_twice.is_success(), "{used_twice:?}");
    assert_eq!(
        used_once.gas_used,
        once.calldata_floor_total_at(Fork::IStar)
    );
    assert_eq!(
        used_twice.gas_used,
        twice.calldata_floor_total_at(Fork::IStar)
    );
    // One entry more costs its share of the proof, its verification cost and the
    // 32 bytes of its signer.
    assert_eq!(
        used_twice.gas_used - used_once.gas_used,
        LEANSTARK_VERIFICATION_GAS + LEANSPHINCS_VERIFICATION_GAS + 32 * 64
    );
}
