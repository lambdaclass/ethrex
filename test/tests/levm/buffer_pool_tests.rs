//! CREATE initcode is copied into a buffer drawn from the VM's `initcode_pool` and returned
//! to it when the frame ends. (A call's arguments need no buffer: the callee reads them in
//! place in the caller's memory; see `call_args_tests`.)
//!
//! Inside zkVM guests whose bump allocators never free, a fresh copy per CREATE is never
//! reclaimed. These tests pin the reuse, and that a buffer something still references is
//! never handed out again.

use bytes::Bytes;
use ethrex_common::{
    Address, H256, U256,
    evm::calculate_create_address,
    types::{Account, Code, EIP1559Transaction, Fork, InitcodeBuffer, Transaction, TxKind},
};
use ethrex_crypto::NativeCrypto;
use ethrex_levm::{
    call_frame::Stack,
    db::gen_db::GeneralizedDatabase,
    environment::{EVMConfig, Environment},
    errors::ExecutionReport,
    memory::Memory,
    tracing::LevmCallTracer,
    vm::{VM, VMType},
};
use rustc_hash::FxHashMap;
use std::sync::Arc;

use super::test_db::TestDatabase;

const ORIGIN: u64 = 0x1000;
const CALLER: u64 = 0xCA11;
const CREATOR: u64 = 0xC4EA;
const CREATES: u8 = 8;
const INITCODE_SIZE: u16 = 1024;
const GAS_LIMIT: u64 = 10_000_000;
/// Covers a CREATE's regular cost (about 32k gas) but not the 183,600 state gas a new
/// account is charged from it under Amsterdam.
const CREATOR_GAS: u16 = 50_000;

/// Runs CREATE `creates` times in a loop, each time with `INITCODE_SIZE` bytes of zeroed
/// memory as initcode, which stops at once and deploys nothing.
fn create_loop(creates: u8) -> Bytes {
    let mut code = vec![0x60, creates, 0x5b]; // PUSH1 creates, JUMPDEST (pc 2)
    code.push(0x61); // PUSH2 size
    code.extend_from_slice(&INITCODE_SIZE.to_be_bytes());
    code.extend_from_slice(&[0x60, 0x00, 0x60, 0x00]); // offset, value
    code.extend_from_slice(&[0xf0, 0x50]); // CREATE, POP
    code.extend_from_slice(&[0x60, 0x01, 0x90, 0x03]); // counter - 1
    code.extend_from_slice(&[0x80, 0x60, 0x02, 0x57]); // DUP1, PUSH1 2, JUMPI
    code.push(0x00); // STOP
    Bytes::from(code)
}

/// Runs one CREATE with `INITCODE_SIZE` bytes of zeroed memory as initcode.
fn create_once() -> Bytes {
    let mut code = vec![0x61]; // PUSH2 size
    code.extend_from_slice(&INITCODE_SIZE.to_be_bytes());
    code.extend_from_slice(&[0x60, 0x00, 0x60, 0x00]); // offset, value
    code.extend_from_slice(&[0xf0, 0x00]); // CREATE, STOP
    Bytes::from(code)
}

/// Calls `target` once with `gas`, no arguments and no return data.
fn call_with_gas(target: Address, gas: u16) -> Bytes {
    let mut code = vec![0x60, 0x00, 0x60, 0x00]; // retSize, retOffset
    code.extend_from_slice(&[0x60, 0x00, 0x60, 0x00, 0x60, 0x00]); // argsSize, argsOffset, value
    code.push(0x73); // PUSH20 target
    code.extend_from_slice(target.as_bytes());
    code.push(0x61); // PUSH2 gas
    code.extend_from_slice(&gas.to_be_bytes());
    code.extend_from_slice(&[0xf1, 0x50, 0x00]); // CALL, POP, STOP
    Bytes::from(code)
}

/// Capacities of the buffers left in the VM's pools after a transaction.
struct Pools {
    initcode: Vec<usize>,
    /// How many bytes of initcode each pooled initcode buffer still holds.
    initcode_held: Vec<usize>,
}

/// The accounts for a transaction into a contract with `caller_code`.
fn caller_accounts(caller_code: Bytes) -> FxHashMap<Address, Account> {
    let mut accounts: FxHashMap<Address, Account> = FxHashMap::default();
    accounts.insert(
        Address::from_low_u64_be(ORIGIN),
        Account::new(
            U256::from(10u64).pow(18.into()),
            Code::default(),
            0,
            FxHashMap::default(),
        ),
    );
    accounts.insert(
        Address::from_low_u64_be(CALLER),
        Account::new(
            U256::zero(),
            Code::from_bytecode(caller_code, &NativeCrypto),
            1,
            FxHashMap::default(),
        ),
    );
    accounts.insert(
        Address::from_low_u64_be(CREATOR),
        Account::new(
            U256::zero(),
            Code::from_bytecode(create_once(), &NativeCrypto),
            1,
            FxHashMap::default(),
        ),
    );
    accounts
}

/// The state for a transaction into a contract with `caller_code`.
fn caller_db(caller_code: Bytes) -> GeneralizedDatabase {
    db_with(caller_accounts(caller_code))
}

fn db_with(accounts: FxHashMap<Address, Account>) -> GeneralizedDatabase {
    let mut store = TestDatabase::new();
    store.accounts = accounts.clone();
    GeneralizedDatabase::new_with_account_state(Arc::new(store), accounts)
}

/// The environment of a transaction from `ORIGIN`.
fn caller_env(fork: Fork, gas_limit: u64) -> Environment {
    Environment {
        origin: Address::from_low_u64_be(ORIGIN),
        gas_limit,
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
        block_gas_limit: gas_limit.max(30_000_000),
        is_privileged: false,
        fee_token: None,
        disable_balance_check: true,
        disable_nonce_check: false,
        disable_gas_allowance_check: false,
        disable_sender_eoa_check: false,
        is_system_call: false,
    }
}

/// A transaction into the caller contract.
fn caller_tx(gas_limit: u64) -> Transaction {
    Transaction::EIP1559Transaction(EIP1559Transaction {
        chain_id: 1,
        gas_limit,
        to: TxKind::Call(Address::from_low_u64_be(CALLER)),
        ..Default::default()
    })
}

/// Runs a transaction into a contract with `caller_code`.
fn run_caller(caller_code: Bytes) -> Pools {
    let mut db = caller_db(caller_code);
    run_on(&mut db, Fork::Amsterdam, GAS_LIMIT).0
}

/// Runs a transaction into the caller contract in `db`, on `fork` with `gas_limit`.
fn run_on(db: &mut GeneralizedDatabase, fork: Fork, gas_limit: u64) -> (Pools, ExecutionReport) {
    let tx = caller_tx(gas_limit);
    let mut vm = VM::new(
        caller_env(fork, gas_limit),
        db,
        &tx,
        LevmCallTracer::disabled(),
        VMType::L1,
        &NativeCrypto,
        None,
    )
    .expect("VM::new");
    let report = vm.execute().expect("execute");
    assert!(report.is_success(), "caller tx failed: {:?}", report.result);
    let pools = Pools {
        initcode: vm
            .initcode_pool
            .iter()
            .map(InitcodeBuffer::capacity)
            .collect(),
        initcode_held: vm
            .initcode_pool
            .iter()
            .map(|buffer| buffer.initcode().len())
            .collect(),
    };
    (pools, report)
}

/// The buffer pools block execution keeps across its transactions.
#[derive(Default)]
struct BlockPools {
    stacks: Vec<Stack>,
    memories: Vec<Memory>,
    initcode: Vec<InitcodeBuffer>,
}

/// Runs a transaction into a contract with `caller_code` the way block execution does:
/// the VM draws its buffers from `pools` and returns them there afterwards.
fn run_pooled(caller_code: Bytes, pools: &mut BlockPools) {
    let mut db = caller_db(caller_code);
    let tx = caller_tx(GAS_LIMIT);
    let mut vm = VM::new_pooled(
        caller_env(Fork::Amsterdam, GAS_LIMIT),
        &mut db,
        &tx,
        LevmCallTracer::disabled(),
        VMType::L1,
        &NativeCrypto,
        None,
        &mut pools.stacks,
        &mut pools.memories,
        &mut pools.initcode,
    )
    .expect("VM::new_pooled");
    let report = vm.execute().expect("execute");
    assert!(report.is_success(), "caller tx failed: {:?}", report.result);
    vm.reclaim_into(&mut pools.stacks, &mut pools.memories, &mut pools.initcode);
}

#[test]
fn sequential_creates_reuse_one_initcode_buffer() {
    let pools = run_caller(create_loop(CREATES));

    // Each CREATE frame's initcode went back to the pool for the next CREATE to reuse.
    assert_eq!(pools.initcode.len(), 1);
    assert!(pools.initcode[0] >= usize::from(INITCODE_SIZE));
}

#[test]
fn an_initcode_buffer_is_allocated_once_with_room_for_its_padding() {
    let pools = run_caller(create_loop(1));

    // The initcode plus its 33 bytes of STOP padding, in one allocation. Padding a buffer
    // sized to the bare initcode would grow it to twice the initcode by copying it, which
    // a bump allocator never reclaims.
    assert_eq!(pools.initcode, [usize::from(INITCODE_SIZE) + 33]);
}

#[test]
fn a_create_that_fails_after_copying_its_initcode_returns_the_buffer() {
    // The creator runs out of gas on the new account's state gas, after the initcode was
    // copied into a pooled buffer. Its CALL fails, the transaction does not.
    let mut db = caller_db(call_with_gas(
        Address::from_low_u64_be(CREATOR),
        CREATOR_GAS,
    ));

    let (pools, _) = run_on(&mut db, Fork::Amsterdam, GAS_LIMIT);

    // A CREATE that went ahead would also leave its buffer, emptied, in the pool.
    let created = calculate_create_address(Address::from_low_u64_be(CREATOR), 1);
    assert!(
        db.get_account(created).expect("load account").is_empty(),
        "the CREATE did not fail"
    );
    assert_eq!(pools.initcode.len(), 1);
    // Emptied before it went back: no CREATE can run another one's initcode from the pool.
    assert_eq!(pools.initcode_held, [0]);
}

#[test]
fn consecutive_transactions_reuse_one_initcode_buffer() {
    let mut pools = BlockPools::default();

    run_pooled(create_loop(1), &mut pools);
    assert_eq!(pools.initcode.len(), 1);
    let first = pools.initcode[0].initcode().as_ptr();
    run_pooled(create_loop(1), &mut pools);

    assert_eq!(pools.initcode.len(), 1);
    assert_eq!(pools.initcode[0].initcode().as_ptr(), first);
}

#[test]
fn a_create_at_an_existing_address_returns_its_initcode_buffer() {
    // The caller's first CREATE goes to the address given by its nonce of 1, which already
    // holds an account.
    let caller = Address::from_low_u64_be(CALLER);
    let mut accounts = caller_accounts(create_once());
    accounts.insert(
        calculate_create_address(caller, 1),
        Account::new(U256::zero(), Code::default(), 1, FxHashMap::default()),
    );

    let (pools, report) = run_on(&mut db_with(accounts), Fork::Amsterdam, GAS_LIMIT);

    // A collision consumes all the gas handed to the creation, which a CREATE that went
    // ahead and ran the empty initcode would mostly give back.
    assert!(
        report.gas_used > GAS_LIMIT / 2,
        "the CREATE did not collide"
    );
    assert_eq!(pools.initcode, [usize::from(INITCODE_SIZE) + 33]);
    assert_eq!(pools.initcode_held, [0]);
}
