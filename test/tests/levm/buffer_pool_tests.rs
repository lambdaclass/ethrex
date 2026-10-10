//! Child-frame calldata and CREATE initcode are copied into buffers drawn from the VM's
//! pools (`calldata_pool`, `initcode_pool`) and returned to them when the frame ends.
//!
//! Inside zkVM guests whose bump allocators never free, a fresh copy per call is never
//! reclaimed: a contract issuing 50000 calls with 50 KB of arguments spent about 2.4 GiB
//! of a 512 MiB heap. These tests pin the reuse, and that a buffer something still
//! references is never handed out again.

use bytes::{Bytes, BytesMut};
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
    errors::DatabaseError,
    precompiles::PrecompileCache,
    tracing::LevmCallTracer,
    vm::{VM, VMType},
};
use rustc_hash::FxHashMap;
use std::sync::Arc;

use super::test_db::TestDatabase;

const ORIGIN: u64 = 0x1000;
const CALLER: u64 = 0xCA11;
const CALLEE: u64 = 0xCA1E;
const SHA256_PRECOMPILE: u64 = 0x02;
const IDENTITY_PRECOMPILE: u64 = 0x04;
const CALLS: u8 = 64;
const ARGS_SIZE: u16 = 1024;
const CREATES: u8 = 8;
const INITCODE_SIZE: u16 = 1024;
const GAS_LIMIT: u64 = 10_000_000;

/// [`TestDatabase`] plus the precompile result cache that block execution has.
struct WithPrecompileCache {
    inner: TestDatabase,
    cache: Option<PrecompileCache>,
}

impl Database for WithPrecompileCache {
    fn get_account_state(&self, address: Address) -> Result<AccountState, DatabaseError> {
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
        self.cache.as_ref()
    }
}

/// Calls `target` `calls` times in a loop, each time passing `ARGS_SIZE` bytes of memory
/// as arguments and asking for no return data.
fn call_loop(target: Address, calls: u8) -> Bytes {
    let mut code = vec![0x60, calls, 0x5b]; // PUSH1 calls, JUMPDEST (pc 2)
    code.extend_from_slice(&[0x60, 0x00, 0x60, 0x00]); // retSize, retOffset
    code.push(0x61); // PUSH2 argsSize
    code.extend_from_slice(&ARGS_SIZE.to_be_bytes());
    code.extend_from_slice(&[0x60, 0x00, 0x60, 0x00]); // argsOffset, value
    code.push(0x73); // PUSH20 target
    code.extend_from_slice(target.as_bytes());
    code.extend_from_slice(&[0x5a, 0xf1, 0x50]); // GAS, CALL, POP
    code.extend_from_slice(&[0x60, 0x01, 0x90, 0x03]); // counter - 1
    code.extend_from_slice(&[0x80, 0x60, 0x02, 0x57]); // DUP1, PUSH1 2, JUMPI
    code.push(0x00); // STOP
    Bytes::from(code)
}

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

/// Capacities of the buffers left in the VM's pools after a transaction.
struct Pools {
    calldata: Vec<usize>,
    initcode: Vec<usize>,
}

/// Runs a transaction into the caller contract and returns the capacities of the buffers
/// left in the calldata pool.
fn pooled_buffer_capacities(target: Address) -> Vec<usize> {
    run_caller(call_loop(target, CALLS), None).calldata
}

/// Runs a transaction into a contract with `caller_code`.
fn run_caller(caller_code: Bytes, precompile_cache: Option<PrecompileCache>) -> Pools {
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
        Address::from_low_u64_be(CALLEE),
        Account::new(
            U256::zero(),
            Code::from_bytecode(Bytes::from_static(&[0x00]), &NativeCrypto),
            1,
            FxHashMap::default(),
        ),
    );
    let mut store = TestDatabase::new();
    store.accounts = accounts.clone();
    let store = WithPrecompileCache {
        inner: store,
        cache: precompile_cache,
    };
    let mut db = GeneralizedDatabase::new_with_account_state(Arc::new(store), accounts);

    let fork = Fork::Amsterdam;
    let env = Environment {
        origin: Address::from_low_u64_be(ORIGIN),
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
        block_gas_limit: 30_000_000,
        is_privileged: false,
        fee_token: None,
        disable_balance_check: true,
        disable_nonce_check: false,
        disable_gas_allowance_check: false,
        disable_sender_eoa_check: false,
        is_system_call: false,
    };
    let tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        chain_id: 1,
        gas_limit: GAS_LIMIT,
        to: TxKind::Call(Address::from_low_u64_be(CALLER)),
        ..Default::default()
    });

    let mut vm = VM::new(
        env,
        &mut db,
        &tx,
        LevmCallTracer::disabled(),
        VMType::L1,
        &NativeCrypto,
        None,
    )
    .expect("VM::new");
    let report = vm.execute().expect("execute");
    assert!(report.is_success(), "caller tx failed: {:?}", report.result);
    Pools {
        calldata: vm.calldata_pool.iter().map(BytesMut::capacity).collect(),
        initcode: vm.initcode_pool.iter().map(BytesMut::capacity).collect(),
    }
}

#[test]
fn sequential_calls_reuse_one_calldata_buffer() {
    let capacities = pooled_buffer_capacities(Address::from_low_u64_be(CALLEE));

    // Every call took the buffer the previous one returned, so a single buffer is left.
    // Without reuse there would be none (never returned) or one per call (never taken).
    assert_eq!(capacities.len(), 1);
    assert!(capacities[0] >= usize::from(ARGS_SIZE));
}

#[test]
fn calldata_still_shared_by_an_output_is_not_recycled() {
    // The identity precompile returns its input, so the caller keeps that buffer as
    // return data: handing it out again would let the next call overwrite it.
    let capacities = pooled_buffer_capacities(Address::from_low_u64_be(IDENTITY_PRECOMPILE));

    assert!(capacities.is_empty());
}

#[test]
fn a_cached_precompile_call_does_not_pin_the_calldata_buffer() {
    // Block execution caches precompile results. The cache keeps its own copy of the
    // input, so the call buffer still goes back to the pool instead of being held, with
    // all its capacity, by an entry whose budget only counts the input's length. One call,
    // so the result is a miss that gets inserted rather than a hit on an earlier entry.
    let capacities = run_caller(
        call_loop(Address::from_low_u64_be(SHA256_PRECOMPILE), 1),
        Some(PrecompileCache::new()),
    )
    .calldata;

    assert_eq!(capacities.len(), 1);
}

#[test]
fn sequential_creates_reuse_one_initcode_buffer() {
    let pools = run_caller(create_loop(CREATES), None);

    // Each CREATE frame's initcode went back to the pool for the next CREATE to reuse.
    assert_eq!(pools.initcode.len(), 1);
    assert!(pools.initcode[0] >= usize::from(INITCODE_SIZE));
}
