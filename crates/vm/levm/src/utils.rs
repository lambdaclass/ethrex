use crate::{
    account::{Account, AccountInfo},
    db::{Cache, Db},
    environment::Environment,
    operations::Operation,
    vm::VM,
};
use bytes::Bytes;
use ethereum_rust_core::{types::TxKind, Address, U256};
use std::{collections::HashMap, sync::Arc};

pub fn ops_to_bytecode(operations: &[Operation]) -> Bytes {
    operations
        .iter()
        .flat_map(Operation::to_bytecode)
        .collect::<Bytes>()
}

pub fn new_vm_with_bytecode(bytecode: Bytes) -> VM {
    new_vm_with_ops_addr_bal_db(
        bytecode,
        Address::from_low_u64_be(100),
        U256::MAX,
        Db::new(),
        Cache::default(),
    )
}

pub fn new_vm_with_ops(operations: &[Operation]) -> VM {
    let bytecode = ops_to_bytecode(operations);
    new_vm_with_ops_addr_bal_db(
        bytecode,
        Address::from_low_u64_be(100),
        U256::MAX,
        Db::new(),
        Cache::default(),
    )
}

pub fn new_vm_with_ops_db(operations: &[Operation], db: Db) -> VM {
    let bytecode = ops_to_bytecode(operations);
    new_vm_with_ops_addr_bal_db(
        bytecode,
        Address::from_low_u64_be(100),
        U256::MAX,
        db,
        Cache::default(),
    )
}

/// This function is for testing purposes only.
pub fn new_vm_with_ops_addr_bal_db(
    contract_bytecode: Bytes,
    sender_address: Address,
    sender_balance: U256,
    mut db: Db,
    mut cache: Cache,
) -> VM {
    let accounts = [
        // This is the contract account that is going to be executed
        (
            Address::from_low_u64_be(42),
            Account {
                info: AccountInfo {
                    nonce: 0,
                    balance: U256::MAX,
                    bytecode: contract_bytecode,
                },
                storage: HashMap::new(),
            },
        ),
        (
            // This is the sender account
            sender_address,
            Account {
                info: AccountInfo {
                    nonce: 0,
                    balance: sender_balance,
                    bytecode: Bytes::default(),
                },
                storage: HashMap::new(),
            },
        ),
    ];

    db.add_accounts(accounts.to_vec());

    // add to cache accounts from list accounts
    cache.add_account(&accounts[0].0, &accounts[0].1);
    cache.add_account(&accounts[1].0, &accounts[1].1);

    let env = Environment::default_from_address(sender_address);

    VM::new(
        TxKind::Call(Address::from_low_u64_be(42)),
        env,
        Default::default(),
        Default::default(),
        Arc::new(db),
        cache,
    )
}
