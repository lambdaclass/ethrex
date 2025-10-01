use bytes::Bytes;
use ethrex_blockchain::vm::StoreVmDatabase;
use ethrex_common::H256;
use ethrex_common::{
    Address, U256,
    types::{Account, EIP1559Transaction, Transaction, TxKind},
};
use ethrex_levm::errors::VMError;
use ethrex_levm::{
    Environment,
    db::gen_db::GeneralizedDatabase,
    errors::TxResult,
    tracing::LevmCallTracer,
    vm::{VM, VMType},
};
use ethrex_storage::Store;
use ethrex_vm::DynVmDatabase;
use std::collections::BTreeMap;
use std::hint::black_box;
use std::sync::Arc;

// Use a constant byte array to define the Address at compile time.
const SENDER_ADDRESS: u64 = 0x100;
const CONTRACT_ADDRESS: u64 = 0x42;

pub fn run_with_levm(contract_code: &str, runs: u64, calldata: &str) {
    let bytecode = Bytes::from(hex::decode(contract_code).unwrap());
    let calldata = Bytes::from(hex::decode(calldata).unwrap());

    let mut db = init_db(bytecode);

    // when using stateful execute() we have to use nonce when instantiating the vm. Otherwise use 0.
    for _nonce in 0..runs - 1 {
        let mut vm = init_vm(&mut db, 0, calldata.clone()).unwrap();
        let tx_report = black_box(vm.stateless_execute().unwrap());
        assert!(tx_report.is_success());
    }
    let mut vm = init_vm(&mut db, 0, calldata.clone()).unwrap();
    let tx_report = black_box(vm.stateless_execute().unwrap());

    assert!(tx_report.is_success(), "{:?}", tx_report.result);

    match tx_report.result {
        TxResult::Success => {
            println!("output: \t\t0x{}", hex::encode(tx_report.output));
        }
        TxResult::Revert(error) => panic!("Execution failed: {error:?}"),
    }
}

// Auxiliary functions for initializing the Database and the VM with the appropriate values.

fn init_db(bytecode: Bytes) -> GeneralizedDatabase {
    // The store type for this bench shouldn't matter as all operations use the LEVM cache
    let in_memory_db = Store::new("", ethrex_storage::EngineType::InMemory).unwrap();
    let store: DynVmDatabase = Box::new(StoreVmDatabase::new(in_memory_db, H256::zero()));

    let cache = BTreeMap::from([
        (
            Address::from_low_u64_be(CONTRACT_ADDRESS),
            Account::new(U256::MAX, bytecode.clone(), 0, BTreeMap::new()),
        ),
        (
            Address::from_low_u64_be(SENDER_ADDRESS),
            Account::new(U256::MAX, Bytes::new(), 0, BTreeMap::new()),
        ),
    ]);

    GeneralizedDatabase::new_with_account_state(Arc::new(store), cache)
}

fn init_vm(db: &mut GeneralizedDatabase, nonce: u64, calldata: Bytes) -> Result<VM, VMError> {
    let env = Environment {
        origin: Address::from_low_u64_be(SENDER_ADDRESS),
        tx_nonce: nonce,
        gas_limit: (i64::MAX - 1) as u64,
        block_gas_limit: (i64::MAX - 1) as u64,
        ..Default::default()
    };

    let tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        to: TxKind::Call(Address::from_low_u64_be(CONTRACT_ADDRESS)),
        data: calldata,
        ..Default::default()
    });
    VM::new(env, db, &tx, LevmCallTracer::disabled(), VMType::L1)
}
