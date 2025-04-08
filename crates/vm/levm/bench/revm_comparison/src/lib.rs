use bytes::Bytes;
use ethrex_common::{
    types::{code_hash, Account, AccountInfo, TxKind},
    Address as EthrexAddress, U256,
};
use ethrex_levm::{
    db::{cache, CacheDB},
    errors::{TxResult, VMError},
    vm::{GeneralizedDatabase, VM},
    Environment,
};
use ethrex_vm::db::ExecutionDB;
use revm::{
    db::BenchmarkDB,
    primitives::{address, Address, Bytecode, TransactTo},
    Evm,
};
use sha3::{Digest, Keccak256};
use std::hint::black_box;
use std::io::Read;
use std::{collections::HashMap, fs::File, sync::Arc};

pub fn run_with_levm(program: &str, runs: u64, calldata: &str) {
    let bytecode = Bytes::from(hex::decode(program).unwrap());
    let calldata = Bytes::from(hex::decode(calldata).unwrap());

    let code_hash = code_hash(&bytecode);
    let sender_address = EthrexAddress::from_low_u64_be(100);
    let accounts = [
        // This is the contract account that is going to be executed
        (
            EthrexAddress::from_low_u64_be(42),
            Account {
                info: AccountInfo {
                    nonce: 0,
                    balance: U256::MAX,
                    code_hash,
                },
                storage: HashMap::new(),
                code: bytecode.clone(),
            },
        ),
        (
            // This is the sender account
            sender_address,
            Account {
                info: AccountInfo {
                    nonce: 0,
                    balance: U256::MAX,
                    code_hash,
                },
                storage: HashMap::new(),
                code: Bytes::new(),
            },
        ),
    ];

    let mut execution_db = ExecutionDB::default();

    accounts.iter().for_each(|(address, account)| {
        execution_db.accounts.insert(*address, account.info.clone());
    });

    let mut db = GeneralizedDatabase::new(Arc::new(execution_db), CacheDB::new());

    cache::insert_account(
        &mut db.cache,
        accounts[0].0,
        ethrex_levm::Account::new(
            accounts[0].1.info.balance,
            accounts[0].1.code.clone(),
            accounts[0].1.info.nonce,
            HashMap::new(),
        ),
    );
    cache::insert_account(
        &mut db.cache,
        accounts[1].0,
        ethrex_levm::Account::new(
            accounts[1].1.info.balance,
            accounts[1].1.code.clone(),
            accounts[1].1.info.nonce,
            HashMap::new(),
        ),
    );

    for _ in 0..runs - 1 {
        let mut vm = new_vm_with_bytecode(&mut db, 0).unwrap();
        vm.call_frames.last_mut().unwrap().calldata = calldata.clone();
        vm.env.gas_limit = u64::MAX - 1;
        vm.env.block_gas_limit = u64::MAX;
        let tx_report = black_box(vm.stateless_execute().unwrap());
        assert!(tx_report.result == TxResult::Success);
    }
    let mut vm = new_vm_with_bytecode(&mut db, 0).unwrap();
    vm.call_frames.last_mut().unwrap().calldata = calldata.clone();
    vm.env.gas_limit = u64::MAX - 1;
    vm.env.block_gas_limit = u64::MAX;
    let tx_report = black_box(vm.stateless_execute().unwrap());
    assert!(tx_report.result == TxResult::Success);

    match tx_report.result {
        TxResult::Success => {
            println!("output: \t\t0x{}", hex::encode(tx_report.output));
        }
        TxResult::Revert(error) => panic!("Execution failed: {:?}", error),
    }
}

pub fn run_with_revm(program: &str, runs: u64, calldata: &str) {
    let rich_acc_address = address!("1000000000000000000000000000000000000000");
    let bytes = hex::decode(program).unwrap();
    let raw = Bytecode::new_raw(bytes.clone().into());

    let mut evm = Evm::builder()
        .modify_tx_env(|tx| {
            tx.caller = rich_acc_address;
            tx.transact_to = TransactTo::Call(Address::ZERO);
            tx.data = hex::decode(calldata).unwrap().into();
        })
        .with_db(BenchmarkDB::new_bytecode(raw))
        .build();

    let result = evm.transact().unwrap();
    assert!(result.result.is_success());

    for _ in 0..runs - 1 {
        let result = black_box(evm.transact()).unwrap();
        assert!(result.result.is_success());
    }
    let result = black_box(evm.transact()).unwrap();
    assert!(result.result.is_success());

    println!("output: \t\t{}", result.result.into_output().unwrap());
}

pub fn generate_calldata(function: &str, n: u64) -> String {
    let function_signature = format!("{}(uint256)", function);
    let hash = Keccak256::digest(function_signature.as_bytes());
    let function_selector = &hash[..4];

    // Encode argument n (uint256, padded to 32 bytes)
    let mut encoded_n = [0u8; 32];
    encoded_n[24..].copy_from_slice(&n.to_be_bytes());

    // Combine the function selector and the encoded argument
    let calldata: Vec<u8> = function_selector
        .iter()
        .chain(encoded_n.iter())
        .copied()
        .collect();

    hex::encode(calldata)
}

pub fn load_contract_bytecode(bench_name: &str) -> String {
    let path = format!(
        "bench/revm_comparison/contracts/bin/{}.bin-runtime",
        bench_name
    );
    load_file_bytecode(&path)
}

fn load_file_bytecode(path: &str) -> String {
    println!("Current directory: {:?}", std::env::current_dir().unwrap());
    println!("Loading bytecode from file {}", path);
    let mut file = File::open(path).unwrap();
    let mut contents = String::new();
    file.read_to_string(&mut contents).unwrap();
    contents
}

pub fn new_vm_with_bytecode(db: &mut GeneralizedDatabase, nonce: u64) -> Result<VM, VMError> {
    new_vm_with_ops_addr_bal_db(EthrexAddress::from_low_u64_be(100), nonce, db)
}

/// This function is for testing purposes only.
fn new_vm_with_ops_addr_bal_db(
    sender_address: EthrexAddress,
    nonce: u64,
    db: &mut GeneralizedDatabase,
) -> Result<VM, VMError> {
    let env = Environment {
        origin: sender_address,
        tx_nonce: nonce,
        gas_limit: 100000000000,
        ..Default::default()
    };

    VM::new(
        TxKind::Call(EthrexAddress::from_low_u64_be(42)),
        env,
        Default::default(),
        Default::default(),
        db,
        Vec::new(),
        None,
    )
}
