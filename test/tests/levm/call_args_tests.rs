//! A CALL-family opcode passes its arguments to the callee as a view of the caller's
//! memory instead of a copy. These tests check that the callee sees exactly the bytes a
//! copy would have held, for contract and precompile callees, and that a call's cost no
//! longer grows with the size of its argument region.

use bytes::Bytes;
use ethrex_common::{
    Address, H256, U256,
    types::{
        Account, AccountState, ChainConfig, Code, CodeMetadata, EIP1559Transaction, Fork,
        Transaction, TxKind,
    },
};
use ethrex_crypto::NativeCrypto;
use ethrex_levm::{
    db::{Database, gen_db::GeneralizedDatabase},
    environment::{EVMConfig, Environment},
    errors::{DatabaseError, ExecutionReport, InternalError, TxResult, VMError},
    precompiles::PrecompileCache,
    tracing::LevmCallTracer,
    vm::{VM, VMType},
};
use rustc_hash::FxHashMap;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use super::test_db::TestDatabase;

const SENDER: u64 = 0x1000;
const CALLER: u64 = 0x2000;
const CALLEE: u64 = 0x3000;
const IDENTITY: u64 = 0x04;
const MODEXP: u64 = 0x05;
const UNLOADABLE: u64 = 0xdead;
/// Under the EIP-7825 transaction gas cap (2^24).
const GAS_LIMIT: u64 = 16_000_000;

const CALL: u8 = 0xf1;
const CALLCODE: u8 = 0xf2;
const DELEGATECALL: u8 = 0xf4;
const STATICCALL: u8 = 0xfa;

/// The test database, optionally with a precompile cache like the one block execution uses.
struct Db {
    inner: TestDatabase,
    precompile_cache: Option<PrecompileCache>,
    /// An account whose state cannot be loaded, as when a witness lacks its node.
    unloadable: Option<Address>,
}

impl Database for Db {
    fn get_account_state(&self, address: Address) -> Result<AccountState, DatabaseError> {
        if self.unloadable == Some(address) {
            return Err(DatabaseError::Custom(format!(
                "{address:#x} is not loadable"
            )));
        }
        self.inner.get_account_state(address)
    }
    fn get_storage_value(&self, address: Address, key: H256) -> Result<U256, DatabaseError> {
        self.inner.get_storage_value(address, key)
    }
    fn get_block_hash(&self, block_number: u64) -> Result<H256, DatabaseError> {
        self.inner.get_block_hash(block_number)
    }
    fn get_chain_config(&self) -> Result<ChainConfig, DatabaseError> {
        self.inner.get_chain_config()
    }
    fn get_account_code(&self, code_hash: H256) -> Result<Code, DatabaseError> {
        self.inner.get_account_code(code_hash)
    }
    fn get_code_metadata(&self, code_hash: H256) -> Result<CodeMetadata, DatabaseError> {
        self.inner.get_code_metadata(code_hash)
    }
    fn precompile_cache(&self) -> Option<&PrecompileCache> {
        self.precompile_cache.as_ref()
    }
}

fn contract(code: Vec<u8>) -> Account {
    Account::new(
        U256::zero(),
        Code::from_bytecode(Bytes::from(code), &NativeCrypto),
        0,
        FxHashMap::default(),
    )
}

/// `store` holding a funded sender and the `CALLER` and `CALLEE` contracts.
fn database(store: Db, caller_code: Vec<u8>, callee_code: Vec<u8>) -> GeneralizedDatabase {
    let accounts: FxHashMap<Address, Account> = [
        (
            Address::from_low_u64_be(SENDER),
            Account::new(
                U256::from(10u64).pow(U256::from(18)),
                Code::default(),
                0,
                FxHashMap::default(),
            ),
        ),
        (Address::from_low_u64_be(CALLER), contract(caller_code)),
        (Address::from_low_u64_be(CALLEE), contract(callee_code)),
    ]
    .into_iter()
    .collect();
    GeneralizedDatabase::new_with_account_state(Arc::new(store), accounts)
}

fn environment() -> Environment {
    let fork = Fork::Osaka;
    Environment {
        origin: Address::from_low_u64_be(SENDER),
        gas_limit: GAS_LIMIT,
        config: EVMConfig::new(fork, EVMConfig::canonical_values(fork)),
        block_number: 1,
        coinbase: Address::from_low_u64_be(0xccc),
        timestamp: 1000,
        prev_randao: Some(H256::zero()),
        difficulty: U256::zero(),
        slot_number: U256::zero(),
        chain_id: U256::from(1),
        base_fee_per_gas: U256::from(1),
        base_blob_fee_per_gas: U256::from(1),
        gas_price: U256::from(1),
        block_excess_blob_gas: None,
        block_blob_gas_used: None,
        tx_blob_hashes: vec![],
        tx_max_priority_fee_per_gas: None,
        tx_max_fee_per_gas: Some(U256::from(1)),
        tx_max_fee_per_blob_gas: None,
        tx_nonce: 0,
        block_gas_limit: GAS_LIMIT,
        is_privileged: false,
        fee_token: None,
        disable_balance_check: false,
        disable_nonce_check: false,
        disable_gas_allowance_check: false,
        disable_sender_eoa_check: false,
        disable_execution_gas_cap: false,
        is_system_call: false,
    }
}

/// A transaction from the sender to the `CALLER` contract.
fn transaction() -> Transaction {
    Transaction::EIP1559Transaction(EIP1559Transaction {
        to: TxKind::Call(Address::from_low_u64_be(CALLER)),
        gas_limit: GAS_LIMIT,
        max_fee_per_gas: 1,
        ..Default::default()
    })
}

/// Runs a transaction from an EOA to the `CALLER` contract.
fn run(caller_code: Vec<u8>, callee_code: Vec<u8>, with_precompile_cache: bool) -> ExecutionReport {
    let store = Db {
        inner: TestDatabase::new(),
        precompile_cache: with_precompile_cache.then(PrecompileCache::new),
        unloadable: None,
    };
    let mut db = database(store, caller_code, callee_code);
    let tx = transaction();
    let mut vm = VM::new(
        environment(),
        &mut db,
        &tx,
        LevmCallTracer::disabled(),
        VMType::L1,
        &NativeCrypto,
        None,
    )
    .expect("vm");
    vm.execute().expect("execution")
}

fn push(code: &mut Vec<u8>, value: &[u8]) {
    assert!((1..=32).contains(&value.len()));
    code.push(0x5f + value.len() as u8);
    code.extend_from_slice(value);
}

fn push_u64(code: &mut Vec<u8>, value: u64) {
    let bytes = value.to_be_bytes();
    let first = bytes.iter().position(|b| *b != 0).unwrap_or(7);
    push(code, &bytes[first..]);
}

/// Pushes the arguments of a CALL-family `opcode` (top of stack last), calls `target` with
/// all the gas left and pops the success flag.
fn call(code: &mut Vec<u8>, opcode: u8, target: u64, args: (u64, u64), ret: (u64, u64)) {
    push_u64(code, ret.1);
    push_u64(code, ret.0);
    push_u64(code, args.1);
    push_u64(code, args.0);
    if matches!(opcode, CALL | CALLCODE) {
        push_u64(code, 0); // value
    }
    push_u64(code, target);
    code.push(0x5a); // GAS
    code.push(opcode);
    code.push(0x50); // POP
}

fn ret(code: &mut Vec<u8>, offset: u64, size: u64) {
    push_u64(code, size);
    push_u64(code, offset);
    code.push(0xf3); // RETURN
}

/// 64 distinct bytes, of which the first `ARGS_LEN` are a call's arguments.
fn pattern() -> [u8; 64] {
    std::array::from_fn(|i| i as u8 + 1)
}
const ARGS_LEN: u64 = 40;
const RET_OFFSET: u64 = 0x100;
const RET_LEN: u64 = 0x90;

/// Writes `pattern()` at memory 0, calls `target` with its first `ARGS_LEN` bytes and
/// returns what the call wrote to the return region.
fn caller_code(opcode: u8, target: u64) -> Vec<u8> {
    let mut code = Vec::new();
    let pattern = pattern();
    push(&mut code, &pattern[..32]);
    push_u64(&mut code, 0);
    code.push(0x52); // MSTORE
    push(&mut code, &pattern[32..]);
    push_u64(&mut code, 0x20);
    code.push(0x52); // MSTORE
    call(
        &mut code,
        opcode,
        target,
        (0, ARGS_LEN),
        (RET_OFFSET, RET_LEN),
    );
    ret(&mut code, RET_OFFSET, RET_LEN);
    code
}

/// First grows its memory to 1 MiB, which moves the shared memory buffer to a new
/// allocation, then returns CALLDATALOAD at 0 and 32 followed by an 80-byte CALLDATACOPY
/// from 0, so both reads cross the end of the input.
fn callee_code() -> Vec<u8> {
    let mut code = Vec::new();
    push_u64(&mut code, 1);
    push_u64(&mut code, 0x10_0000);
    code.push(0x52); // MSTORE
    for offset in [0u64, 0x20] {
        push_u64(&mut code, offset);
        code.push(0x35); // CALLDATALOAD
        push_u64(&mut code, offset);
        code.push(0x52); // MSTORE
    }
    push_u64(&mut code, 0x50); // size
    push_u64(&mut code, 0); // calldata offset
    push_u64(&mut code, 0x40); // memory offset
    code.push(0x37); // CALLDATACOPY
    ret(&mut code, 0, RET_LEN);
    code
}

#[test]
fn every_call_opcode_hands_its_callee_exactly_its_arguments() {
    let args = &pattern()[..ARGS_LEN as usize];
    let mut expected = Vec::new();
    expected.extend_from_slice(&args[..32]);
    expected.extend_from_slice(&args[32..]);
    expected.resize(64, 0);
    expected.extend_from_slice(args);
    expected.resize(RET_LEN as usize, 0);

    for opcode in [CALL, CALLCODE, DELEGATECALL, STATICCALL] {
        let report = run(caller_code(opcode, CALLEE), callee_code(), false);
        assert!(
            report.is_success(),
            "opcode {opcode:#x}: {:?}",
            report.result
        );
        assert_eq!(report.output.as_ref(), expected, "opcode {opcode:#x}");
    }
}

#[test]
fn every_call_opcode_hands_a_precompile_exactly_its_arguments() {
    let mut expected = pattern()[..ARGS_LEN as usize].to_vec();
    expected.resize(RET_LEN as usize, 0);

    for opcode in [CALL, CALLCODE, DELEGATECALL, STATICCALL] {
        let report = run(caller_code(opcode, IDENTITY), vec![], false);
        assert!(
            report.is_success(),
            "opcode {opcode:#x}: {:?}",
            report.result
        );
        assert_eq!(report.output.as_ref(), expected, "opcode {opcode:#x}");
    }
}

/// STATICCALLs MODEXP `iterations` times with an `args_len`-byte argument region of zeros.
/// MODEXP reads only its 96-byte header when the lengths in it are zero.
fn modexp_loop(args_len: u64, iterations: u64) -> Vec<u8> {
    let mut code = Vec::new();
    push_u64(&mut code, iterations);
    let loop_start = code.len() as u64;
    code.push(0x5b); // JUMPDEST
    call(&mut code, STATICCALL, MODEXP, (0, args_len), (0, 0));
    push_u64(&mut code, 1);
    code.push(0x90); // SWAP1
    code.push(0x03); // SUB
    code.push(0x80); // DUP1
    push_u64(&mut code, loop_start);
    code.push(0x57); // JUMPI
    code.push(0x00); // STOP
    code
}

fn time_modexp_loop(args_len: u64, with_precompile_cache: bool) -> Duration {
    let start = Instant::now();
    let report = run(modexp_loop(args_len, 8_000), vec![], with_precompile_cache);
    let elapsed = start.elapsed();
    assert!(
        matches!(report.result, TxResult::Success),
        "{args_len} bytes: {:?}",
        report.result
    );
    elapsed
}

/// A call used to copy its whole argument region, and the precompile cache hashed it, so
/// 8,000 calls over 2 MiB of arguments moved and hashed 16 GiB (over a minute with the
/// cache, several times the 32-byte loop without it, even with the region in CPU cache).
/// Now the cost of a call does not depend on that size: the large-argument loop must take
/// about as long as one with 32 bytes of arguments, whether or not a precompile cache is
/// present.
#[test]
fn a_call_costs_the_same_whatever_the_size_of_its_arguments() {
    for with_precompile_cache in [false, true] {
        let small = time_modexp_loop(32, with_precompile_cache);
        let large = time_modexp_loop(2 * 1024 * 1024, with_precompile_cache);
        assert!(
            large < small * 2 + Duration::from_millis(100),
            "cache {with_precompile_cache}: 32-byte arguments took {small:?}, 2 MiB took {large:?}"
        );
    }
}

/// An error that ends execution, such as a database failure, leaves the failing callee's
/// frame in place, and that frame's arguments are a view of the shared memory buffer.
/// Returning the buffer to the pool must release the view too: the pool checks, in debug
/// builds, that it holds the only reference to the buffer it is about to clear.
#[test]
fn a_callee_that_fails_with_an_error_does_not_keep_the_pooled_memory_buffer() {
    let mut caller_code = Vec::new();
    call(&mut caller_code, CALL, CALLEE, (0, ARGS_LEN), (0, 0));
    let mut callee_code = Vec::new();
    push_u64(&mut callee_code, UNLOADABLE);
    callee_code.push(0x31); // BALANCE
    let store = Db {
        inner: TestDatabase::new(),
        precompile_cache: None,
        unloadable: Some(Address::from_low_u64_be(UNLOADABLE)),
    };
    let mut db = database(store, caller_code, callee_code);
    let tx = transaction();
    let (mut stack_pool, mut memory_pool, mut initcode_pool) = (Vec::new(), Vec::new(), Vec::new());
    let mut vm = VM::new_pooled(
        environment(),
        &mut db,
        &tx,
        LevmCallTracer::disabled(),
        VMType::L1,
        &NativeCrypto,
        None,
        &mut stack_pool,
        &mut memory_pool,
        &mut initcode_pool,
    )
    .expect("vm");

    let result = vm.execute();
    assert!(
        matches!(result, Err(VMError::Internal(InternalError::Database(_)))),
        "{result:?}"
    );
    vm.reclaim_into(&mut stack_pool, &mut memory_pool, &mut initcode_pool);
    assert_eq!(memory_pool.len(), 1);
}
