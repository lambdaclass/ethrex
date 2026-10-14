//! Allocation counts for code that runs inside zkVM guests, whose bump allocators never
//! free: there, every allocation made per call or per CREATE is memory the guest never gets
//! back. A counting global allocator counts the allocations made on the current thread, so
//! tests running in parallel don't see each other's.

use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
};

use bytes::Bytes;
use ethrex_common::{
    Address, H256, U256,
    types::{
        Account, AccountState, Block, BlockBody, BlockHeader, ChainConfig, Code, CodeMetadata,
        EIP1559Transaction, Fork, InitcodeBuffer, Transaction, TxKind,
    },
};
use ethrex_crypto::NativeCrypto;
use ethrex_l2_rpc::signer::{LocalSigner, Signable, Signer};
use ethrex_levm::{
    db::{Database, gen_db::GeneralizedDatabase},
    environment::{EVMConfig, Environment},
    errors::DatabaseError,
    precompiles::PrecompileCache,
    tracing::LevmCallTracer,
    vm::{VM, VMType},
};
use ethrex_vm::backends::levm::LEVM;
use rustc_hash::FxHashMap;
use secp256k1::SecretKey;
use std::sync::Arc;

#[path = "levm/test_db.rs"]
mod test_db;

const ORIGIN: u64 = 0x1000;
const CALLER: u64 = 0xCA11;
const CALLEE: u64 = 0xCA1E;
const GAS_LIMIT: u64 = 10_000_000;

struct CountingAllocator;

thread_local! {
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
    static ALLOCATED_BYTES: Cell<usize> = const { Cell::new(0) };
}

fn count_allocation(bytes: usize) {
    // `try_with` because the allocator also runs while thread-locals are torn down.
    let _ = ALLOCATIONS.try_with(|count| count.set(count.get() + 1));
    let _ = ALLOCATED_BYTES.try_with(|total| total.set(total.get() + bytes));
}

// SAFETY: every method forwards to `System` unchanged; counting has no effect on the memory.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count_allocation(layout.size());
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        count_allocation(layout.size());
        unsafe { System.alloc_zeroed(layout) }
    }

    // A bump allocator serves a realloc with a new allocation, so it counts as one.
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        count_allocation(new_size);
        unsafe { System.realloc(ptr, layout, new_size) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

/// Runs `f` and returns how many allocations it made on this thread.
fn allocations_in(f: impl FnOnce()) -> usize {
    allocated_in(f).0
}

/// Runs `f` and returns how many allocations it made on this thread, and how many bytes
/// they hold together.
fn allocated_in(f: impl FnOnce()) -> (usize, usize) {
    let before = (ALLOCATIONS.with(Cell::get), ALLOCATED_BYTES.with(Cell::get));
    f();
    (
        ALLOCATIONS.with(Cell::get) - before.0,
        ALLOCATED_BYTES.with(Cell::get) - before.1,
    )
}

#[test]
fn a_repeated_precompile_cache_insert_copies_nothing() {
    let cache = PrecompileCache::new();
    let address = Address::from_low_u64_be(1);
    let calldata = [7u8; 64];
    let output = [9u8; 32];
    cache.insert(address, &calldata, &output, 1);
    assert!(
        cache.get(&address, &calldata).is_some(),
        "the first insert was not cached"
    );

    // The warmer and the executor both insert the calls they compute. The second insert
    // finds the entry, and must not copy the calldata and output only to drop them.
    let allocations = allocations_in(|| cache.insert(address, &calldata, &output, 1));

    assert_eq!(allocations, 0);
}

#[test]
fn the_jumpdest_bitmap_is_built_in_one_allocation() {
    // Every byte a JUMPDEST, so the bitmap is kept rather than swapped for the shared empty
    // one. Building it in a Vec and then moving it into an Arc allocates it twice.
    let code = [0x5b; 64];

    let allocations = allocations_in(|| drop(Code::compute_jumpdests(&code)));

    assert_eq!(allocations, 1);
}

#[test]
fn empty_code_gets_the_shared_empty_bitmap_without_allocating() {
    // The first use builds the shared empty bitmap; count from the second.
    drop(Code::compute_jumpdests(&[]));

    let allocations = allocations_in(|| drop(Code::compute_jumpdests(&[])));

    assert_eq!(allocations, 0);
}

#[test]
fn initcode_of_the_same_size_reuses_every_allocation_of_the_one_before() {
    let initcode = [0x5b; 1024];
    let mut buffer = InitcodeBuffer::default();
    buffer.set(&initcode);
    let mut buffer = Code::from_initcode_buffer(buffer).into_initcode_buffer();

    // The bytecode buffer already has room for the padding, so it becomes the code's bytes
    // without growing and comes back whole, and the bitmap comes back with it.
    let allocations = allocations_in(|| {
        buffer.set(&initcode);
        buffer = Code::from_initcode_buffer(std::mem::take(&mut buffer)).into_initcode_buffer();
    });

    assert_eq!(allocations, 0);
}

/// Appends a CALL to `CALLEE` with all the gas left and `args` bytes of memory as arguments.
fn push_call(code: &mut Vec<u8>, args: u16) {
    code.extend_from_slice(&[0x60, 0x00, 0x60, 0x00]); // retSize, retOffset
    code.push(0x61); // PUSH2 argsSize
    code.extend_from_slice(&args.to_be_bytes());
    code.extend_from_slice(&[0x60, 0x00, 0x60, 0x00]); // argsOffset, value
    code.push(0x73); // PUSH20 callee
    code.extend_from_slice(Address::from_low_u64_be(CALLEE).as_bytes());
    code.extend_from_slice(&[0x5a, 0xf1, 0x50]); // GAS, CALL, POP
}

/// Calls `CALLEE` `calls` times with `args` bytes of memory as arguments.
fn repeated_calls(calls: u16, args: u16) -> Bytes {
    let mut code = vec![0x61]; // PUSH2 calls
    code.extend_from_slice(&calls.to_be_bytes());
    let loop_start = u16::try_from(code.len()).expect("short code");
    code.push(0x5b); // JUMPDEST
    push_call(&mut code, args);
    code.extend_from_slice(&[0x60, 0x01, 0x90, 0x03]); // counter - 1
    code.extend_from_slice(&[0x80, 0x61]); // DUP1, PUSH2 loop_start
    code.extend_from_slice(&loop_start.to_be_bytes());
    code.extend_from_slice(&[0x57, 0x00]); // JUMPI, STOP
    Bytes::from(code)
}

/// Runs a transaction into a contract with `caller_code`, and returns the bytes its
/// execution allocates.
fn bytes_allocated_by(caller_code: Bytes) -> usize {
    let mut accounts: FxHashMap<Address, Account> = FxHashMap::default();
    let account = |code: Code, nonce| Account::new(U256::zero(), code, nonce, FxHashMap::default());
    accounts.insert(
        Address::from_low_u64_be(ORIGIN),
        account(Code::default(), 0),
    );
    accounts.insert(
        Address::from_low_u64_be(CALLER),
        account(Code::from_bytecode(caller_code, &NativeCrypto), 1),
    );
    accounts.insert(
        Address::from_low_u64_be(CALLEE),
        account(
            Code::from_bytecode(Bytes::from_static(&[0x00]), &NativeCrypto),
            1,
        ),
    );
    let mut store = test_db::TestDatabase::new();
    store.accounts = accounts.clone();
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
    let mut succeeded = false;
    let (_, bytes) = allocated_in(|| succeeded = vm.execute().expect("execute").is_success());
    assert!(succeeded, "the caller's transaction failed");
    bytes
}

/// Bytes each call of [`repeated_calls`] with `args` bytes of arguments allocates.
fn bytes_per_call(args: u16) -> usize {
    let few = bytes_allocated_by(repeated_calls(100, args));
    let many = bytes_allocated_by(repeated_calls(300, args));
    (many - few) / 200
}

#[test]
fn a_call_allocates_nothing_for_its_arguments() {
    // The callee reads its arguments in place in the caller's memory, so a call allocates
    // the same whatever their size. Calls with no arguments measure everything else a call
    // allocates.
    let without_arguments = bytes_per_call(0);
    for args in [4, 64, 1024] {
        assert_eq!(
            bytes_per_call(args),
            without_arguments,
            "a call with {args} bytes of arguments allocates more than one without"
        );
    }
}

/// Arguments each transaction of [`block_of_calls`] passes in its call.
const BLOCK_CALL_ARGS: u16 = 0xffff;

/// [`test_db::TestDatabase`] on a chain at Shanghai from genesis, which needs no system
/// contract deployed.
struct ShanghaiDatabase(test_db::TestDatabase);

impl Database for ShanghaiDatabase {
    fn get_account_state(&self, address: Address) -> Result<AccountState, DatabaseError> {
        self.0.get_account_state(address)
    }
    fn get_storage_value(&self, address: Address, key: H256) -> Result<U256, DatabaseError> {
        self.0.get_storage_value(address, key)
    }
    fn get_block_hash(&self, block_number: u64) -> Result<H256, DatabaseError> {
        self.0.get_block_hash(block_number)
    }
    fn get_chain_config(&self) -> Result<ChainConfig, DatabaseError> {
        Ok(ChainConfig {
            chain_id: 1,
            shanghai_time: Some(0),
            ..Default::default()
        })
    }
    fn get_account_code(&self, code_hash: H256) -> Result<Code, DatabaseError> {
        self.0.get_account_code(code_hash)
    }
    fn get_code_metadata(&self, code_hash: H256) -> Result<CodeMetadata, DatabaseError> {
        self.0.get_code_metadata(code_hash)
    }
}

/// A block of `txs` transactions from `signer`, each into a contract that calls `CALLEE`
/// once with `BLOCK_CALL_ARGS` bytes of arguments.
async fn block_of_calls(txs: u64, signer: &Signer) -> Block {
    let mut transactions = vec![];
    for nonce in 0..txs {
        let mut tx = Transaction::EIP1559Transaction(EIP1559Transaction {
            chain_id: 1,
            nonce,
            max_fee_per_gas: 7,
            gas_limit: 1_000_000,
            to: TxKind::Call(Address::from_low_u64_be(CALLER)),
            ..Default::default()
        });
        tx.sign_inplace(signer).await.expect("sign");
        transactions.push(tx);
    }
    Block {
        header: BlockHeader {
            number: 1,
            timestamp: 1,
            gas_limit: 30_000_000,
            base_fee_per_gas: Some(7),
            ..Default::default()
        },
        body: BlockBody {
            transactions,
            ommers: vec![],
            withdrawals: Some(vec![]),
        },
    }
}

/// Executes `block` with `LEVM::execute_block`, and returns the bytes it allocates.
fn bytes_allocated_executing(block: &Block, sender: Address) -> usize {
    let mut code = vec![];
    push_call(&mut code, BLOCK_CALL_ARGS);
    code.push(0x00); // STOP
    let mut accounts: FxHashMap<Address, Account> = FxHashMap::default();
    accounts.insert(
        sender,
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
            Code::from_bytecode(Bytes::from(code), &NativeCrypto),
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
    let mut store = test_db::TestDatabase::new();
    store.accounts = accounts.clone();
    let mut db =
        GeneralizedDatabase::new_with_account_state(Arc::new(ShanghaiDatabase(store)), accounts);
    let mut receipts = 0;
    let (_, bytes) = allocated_in(|| {
        let (result, _) = LEVM::execute_block(block, &mut db, VMType::L1, &NativeCrypto, None)
            .expect("execute_block");
        receipts = result.receipts.len();
    });
    assert_eq!(receipts, block.body.transactions.len());
    bytes
}

#[tokio::test(flavor = "current_thread")]
async fn a_block_does_not_copy_its_call_arguments() {
    let secret = SecretKey::from_slice(&[0x11; 32]).expect("secret key");
    let signer: Signer = LocalSigner::new(secret).into();
    let few = block_of_calls(2, &signer).await;
    let many = block_of_calls(4, &signer).await;

    let per_transaction = (bytes_allocated_executing(&many, signer.address())
        - bytes_allocated_executing(&few, signer.address()))
        / 2;

    // Each transaction passes 64 KiB of arguments to its call, which reads them in place in
    // the caller's memory: all a transaction allocates comes to less than one copy of them.
    assert!(
        per_transaction < usize::from(BLOCK_CALL_ARGS),
        "each transaction allocated {per_transaction} bytes"
    );
}
