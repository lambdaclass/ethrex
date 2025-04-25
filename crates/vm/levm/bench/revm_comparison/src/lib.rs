use bytes::Bytes;
use ethrex_common::{
    types::{code_hash, Account, AccountInfo, EIP1559Transaction, Transaction, TxKind},
    Address as EthrexAddress, U256,
};
use ethrex_levm::{
    db::{gen_db::GeneralizedDatabase, CacheDB},
    errors::{TxResult, VMError},
    vm::VM,
    Environment,
};
use ethrex_vm::db::ExecutionDB;
use revm::{
    db::CacheDB as RevmCacheDB,
    primitives::{alloy_primitives::U160, Address, TransactTo},
    Evm,
};
use sha3::{Digest, Keccak256};
use std::hint::black_box;
use std::io::Read;
use std::{collections::HashMap, fs::File, sync::Arc};

const SENDER_ADDRESS: u64 = 0x64;
const CONTRACT_ADDRESS: u64 = 0x2A;

pub fn run_with_levm(program: &str, runs: u64, calldata: &str) {
    let calldata = Bytes::from(hex::decode(calldata).unwrap());

    let sender_address = EthrexAddress::from_low_u64_be(SENDER_ADDRESS);
    let bytecode = Bytes::from(hex::decode(program).unwrap());
    let execution_db = setup_execution_db(sender_address, bytecode);

    let mut db = GeneralizedDatabase::new(Arc::new(execution_db), CacheDB::new());

    // when using stateful execute() we have to use nonce when instantiating the vm. Otherwise use 0.
    for _nonce in 0..runs {
        let mut vm = new_vm_with_bytecode(&mut db, 0).unwrap();
        vm.call_frames.last_mut().unwrap().calldata = calldata.clone();
        vm.env.gas_limit = u64::MAX - 1;
        vm.env.block_gas_limit = u64::MAX;
        let tx_report = black_box(vm.stateless_execute().unwrap());
        assert!(tx_report.result == TxResult::Success);

        if _nonce == runs - 1 {
            println!("LEVM output: 0x{}", hex::encode(tx_report.output));
        }
    }
}

pub fn run_with_revm(program: &str, runs: u64, calldata: &str) {
    let sender_address = EthrexAddress::from_low_u64_be(SENDER_ADDRESS);
    let bytecode = Bytes::from(hex::decode(program).unwrap());

    let execution_db = setup_execution_db(sender_address, bytecode);

    let mut revm_cache_db = RevmCacheDB::new(execution_db);

    for i in 0..runs {
        let mut evm = Evm::builder()
            .modify_tx_env(|tx| {
                tx.caller = Address::from(sender_address.0);
                tx.transact_to = TransactTo::Call(Address::from(U160::from(CONTRACT_ADDRESS)));
                tx.data = hex::decode(calldata).unwrap().into();
            })
            .with_db(&mut revm_cache_db)
            .build();

        let result = black_box(evm.transact()).unwrap();
        assert!(result.result.is_success());

        if i == runs - 1 {
            println!(
                "REVM output: 0x{}",
                hex::encode(result.result.into_output().unwrap())
            );
        }
    }
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
    new_vm_with_ops_addr_bal_db(EthrexAddress::from_low_u64_be(SENDER_ADDRESS), nonce, db)
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

    let tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        to: TxKind::Call(EthrexAddress::from_low_u64_be(42)),
        ..Default::default()
    });
    VM::new(env, db, &tx)
}

fn setup_execution_db(sender_address: EthrexAddress, bytecode: Bytes) -> ExecutionDB {
    let mut execution_db = ExecutionDB::default();

    let code_hash = code_hash(&bytecode);
    let accounts = [
        // This is the contract account that is going to be executed
        (
            EthrexAddress::from_low_u64_be(CONTRACT_ADDRESS),
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
                    code_hash: ethrex_common::types::code_hash(&Bytes::new()),
                },
                storage: HashMap::new(),
                code: Bytes::new(),
            },
        ),
    ];

    accounts.iter().for_each(|(address, account)| {
        execution_db.accounts.insert(*address, account.info.clone());
        execution_db
            .code
            .insert(account.info.code_hash, account.code.clone());
        execution_db.storage.insert(
            *address,
            account.storage.iter().map(|(k, v)| (*k, *v)).collect(),
        );
        execution_db
            .block_hashes
            .insert(0, *ethrex_common::types::EMPTY_TRIE_HASH);
    });

    execution_db
}
