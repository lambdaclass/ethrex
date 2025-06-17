use bytes::Bytes;
use ethrex_blockchain::vm::StoreVmDatabase;
use ethrex_common::H256;
use ethrex_common::{
    Address, U256,
    types::{Account, AccountInfo, EIP1559Transaction, Transaction, TxKind, code_hash},
};
use ethrex_levm::{
    Environment,
    db::{CacheDB, cache, gen_db::GeneralizedDatabase},
    errors::TxResult,
    tracing::LevmCallTracer,
    vm::VM,
};
use ethrex_storage::Store;
use ethrex_vm::DynVmDatabase;
use std::hint::black_box;
use std::{collections::HashMap, sync::Arc};

pub fn run_with_levm(contract_code: &str, runs: u64, calldata: &str) {
    let bytecode = Bytes::from(hex::decode(contract_code).unwrap());
    let calldata = Bytes::from(hex::decode(calldata).unwrap());

    let code_hash = code_hash(&bytecode);
    let sender_address = Address::from_low_u64_be(100);
    let accounts = [
        // This is the contract account that is going to be executed
        (
            Address::from_low_u64_be(42),
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

    // The store type for this bench shouldn't matter as all operations use the LEVM cache
    let in_memory_db = Store::new("", ethrex_storage::EngineType::InMemory).unwrap();
    let store: DynVmDatabase = Box::new(StoreVmDatabase::new(in_memory_db, H256::zero()));
    let mut db = GeneralizedDatabase::new(Arc::new(store), CacheDB::new());

    cache::insert_account(
        &mut db.cache,
        accounts[0].0,
        Account::new(
            accounts[0].1.info.balance,
            accounts[0].1.code.clone(),
            accounts[0].1.info.nonce,
            HashMap::new(),
        ),
    );
    cache::insert_account(
        &mut db.cache,
        accounts[1].0,
        Account::new(
            accounts[1].1.info.balance,
            accounts[1].1.code.clone(),
            accounts[1].1.info.nonce,
            HashMap::new(),
        ),
    );
    db.immutable_cache = db.cache.clone();

    // when using stateful execute() we have to use nonce when instantiating the vm. Otherwise use 0.
    for _nonce in 0..runs - 1 {
        let mut vm = new_vm_with_bytecode(&mut db, 0, calldata.clone());
        vm.env.gas_limit = u64::MAX - 1;
        vm.env.block_gas_limit = u64::MAX;
        let tx_report = black_box(vm.stateless_execute().unwrap());
        assert!(tx_report.result == TxResult::Success);
    }
    let mut vm = new_vm_with_bytecode(&mut db, 0, calldata.clone());
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

pub fn new_vm_with_bytecode(db: &mut GeneralizedDatabase, nonce: u64, calldata: Bytes) -> VM {
    new_vm_with_ops_addr_bal_db(Address::from_low_u64_be(100), nonce, db, calldata)
}

/// This function is for testing purposes only.
fn new_vm_with_ops_addr_bal_db(
    sender_address: Address,
    nonce: u64,
    db: &mut GeneralizedDatabase,
    calldata: Bytes,
) -> VM {
    let env = Environment {
        origin: sender_address,
        tx_nonce: nonce,
        gas_limit: 100000000000,
        ..Default::default()
    };

    let tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        to: TxKind::Call(Address::from_low_u64_be(42)),
        data: calldata,
        ..Default::default()
    });
    VM::new(env, db, &tx, LevmCallTracer::disabled())
}
