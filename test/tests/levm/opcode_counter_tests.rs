//! Tests for the opcode counter, which counts the opcodes of every VM that
//! runs over a `GeneralizedDatabase`.
//!
//! Each test runs a small bytecode with `LEVM::execute_tx` and checks the
//! counts. The rules are the ones of the `CountTracer` in execution-specs:
//! an opcode counts when the interpreter reaches it, and neither an undefined
//! byte nor the implicit `STOP` past the end of the code count.

use super::test_db::TestDatabase;
use bytes::Bytes;
use ethrex_common::{
    Address, U256,
    types::{Account, BlockHeader, Code, EIP1559Transaction, Transaction, TxKind},
};
use ethrex_crypto::NativeCrypto;
use ethrex_levm::db::gen_db::GeneralizedDatabase;
use ethrex_levm::opcodes::Opcode;
use ethrex_levm::vm::VMType;
use ethrex_vm::backends::levm::LEVM;
use once_cell::sync::OnceCell;
use rustc_hash::FxHashMap;
use std::sync::Arc;

const CONTRACT: u64 = 0xC000;
const CALLEE: u64 = 0xC001;
const SENDER: u64 = 0x1000;

fn op(opcode: Opcode) -> u8 {
    opcode.into()
}

fn tx(nonce: u64) -> Transaction {
    Transaction::EIP1559Transaction(EIP1559Transaction {
        chain_id: 1,
        nonce,
        max_priority_fee_per_gas: 1,
        max_fee_per_gas: 10,
        gas_limit: 100_000,
        to: TxKind::Call(Address::from_low_u64_be(CONTRACT)),
        value: U256::zero(),
        data: Bytes::new(),
        access_list: vec![],
        signature_y_parity: 0u8.into(),
        signature_r: U256::one(),
        signature_s: U256::one(),
        inner_hash: OnceCell::new(),
        sender_cache: {
            let cell = OnceCell::new();
            let _ = cell.set(Address::from_low_u64_be(SENDER));
            cell
        },
        cached_canonical: OnceCell::new(),
    })
}

/// A database with `code` in the contract the transactions call, and
/// `callee_code` in a second contract. The fork is the one of the default
/// chain config, which is before Shanghai.
fn database(code: Vec<u8>, callee_code: Vec<u8>) -> GeneralizedDatabase {
    let contract = |code: Vec<u8>| {
        Account::new(
            U256::zero(),
            Code::from_bytecode(Bytes::from(code), &NativeCrypto),
            1,
            FxHashMap::default(),
        )
    };
    let mut accounts = FxHashMap::default();
    accounts.insert(Address::from_low_u64_be(CONTRACT), contract(code));
    accounts.insert(Address::from_low_u64_be(CALLEE), contract(callee_code));
    accounts.insert(
        Address::from_low_u64_be(SENDER),
        Account::new(
            U256::from(10u64).pow(U256::from(19)),
            Code::default(),
            0,
            FxHashMap::default(),
        ),
    );
    GeneralizedDatabase::new(Arc::new(TestDatabase { accounts }))
}

fn execute(db: &mut GeneralizedDatabase, nonce: u64) {
    let header = BlockHeader {
        coinbase: Address::from_low_u64_be(0xCCC),
        base_fee_per_gas: Some(1),
        gas_limit: 30_000_000,
        ..Default::default()
    };
    LEVM::execute_tx(
        &tx(nonce),
        Address::from_low_u64_be(SENDER),
        &header,
        db,
        VMType::L1,
        &NativeCrypto,
        None,
    )
    .expect("the transaction is valid");
}

/// The counts after one transaction calls a contract with `code`.
fn counts_of(code: Vec<u8>) -> Vec<(Opcode, u64)> {
    let mut db = database(code, vec![]);
    db.opcode_counter = Some(Box::default());
    execute(&mut db, 0);
    counts(&db)
}

fn counts(db: &GeneralizedDatabase) -> Vec<(Opcode, u64)> {
    db.opcode_counter
        .as_ref()
        .expect("the counter is on")
        .counts()
        .map(|(opcode, count)| (Opcode::from(opcode), count))
        .collect()
}

#[test]
fn counts_each_opcode_it_reaches() {
    let code = vec![
        op(Opcode::PUSH1),
        1,
        op(Opcode::PUSH1),
        2,
        op(Opcode::ADD),
        op(Opcode::POP),
        op(Opcode::STOP),
    ];
    assert_eq!(
        counts_of(code),
        vec![
            (Opcode::STOP, 1),
            (Opcode::ADD, 1),
            (Opcode::POP, 1),
            (Opcode::PUSH1, 2),
        ]
    );
}

#[test]
fn does_not_count_without_a_counter() {
    let mut db = database(vec![op(Opcode::PUSH1), 1, op(Opcode::STOP)], vec![]);
    execute(&mut db, 0);
    assert!(db.opcode_counter.is_none());
}

#[test]
fn does_not_count_the_implicit_stop() {
    // The code ends after the PUSH1, and the interpreter stops there as if
    // it had read a STOP.
    assert_eq!(
        counts_of(vec![op(Opcode::PUSH1), 1]),
        vec![(Opcode::PUSH1, 1)]
    );
}

#[test]
fn counts_the_jumpdest_of_a_taken_jump() {
    // The JUMP handler runs the JUMPDEST it lands on, so the dispatch loop
    // only sees the JUMPDEST at the start.
    let code = vec![
        op(Opcode::JUMPDEST),
        op(Opcode::PUSH1),
        5,
        op(Opcode::JUMP),
        op(Opcode::INVALID),
        op(Opcode::JUMPDEST),
        op(Opcode::STOP),
    ];
    assert_eq!(
        counts_of(code),
        vec![
            (Opcode::STOP, 1),
            (Opcode::JUMP, 1),
            (Opcode::JUMPDEST, 2),
            (Opcode::PUSH1, 1),
        ]
    );
}

#[test]
fn counts_the_jumpdest_of_a_jumpi_only_if_it_jumps() {
    let jumpi_with_condition = |condition: u8| {
        vec![
            op(Opcode::PUSH1),
            condition,
            op(Opcode::PUSH1),
            6,
            op(Opcode::JUMPI),
            op(Opcode::STOP),
            op(Opcode::JUMPDEST),
            op(Opcode::STOP),
        ]
    };
    assert_eq!(
        counts_of(jumpi_with_condition(1)),
        vec![
            (Opcode::STOP, 1),
            (Opcode::JUMPI, 1),
            (Opcode::JUMPDEST, 1),
            (Opcode::PUSH1, 2),
        ]
    );
    assert_eq!(
        counts_of(jumpi_with_condition(0)),
        vec![(Opcode::STOP, 1), (Opcode::JUMPI, 1), (Opcode::PUSH1, 2)]
    );
}

#[test]
fn counts_an_opcode_that_fails() {
    // The ADD has nothing on the stack to add.
    assert_eq!(counts_of(vec![op(Opcode::ADD)]), vec![(Opcode::ADD, 1)]);

    // The JUMP has no JUMPDEST to land on, so it doesn't run one.
    let code = vec![op(Opcode::PUSH1), 3, op(Opcode::JUMP), op(Opcode::STOP)];
    assert_eq!(counts_of(code), vec![(Opcode::JUMP, 1), (Opcode::PUSH1, 1)]);
}

#[test]
fn does_not_count_an_undefined_opcode() {
    // 0x0c has never been an opcode, 0xfe is the designated invalid one, and
    // PUSH0 is not an opcode in the fork of the test database.
    for undefined in [0x0c, op(Opcode::INVALID), op(Opcode::PUSH0)] {
        let code = vec![op(Opcode::PUSH1), 1, undefined, op(Opcode::STOP)];
        assert_eq!(counts_of(code), vec![(Opcode::PUSH1, 1)]);
    }
}

#[test]
fn counts_the_opcodes_of_a_call() {
    // CALL(gas, callee, 0, 0, 0, 0, 0) with all the gas that is left.
    let code = vec![
        op(Opcode::PUSH1),
        0,
        op(Opcode::DUP1),
        op(Opcode::DUP1),
        op(Opcode::DUP1),
        op(Opcode::DUP1),
        op(Opcode::PUSH2),
        0xC0,
        0x01,
        op(Opcode::GAS),
        op(Opcode::CALL),
        op(Opcode::STOP),
    ];
    let callee_code = vec![op(Opcode::ADDRESS), op(Opcode::POP)];
    let mut db = database(code, callee_code);
    db.opcode_counter = Some(Box::default());
    execute(&mut db, 0);

    assert_eq!(
        counts(&db),
        vec![
            (Opcode::STOP, 1),
            (Opcode::ADDRESS, 1),
            (Opcode::POP, 1),
            (Opcode::GAS, 1),
            (Opcode::PUSH1, 1),
            (Opcode::PUSH2, 1),
            (Opcode::DUP1, 4),
            (Opcode::CALL, 1),
        ]
    );
}

#[test]
fn adds_up_the_transactions_over_one_database() {
    let mut db = database(vec![op(Opcode::PUSH1), 1, op(Opcode::STOP)], vec![]);
    db.opcode_counter = Some(Box::default());
    execute(&mut db, 0);
    execute(&mut db, 1);

    assert_eq!(counts(&db), vec![(Opcode::STOP, 2), (Opcode::PUSH1, 2)]);
}
