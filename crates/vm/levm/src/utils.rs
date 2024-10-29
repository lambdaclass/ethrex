use crate::{
    operations::Operation,
    vm::{Account, Db, VM},
};
use bytes::Bytes;
use ethereum_types::{Address, U256};
use std::collections::HashMap;

pub fn ops_to_bytecde(operations: &[Operation]) -> Bytes {
    operations
        .iter()
        .flat_map(Operation::to_bytecode)
        .collect::<Bytes>()
}

pub fn new_vm_with_bytecode(bytecode: Bytes) -> VM {
    new_vm_with_ops_addr_bal(bytecode, Address::from_low_u64_be(100), U256::MAX)
}

pub fn new_vm_with_ops(operations: &[Operation]) -> VM {
    let bytecode = ops_to_bytecde(operations);
    new_vm_with_ops_addr_bal(bytecode, Address::from_low_u64_be(100), U256::MAX)
}

pub fn new_vm_with_ops_addr_bal(bytecode: Bytes, address: Address, balance: U256) -> VM {
    let accounts = [
        (
            Address::from_low_u64_be(42),
            Account {
                address: Address::from_low_u64_be(42),
                balance: U256::MAX,
                bytecode,
                storage: HashMap::new(),
                nonce: 0,
            },
        ),
        (
            address,
            Account {
                address,
                balance,
                bytecode: Bytes::default(),
                storage: HashMap::new(),
                nonce: 0,
            },
        ),
    ];

    let mut state = Db {
        accounts: accounts.into(),
        block_hashes: Default::default(),
    };

    // add the account with code to call

    // add the account passed by parameter

    VM::new(
        Some(Address::from_low_u64_be(42)),
        address,
        Default::default(),
        Default::default(),
        U256::MAX, // arbitrary gas limit for now...
        Default::default(),
        Default::default(),
        Default::default(),
        Default::default(),
        U256::one(),
        Default::default(),
        Default::default(),
        &mut state,
        Default::default(),
        Default::default(),
        Default::default(),
        Default::default(),
        None,
    )
    .unwrap()
}
