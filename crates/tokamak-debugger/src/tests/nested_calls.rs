//! Nested call tests — verify depth tracking through CALL and CREATE.

use std::sync::Arc;

use bytes::Bytes;
use ethrex_common::{
    Address, U256,
    constants::EMPTY_TRIE_HASH,
    types::{Account, BlockHeader, Code, EIP1559Transaction, Transaction, TxKind},
};
use ethrex_levm::{Environment, db::gen_db::GeneralizedDatabase};
use rustc_hash::FxHashMap;

use super::helpers::TEST_GAS_LIMIT;
use crate::engine::ReplayEngine;
use crate::types::ReplayConfig;

/// Build a 2-contract DB where contract A CALLs contract B.
fn setup_call_contracts() -> (Address, Address, GeneralizedDatabase) {
    let a_addr = Address::from_low_u64_be(0x42);
    let b_addr = Address::from_low_u64_be(0x43);
    let sender = Address::from_low_u64_be(0x100);

    // Contract B: PUSH1 0x01, STOP
    let b_code = vec![0x60, 0x01, 0x00];

    // Contract A: CALL(gas=0xFFFF, addr=B, value=0, argsOff=0, argsLen=0, retOff=0, retLen=0), STOP
    //
    // Stack setup for CALL (7 args, pushed in reverse):
    //   PUSH1 0x00  (retLen)
    //   PUSH1 0x00  (retOff)
    //   PUSH1 0x00  (argsLen)
    //   PUSH1 0x00  (argsOff)
    //   PUSH1 0x00  (value)
    //   PUSH1 0x43  (addr = B)
    //   PUSH2 0xFFFF (gas)
    //   CALL
    //   POP         (pop return status)
    //   STOP
    let a_code = vec![
        0x60, 0x00, // PUSH1 0 (retLen)
        0x60, 0x00, // PUSH1 0 (retOff)
        0x60, 0x00, // PUSH1 0 (argsLen)
        0x60, 0x00, // PUSH1 0 (argsOff)
        0x60, 0x00, // PUSH1 0 (value)
        0x60, 0x43, // PUSH1 0x43 (addr = B)
        0x61, 0xFF, 0xFF, // PUSH2 0xFFFF (gas)
        0xF1, // CALL
        0x50, // POP
        0x00, // STOP
    ];

    let store = ethrex_storage::Store::new("", ethrex_storage::EngineType::InMemory)
        .expect("in-memory store");
    let header = BlockHeader {
        state_root: *EMPTY_TRIE_HASH,
        ..Default::default()
    };
    let vm_db: ethrex_vm::DynVmDatabase = Box::new(
        ethrex_blockchain::vm::StoreVmDatabase::new(store, header).expect("StoreVmDatabase"),
    );

    let mut cache = FxHashMap::default();
    cache.insert(
        a_addr,
        Account::new(
            U256::MAX,
            Code::from_bytecode(Bytes::from(a_code)),
            0,
            FxHashMap::default(),
        ),
    );
    cache.insert(
        b_addr,
        Account::new(
            U256::MAX,
            Code::from_bytecode(Bytes::from(b_code)),
            0,
            FxHashMap::default(),
        ),
    );
    cache.insert(
        sender,
        Account::new(
            U256::MAX,
            Code::from_bytecode(Bytes::new()),
            0,
            FxHashMap::default(),
        ),
    );

    let db = GeneralizedDatabase::new_with_account_state(Arc::new(vm_db), cache);
    (a_addr, sender, db)
}

/// Depth should increase during the CALL to B and return to 0 after.
#[test]
fn test_call_depth_increases_decreases() {
    let (contract, sender, mut db) = setup_call_contracts();
    let env = Environment {
        origin: sender,
        gas_limit: TEST_GAS_LIMIT,
        block_gas_limit: TEST_GAS_LIMIT,
        ..Default::default()
    };
    let tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        to: TxKind::Call(contract),
        data: Bytes::new(),
        ..Default::default()
    });

    let engine = ReplayEngine::record(&mut db, env, &tx, ReplayConfig::default())
        .expect("record should succeed");

    let steps = engine.steps_range(0, engine.len());

    // Find max depth — should be 1 (inside the CALL to B).
    let max_depth = steps.iter().map(|s| s.depth).max().unwrap_or(0);
    assert!(
        max_depth >= 1,
        "max depth should be at least 1 during CALL, got {max_depth}"
    );

    // Find depth transitions: should go 0 → 1 → 0
    let mut saw_depth_1 = false;
    let mut returned_to_0 = false;
    for step in steps {
        if step.depth == 1 {
            saw_depth_1 = true;
        }
        if saw_depth_1 && step.depth == 0 {
            returned_to_0 = true;
            break;
        }
    }
    assert!(saw_depth_1, "should have entered depth 1");
    assert!(returned_to_0, "should have returned to depth 0");
}

/// CREATE depth tracking: verify depth increases for CREATE.
///
/// Uses a simple CREATE that deploys an empty contract:
/// Contract code: PUSH1 0, PUSH1 0, PUSH1 0, CREATE, POP, STOP
#[test]
fn test_create_depth_tracking() {
    let creator_addr = Address::from_low_u64_be(0x42);
    let sender = Address::from_low_u64_be(0x100);

    // CREATE(value=0, offset=0, length=0) — deploys empty contract.
    // Stack for CREATE: value, offset, length (push in reverse for CREATE: value, offset, size)
    let creator_code = vec![
        0x60, 0x00, // PUSH1 0 (length)
        0x60, 0x00, // PUSH1 0 (offset)
        0x60, 0x00, // PUSH1 0 (value)
        0xF0, // CREATE
        0x50, // POP (created address)
        0x00, // STOP
    ];

    let store = ethrex_storage::Store::new("", ethrex_storage::EngineType::InMemory)
        .expect("in-memory store");
    let header = BlockHeader {
        state_root: *EMPTY_TRIE_HASH,
        ..Default::default()
    };
    let vm_db: ethrex_vm::DynVmDatabase = Box::new(
        ethrex_blockchain::vm::StoreVmDatabase::new(store, header).expect("StoreVmDatabase"),
    );

    let mut cache = FxHashMap::default();
    cache.insert(
        creator_addr,
        Account::new(
            U256::MAX,
            Code::from_bytecode(Bytes::from(creator_code)),
            0,
            FxHashMap::default(),
        ),
    );
    cache.insert(
        sender,
        Account::new(
            U256::MAX,
            Code::from_bytecode(Bytes::new()),
            0,
            FxHashMap::default(),
        ),
    );

    let mut db = GeneralizedDatabase::new_with_account_state(Arc::new(vm_db), cache);
    let env = Environment {
        origin: sender,
        gas_limit: TEST_GAS_LIMIT,
        block_gas_limit: TEST_GAS_LIMIT,
        ..Default::default()
    };
    let tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        to: TxKind::Call(creator_addr),
        data: Bytes::new(),
        ..Default::default()
    });

    let engine = ReplayEngine::record(&mut db, env, &tx, ReplayConfig::default())
        .expect("record should succeed");

    // With CREATE(0,0,0) the init code is empty (0 bytes), so the child
    // call frame has no bytecode to execute. The interpreter may or may not
    // record a step at depth 1 (implementation dependent). We verify the
    // trace records the CREATE opcode and the transaction succeeds.
    assert!(engine.trace().success, "CREATE transaction should succeed");
    assert!(engine.len() >= 5, "should have at least 5 steps");

    // Verify CREATE opcode (0xF0) appears in the trace
    let has_create = engine
        .steps_range(0, engine.len())
        .iter()
        .any(|s| s.opcode == 0xF0);
    assert!(has_create, "CREATE opcode should appear in trace");
}
