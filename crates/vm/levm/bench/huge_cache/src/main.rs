use std::{collections::HashMap, sync::Arc};

use bytes::Bytes;
use ethrex_common::{
    types::{EIP1559Transaction, Transaction, TxKind},
    Address, H256, U256,
};
use ethrex_levm::{
    db::{gen_db::GeneralizedDatabase, CacheDB},
    errors::VMError,
    vm::VM,
    Environment, StorageSlot,
};
use ethrex_vm::ExecutionDB;

fn main() {
    let huge_cache = CacheDB::new();
    let execution_db = ExecutionDB::default();
    let mut db = GeneralizedDatabase::new(Arc::new(execution_db), huge_cache);
    let number_of_accounts = 1000000;
    println!("Filling cache with {number_of_accounts} random accounts...");
    fill_cache_with_random_accounts(&mut db, number_of_accounts);
    println!("Cache filled with random accounts.");

    let cache_size = std::mem::size_of_val(&db.cache)
        + db.cache
            .iter()
            .map(|(k, v)| std::mem::size_of_val(k) + std::mem::size_of_val(v))
            .sum::<usize>();
    println!("Cache size in memory: {} bytes", cache_size);

    // ADDRESS 42 NEEDS TO HAVE THE CODE WHEN USING new_vm_with_bytecode
    // empty contract for now!
    let contract_address = Address::from_low_u64_be(42);
    let contract_code = Bytes::new();
    let contract_account =
        ethrex_levm::Account::new(U256::MAX, contract_code.clone(), 0, Default::default());
    db.cache.insert(contract_address, contract_account);

    // measure time that it takes to clone cache
    // for i in 0..100 {
    //     let start = std::time::Instant::now();
    //     let cloned_cache = db.cache.clone();
    //     let elapsed = start.elapsed();
    //     println!("Time to clone cache: {:?}", elapsed);
    // }
    // If cache is huge then it will take a lot of time to clone it

    let runs = 10;
    // stateless execution (is like normal execution but cloning the cache)
    for i in 0..runs {
        let start = std::time::Instant::now();
        let mut vm = new_vm_with_bytecode(&mut db, 0).unwrap();
        vm.call_frames.last_mut().unwrap().calldata = contract_code.clone();
        vm.env.gas_limit = u64::MAX - 1;
        vm.env.block_gas_limit = u64::MAX;
        let result = vm.stateless_execute().unwrap();
        let elapsed = start.elapsed();
        println!("Time to execute basic transaction: {:?}", elapsed);
    }

    // normal execution
    for i in 0..runs {
        let start = std::time::Instant::now();
        let mut vm = new_vm_with_bytecode(&mut db, i).unwrap();
        vm.call_frames.last_mut().unwrap().calldata = contract_code.clone();
        vm.env.gas_limit = u64::MAX - 1;
        vm.env.block_gas_limit = u64::MAX;
        let result = vm.execute().unwrap();
        let elapsed = start.elapsed();
        println!("Time to execute basic transaction: {:?}", elapsed);
    }
}

pub fn new_vm_with_bytecode(db: &mut GeneralizedDatabase, nonce: u64) -> Result<VM, VMError> {
    new_vm_with_ops_addr_bal_db(Address::from_low_u64_be(100), nonce, db)
}

/// This function is for testing purposes only.
fn new_vm_with_ops_addr_bal_db(
    sender_address: Address,
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
        to: TxKind::Call(Address::from_low_u64_be(42)),
        ..Default::default()
    });

    VM::new(env, db, &tx)
}

fn fill_cache_with_random_accounts(
    db: &mut GeneralizedDatabase,
    number_of_accounts: usize,
) -> Vec<(Address, ethrex_levm::Account)> {
    let mut accounts = Vec::new();
    for i in 500..500 + number_of_accounts {
        let address = Address::from_low_u64_be(i as u64);

        // add 10 storage slots to each account
        let mut storage: HashMap<H256, StorageSlot> = HashMap::new();
        for j in 0..10 {
            let key = H256::from_low_u64_be(j as u64);
            let value = StorageSlot {
                original_value: U256::from(10 + j),
                current_value: U256::from(10 + j),
            };
            storage.insert(key, value);
        }

        let account = ethrex_levm::Account::new(U256::MAX, Bytes::new(), 0, Default::default());
        accounts.push((address, account));
    }

    for (address, account) in &accounts {
        db.cache.insert(*address, account.clone());
    }

    accounts
}
