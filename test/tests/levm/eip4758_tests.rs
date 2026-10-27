//! Tests for EIP-4758: Deactivate SELFDESTRUCT.
//!
//! From Hegota the opcode is SENDALL: it moves the executing account's whole balance to the
//! beneficiary and halts, and never deletes the account. The EIP-6780 exception that still
//! destroyed an account created in the same transaction is gone, so such an account keeps its
//! code, storage and nonce. Amsterdam controls pin the EIP-6780/EIP-8246 deletion they replace.

use bytes::Bytes;
use ethrex_common::{
    Address, H256, U256,
    constants::EMPTY_KECCAK_HASH,
    evm::calculate_create_address,
    types::{Account, Code, EIP1559Transaction, Fork, Transaction, TxKind},
};
use ethrex_crypto::NativeCrypto;
use ethrex_levm::{
    db::gen_db::GeneralizedDatabase,
    environment::{EVMConfig, Environment},
    tracing::LevmCallTracer,
    vm::{VM, VMType},
};
use rustc_hash::FxHashMap;
use std::sync::Arc;

use crate::levm::test_db::TestDatabase;

const SENDER: u64 = 0x1000;
const FACTORY: u64 = 0x3000;
const BENEFICIARY: u64 = 0x4000;
const FACTORY_NONCE: u64 = 1;
const GAS_LIMIT: u64 = 1_000_000;
const CHILD_VALUE: u64 = 5000;
const STORED_VALUE: u8 = 0x2a;

/// `PUSH20 beneficiary, SELFDESTRUCT`.
fn sendall_runtime(beneficiary: Address) -> Vec<u8> {
    let mut code = vec![0x73]; // PUSH20
    code.extend_from_slice(beneficiary.as_bytes());
    code.push(0xff); // SELFDESTRUCT / SENDALL
    code
}

/// Init code that writes `STORED_VALUE` to slot 0 and deploys `runtime` (at most 32 bytes).
fn storing_init_code(runtime: &[u8]) -> Vec<u8> {
    let runtime_len = u8::try_from(runtime.len()).expect("runtime fits a PUSH");
    let mut code = vec![0x60, STORED_VALUE, 0x60, 0x00, 0x55]; // SSTORE(0, STORED_VALUE)
    code.push(0x5f + runtime_len); // PUSH<len> runtime
    code.extend_from_slice(runtime);
    code.extend_from_slice(&[0x60, 0x00, 0x52]); // MSTORE(0, runtime), right-aligned
    code.extend_from_slice(&[0x60, runtime_len, 0x60, 32 - runtime_len, 0xf3]); // RETURN
    code
}

/// Factory code: CREATE a child from `init_code` with `CHILD_VALUE`, CALL it once, STOP.
fn create_then_call(init_code: &[u8]) -> Bytes {
    let mut code = Vec::new();
    for (i, byte) in init_code.iter().enumerate() {
        let offset = u8::try_from(i).expect("init code fits in 256 bytes");
        code.extend_from_slice(&[0x60, *byte, 0x60, offset, 0x53]); // MSTORE8
    }
    let init_len = u8::try_from(init_code.len()).expect("init code fits in 256 bytes");
    code.extend_from_slice(&[0x60, init_len, 0x60, 0x00]); // size, offset
    code.push(0x61); // PUSH2 value
    code.extend_from_slice(&u16::try_from(CHILD_VALUE).unwrap().to_be_bytes());
    code.push(0xf0); // CREATE
    // CALL(gas, child, 0, 0, 0, 0, 0)
    code.extend_from_slice(&[0x60, 0x00, 0x60, 0x00, 0x60, 0x00, 0x60, 0x00, 0x60, 0x00]);
    code.push(0x85); // DUP6: the child address
    code.push(0x5a); // GAS
    code.push(0xf1); // CALL
    code.extend_from_slice(&[0x50, 0x50, 0x00]); // POP, POP, STOP
    Bytes::from(code)
}

fn env(fork: Fork) -> Environment {
    Environment {
        origin: Address::from_low_u64_be(SENDER),
        gas_limit: GAS_LIMIT,
        config: EVMConfig::new(fork, EVMConfig::canonical_values(fork)),
        block_number: 1,
        coinbase: Address::from_low_u64_be(0xCCC),
        timestamp: 1000,
        prev_randao: Some(H256::zero()),
        difficulty: U256::zero(),
        slot_number: U256::zero(),
        chain_id: U256::from(1),
        base_fee_per_gas: U256::zero(),
        base_blob_fee_per_gas: U256::from(1),
        gas_price: U256::zero(),
        block_excess_blob_gas: None,
        block_blob_gas_used: None,
        tx_blob_hashes: vec![],
        tx_max_priority_fee_per_gas: None,
        tx_max_fee_per_gas: Some(U256::zero()),
        tx_max_fee_per_blob_gas: None,
        tx_nonce: 0,
        block_gas_limit: GAS_LIMIT * 2,
        is_privileged: false,
        fee_token: None,
        disable_balance_check: true,
        disable_nonce_check: false,
        disable_gas_allowance_check: false,
        disable_sender_eoa_check: false,
        is_system_call: false,
    }
}

/// Runs a transaction to a factory that creates a child storing to slot 0 and whose runtime
/// SELFDESTRUCTs to `BENEFICIARY`, then calls that child. Returns the child and the database.
fn create_and_sweep_child(fork: Fork) -> (Address, GeneralizedDatabase) {
    let sender = Address::from_low_u64_be(SENDER);
    let factory = Address::from_low_u64_be(FACTORY);
    let beneficiary = Address::from_low_u64_be(BENEFICIARY);
    let init_code = storing_init_code(&sendall_runtime(beneficiary));

    let accounts: FxHashMap<Address, Account> = [
        (
            sender,
            Account::new(
                U256::from(10u64).pow(18.into()),
                Code::default(),
                0,
                FxHashMap::default(),
            ),
        ),
        (
            factory,
            Account::new(
                U256::from(CHILD_VALUE),
                Code::from_bytecode(create_then_call(&init_code), &NativeCrypto),
                FACTORY_NONCE,
                FxHashMap::default(),
            ),
        ),
    ]
    .into_iter()
    .collect();
    let mut store = TestDatabase::new();
    store.accounts = accounts.clone();
    let mut db = GeneralizedDatabase::new_with_account_state(Arc::new(store), accounts);

    let tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        chain_id: 1,
        to: TxKind::Call(factory),
        gas_limit: GAS_LIMIT,
        ..Default::default()
    });
    let mut vm = VM::new(
        env(fork),
        &mut db,
        &tx,
        LevmCallTracer::disabled(),
        VMType::L1,
        &NativeCrypto,
        None,
    )
    .expect("VM::new");
    let report = vm.execute().expect("execute");
    assert!(report.is_success(), "transaction must succeed: {report:?}");

    (calculate_create_address(factory, FACTORY_NONCE), db)
}

#[test]
fn sendall_keeps_an_account_created_in_the_same_transaction() {
    let (child, mut db) = create_and_sweep_child(Fork::Hegota);
    let beneficiary = Address::from_low_u64_be(BENEFICIARY);

    let child_account = db.get_account(child).expect("child account").clone();
    assert_eq!(child_account.info.nonce, 1, "the child keeps its nonce");
    assert_eq!(
        db.get_account_code(child).expect("child code").code(),
        sendall_runtime(beneficiary).as_slice(),
        "the child keeps its code"
    );
    assert_eq!(
        child_account.storage.get(&H256::zero()).copied(),
        Some(U256::from(STORED_VALUE)),
        "the child keeps its storage"
    );
    assert!(child_account.info.balance.is_zero(), "the sweep empties it");
    assert_eq!(
        db.get_account(beneficiary)
            .expect("beneficiary")
            .info
            .balance,
        U256::from(CHILD_VALUE),
        "the beneficiary receives the whole balance"
    );
}

#[test]
fn selfdestruct_destroys_an_account_created_in_the_same_transaction_before_hegota() {
    let (child, mut db) = create_and_sweep_child(Fork::Amsterdam);
    let beneficiary = Address::from_low_u64_be(BENEFICIARY);

    let child_account = db.get_account(child).expect("child account").clone();
    assert_eq!(child_account.info.nonce, 0, "EIP-6780 clears the nonce");
    assert_eq!(
        child_account.info.code_hash, *EMPTY_KECCAK_HASH,
        "EIP-6780 clears the code"
    );
    assert!(
        child_account.storage.values().all(U256::is_zero),
        "EIP-6780 clears the storage"
    );
    assert_eq!(
        db.get_account(beneficiary)
            .expect("beneficiary")
            .info
            .balance,
        U256::from(CHILD_VALUE),
        "the beneficiary receives the whole balance"
    );
}
