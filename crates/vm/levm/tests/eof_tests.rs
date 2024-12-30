use std::{collections::HashMap, sync::Arc};

use bytes::Bytes;
use ethrex_core::{types::TxKind, Address, U256};
use ethrex_levm::{
    account::Account,
    db::{cache, CacheDB, Db},
    errors::{OutOfGasError, VMError},
    utils::new_vm_with_bytecode,
    vm::VM,
    AccountInfo, Environment,
};

// Based on utils::new_vm_with_ops_addr_bal_db
pub fn new_vm_create(contract_bytecode: Bytes) -> Result<VM, VMError> {
    let sender_address = Address::from_low_u64_be(100);
    let mut db = Db::new();
    let mut cache = CacheDB::default();
    let accounts = [(
        // This is the sender account
        sender_address,
        Account {
            info: AccountInfo {
                nonce: 0,
                balance: U256::MAX,
                bytecode: Bytes::default(),
            },
            storage: HashMap::new(),
        },
    )];

    db.add_accounts(accounts.to_vec());

    // add to cache accounts from list accounts
    cache::insert_account(&mut cache, accounts[0].0, accounts[0].1.clone());

    let env = Environment::default_from_address(sender_address);

    VM::new(
        TxKind::Create,
        env,
        Default::default(),
        contract_bytecode,
        Arc::new(db),
        cache,
        Vec::new(),
    )
}

// These tests are taken from https://github.com/ethereum/evmone/blob/master/test/unittests/eof_example_test.cpp
// and adapted to our current implementation of EVM

#[test]
fn eof_examples_minimal() {
    // Example 1: A minimal valid EOF container doing nothing.
    /*
                                                           Code section: STOP
                        Header: 1 code section 1 byte long |
                        |                                  |
             version    |                    Header terminator
             |          |___________         |             |
       "EF00 01 01 0004 02 0001 0001 04 0000 00 00 80 0000 00"
                |‾‾‾‾‾‾              |‾‾‾‾‾‾    |‾‾‾‾‾‾‾‾‾
                |                    Header: data section 0 bytes long
                |                               |
                Header: types section 4 bytes long
                                                |
                                                Types section: first code section 0 inputs,
                                                non-returning, max stack height 0
    */
    let bytecode = hex::decode("EF00010100040200010001040000000080000000").unwrap();
    let vm = new_vm_with_bytecode(Bytes::from(bytecode));

    // Assert VM was created successfully
    assert!(vm.is_ok());
}

#[test]
fn eof_examples_static_relative_jump_loop() {
    // Example 2: EOF container looping infinitely using the static relative jump instruction RJUMP.
    /*
                                                           Code section: RJUMP back to start (-3)
                                                           - infinite loop
                                                           |
                        Header: 1 code section 3 bytes long
                        |                                  |
             version    |                    Header terminator
             |          |___________         |             |
       "EF00 01 01 0004 02 0001 0003 04 0000 00 00 80 0000 E0FFFD"
                |‾‾‾‾‾‾              |‾‾‾‾‾‾    |‾‾‾‾‾‾‾‾‾
                |                    Header: data section 0 bytes long
                |                               |
                Header: types section 4 bytes long
                                                |
                                                Types section: first code section 0 inputs,
                                                non-returning, max stack height 0
    */

    let bytecode = hex::decode("EF000101000402000100030400000000800000E0FFFD").unwrap();
    let mut vm = new_vm_with_bytecode(Bytes::from(bytecode)).unwrap();

    let result = vm.transact();

    // Tests the code is valid EOF and the infinite loop runs out of gas.
    assert!(result.is_err_and(|err| err.eq(&VMError::OutOfGas(OutOfGasError::GasCostOverflow))));
}

#[test]
fn eof_examples_callf() {
    // Example 3: EOF container with two code sections, one calling the other passing a single argument on the
    // stack and retrieving the same single value back from the stack on return.
    /*
                                                            First code section: PUSH1(0x2A),
                                                            CALLF second section and STOP
                        Header: 2 code sections:                           |
                        |       - first code section 6 bytes long          | Second code section:
                        |       - second code section 1 byte long          | return the input
                        |                                                  |              |
             version    |                         Header terminator        |              |
             |          |________________         |                        |_____________ |
       "EF00 01 01 0008 02 0002 0006 0001 04 0000 00 00 80 0001 01 01 0001 602A E30001 00 E4"
                |‾‾‾‾‾‾                   |‾‾‾‾‾‾    |‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾
                |                         Header: data section 0 bytes long
                |                                    |
                Header: types section 8 bytes long   |
                                                     |
                                                     Types section: first code section 0 inputs,
                                                     non-returning, max stack height 1;
                                                     second code section 1 input,
                                                     1 output, max stack height 1

    */
    let bytecode =
        hex::decode("EF000101000802000200060001040000000080000101010001602AE3000100E4").unwrap();
    let mut vm = new_vm_with_bytecode(Bytes::from(bytecode)).unwrap();

    let result = vm.transact();

    // Tests the code is valid EOF.
    assert!(result.is_ok());
    // Should add more asserts, like gas consuming or expected output
}

#[test]
fn eof_examples_creation_tx() {
    // Example 4: A creation transaction used to create a new EOF contract.
    /*
           Initcontainer
                                     Code section: PUSH0 [aux data size], PUSH0 [aux data offset] and
                                                   RETURNCONTRACT first subcontainer
                                                                            |
                            Header: 1 code section 4 bytes long             |
                            |                                               |
                 version    |                                 Header terminator
                 |          |___________                      |             |________
           "EF00 01 01 0004 02 0001 0004 03 0001 0014 04 0000 00 00 80 0002 5F5F EE00"
                    |‾‾‾‾‾‾              |‾‾‾‾‾‾‾‾‾‾‾ |‾‾‾‾‾‾    |‾‾‾‾‾‾‾‾‾
                    |                    |            Header: data section 0 bytes long
                    |                    |                       |
                    Header: types section 4 bytes long           Types section: first code section
                                         |                       0 inputs, non-returning,
                                         |                       max stack height 2
                    Header: 1 subcontainer 20 bytes long

           Deployed container (contract doing nothing, like Example 1)
           "EF00 01 01 0004 02 0001 0001 04 0000 00 00 80 0000 00"
    */
    let mut bytecode = hex::decode("EF00010100040200010004030001001404000000008000025F5FEE00EF00010100040200010001040000000080000000").unwrap();

    // Append "ABCDE" as calldata
    bytecode.extend(hex::decode("ABCDE").unwrap());

    let mut vm = new_vm_create(Bytes::from(bytecode)).unwrap();

    let result = vm.transact().unwrap();

    // Assert that addr of the newly created contract is calculated using the deployer's address and nonce.
    let expected_address = Address::from_low_u64_be(
        u64::from_str_radix("3442a1dec1e72f337007125aa67221498cdd759d", 16).unwrap(),
    );

    // Checking in account updates for the created account
    assert!(result.new_state.contains_key(&expected_address));
}

#[test]
fn eof_test_5() {
    // Example 5: A factory contract with an EOFCREATE instruction is being called in order
    // to deploy its subcontainer as a new EOF contract.
    /*
            Factory container
                                  Code section: PUSH0 [input size], PUSH0 [input offset], PUSH1 [salt],
                                                PUSH0 [endowment value],
                                                EOFCREATE from first subcontainer and STOP
                                                                             |
                             Header: 1 code section 8 bytes long             |
                             |                                               |
                  version    |                                 Header terminator
                  |          |___________                      |             |____________________
            "EF00 01 01 0004 02 0001 0008 03 0001 0030 04 0000 00 00 80 0004 5F 5F 60FF 5F EC00 00"
                     |‾‾‾‾‾‾              |‾‾‾‾‾‾‾‾‾‾‾ |‾‾‾‾‾‾    |‾‾‾‾‾‾‾‾‾
                     |                    |            Header: data section 0 bytes long
                     |                    |                       |
                     Header: types section 4 bytes long           Types section: first code section
                                          |                       0 inputs, non-returning,
                                          |                       max stack height 4
                     Header: 1 subcontainer 48 bytes long

            Initcontainer
                                  Code section: PUSH0 [aux data size], PUSH0 [aux data offset],
                                                RETURNCONTRACT first subcontainer
                                                                             |
                             Header: 1 code section 4 bytes long             |
                             |                                               |
                  version    |                                 Header terminator
                  |          |___________                      |             |_________
            "EF00 01 01 0004 02 0001 0004 03 0001 0014 04 0000 00 00 80 0002 5F 5F EE00"
                     |‾‾‾‾‾‾              |‾‾‾‾‾‾‾‾‾‾‾ |‾‾‾‾‾‾    |‾‾‾‾‾‾‾‾‾
                     |                    |            Header: data section 0 bytes long
                     |                    |                       |
                     Header: types section 4 bytes long           Types section: first code section
                                          |                       0 inputs, non-returning,
                                          |                       max stack height 2
                     Header: 1 subcontainer 20 bytes long

            Deployed container (contract doing nothing, like Example 1)
            "EF00 01 01 0004 02 0001 0001 04 0000 00 00 80 0000 00"
    */
    let bytecode = hex::decode("EF00010100040200010008030001003004000000008000045F5F60FF5FEC0000EF00010100040200010004030001001404000000008000025F5FEE00EF00010100040200010001040000000080000000").unwrap();

    let mut vm = new_vm_with_bytecode(Bytes::from(bytecode)).unwrap();

    let result = vm.transact().unwrap();

    // Assert that addr of the newly created contract is calculated using the deployer's address and nonce.
    let expected_address = Address::from_low_u64_be(
        u64::from_str_radix("5ea44d9b32a04ae2c15fe4fa8ebf9a8a5a1e7e89", 16).unwrap(),
    );

    // Checking in account updates for the created account
    assert!(result.new_state.contains_key(&expected_address));
}

#[test]
fn eof_test_6() {
    // Example 6: A basic EOF contract with a data section being used to load a byte of data onto the stack.
    /*
                            Code section: DATALOADN onto the stack the first word of data, STOP
                                                           |
                        Header: 1 code section 4 bytes long
                        |                                  |
             version    |                    Header terminator       Data section
             |          |___________         |             |________ |_________________________________________________________________
       "EF00 01 01 0004 02 0001 0004 04 0021 00 00 80 0001 D10000 00 454F462068617320736F6D65206772656174206578616D706C6573206865726521"
                |‾‾‾‾‾‾              |‾‾‾‾‾‾    |‾‾‾‾‾‾‾‾‾
                |                    Header: data section 33 bytes long
                |                               |
                Header: types section 4 bytes long
                                                |
                                                Types section: first code section 0 inputs,
                                                non-returning, max stack height 1
    */
    let bytecode = hex::decode("EF000101000402000100040400210000800001D1000000454F462068617320736F6D65206772656174206578616D706C6573206865726521").unwrap();

    let mut vm = new_vm_with_bytecode(Bytes::from(bytecode)).unwrap();

    let result = vm.transact();

    // Tests the code is valid EOF.
    assert!(result.is_ok());
    // Should add more asserts, like gas consuming or expected output
}
