#![allow(clippy::indexing_slicing)]
#![allow(clippy::unwrap_used)]

use bytes::Bytes;
use ethrex_core::{types::TxKind, Address, H256, U256};
use ethrex_levm::{
    account::Account,
    constants::*,
    db::{cache, CacheDB, Db},
    errors::{OutOfGasError, TxResult, VMError},
    gas_cost, memory,
    operations::Operation,
    utils::{new_vm_with_ops, new_vm_with_ops_addr_bal_db, new_vm_with_ops_db, ops_to_bytecode},
    vm::{word_to_address, Storage, VM},
    Environment,
};
use std::{collections::HashMap, sync::Arc};

fn create_opcodes(size: usize, offset: usize, value_to_transfer: usize) -> Vec<Operation> {
    vec![
        Operation::Push((16, U256::from(size))),
        Operation::Push((1, U256::from(offset))),
        Operation::Push((1, U256::from(value_to_transfer))),
        Operation::Create,
        Operation::Stop,
    ]
}

fn callee_return_bytecode(return_value: U256) -> Bytes {
    let ops = vec![
        Operation::Push((32, return_value)), // value
        Operation::Push((32, U256::zero())), // offset
        Operation::Mstore,
        Operation::Push((32, U256::from(32))), // size
        Operation::Push((32, U256::zero())),   // offset
        Operation::Return,
    ];

    ops_to_bytecode(&ops).unwrap()
}

pub fn store_data_in_memory_operations(data: &[u8], memory_offset: usize) -> Vec<Operation> {
    vec![
        Operation::Push((32_u8, U256::from_big_endian(data))),
        Operation::Push((1_u8, U256::from(memory_offset))),
        Operation::Mstore,
    ]
}

#[test]
fn add_op() {
    let mut vm = new_vm_with_ops(&[
        Operation::Push((32, U256::one())),
        Operation::Push((32, U256::zero())),
        Operation::Add,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    assert!(vm.current_call_frame_mut().unwrap().stack.pop().unwrap() == U256::one());
    assert!(vm.current_call_frame_mut().unwrap().pc() == 68);
}

#[test]
fn mul_op() {
    let mut vm = new_vm_with_ops(&[
        Operation::Push((1, U256::from(2))),
        Operation::Push((1, U256::from(4))),
        Operation::Mul,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    assert!(vm.current_call_frame_mut().unwrap().stack.pop().unwrap() == U256::from(8));
}

#[test]
fn sub_op() {
    let mut vm = new_vm_with_ops(&[
        Operation::Push((1, U256::from(3))),
        Operation::Push((1, U256::from(5))),
        Operation::Sub,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    assert!(vm.current_call_frame_mut().unwrap().stack.pop().unwrap() == U256::from(2));
}

#[test]
fn div_op() {
    // 11 // 2 = 5
    let mut vm = new_vm_with_ops(&[
        Operation::Push((1, U256::from(2))),
        Operation::Push((1, U256::from(11))),
        Operation::Div,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    assert!(vm.current_call_frame_mut().unwrap().stack.pop().unwrap() == U256::from(5));

    // In EVM: 10 / 0 = 0
    let mut vm = new_vm_with_ops(&[
        Operation::Push((1, U256::zero())),
        Operation::Push((1, U256::from(10))),
        Operation::Div,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    assert!(vm.current_call_frame_mut().unwrap().stack.pop().unwrap() == U256::zero());
}

#[test]
fn sdiv_op() {
    // Values are treated as two's complement signed 256-bit integers
    let mut vm = new_vm_with_ops(&[
        Operation::Push((32, U256::MAX)),
        Operation::Push((32, U256::MAX - 1)),
        Operation::Sdiv,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    assert!(vm.current_call_frame_mut().unwrap().stack.pop().unwrap() == U256::from(2));
}

#[test]
fn mod_op() {
    // 10 % 3 = 1
    let mut vm = new_vm_with_ops(&[
        Operation::Push((1, U256::from(3))),
        Operation::Push((1, U256::from(10))),
        Operation::Mod,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    assert!(vm.current_call_frame_mut().unwrap().stack.pop().unwrap() == U256::from(1));
}

#[test]
fn smod_op() {
    // First Example
    // 10 % 3 = 1
    let mut vm = new_vm_with_ops(&[
        Operation::Push((1, U256::from(3))),
        Operation::Push((1, U256::from(10))),
        Operation::SMod,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    assert_eq!(
        vm.current_call_frame_mut().unwrap().stack.pop().unwrap(),
        U256::one()
    );

    // Second Example
    // Example taken from evm.codes
    // In 2's complement it is: -8 % -3 = -2
    let a = U256::from_str_radix(
        "0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFD",
        16,
    )
    .unwrap();
    let b = U256::from_str_radix(
        "0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF8",
        16,
    )
    .unwrap();
    // Values are treated as two's complement signed 256-bit integers
    let mut vm = new_vm_with_ops(&[
        Operation::Push((32, a)),
        Operation::Push((32, b)),
        Operation::SMod,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let c = U256::from_str_radix(
        "0xfffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffe",
        16,
    )
    .unwrap();

    assert_eq!(vm.current_call_frame_mut().unwrap().stack.pop().unwrap(), c);
}

#[test]
fn addmod_op() {
    // (10 + 10) % 8 = 4
    let mut vm = new_vm_with_ops(&[
        Operation::Push((1, U256::from(8))),
        Operation::Push((1, U256::from(10))),
        Operation::Push((1, U256::from(10))),
        Operation::Addmod,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    assert!(vm.current_call_frame_mut().unwrap().stack.pop().unwrap() == U256::from(4));
}

#[test]
fn mulmod_op() {
    // (10 * 10) % 8 = 4
    let mut vm = new_vm_with_ops(&[
        Operation::Push((1, U256::from(8))),
        Operation::Push((1, U256::from(10))),
        Operation::Push((1, U256::from(10))),
        Operation::Mulmod,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    assert!(vm.current_call_frame_mut().unwrap().stack.pop().unwrap() == U256::from(4));
}

#[test]
fn exp_op() {
    // 10^2 = 100
    let mut vm = new_vm_with_ops(&[
        Operation::Push((1, U256::from(2))),
        Operation::Push((1, U256::from(10))),
        Operation::Exp,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    assert!(vm.current_call_frame_mut().unwrap().stack.pop().unwrap() == U256::from(100));
}

#[test]
fn sign_extend_op() {
    // Case 1: Input: 0, 0x7F. Output: 0x7F
    let mut vm = new_vm_with_ops(&[
        Operation::Push((1, U256::from(0x7F))),
        Operation::Push((1, U256::zero())),
        Operation::SignExtend,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();
    assert!(vm.current_call_frame_mut().unwrap().stack.pop().unwrap() == U256::from(0x7F));

    // Case 2: Input: 0, 0xFF. Output: 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF
    let mut vm = new_vm_with_ops(&[
        Operation::Push((1, U256::from(0xFF))),
        Operation::Push((1, U256::zero())),
        Operation::SignExtend,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();
    assert!(vm.current_call_frame_mut().unwrap().stack.pop().unwrap() == U256::MAX);
}

#[test]
fn lt_op() {
    // Input: 9, 10. Output: 1
    let mut vm = new_vm_with_ops(&[
        Operation::Push((1, U256::from(10))),
        Operation::Push((1, U256::from(9))),
        Operation::Lt,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    assert!(vm.current_call_frame_mut().unwrap().stack.pop().unwrap() == U256::one());
}

#[test]
fn gt_op() {
    // Input: 10, 9. Output: 1
    let mut vm = new_vm_with_ops(&[
        Operation::Push((1, U256::from(9))),
        Operation::Push((1, U256::from(10))),
        Operation::Gt,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    assert!(vm.current_call_frame_mut().unwrap().stack.pop().unwrap() == U256::one());
}

#[test]
fn slt_op() {
    // Input: 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF, 0. Output: 1
    let mut vm = new_vm_with_ops(&[
        Operation::Push((32, U256::zero())),
        Operation::Push((32, U256::MAX)),
        Operation::Slt,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    assert!(vm.current_call_frame_mut().unwrap().stack.pop().unwrap() == U256::one());
}

#[test]
fn sgt_op() {
    // Input: 0, 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF. Output: 1
    let mut vm = new_vm_with_ops(&[
        Operation::Push((32, U256::MAX)),
        Operation::Push((32, U256::zero())),
        Operation::Sgt,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    assert!(vm.current_call_frame_mut().unwrap().stack.pop().unwrap() == U256::one());
}

#[test]
fn eq_op() {
    // Case 1: Input: 10, 10. Output: 1 (true)
    let mut vm = new_vm_with_ops(&[
        Operation::Push((1, U256::from(10))),
        Operation::Push((1, U256::from(10))),
        Operation::Eq,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();
    assert!(vm.current_call_frame_mut().unwrap().stack.pop().unwrap() == U256::one());

    // Case 2: Input: 10, 20. Output: 0 (false)
    let mut vm = new_vm_with_ops(&[
        Operation::Push((1, U256::from(10))),
        Operation::Push((1, U256::from(20))),
        Operation::Eq,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();
    assert!(vm.current_call_frame_mut().unwrap().stack.pop().unwrap() == U256::zero());
}

#[test]
fn is_zero_op() {
    // Case 1: Input is 0, Output should be 1 (since 0 == 0 is true)
    let mut vm = new_vm_with_ops(&[
        Operation::Push((1, U256::zero())),
        Operation::IsZero,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();
    assert!(vm.current_call_frame_mut().unwrap().stack.pop().unwrap() == U256::one());

    // Case 2: Input is non-zero (e.g., 10), Output should be 0 (since 10 != 0 is false)
    let mut vm = new_vm_with_ops(&[
        Operation::Push((1, U256::from(10))),
        Operation::IsZero,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();
    assert!(vm.current_call_frame_mut().unwrap().stack.pop().unwrap() == U256::zero());
}

#[test]
fn and_basic() {
    let mut vm = new_vm_with_ops(&[
        Operation::Push((32, U256::from(0b1010))),
        Operation::Push((32, U256::from(0b1100))),
        Operation::And,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let result = vm.current_call_frame_mut().unwrap().stack.pop().unwrap();
    assert_eq!(result, U256::from(0b1000));
    assert_eq!(current_call_frame.gas_used, U256::from(9));
}

#[test]
fn and_binary_with_zero() {
    let mut vm = new_vm_with_ops(&[
        Operation::Push((32, U256::from(0b1010))),
        Operation::Push((32, U256::zero())),
        Operation::And,
        Operation::Stop,
    ])
    .unwrap();
    let expected_consumed_gas = gas_cost::AND + gas_cost::PUSHN.checked_mul(U256::from(2)).unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let result = vm.current_call_frame_mut().unwrap().stack.pop().unwrap();
    assert_eq!(result, U256::zero());
    assert_eq!(current_call_frame.gas_used, expected_consumed_gas);
}

#[test]
fn and_with_hex_numbers() {
    let mut vm = new_vm_with_ops(&[
        Operation::Push((32, U256::from(0xFFFF))),
        Operation::Push((32, U256::from(0xF0F0))),
        Operation::And,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let result = vm.current_call_frame_mut().unwrap().stack.pop().unwrap();
    assert_eq!(result, U256::from(0xF0F0));
    assert_eq!(current_call_frame.gas_used, U256::from(9));

    let mut vm = new_vm_with_ops(&[
        Operation::Push((32, U256::from(0xF000))),
        Operation::Push((32, U256::from(0xF0F0))),
        Operation::And,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let result = vm.current_call_frame_mut().unwrap().stack.pop().unwrap();
    assert_eq!(result, U256::from(0xF000));
    assert_eq!(current_call_frame.gas_used, U256::from(9));

    let mut vm = new_vm_with_ops(&[
        Operation::Push((32, U256::from(0xB020))),
        Operation::Push((32, U256::from(0x1F0F))),
        Operation::And,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let result = vm.current_call_frame_mut().unwrap().stack.pop().unwrap();
    assert_eq!(result, U256::from(0b1000000000000));
    assert_eq!(current_call_frame.gas_used, U256::from(9));
}

#[test]
fn or_basic() {
    let mut vm = new_vm_with_ops(&[
        Operation::Push((32, U256::from(0b1010))),
        Operation::Push((32, U256::from(0b1100))),
        Operation::Or,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let result = vm.current_call_frame_mut().unwrap().stack.pop().unwrap();
    assert_eq!(result, U256::from(0b1110));
    assert_eq!(current_call_frame.gas_used, U256::from(9));

    let mut vm = new_vm_with_ops(&[
        Operation::Push((32, U256::from(0b1010))),
        Operation::Push((32, U256::zero())),
        Operation::Or,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let result = vm.current_call_frame_mut().unwrap().stack.pop().unwrap();
    assert_eq!(result, U256::from(0b1010));
    assert_eq!(current_call_frame.gas_used, U256::from(9));

    let mut vm = new_vm_with_ops(&[
        Operation::Push((32, U256::from(u64::MAX))),
        Operation::Push((32, U256::zero())),
        Operation::Or,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let result = vm.current_call_frame_mut().unwrap().stack.pop().unwrap();
    assert_eq!(result, U256::from(0xFFFFFFFFFFFFFFFF_u64));
    assert_eq!(current_call_frame.gas_used, U256::from(9));
}

#[test]
fn or_with_hex_numbers() {
    let mut vm = new_vm_with_ops(&[
        Operation::Push((32, U256::from(0xFFFF))),
        Operation::Push((32, U256::from(0xF0F0))),
        Operation::Or,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let result = vm.current_call_frame_mut().unwrap().stack.pop().unwrap();
    assert_eq!(result, U256::from(0xFFFF));
    assert_eq!(current_call_frame.gas_used, U256::from(9));

    let mut vm = new_vm_with_ops(&[
        Operation::Push((32, U256::from(0xF000))),
        Operation::Push((32, U256::from(0xF0F0))),
        Operation::Or,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let result = vm.current_call_frame_mut().unwrap().stack.pop().unwrap();
    assert_eq!(result, U256::from(0xF0F0));
    assert_eq!(current_call_frame.gas_used, U256::from(9));

    let mut vm = new_vm_with_ops(&[
        Operation::Push((32, U256::from(0xB020))),
        Operation::Push((32, U256::from(0x1F0F))),
        Operation::Or,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let result = vm.current_call_frame_mut().unwrap().stack.pop().unwrap();
    assert_eq!(result, U256::from(0b1011111100101111));
    assert_eq!(current_call_frame.gas_used, U256::from(9));
}

#[test]
fn xor_basic() {
    let mut vm = new_vm_with_ops(&[
        Operation::Push((32, U256::from(0b1010))),
        Operation::Push((32, U256::from(0b1100))),
        Operation::Xor,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let result = vm.current_call_frame_mut().unwrap().stack.pop().unwrap();
    assert_eq!(result, U256::from(0b110));
    assert_eq!(current_call_frame.gas_used, U256::from(9));

    let mut vm = new_vm_with_ops(&[
        Operation::Push((32, U256::from(0b1010))),
        Operation::Push((32, U256::zero())),
        Operation::Xor,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let result = vm.current_call_frame_mut().unwrap().stack.pop().unwrap();
    assert_eq!(result, U256::from(0b1010));
    assert_eq!(current_call_frame.gas_used, U256::from(9));

    let mut vm = new_vm_with_ops(&[
        Operation::Push((32, U256::from(u64::MAX))),
        Operation::Push((32, U256::zero())),
        Operation::Xor,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let result = vm.current_call_frame_mut().unwrap().stack.pop().unwrap();
    assert_eq!(result, U256::from(u64::MAX));
    assert_eq!(current_call_frame.gas_used, U256::from(9));

    let mut vm = new_vm_with_ops(&[
        Operation::Push((32, U256::from(u64::MAX))),
        Operation::Push((32, U256::from(u64::MAX))),
        Operation::Xor,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let result = vm.current_call_frame_mut().unwrap().stack.pop().unwrap();
    assert_eq!(result, U256::zero());
    assert_eq!(current_call_frame.gas_used, U256::from(9));
}

#[test]
fn xor_with_hex_numbers() {
    let mut vm = new_vm_with_ops(&[
        Operation::Push((32, U256::from(0xF0))),
        Operation::Push((32, U256::from(0xF))),
        Operation::Xor,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let result = vm.current_call_frame_mut().unwrap().stack.pop().unwrap();
    assert_eq!(result, U256::from(0xFF));
    assert_eq!(current_call_frame.gas_used, U256::from(9));

    let mut vm = new_vm_with_ops(&[
        Operation::Push((32, U256::from(0xFF))),
        Operation::Push((32, U256::from(0xFF))),
        Operation::Xor,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let result = vm.current_call_frame_mut().unwrap().stack.pop().unwrap();
    assert_eq!(result, U256::zero());
    assert_eq!(current_call_frame.gas_used, U256::from(9));

    let mut vm = new_vm_with_ops(&[
        Operation::Push((32, U256::from(0xFFFF))),
        Operation::Push((32, U256::from(0xF0F0))),
        Operation::Xor,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let result = vm.current_call_frame_mut().unwrap().stack.pop().unwrap();
    assert_eq!(result, U256::from(0xF0F));
    assert_eq!(current_call_frame.gas_used, U256::from(9));

    let mut vm = new_vm_with_ops(&[
        Operation::Push((32, U256::from(0xF000))),
        Operation::Push((32, U256::from(0xF0F0))),
        Operation::Xor,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let result = vm.current_call_frame_mut().unwrap().stack.pop().unwrap();
    assert_eq!(result, U256::from(0xF0));
    assert_eq!(current_call_frame.gas_used, U256::from(9));

    let mut vm = new_vm_with_ops(&[
        Operation::Push((32, U256::from(0x4C0F))),
        Operation::Push((32, U256::from(0x3A4B))),
        Operation::Xor,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let result = vm.current_call_frame_mut().unwrap().stack.pop().unwrap();
    assert_eq!(result, U256::from(0b111011001000100));
    assert_eq!(current_call_frame.gas_used, U256::from(9));
}

#[test]
fn not() {
    let mut vm = new_vm_with_ops(&[
        Operation::Push((32, U256::from(0b1010))),
        Operation::Not,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let result = vm.current_call_frame_mut().unwrap().stack.pop().unwrap();
    let expected = !U256::from(0b1010);
    assert_eq!(result, expected);
    assert_eq!(current_call_frame.gas_used, U256::from(6));

    let mut vm = new_vm_with_ops(&[
        Operation::Push((32, U256::MAX)),
        Operation::Not,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let result = vm.current_call_frame_mut().unwrap().stack.pop().unwrap();
    assert_eq!(result, U256::zero());
    assert_eq!(current_call_frame.gas_used, U256::from(6));

    let mut vm = new_vm_with_ops(&[
        Operation::Push((32, U256::zero())),
        Operation::Not,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let result = vm.current_call_frame_mut().unwrap().stack.pop().unwrap();
    assert_eq!(result, U256::MAX);
    assert_eq!(current_call_frame.gas_used, U256::from(6));

    let mut vm = new_vm_with_ops(&[
        Operation::Push((32, U256::from(1))),
        Operation::Not,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let result = vm.current_call_frame_mut().unwrap().stack.pop().unwrap();
    assert_eq!(result, U256::MAX - 1);
    assert_eq!(current_call_frame.gas_used, U256::from(6));
}

#[test]
fn byte_basic() {
    let mut vm = new_vm_with_ops(&[
        Operation::Push((32, U256::from(0xF0F1))),
        Operation::Push((32, U256::from(31))),
        Operation::Byte,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let result = vm.current_call_frame_mut().unwrap().stack.pop().unwrap();
    assert_eq!(result, U256::from(0xF1));
    assert_eq!(current_call_frame.gas_used, U256::from(9));

    let mut vm = new_vm_with_ops(&[
        Operation::Push((32, U256::from(0x33ED))),
        Operation::Push((32, U256::from(30))),
        Operation::Byte,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let result = vm.current_call_frame_mut().unwrap().stack.pop().unwrap();
    assert_eq!(result, U256::from(0x33));
    assert_eq!(current_call_frame.gas_used, U256::from(9));
}

#[test]
fn byte_edge_cases() {
    let mut vm = new_vm_with_ops(&[
        Operation::Push((32, U256::MAX)),
        Operation::Push((32, U256::from(0))),
        Operation::Byte,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let result = vm.current_call_frame_mut().unwrap().stack.pop().unwrap();
    assert_eq!(result, U256::from(0xFF));
    assert_eq!(current_call_frame.gas_used, U256::from(9));

    let mut vm = new_vm_with_ops(&[
        Operation::Push((32, U256::MAX)),
        Operation::Push((32, U256::from(12))),
        Operation::Byte,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let result = vm.current_call_frame_mut().unwrap().stack.pop().unwrap();
    assert_eq!(result, U256::from(0xFF));
    assert_eq!(current_call_frame.gas_used, U256::from(9));

    let mut vm = new_vm_with_ops(&[
        Operation::Push((32, U256::from(0x00E0D0000))),
        Operation::Push((32, U256::from(29))),
        Operation::Byte,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let result = vm.current_call_frame_mut().unwrap().stack.pop().unwrap();
    assert_eq!(result, U256::from(0x0D));
    assert_eq!(current_call_frame.gas_used, U256::from(9));

    let mut vm = new_vm_with_ops(&[
        Operation::Push((32, U256::from(0xFDEA179))),
        Operation::Push((32, U256::from(50))),
        Operation::Byte,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let result = vm.current_call_frame_mut().unwrap().stack.pop().unwrap();
    assert_eq!(result, U256::zero());
    assert_eq!(current_call_frame.gas_used, U256::from(9));

    let mut vm = new_vm_with_ops(&[
        Operation::Push((32, U256::from(0xFDEA179))),
        Operation::Push((32, U256::from(32))),
        Operation::Byte,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let result = vm.current_call_frame_mut().unwrap().stack.pop().unwrap();
    assert_eq!(result, U256::zero());
    assert_eq!(current_call_frame.gas_used, U256::from(9));

    let mut vm = new_vm_with_ops(&[
        Operation::Push((32, U256::zero())),
        Operation::Push((32, U256::from(15))),
        Operation::Byte,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let result = vm.current_call_frame_mut().unwrap().stack.pop().unwrap();
    assert_eq!(result, U256::zero());
    assert_eq!(current_call_frame.gas_used, U256::from(9));

    let word = U256::from_big_endian(&[
        0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x57, 0x08, 0x09, 0x90, 0x0B, 0x0C, 0x0D, 0x0E,
        0x0F, 0x10, 0x11, 0x12, 0xDD, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D,
        0x1E, 0x40,
    ]);

    let mut vm = new_vm_with_ops(&[
        Operation::Push((32, word)),
        Operation::Push((32, U256::from(10))),
        Operation::Byte,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let result = vm.current_call_frame_mut().unwrap().stack.pop().unwrap();
    assert_eq!(result, U256::from(0x90));
    assert_eq!(current_call_frame.gas_used, U256::from(9));

    let mut vm = new_vm_with_ops(&[
        Operation::Push((32, word)),
        Operation::Push((32, U256::from(7))),
        Operation::Byte,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let result = vm.current_call_frame_mut().unwrap().stack.pop().unwrap();
    assert_eq!(result, U256::from(0x57));
    assert_eq!(current_call_frame.gas_used, U256::from(9));

    let mut vm = new_vm_with_ops(&[
        Operation::Push((32, word)),
        Operation::Push((32, U256::from(19))),
        Operation::Byte,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let result = vm.current_call_frame_mut().unwrap().stack.pop().unwrap();
    assert_eq!(result, U256::from(0xDD));
    assert_eq!(current_call_frame.gas_used, U256::from(9));

    let mut vm = new_vm_with_ops(&[
        Operation::Push((32, word)),
        Operation::Push((32, U256::from(31))),
        Operation::Byte,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let result = vm.current_call_frame_mut().unwrap().stack.pop().unwrap();
    assert_eq!(result, U256::from(0x40));
    assert_eq!(current_call_frame.gas_used, U256::from(9));
}

#[test]
fn shl_basic() {
    let mut vm = new_vm_with_ops(&[
        Operation::Push((32, U256::from(0xDDDD))),
        Operation::Push((32, U256::from(0))),
        Operation::Shl,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let result = vm.current_call_frame_mut().unwrap().stack.pop().unwrap();
    assert_eq!(result, U256::from(0xDDDD));
    assert_eq!(current_call_frame.gas_used, U256::from(9));

    let mut vm = new_vm_with_ops(&[
        Operation::Push((32, U256::from(0x12345678))),
        Operation::Push((32, U256::from(1))),
        Operation::Shl,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let result = vm.current_call_frame_mut().unwrap().stack.pop().unwrap();
    assert_eq!(result, U256::from(0x2468acf0));
    assert_eq!(current_call_frame.gas_used, U256::from(9));

    let mut vm = new_vm_with_ops(&[
        Operation::Push((32, U256::from(0x12345678))),
        Operation::Push((32, U256::from(4))),
        Operation::Shl,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let result = vm.current_call_frame_mut().unwrap().stack.pop().unwrap();
    assert_eq!(result, U256::from(4886718336_u64));
    assert_eq!(current_call_frame.gas_used, U256::from(9));

    let mut vm = new_vm_with_ops(&[
        Operation::Push((32, U256::from(0xFF))),
        Operation::Push((32, U256::from(4))),
        Operation::Shl,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let result = vm.current_call_frame_mut().unwrap().stack.pop().unwrap();
    assert_eq!(result, U256::from(0xFF << 4));
    assert_eq!(current_call_frame.gas_used, U256::from(9));
}

#[test]
fn shl_edge_cases() {
    let mut vm = new_vm_with_ops(&[
        Operation::Push((32, U256::from(0x1))),
        Operation::Push((32, U256::from(256))),
        Operation::Shl,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let result = vm.current_call_frame_mut().unwrap().stack.pop().unwrap();
    assert_eq!(result, U256::zero());
    assert_eq!(current_call_frame.gas_used, U256::from(9));

    let mut vm = new_vm_with_ops(&[
        Operation::Push((32, U256::zero())),
        Operation::Push((32, U256::from(200))),
        Operation::Shl,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let result = vm.current_call_frame_mut().unwrap().stack.pop().unwrap();
    assert_eq!(result, U256::zero());
    assert_eq!(current_call_frame.gas_used, U256::from(9));

    let mut vm = new_vm_with_ops(&[
        Operation::Push((32, U256::MAX)),
        Operation::Push((32, U256::from(1))),
        Operation::Shl,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let result = vm.current_call_frame_mut().unwrap().stack.pop().unwrap();
    assert_eq!(result, U256::MAX - 1);
    assert_eq!(current_call_frame.gas_used, U256::from(9));
}

#[test]
fn shr_basic() {
    let mut vm = new_vm_with_ops(&[
        Operation::Push((32, U256::from(0xDDDD))),
        Operation::Push((32, U256::from(0))),
        Operation::Shr,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let result = vm.current_call_frame_mut().unwrap().stack.pop().unwrap();
    assert_eq!(result, U256::from(0xDDDD));
    assert_eq!(current_call_frame.gas_used, U256::from(9));

    let mut vm = new_vm_with_ops(&[
        Operation::Push((32, U256::from(0x12345678))),
        Operation::Push((32, U256::from(1))),
        Operation::Shr,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let result = vm.current_call_frame_mut().unwrap().stack.pop().unwrap();
    assert_eq!(result, U256::from(0x91a2b3c));
    assert_eq!(current_call_frame.gas_used, U256::from(9));

    let mut vm = new_vm_with_ops(&[
        Operation::Push((32, U256::from(0x12345678))),
        Operation::Push((32, U256::from(4))),
        Operation::Shr,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let result = vm.current_call_frame_mut().unwrap().stack.pop().unwrap();
    assert_eq!(result, U256::from(0x1234567));
    assert_eq!(current_call_frame.gas_used, U256::from(9));

    let mut vm = new_vm_with_ops(&[
        Operation::Push((32, U256::from(0xFF))),
        Operation::Push((32, U256::from(4))),
        Operation::Shr,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let result = vm.current_call_frame_mut().unwrap().stack.pop().unwrap();
    assert_eq!(result, U256::from(0xF));
    assert_eq!(current_call_frame.gas_used, U256::from(9));
}

#[test]
fn shr_edge_cases() {
    let mut vm = new_vm_with_ops(&[
        Operation::Push((32, U256::from(0x1))),
        Operation::Push((32, U256::from(256))),
        Operation::Shr,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let result = vm.current_call_frame_mut().unwrap().stack.pop().unwrap();
    assert_eq!(result, U256::zero());
    assert_eq!(current_call_frame.gas_used, U256::from(9));

    let mut vm = new_vm_with_ops(&[
        Operation::Push((32, U256::zero())),
        Operation::Push((32, U256::from(200))),
        Operation::Shr,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let result = vm.current_call_frame_mut().unwrap().stack.pop().unwrap();
    assert_eq!(result, U256::zero());
    assert_eq!(current_call_frame.gas_used, U256::from(9));

    let mut vm = new_vm_with_ops(&[
        Operation::Push((32, U256::MAX)),
        Operation::Push((32, U256::from(1))),
        Operation::Shr,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let result = vm.current_call_frame_mut().unwrap().stack.pop().unwrap();
    assert_eq!(result, U256::MAX >> 1);
    assert_eq!(current_call_frame.gas_used, U256::from(9));
}

#[test]
fn sar_shift_by_0() {
    let mut vm = new_vm_with_ops(&[
        Operation::Push((32, U256::from(0x12345678))),
        Operation::Push((32, U256::from(0))),
        Operation::Sar,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let result = vm.current_call_frame_mut().unwrap().stack.pop().unwrap();
    assert_eq!(result, U256::from(0x12345678));
    assert_eq!(current_call_frame.gas_used, U256::from(9));
}

#[test]
fn sar_shifting_large_value_with_all_bits_set() {
    let word = U256::from_big_endian(&[
        0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        0xff, 0xff,
    ]);

    let mut vm = new_vm_with_ops(&[
        Operation::Push((32, word)),
        Operation::Push((32, U256::from(8))),
        Operation::Sar,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let result = vm.current_call_frame_mut().unwrap().stack.pop().unwrap();
    let expected = U256::from_big_endian(&[
        0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        0xff, 0xff,
    ]);
    assert_eq!(result, expected);
    assert_eq!(current_call_frame.gas_used, U256::from(9));
}

#[test]
fn sar_shifting_negative_value_and_small_shift() {
    let word_neg = U256::from_big_endian(&[
        0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00,
    ]);

    let mut vm = new_vm_with_ops(&[
        Operation::Push((32, word_neg)),
        Operation::Push((32, U256::from(4))),
        Operation::Sar,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let result = vm.current_call_frame_mut().unwrap().stack.pop().unwrap();
    let expected = U256::from_big_endian(&[
        0xf8, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00,
    ]);
    assert_eq!(result, expected);
    assert_eq!(current_call_frame.gas_used, U256::from(9));
}

#[test]
fn sar_shift_positive_value() {
    let mut vm = new_vm_with_ops(&[
        Operation::Push((32, U256::from(0x7FFFFF))),
        Operation::Push((32, U256::from(4))),
        Operation::Sar,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let result = vm.current_call_frame_mut().unwrap().stack.pop().unwrap();
    assert_eq!(result, U256::from(0x07FFFF));
    assert_eq!(current_call_frame.gas_used, U256::from(9));
}

#[test]
fn sar_shift_negative_value() {
    let word_neg = U256::from_big_endian(&[
        0x8f, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        0xff, 0xff,
    ]);

    let mut vm = new_vm_with_ops(&[
        Operation::Push((32, word_neg)),
        Operation::Push((32, U256::from(4))),
        Operation::Sar,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let result = vm.current_call_frame_mut().unwrap().stack.pop().unwrap();
    let expected = U256::from_big_endian(&[
        0xf8, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        0xff, 0xff,
    ]);
    // change 0x8f to 0xf8
    assert_eq!(result, expected);
    assert_eq!(current_call_frame.gas_used, U256::from(9));
}

#[test]
fn keccak256_zero_offset_size_four() {
    let operations = [
        // Put the required value in memory
        Operation::Push((
            32,
            U256::from("0xFFFFFFFF00000000000000000000000000000000000000000000000000000000"),
        )),
        Operation::Push0,
        Operation::Mstore, // gas_cost = 3 + 3 = 6
        // Call the opcode
        Operation::Push((1, 4.into())), // size
        Operation::Push0,               // offset
        Operation::Keccak256,           // gas_cost = 30 + 6 + 0 = 36
        Operation::Stop,
    ];

    let mut vm = new_vm_with_ops(&operations).unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    assert_eq!(
        vm.current_call_frame_mut().unwrap().stack.pop().unwrap(),
        U256::from("0x29045a592007d0c246ef02c2223570da9522d0cf0f73282c79a1bc8f0bb2c238")
    );
    assert_eq!(vm.current_call_frame_mut().unwrap().pc(), 40);
    assert_eq!(current_call_frame.gas_used, U256::from(52));
}

#[test]
fn keccak256_zero_offset_size_bigger_than_actual_memory() {
    let operations = [
        // Put the required value in memory
        Operation::Push((
            32,
            U256::from("0xFFFFFFFF00000000000000000000000000000000000000000000000000000000"),
        )),
        Operation::Push0,
        Operation::Mstore, // gas_cost = 3 + 3 = 6
        // Call the opcode
        Operation::Push((1, 33.into())), // size > memory.data.len() (32)
        Operation::Push0,                // offset
        Operation::Keccak256,
        Operation::Stop,
    ];

    let mut vm = new_vm_with_ops(&operations).unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    assert!(
        vm.current_call_frame_mut().unwrap().stack.pop().unwrap()
            == U256::from("0xae75624a7d0413029c1e0facdd38cc8e177d9225892e2490a69c2f1f89512061")
    );
    assert_eq!(vm.current_call_frame_mut().unwrap().pc(), 40);
    assert_eq!(current_call_frame.gas_used, U256::from(61));
}

#[test]
fn keccak256_zero_offset_zero_size() {
    let operations = [
        Operation::Push0, // size
        Operation::Push0, // offset
        Operation::Keccak256,
        Operation::Stop,
    ];

    let mut vm = new_vm_with_ops(&operations).unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    assert_eq!(
        vm.current_call_frame_mut().unwrap().stack.pop().unwrap(),
        U256::from("0xc5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470")
    );
    assert_eq!(vm.current_call_frame_mut().unwrap().pc(), 4);
    assert_eq!(current_call_frame.gas_used, U256::from(34));
}

#[test]
fn keccak256_offset_four_size_four() {
    let operations = [
        // Put the required value in memory
        Operation::Push((
            32,
            U256::from("0xFFFFFFFF00000000000000000000000000000000000000000000000000000000"),
        )),
        Operation::Push0,
        Operation::Mstore,
        // Call the opcode
        Operation::Push((1, 4.into())), // size
        Operation::Push((1, 4.into())), // offset
        Operation::Keccak256,
        Operation::Stop,
    ];

    let mut vm = new_vm_with_ops(&operations).unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    assert_eq!(
        vm.current_call_frame_mut().unwrap().stack.pop().unwrap(),
        U256::from("0xe8e77626586f73b955364c7b4bbf0bb7f7685ebd40e852b164633a4acbd3244c")
    );
    assert_eq!(vm.current_call_frame_mut().unwrap().pc(), 41);
    assert_eq!(current_call_frame.gas_used, U256::from(53));
}

#[test]
fn mstore() {
    let mut vm = new_vm_with_ops(&[
        Operation::Push((32, U256::from(0x33333))),
        Operation::Push((32, U256::zero())),
        Operation::Mstore,
        Operation::Msize,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    assert_eq!(
        vm.current_call_frame_mut().unwrap().stack.pop().unwrap(),
        U256::from(32)
    );
    assert_eq!(vm.current_call_frame_mut().unwrap().pc(), 69);
    assert_eq!(current_call_frame.gas_used, U256::from(14));
}

#[test]
fn mstore_saves_correct_value() {
    let mut vm = new_vm_with_ops(&[
        Operation::Push((32, U256::from(0x33333))), // value
        Operation::Push((32, U256::zero())),        // offset
        Operation::Mstore,
        Operation::Msize,
        Operation::Stop,
    ])
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let stored_value = memory::load_word(
        &mut vm.current_call_frame_mut().unwrap().memory,
        U256::zero(),
    )
    .unwrap();

    assert_eq!(stored_value, U256::from(0x33333));

    let memory_size = vm.current_call_frame_mut().unwrap().stack.pop().unwrap();
    assert_eq!(memory_size, U256::from(32));
    assert_eq!(current_call_frame.gas_used, U256::from(14));
}

#[test]
fn mstore8() {
    let operations = [
        Operation::Push((32, U256::from(0xAB))), // value
        Operation::Push((32, U256::zero())),     // offset
        Operation::Mstore8,
        Operation::Stop,
    ];

    let mut vm = new_vm_with_ops(&operations).unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let stored_value = memory::load_word(
        &mut vm.current_call_frame_mut().unwrap().memory,
        U256::zero(),
    )
    .unwrap();

    let mut value_bytes = [0u8; 32];
    stored_value.to_big_endian(&mut value_bytes);

    assert_eq!(value_bytes[0..1], [0xAB]);
    assert_eq!(current_call_frame.gas_used, U256::from(12));
}

#[test]
fn mcopy() {
    let operations = [
        Operation::Push((32, U256::from(32))),      // size
        Operation::Push((32, U256::from(0))),       // source offset
        Operation::Push((32, U256::from(64))),      // destination offset
        Operation::Push((32, U256::from(0x33333))), // value
        Operation::Push((32, U256::from(0))),       // offset
        Operation::Mstore,
        Operation::Mcopy,
        Operation::Msize,
        Operation::Stop,
    ];

    let mut vm = new_vm_with_ops(&operations).unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let copied_value = memory::load_word(
        &mut vm.current_call_frame_mut().unwrap().memory,
        U256::from(64),
    )
    .unwrap();
    assert_eq!(copied_value, U256::from(0x33333));

    let memory_size = vm.current_call_frame_mut().unwrap().stack.pop().unwrap();
    assert_eq!(memory_size, U256::from(96));
    assert_eq!(current_call_frame.gas_used, U256::from(35));
}

#[test]
fn mload() {
    let operations = [
        Operation::Push((32, U256::from(0x33333))), // value
        Operation::Push((32, U256::zero())),        // offset
        Operation::Mstore,
        Operation::Push((32, U256::zero())), // offset
        Operation::Mload,
        Operation::Stop,
    ];

    let mut vm = new_vm_with_ops(&operations).unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let loaded_value = vm.current_call_frame_mut().unwrap().stack.pop().unwrap();
    assert_eq!(loaded_value, U256::from(0x33333));
    assert_eq!(current_call_frame.gas_used, U256::from(18));
}

#[test]
fn msize() {
    let operations = [Operation::Msize, Operation::Stop];

    let mut vm = new_vm_with_ops(&operations).unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let initial_size = vm.current_call_frame_mut().unwrap().stack.pop().unwrap();
    assert_eq!(initial_size, U256::from(0));
    assert_eq!(current_call_frame.gas_used, U256::from(2));

    let operations = [
        Operation::Push((32, U256::from(0x33333))), // value
        Operation::Push((32, U256::zero())),        // offset
        Operation::Mstore,
        Operation::Msize,
        Operation::Stop,
    ];

    let mut vm = new_vm_with_ops(&operations).unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let after_store_size = vm.current_call_frame_mut().unwrap().stack.pop().unwrap();
    assert_eq!(after_store_size, U256::from(32));
    assert_eq!(current_call_frame.gas_used, U256::from(14));

    let operations = [
        Operation::Push((32, U256::from(0x55555))), // value
        Operation::Push((32, U256::from(64))),      // offset
        Operation::Mstore,
        Operation::Msize,
        Operation::Stop,
    ];

    let mut vm = new_vm_with_ops(&operations).unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let final_size = vm.current_call_frame_mut().unwrap().stack.pop().unwrap();
    assert_eq!(final_size, U256::from(96));
    assert_eq!(current_call_frame.gas_used, U256::from(20));
}

#[test]
fn mstore_mload_offset_not_multiple_of_32() {
    let operations = [
        Operation::Push((32, 0xabcdef.into())), // value
        Operation::Push((32, 10.into())),       // offset
        Operation::Mstore,
        Operation::Push((32, 10.into())), // offset
        Operation::Mload,
        Operation::Msize,
        Operation::Stop,
    ];

    let mut vm = new_vm_with_ops(&operations).unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let memory_size = vm.current_call_frame_mut().unwrap().stack.pop().unwrap();
    let loaded_value = vm.current_call_frame_mut().unwrap().stack.pop().unwrap();

    assert_eq!(loaded_value, U256::from(0xabcdef));
    assert_eq!(memory_size, U256::from(64));
    assert_eq!(current_call_frame.gas_used, U256::from(23));

    // check with big offset

    let operations = [
        Operation::Push((32, 0x123456.into())), // value
        Operation::Push((32, 2000.into())),     // offset
        Operation::Mstore,
        Operation::Push((32, 2000.into())), // offset
        Operation::Mload,
        Operation::Msize,
        Operation::Stop,
    ];

    let mut vm = new_vm_with_ops(&operations).unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let memory_size = vm.current_call_frame_mut().unwrap().stack.pop().unwrap();
    let loaded_value = vm.current_call_frame_mut().unwrap().stack.pop().unwrap();

    assert_eq!(loaded_value, U256::from(0x123456));
    assert_eq!(memory_size, U256::from(2048));
    assert_eq!(current_call_frame.gas_used, U256::from(217));
}

#[test]
fn mload_uninitialized_memory() {
    let operations = [
        Operation::Push((32, 50.into())), // offset
        Operation::Mload,
        Operation::Msize,
        Operation::Stop,
    ];

    let mut vm = new_vm_with_ops(&operations).unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let memory_size = vm.current_call_frame_mut().unwrap().stack.pop().unwrap();
    let loaded_value = vm.current_call_frame_mut().unwrap().stack.pop().unwrap();

    assert_eq!(loaded_value, U256::zero());
    assert_eq!(memory_size, U256::from(96));
    assert_eq!(current_call_frame.gas_used, U256::from(17));
}

#[test]
fn call_returns_if_bytecode_empty() {
    let callee_bytecode = vec![].into();

    let callee_address = Address::from_low_u64_be(U256::from(2).low_u64());
    let callee_address_u256 = U256::from(2);
    // let callee_account = Account::new(U256::from(500000), callee_bytecode);
    let callee_account = Account::default()
        .with_balance(50000.into())
        .with_bytecode(callee_bytecode);

    let caller_ops = vec![
        Operation::Push((32, U256::from(32))),      // ret_size
        Operation::Push((32, U256::from(0))),       // ret_offset
        Operation::Push((32, U256::from(0))),       // args_size
        Operation::Push((32, U256::from(0))),       // args_offset
        Operation::Push((32, U256::zero())),        // value
        Operation::Push((32, callee_address_u256)), // address
        Operation::Push((32, U256::from(100_000))), // gas
        Operation::Call,
        Operation::Stop,
    ];

    let mut db = Db::new();
    db.add_accounts(vec![(callee_address, callee_account.clone())]);

    let mut cache = CacheDB::default();
    cache::insert_account(&mut cache, callee_address, callee_account);

    let mut vm = new_vm_with_ops_addr_bal_db(
        ops_to_bytecode(&caller_ops).unwrap(),
        Address::from_low_u64_be(U256::from(1).low_u64()),
        U256::zero(),
        db,
        cache,
    )
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let success = vm.current_call_frame_mut().unwrap().stack.pop().unwrap();
    assert_eq!(success, U256::one());
}

#[test]
fn call_changes_callframe_and_stores() {
    let callee_return_value = U256::from(0xAAAAAAA);
    let callee_bytecode = callee_return_bytecode(callee_return_value);
    let callee_address = Address::from_low_u64_be(U256::from(2).low_u64());
    let callee_address_u256 = U256::from(2);
    let callee_account = Account::default()
        .with_balance(50000.into())
        .with_bytecode(callee_bytecode);

    let caller_ops = vec![
        Operation::Push((32, U256::from(32))),      // ret_size
        Operation::Push((32, U256::from(0))),       // ret_offset
        Operation::Push((32, U256::from(0))),       // args_size
        Operation::Push((32, U256::from(0))),       // args_offset
        Operation::Push((32, U256::zero())),        // value
        Operation::Push((32, callee_address_u256)), // address
        Operation::Push((32, U256::from(100_000))), // gas
        Operation::Call,
        Operation::Stop,
    ];

    let mut db = Db::new();
    db.add_accounts(vec![(callee_address, callee_account.clone())]);

    let mut cache = CacheDB::default();
    cache::insert_account(&mut cache, callee_address, callee_account);

    let mut vm = new_vm_with_ops_addr_bal_db(
        ops_to_bytecode(&caller_ops).unwrap(),
        Address::from_low_u64_be(U256::from(1).low_u64()),
        U256::zero(),
        db,
        cache,
    )
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let current_call_frame = vm.current_call_frame_mut().unwrap();

    let success = current_call_frame.stack.pop().unwrap() == U256::one();
    assert!(success);

    // These are ret_offset and ret_size used in CALL operation before.
    let ret_offset = current_call_frame.sub_return_data_offset;
    let ret_size = current_call_frame.sub_return_data_size;

    // Return data of the sub-context will be in the memory position of the current context reserved for that purpose (ret_offset and ret_size)
    let return_data =
        memory::load_range(&mut current_call_frame.memory, ret_offset, ret_size).unwrap();

    assert_eq!(U256::from_big_endian(return_data), U256::from(0xAAAAAAA));
}

#[test]
fn nested_calls() {
    let callee3_return_value = U256::from(0xAAAAAAA);
    let callee3_bytecode = callee_return_bytecode(callee3_return_value);
    let callee3_address = Address::from_low_u64_be(U256::from(3).low_u64());
    let callee3_address_u256 = U256::from(3);
    let callee3_account = Account::default()
        .with_balance(50_000.into())
        .with_bytecode(callee3_bytecode);

    let mut callee2_ops = vec![
        Operation::Push((32, U256::from(32))),       // ret_size
        Operation::Push((32, U256::from(0))),        // ret_offset
        Operation::Push((32, U256::from(0))),        // args_size
        Operation::Push((32, U256::from(0))),        // args_offset
        Operation::Push((32, U256::zero())),         // value
        Operation::Push((32, callee3_address_u256)), // address
        Operation::Push((32, U256::from(100_000))),  // gas
        Operation::Call,
    ];

    let callee2_return_value = U256::from(0xBBBBBBB);

    let callee2_return_bytecode = vec![
        Operation::Push((32, callee2_return_value)), // value
        Operation::Push((32, U256::from(32))),       // offset
        Operation::Mstore,
        Operation::Push((32, U256::from(32))), // size
        Operation::Push((32, U256::zero())),   // returndata_offset
        Operation::Push((32, U256::zero())),   // dest_offset
        Operation::ReturnDataCopy,
        Operation::Push((32, U256::from(64))), // size
        Operation::Push((32, U256::zero())),   // offset
        Operation::Return,
    ];

    callee2_ops.extend(callee2_return_bytecode);

    let callee2_bytecode = ops_to_bytecode(&callee2_ops).unwrap();

    let callee2_address = Address::from_low_u64_be(U256::from(2).low_u64());
    let callee2_address_u256 = U256::from(2);

    let callee2_account = Account::default()
        .with_balance(50000.into())
        .with_bytecode(callee2_bytecode);

    let caller_ops = vec![
        Operation::Push((32, U256::from(64))),       // ret_size
        Operation::Push((32, U256::from(0))),        // ret_offset
        Operation::Push((32, U256::from(0))),        // args_size
        Operation::Push((32, U256::from(0))),        // args_offset
        Operation::Push((32, U256::zero())),         // value
        Operation::Push((32, callee2_address_u256)), // address
        Operation::Push((32, U256::from(100_000))),  // gas
        Operation::Call,
        Operation::Stop,
    ];

    let caller_address = Address::from_low_u64_be(U256::from(1).low_u64());
    let caller_balance = U256::from(1_000_000);

    let mut db = Db::new();
    db.add_accounts(vec![
        (callee2_address, callee2_account.clone()),
        (callee3_address, callee3_account.clone()),
    ]);

    let mut cache = CacheDB::default();
    cache::insert_account(&mut cache, callee2_address, callee2_account);
    cache::insert_account(&mut cache, callee3_address, callee3_account);

    let mut vm = new_vm_with_ops_addr_bal_db(
        ops_to_bytecode(&caller_ops).unwrap(),
        caller_address,
        caller_balance,
        db,
        cache,
    )
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let current_call_frame = vm.current_call_frame_mut().unwrap();

    let success = current_call_frame.stack.pop().unwrap();
    assert_eq!(success, U256::one());

    let ret_offset: usize = 0;
    let ret_size = 64;
    let return_data = current_call_frame
        .sub_return_data
        .slice(ret_offset..ret_offset + ret_size);

    let mut expected_bytes = vec![0u8; 64];
    // place 0xAAAAAAA at 0..32
    let mut callee3_return_value_bytes = [0u8; 32];
    callee3_return_value.to_big_endian(&mut callee3_return_value_bytes);
    expected_bytes[..32].copy_from_slice(&callee3_return_value_bytes);

    // place 0xBBBBBBB at 32..64
    let mut callee2_return_value_bytes = [0u8; 32];
    callee2_return_value.to_big_endian(&mut callee2_return_value_bytes);
    expected_bytes[32..].copy_from_slice(&callee2_return_value_bytes);

    assert_eq!(return_data, expected_bytes);
}

#[test]
fn staticcall_changes_callframe_is_static() {
    let callee_return_value = U256::from(0xAAAAAAA);
    let callee_ops = [
        Operation::Push((32, callee_return_value)), // value
        Operation::Push((32, U256::zero())),        // offset
        Operation::Mstore,
        Operation::Stop,
    ];

    let callee_bytecode = ops_to_bytecode(&callee_ops).unwrap();

    let callee_address = Address::from_low_u64_be(U256::from(2).low_u64());
    let callee_address_u256 = U256::from(2);
    let callee_account = Account::default()
        .with_balance(50000.into())
        .with_bytecode(callee_bytecode);

    let caller_ops = vec![
        Operation::Push((32, U256::from(32))),      // ret_size
        Operation::Push((32, U256::from(0))),       // ret_offset
        Operation::Push((32, U256::from(0))),       // args_size
        Operation::Push((32, U256::from(0))),       // args_offset
        Operation::Push((32, U256::zero())),        // value
        Operation::Push((32, callee_address_u256)), // address
        Operation::Push((32, U256::from(100_000))), // gas
        Operation::StaticCall,
    ];

    let mut db = Db::new();
    db.add_accounts(vec![(callee_address, callee_account.clone())]);

    let mut cache = CacheDB::default();
    cache::insert_account(&mut cache, callee_address, callee_account);

    let mut vm = new_vm_with_ops_addr_bal_db(
        ops_to_bytecode(&caller_ops).unwrap(),
        Address::from_low_u64_be(U256::from(1).low_u64()),
        U256::zero(),
        db,
        cache,
    )
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let mut current_call_frame = vm.call_frames[0].clone();

    let ret_offset = U256::zero();
    let ret_size = 32;
    let return_data =
        memory::load_range(&mut current_call_frame.memory, ret_offset, ret_size).unwrap();

    assert_eq!(U256::from_big_endian(return_data), U256::from(0xAAAAAAA));
    assert!(current_call_frame.is_static);
}

#[test]
fn pop_on_empty_stack() {
    let operations = [Operation::Pop, Operation::Stop];

    let mut vm = new_vm_with_ops(&operations).unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    let tx_report = vm.execute(&mut current_call_frame).unwrap();

    // result should be a Halt with error VMError::StackUnderflow

    assert!(matches!(
        tx_report.result,
        TxResult::Revert(VMError::StackUnderflow)
    ));
    // TODO: assert consumed gas
}

#[test]
fn pc_op() {
    let operations = [Operation::PC, Operation::Stop];
    let mut vm = new_vm_with_ops(&operations).unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    assert_eq!(
        vm.current_call_frame_mut().unwrap().stack.pop().unwrap(),
        U256::from(0)
    );
    assert_eq!(current_call_frame.gas_used, U256::from(2));
}

#[test]
fn pc_op_with_push_offset() {
    let operations = [
        Operation::Push((32, U256::one())),
        Operation::PC,
        Operation::Stop,
    ];

    let mut vm = new_vm_with_ops(&operations).unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    assert_eq!(
        vm.current_call_frame_mut().unwrap().stack.pop().unwrap(),
        U256::from(33)
    );
    assert_eq!(current_call_frame.gas_used, U256::from(5));
}

// #[test]
// fn delegatecall_changes_own_storage_and_regular_call_doesnt() {
//     // --- DELEGATECALL --- changes account 1 storage
//     let callee_return_value = U256::from(0xBBBBBBB);
//     let callee_ops = [
//         Operation::Push((32, callee_return_value)), // value
//         Operation::Push((32, U256::zero())),        // key
//         Operation::Sstore,
//         Operation::Stop,
//     ];

//     let callee_bytecode = callee_ops
//         .iter()
//         .flat_map(Operation::to_bytecode)
//         .collect::<Bytes>();

//     let callee_address = Address::from_low_u64_be(U256::from(2).low_u64());
//     let callee_address_u256 = U256::from(2);
//     let callee_account = Account::default()
//         .with_balance(50000.into())
//         .with_bytecode(callee_bytecode);

//     let caller_ops = vec![
//         Operation::Push((32, U256::from(32))),      // ret_size
//         Operation::Push((32, U256::from(0))),       // ret_offset
//         Operation::Push((32, U256::from(0))),       // args_size
//         Operation::Push((32, U256::from(0))),       // args_offset
//         Operation::Push((32, callee_address_u256)), // code address
//         Operation::Push((32, U256::from(100_000))), // gas
//         Operation::DelegateCall,
//     ];

//     let mut db = Db::new();
//     db.add_accounts(vec![(callee_address, callee_account.clone())]);

//     let mut cache = CacheDB::default();
//     cache::insert_account(&mut cache, callee_address, callee_account);

//     let mut vm = new_vm_with_ops_addr_bal_db(
//         ops_to_bytecode(&caller_ops).unwrap(),
//         Address::from_low_u64_be(U256::from(1).low_u64()),
//         U256::from(1000),
//         db,
//         cache,
//     );

//     let current_call_frame = vm.current_call_frame_mut().unwrap();
//     current_call_frame.msg_sender = Address::from_low_u64_be(U256::from(1).low_u64());
//     current_call_frame.to = Address::from_low_u64_be(U256::from(5).low_u64());

//     let mut current_call_frame = vm.call_frames.pop().unwrap();
//     vm.execute(&mut current_call_frame).unwrap();

//     let storage_slot = vm.cache.get_storage_slot(
//         Address::from_low_u64_be(U256::from(1).low_u64()),
//         U256::zero(),
//     );
//     let slot = StorageSlot {
//         original_value: U256::from(0xBBBBBBB),
//         current_value: U256::from(0xBBBBBBB),
//     };

//     assert_eq!(storage_slot, Some(slot));

//     // --- CALL --- changes account 2 storage

//     let callee_return_value = U256::from(0xAAAAAAA);
//     let callee_ops = [
//         Operation::Push((32, callee_return_value)), // value
//         Operation::Push((32, U256::zero())),        // key
//         Operation::Sstore,
//         Operation::Stop,
//     ];

//     let callee_bytecode = callee_ops
//         .iter()
//         .flat_map(Operation::to_bytecode)
//         .collect::<Bytes>();

//     let callee_address = Address::from_low_u64_be(U256::from(2).low_u64());
//     let callee_address_u256 = U256::from(2);
//     let callee_account = Account::default()
//         .with_balance(50000.into())
//         .with_bytecode(callee_bytecode);

//     let caller_ops = vec![
//         Operation::Push((32, U256::from(32))),      // ret_size
//         Operation::Push((32, U256::from(0))),       // ret_offset
//         Operation::Push((32, U256::from(0))),       // args_size
//         Operation::Push((32, U256::from(0))),       // args_offset
//         Operation::Push((32, U256::zero())),        // value
//         Operation::Push((32, callee_address_u256)), // address
//         Operation::Push((32, U256::from(100_000))), // gas
//         Operation::Call,
//     ];

//     let mut db = Db::new();
//     db.add_accounts(vec![(callee_address, callee_account.clone())]);

//     let mut cache = CacheDB::default();
//     cache::insert_account(&mut cache, callee_address, callee_account);

//     let mut vm = new_vm_with_ops_addr_bal_db(
//         ops_to_bytecode(&caller_ops).unwrap(),
//         Address::from_low_u64_be(U256::from(1).low_u64()),
//         U256::zero(),
//         db,
//         cache
//     );

//     let current_call_frame = vm.current_call_frame_mut().unwrap();
//     current_call_frame.msg_sender = Address::from_low_u64_be(U256::from(1).low_u64());
//     current_call_frame.to = Address::from_low_u64_be(U256::from(5).low_u64());

//     let mut current_call_frame = vm.call_frames.pop().unwrap();
//     vm.execute(&mut current_call_frame).unwrap();

//     let storage_slot = vm.cache.get_storage_slot(callee_address, U256::zero());
//     let slot = StorageSlot {
//         original_value: U256::from(0xAAAAAAA),
//         current_value: U256::from(0xAAAAAAA),
//     };

//     assert_eq!(storage_slot, Some(slot));
// }

// #[test]
// fn delegatecall_and_callcode_differ_on_value_and_msg_sender() {
//     // --- DELEGATECALL
//     let callee_return_value = U256::from(0xBBBBBBB);
//     let callee_ops = [
//         Operation::Push((32, callee_return_value)), // value
//         Operation::Push((32, U256::zero())),        // key
//         Operation::Sstore,
//         Operation::Stop,
//     ];

//     let callee_bytecode = callee_ops
//         .iter()
//         .flat_map(Operation::to_bytecode)
//         .collect::<Bytes>();

//     let callee_address = Address::from_low_u64_be(U256::from(2).low_u64());
//     let callee_address_u256 = U256::from(2);
//     let callee_account = Account::default()
//         .with_balance(50000.into())
//         .with_bytecode(callee_bytecode);

//     let caller_ops = vec![
//         Operation::Push((32, U256::from(32))),      // ret_size
//         Operation::Push((32, U256::from(0))),       // ret_offset
//         Operation::Push((32, U256::from(0))),       // args_size
//         Operation::Push((32, U256::from(0))),       // args_offset
//         Operation::Push((32, callee_address_u256)), // code address
//         Operation::Push((32, U256::from(100_000))), // gas
//         Operation::DelegateCall,
//     ];

//     let mut db = Db::new();
//     db.add_accounts(vec![(callee_address, callee_account.clone())]);

//     let mut cache = CacheDB::default();
//     cache::insert_account(&mut cache, callee_address, callee_account);

//     let mut vm = new_vm_with_ops_addr_bal_db(
//         ops_to_bytecode(&caller_ops).unwrap(),
//         Address::from_low_u64_be(U256::from(1).low_u64()),
//         U256::from(1000),
//         db,
//         cache
//     );

//     let current_call_frame = vm.current_call_frame_mut().unwrap();
//     current_call_frame.msg_sender = Address::from_low_u64_be(U256::from(1).low_u64());
//     current_call_frame.to = Address::from_low_u64_be(U256::from(5).low_u64());

//     let mut current_call_frame = vm.call_frames.pop().unwrap();
//     vm.execute(&mut current_call_frame).unwrap();

//     let current_call_frame = vm.current_call_frame_mut().unwrap();

//     assert_eq!(
//         current_call_frame.msg_sender,
//         Address::from_low_u64_be(U256::from(1).low_u64())
//     );
//     assert_eq!(current_call_frame.msg_value, U256::from(0));

//     // --- CALLCODE ---

//     let callee_return_value = U256::from(0xAAAAAAA);
//     let callee_ops = [
//         Operation::Push((32, callee_return_value)), // value
//         Operation::Push((32, U256::zero())),        // key
//         Operation::Sstore,
//         Operation::Stop,
//     ];

//     let callee_bytecode = callee_ops
//         .iter()
//         .flat_map(Operation::to_bytecode)
//         .collect::<Bytes>();

//     let callee_address = Address::from_low_u64_be(U256::from(2).low_u64());
//     let callee_address_u256 = U256::from(2);
//     let callee_account = Account::default()
//         .with_balance(50000.into())
//         .with_bytecode(callee_bytecode);

//     let caller_ops = vec![
//         Operation::Push((32, U256::from(0))),       // ret_size
//         Operation::Push((32, U256::from(0))),       // ret_offset
//         Operation::Push((32, U256::from(0))),       // args_size
//         Operation::Push((32, U256::from(0))),       // args_offset
//         Operation::Push((32, U256::from(100))),     // value
//         Operation::Push((32, callee_address_u256)), // address
//         Operation::Push((32, U256::from(100_000))), // gas
//         Operation::CallCode,
//     ];

//     let mut db = Db::new();
//     db.add_accounts(vec![(callee_address, callee_account.clone())]);

//     let mut cache = CacheDB::default();
//     cache::insert_account(&mut cache, callee_address, callee_account);

//     let mut vm = new_vm_with_ops_addr_bal_db(
//         ops_to_bytecode(&caller_ops).unwrap(),
//         Address::from_low_u64_be(U256::from(1).low_u64()),
//         U256::from(1000),
//         db,
//         cache
//     );

//     let mut current_call_frame = vm.call_frames.pop().unwrap();
//     vm.execute(&mut current_call_frame).unwrap();

//     let current_call_frame = vm.call_frames[0].clone();

//     let storage_slot = vm.cache.get_storage_slot(
//         Address::from_low_u64_be(U256::from(1).low_u64()),
//         U256::zero(),
//     );
//     let slot = StorageSlot {
//         original_value: U256::from(0xAAAAAAA),
//         current_value: U256::from(0xAAAAAAA),
//     };
//     assert_eq!(storage_slot, Some(slot));
//     assert_eq!(
//         current_call_frame.msg_sender,
//         Address::from_low_u64_be(U256::from(2).low_u64())
//     );
//     assert_eq!(current_call_frame.msg_value, U256::from(100));
// }

#[test]
fn jump_position_bigger_than_program_bytecode_size() {
    let operations = [
        Operation::Push((32, U256::from(5000))),
        Operation::Jump,
        Operation::Stop,
        Operation::Push((32, U256::from(10))),
        Operation::Stop,
    ];

    let mut vm = new_vm_with_ops(&operations).unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    let tx_report = vm.execute(&mut current_call_frame).unwrap();
    assert!(matches!(
        tx_report.result,
        TxResult::Revert(VMError::InvalidJump)
    ));
    // TODO: assert consumed gas
}

#[test]
fn jumpi_not_zero() {
    let operations = [
        Operation::Push((32, U256::one())),
        Operation::Push((32, U256::from(68))),
        Operation::Jumpi,
        Operation::Stop, // should skip this one
        Operation::Jumpdest,
        Operation::Push((32, U256::from(10))),
        Operation::Stop,
    ];
    let mut vm = new_vm_with_ops(&operations).unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    assert_eq!(
        vm.current_call_frame_mut().unwrap().stack.pop().unwrap(),
        U256::from(10)
    );
    assert_eq!(current_call_frame.gas_used, U256::from(20));
}

#[test]
fn jumpi_for_zero() {
    let operations = [
        Operation::Push((32, U256::from(100))),
        Operation::Push((32, U256::zero())),
        Operation::Push((32, U256::from(100))),
        Operation::Jumpi,
        Operation::Stop,
        Operation::Jumpdest,
        Operation::Push((32, U256::from(10))),
        Operation::Stop,
    ];

    let mut vm = new_vm_with_ops(&operations).unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    assert_eq!(
        vm.current_call_frame_mut().unwrap().stack.pop().unwrap(),
        U256::from(100)
    );
    assert_eq!(current_call_frame.gas_used, U256::from(19));
}

// This test is just for trying things out, not a real test. But it is useful to have this as an example for conversions between bytes and u256.
#[test]
fn testing_bytes_u256_conversion() {
    // From Bytes to U256 to Bytes again
    let data: Bytes = vec![0x11, 0x22, 0x33, 0x44].into();
    println!("{:?}", data);

    let result = U256::from_big_endian(&data);
    println!("{:?}", result);

    // Convert from U256 to bytes
    let mut temp_bytes = vec![0u8; 32];
    result.to_big_endian(&mut temp_bytes);
    println!("{:?}", temp_bytes);

    let mut i = 0;
    while i < temp_bytes.len() {
        if temp_bytes[i] == 0 {
            temp_bytes.remove(i);
        } else {
            i += 1;
        }
    }

    println!("{:?}", temp_bytes);
    let temp_bytes = Bytes::from(temp_bytes);
    println!("{:?}", temp_bytes);

    // Pad the rest with zeroes
    let mut final_data = vec![];
    for i in 0..32 {
        if i < temp_bytes.len() {
            final_data.push(temp_bytes[i]);
        } else {
            final_data.push(0);
        }
    }

    let final_data = Bytes::from(final_data);
    println!("{:?}", final_data);

    let result = U256::from_big_endian(&final_data);
    println!("{:?}", result);
}

#[test]
fn calldataload() {
    let calldata = vec![
        0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF,
        0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09,
    ]
    .into();
    println!("{:?}", calldata);
    let ops = vec![
        Operation::Push((32, U256::from(1))), // offset
        Operation::CallDataLoad,
        Operation::Stop,
    ];
    let mut vm = new_vm_with_ops(&ops).unwrap();

    vm.current_call_frame_mut().unwrap().calldata = calldata;
    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let current_call_frame = vm.current_call_frame_mut().unwrap();

    let top_of_stack = current_call_frame.stack.pop().unwrap();
    assert_eq!(
        top_of_stack,
        U256::from_big_endian(&[
            0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF,
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00
        ])
    );
    assert_eq!(current_call_frame.gas_used, U256::from(6));
}

#[test]
fn calldataload_being_set_by_parent() {
    let ops = vec![
        Operation::Push((32, U256::zero())), // offset
        Operation::CallDataLoad,
        Operation::Push((32, U256::from(0))), // offset
        Operation::Mstore,
        Operation::Push((32, U256::from(32))), // size
        Operation::Push((32, U256::zero())),   // offset
        Operation::Return,
    ];

    let callee_bytecode = ops_to_bytecode(&ops).unwrap();

    let callee_address = Address::from_low_u64_be(U256::from(2).low_u64());
    let callee_address_u256 = U256::from(2);
    let callee_account = Account::default()
        .with_balance(50000.into())
        .with_bytecode(callee_bytecode);

    let calldata = [
        0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF,
        0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E,
        0x0F, 0x10,
    ];

    let caller_ops = vec![
        Operation::Push((32, U256::from_big_endian(&calldata[..32]))), // value
        Operation::Push((32, U256::from(0))),                          // offset
        Operation::Mstore,
        Operation::Push((32, U256::from(32))),      // ret_size
        Operation::Push((32, U256::from(0))),       // ret_offset
        Operation::Push((32, U256::from(32))),      // args_size
        Operation::Push((32, U256::from(0))),       // args_offset
        Operation::Push((32, U256::zero())),        // value
        Operation::Push((32, callee_address_u256)), // address
        Operation::Push((32, U256::from(100_000))), // gas
        Operation::Call,
        Operation::Stop,
    ];

    let mut db = Db::new();
    db.add_accounts(vec![(callee_address, callee_account.clone())]);

    let mut cache = CacheDB::default();
    cache::insert_account(&mut cache, callee_address, callee_account);

    let mut vm = new_vm_with_ops_addr_bal_db(
        ops_to_bytecode(&caller_ops).unwrap(),
        Address::from_low_u64_be(U256::from(1).low_u64()),
        U256::zero(),
        db,
        cache,
    )
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let current_call_frame = vm.current_call_frame_mut().unwrap();

    let calldata = [
        0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF,
        0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E,
        0x0F, 0x10,
    ];

    let expected_data = U256::from_big_endian(&calldata[..32]);

    assert_eq!(
        expected_data,
        memory::load_word(&mut current_call_frame.memory, U256::zero()).unwrap()
    );
    assert_eq!(
        expected_data,
        memory::load_word(&mut current_call_frame.memory, U256::zero()).unwrap()
    );
}

#[test]
fn calldatasize() {
    let calldata = vec![0x11, 0x22, 0x33].into();
    let ops = vec![Operation::CallDataSize, Operation::Stop];
    let mut vm = new_vm_with_ops(&ops).unwrap();

    vm.current_call_frame_mut().unwrap().calldata = calldata;

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let current_call_frame = vm.current_call_frame_mut().unwrap();
    let top_of_stack = current_call_frame.stack.pop().unwrap();
    assert_eq!(top_of_stack, U256::from(3));
    assert_eq!(current_call_frame.gas_used, U256::from(2));
}

#[test]
fn calldatacopy() {
    let calldata = vec![0x11, 0x22, 0x33, 0x44, 0x55].into();
    let ops = vec![
        Operation::Push((32, U256::from(2))), // size
        Operation::Push((32, U256::from(1))), // calldata_offset
        Operation::Push((32, U256::from(0))), // dest_offset
        Operation::CallDataCopy,
        Operation::Stop,
    ];
    let mut vm = new_vm_with_ops(&ops).unwrap();

    vm.current_call_frame_mut().unwrap().calldata = calldata;

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let current_call_frame = vm.current_call_frame_mut().unwrap();
    let memory = memory::load_range(&mut current_call_frame.memory, U256::zero(), 2).unwrap();
    assert_eq!(memory, vec![0x22, 0x33]);
    assert_eq!(current_call_frame.gas_used, U256::from(18));
}

#[test]
fn returndatasize() {
    let returndata = vec![0xAA, 0xBB, 0xCC].into();
    let ops = vec![Operation::ReturnDataSize, Operation::Stop];
    let mut vm = new_vm_with_ops(&ops).unwrap();

    vm.current_call_frame_mut().unwrap().sub_return_data = returndata;

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let current_call_frame = vm.current_call_frame_mut().unwrap();
    let top_of_stack = current_call_frame.stack.pop().unwrap();
    assert_eq!(top_of_stack, U256::from(3));
    assert_eq!(current_call_frame.gas_used, U256::from(2));
}

#[test]
fn returndatacopy() {
    let returndata = vec![0xAA, 0xBB, 0xCC, 0xDD].into();
    let ops = vec![
        Operation::Push((32, U256::from(2))), // size
        Operation::Push((32, U256::from(1))), // returndata_offset
        Operation::Push((32, U256::from(0))), // dest_offset
        Operation::ReturnDataCopy,
        Operation::Stop,
    ];
    let mut vm = new_vm_with_ops(&ops).unwrap();

    vm.current_call_frame_mut().unwrap().sub_return_data = returndata;

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let current_call_frame = vm.current_call_frame_mut().unwrap();
    let memory = memory::load_range(&mut current_call_frame.memory, U256::zero(), 2).unwrap();
    assert_eq!(memory, vec![0xBB, 0xCC]);
    assert_eq!(current_call_frame.gas_used, U256::from(18));
}

#[test]
fn returndatacopy_being_set_by_parent() {
    let callee_bytecode = callee_return_bytecode(U256::from(0xAAAAAAA));

    let callee_address = Address::from_low_u64_be(U256::from(2).low_u64());
    let callee_account = Account::default()
        .with_balance(50000.into())
        .with_bytecode(callee_bytecode);

    let caller_ops = vec![
        Operation::Push((32, U256::from(0))),       // ret_offset
        Operation::Push((32, U256::from(32))),      // ret_size
        Operation::Push((32, U256::from(0))),       // args_size
        Operation::Push((32, U256::from(0))),       // args_offset
        Operation::Push((32, U256::zero())),        // value
        Operation::Push((32, U256::from(2))),       // callee address
        Operation::Push((32, U256::from(100_000))), // gas
        Operation::Call,
        Operation::Push((32, U256::from(32))), // size
        Operation::Push((32, U256::from(0))),  // returndata offset
        Operation::Push((32, U256::from(0))),  // dest offset
        Operation::ReturnDataCopy,
        Operation::Stop,
    ];

    let mut db = Db::new();
    db.add_accounts(vec![(callee_address, callee_account.clone())]);

    let mut cache = CacheDB::default();
    cache::insert_account(&mut cache, callee_address, callee_account);

    let mut vm = new_vm_with_ops_addr_bal_db(
        ops_to_bytecode(&caller_ops).unwrap(),
        Address::from_low_u64_be(U256::from(1).low_u64()),
        U256::zero(),
        db,
        cache,
    )
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let current_call_frame = vm.current_call_frame_mut().unwrap();

    let result = memory::load_word(&mut current_call_frame.memory, U256::zero()).unwrap();

    assert_eq!(result, U256::from(0xAAAAAAA));
}

#[test]
fn blockhash_op() {
    let block_number = 1;
    let block_hash = H256::from_low_u64_be(12345678);
    let current_block_number = U256::from(3);
    let expected_block_hash = U256::from_big_endian(&block_hash.0);

    let operations = [
        Operation::Push((1, U256::from(block_number))),
        Operation::BlockHash,
        Operation::Stop,
    ];

    let mut db = Db::new();
    db.add_block_hashes(vec![(block_number, block_hash)]);

    let mut vm = new_vm_with_ops_addr_bal_db(
        ops_to_bytecode(&operations).unwrap(),
        Address::default(),
        U256::MAX,
        db,
        CacheDB::default(),
    )
    .unwrap();

    vm.env.block_number = current_block_number;

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    assert_eq!(
        vm.current_call_frame_mut().unwrap().stack.pop().unwrap(),
        expected_block_hash
    );
    assert_eq!(current_call_frame.gas_used, U256::from(23));
}

#[test]
fn blockhash_same_block_number() {
    let block_number = U256::one();
    let block_hash = 12345678;
    let current_block_number = block_number;
    let expected_block_hash = U256::zero();

    let operations = [
        Operation::Push((1, block_number)),
        Operation::BlockHash,
        Operation::Stop,
    ];

    let mut vm = new_vm_with_ops(&operations).unwrap();
    let mut storage = Storage::default();
    storage.insert(block_number, H256::from_low_u64_be(block_hash));
    // vm.world_state.insert(
    //     Address::default(),
    //     Account::new(U256::MAX, Bytes::default(), 0, storage),
    // );
    vm.env.block_number = current_block_number;

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    assert_eq!(
        vm.current_call_frame_mut().unwrap().stack.pop().unwrap(),
        expected_block_hash
    );
    assert_eq!(current_call_frame.gas_used, U256::from(23));
}

#[test]
fn blockhash_block_number_not_from_recent_256() {
    let block_number = 1;
    let block_hash = H256::from_low_u64_be(12345678);
    let current_block_number = U256::from(258);
    let expected_block_hash = U256::zero();

    let operations = [
        Operation::Push((1, U256::from(block_number))),
        Operation::BlockHash,
        Operation::Stop,
    ];

    let mut db = Db::new();
    db.add_block_hashes(vec![(block_number, block_hash)]);
    let mut vm = new_vm_with_ops_addr_bal_db(
        ops_to_bytecode(&operations).unwrap(),
        Address::default(),
        U256::MAX,
        db,
        CacheDB::default(),
    )
    .unwrap();

    vm.env.block_number = current_block_number;

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    assert_eq!(
        vm.current_call_frame_mut().unwrap().stack.pop().unwrap(),
        expected_block_hash
    );
    assert_eq!(current_call_frame.gas_used, U256::from(23));
}

#[test]
fn coinbase_op() {
    let coinbase_address = 100;

    let operations = [Operation::Coinbase, Operation::Stop];

    let mut vm = new_vm_with_ops(&operations).unwrap();
    vm.env.coinbase = Address::from_low_u64_be(coinbase_address);

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    assert_eq!(
        vm.current_call_frame_mut().unwrap().stack.pop().unwrap(),
        U256::from(coinbase_address)
    );
    assert_eq!(current_call_frame.gas_used, U256::from(2));
}

#[test]
fn timestamp_op() {
    let timestamp = U256::from(100000);

    let operations = [Operation::Timestamp, Operation::Stop];

    let mut vm = new_vm_with_ops(&operations).unwrap();
    vm.env.timestamp = timestamp;

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    assert_eq!(
        vm.current_call_frame_mut().unwrap().stack.pop().unwrap(),
        timestamp
    );
    assert_eq!(current_call_frame.gas_used, U256::from(2));
}

#[test]
fn number_op() {
    let block_number = U256::from(1000);

    let operations = [Operation::Number, Operation::Stop];

    let mut vm = new_vm_with_ops(&operations).unwrap();
    vm.env.block_number = block_number;

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    assert_eq!(
        vm.current_call_frame_mut().unwrap().stack.pop().unwrap(),
        block_number
    );
    assert_eq!(current_call_frame.gas_used, U256::from(2));
}

#[test]
fn prevrandao_op() {
    let prevrandao = H256::from_low_u64_be(2000);

    let operations = [Operation::Prevrandao, Operation::Stop];

    let mut vm = new_vm_with_ops(&operations).unwrap();
    vm.env.prev_randao = Some(prevrandao);

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    assert_eq!(
        vm.current_call_frame_mut().unwrap().stack.pop().unwrap(),
        U256::from_big_endian(&prevrandao.0)
    );
    assert_eq!(current_call_frame.gas_used, U256::from(2));
}

#[test]
fn gaslimit_op() {
    let gas_limit = TX_BASE_COST * 2;

    let operations = [Operation::Gaslimit, Operation::Stop];

    let mut vm = new_vm_with_ops(&operations).unwrap();
    vm.env.block_gas_limit = gas_limit;

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    assert_eq!(
        vm.current_call_frame_mut().unwrap().stack.pop().unwrap(),
        gas_limit
    );
    assert_eq!(current_call_frame.gas_used, U256::from(2));
}

#[test]
/// Test that the VM detects that it has no more gas.
fn no_more_gas() {
    let operations = [
        Operation::Push((32, U256::one())),
        Operation::Push((32, U256::from(100))),
        Operation::Add,
        Operation::Stop,
    ];

    let mut vm = new_vm_with_ops(&operations).unwrap();

    // We are NOT gonna add the costs of the ADD operation; in order
    // for the vm to run out of gas.
    let not_enough_funds = gas_cost::PUSHN + gas_cost::PUSHN + gas_cost::STOP;

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    current_call_frame.gas_limit = not_enough_funds;
    let tx_report = vm.execute(&mut current_call_frame).unwrap();

    assert_eq!(
        tx_report.result,
        TxResult::Revert(VMError::OutOfGas(OutOfGasError::MaxGasLimitExceeded))
    );
}

#[test]
fn chain_id_op() {
    let chain_id = U256::one();

    let operations = [Operation::Chainid, Operation::Stop];

    let mut vm = new_vm_with_ops(&operations).unwrap();
    vm.env.chain_id = chain_id;

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    assert_eq!(
        vm.current_call_frame_mut().unwrap().stack.pop().unwrap(),
        chain_id
    );
    assert_eq!(current_call_frame.gas_used, U256::from(2));
}

#[test]
fn basefee_op() {
    let base_fee_per_gas = U256::from(1000);

    let operations = [Operation::Basefee, Operation::Stop];

    let mut vm = new_vm_with_ops(&operations).unwrap();
    vm.env.base_fee_per_gas = base_fee_per_gas;

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    assert_eq!(
        vm.current_call_frame_mut().unwrap().stack.pop().unwrap(),
        base_fee_per_gas
    );
    assert_eq!(current_call_frame.gas_used, U256::from(2));
}

// TODO: Add excess_blob_gas and blob_gas_used to env
#[test]
fn blobbasefee_op() {
    let operations = [Operation::BlobBaseFee, Operation::Stop];

    let mut vm = new_vm_with_ops(&operations).unwrap();
    vm.env.block_excess_blob_gas = Some(TARGET_BLOB_GAS_PER_BLOCK * 8);
    vm.env.block_blob_gas_used = Some(U256::zero());

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    assert_eq!(
        vm.current_call_frame_mut().unwrap().stack.pop().unwrap(),
        U256::from(2)
    );
    assert_eq!(current_call_frame.gas_used, U256::from(2));
}

// TODO: Add excess_blob_gas and blob_gas_used to env
#[test]
fn blobbasefee_minimum_cost() {
    let operations = [Operation::BlobBaseFee, Operation::Stop];

    let mut vm = new_vm_with_ops(&operations).unwrap();
    vm.env.block_excess_blob_gas = Some(U256::zero());
    vm.env.block_blob_gas_used = Some(U256::zero());

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    assert_eq!(
        vm.current_call_frame_mut().unwrap().stack.pop().unwrap(),
        U256::one()
    );
    assert_eq!(current_call_frame.gas_used, U256::from(2));
}

#[test]
fn pop_op() {
    let operations = [
        Operation::Push((32, U256::one())),
        Operation::Push((32, U256::from(100))),
        Operation::Pop,
        Operation::Stop,
    ];

    let mut vm = new_vm_with_ops(&operations).unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    assert_eq!(
        vm.current_call_frame_mut().unwrap().stack.pop().unwrap(),
        U256::one()
    );
    assert_eq!(current_call_frame.gas_used, U256::from(8));
}

#[test]
fn jump_op() {
    let operations = [
        Operation::Push((32, U256::from(35))),
        Operation::Jump,
        Operation::Stop, // should skip this one
        Operation::Jumpdest,
        Operation::Push((32, U256::from(10))),
        Operation::Stop,
    ];

    let mut vm = new_vm_with_ops(&operations).unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    assert_eq!(
        vm.current_call_frame_mut().unwrap().stack.pop().unwrap(),
        U256::from(10)
    );
    assert_eq!(vm.current_call_frame_mut().unwrap().pc(), 70);
    assert_eq!(current_call_frame.gas_used, U256::from(15));
}

#[test]
fn jump_not_jumpdest_position() {
    let operations = [
        Operation::Push((32, U256::from(36))),
        Operation::Jump,
        Operation::Stop,
        Operation::Push((32, U256::from(10))),
        Operation::Stop,
    ];

    let mut vm = new_vm_with_ops(&operations).unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    let tx_report = vm.execute(&mut current_call_frame).unwrap();
    assert!(matches!(
        tx_report.result,
        TxResult::Revert(VMError::InvalidJump)
    ));
    // TODO: assert consumed gas
}

#[test]
fn sstore_op() {
    let key = U256::from(80);
    let value = U256::from(100);
    let sender_address = Address::from_low_u64_be(3000);
    let operations = vec![
        Operation::Push((1, value)),
        Operation::Push((1, key)),
        Operation::Sstore,
        Operation::Stop,
    ];

    // We don't need to add address to database because if it doesn't exist it returns and empty account, so no problem there.

    let mut vm = new_vm_with_ops(&operations).unwrap();
    vm.current_call_frame_mut().unwrap().to = sender_address;
    vm.current_call_frame_mut().unwrap().code_address = sender_address;

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    // Convert key in U256 to H256
    let mut bytes = [0u8; 32];
    key.to_big_endian(&mut bytes);
    let key = H256::from(bytes);

    let (storage_slot, _storage_slot_was_cold) =
        vm.access_storage_slot(sender_address, key).unwrap();

    assert_eq!(value, storage_slot.current_value);
}

#[test]
fn sstore_reverts_when_called_in_static() {
    let key = U256::from(80);
    let value = U256::from(100);
    let operations = vec![
        Operation::Push((1, value)),
        Operation::Push((1, key)),
        Operation::Sstore,
        Operation::Stop,
    ];

    let mut vm = new_vm_with_ops(&operations).unwrap();
    vm.current_call_frame_mut().unwrap().is_static = true;
    let mut current_call_frame = vm.call_frames.pop().unwrap();
    let tx_report = vm.execute(&mut current_call_frame).unwrap();

    assert!(matches!(
        tx_report.result,
        TxResult::Revert(VMError::OpcodeNotAllowedInStaticContext)
    ));
}

#[test]
fn sload_op() {
    let key = U256::from(80);
    let value = U256::from(100);
    let sender_address = Address::from_low_u64_be(3000);
    let operations = vec![
        Operation::Push((1, value)),
        Operation::Push((1, key)),
        Operation::Sstore,
        Operation::Push((1, key)),
        Operation::Sload,
        Operation::Stop,
    ];

    let mut db = Db::new();
    db.add_accounts(vec![(sender_address, Account::default())]);

    let mut vm = new_vm_with_ops_db(&operations, db).unwrap();
    vm.current_call_frame_mut().unwrap().msg_sender = sender_address;

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    assert_eq!(
        value,
        vm.current_call_frame_mut().unwrap().stack.pop().unwrap()
    );
}

#[test]
fn sload_untouched_key_of_storage() {
    let key = U256::from(404);
    let sender_address = Address::from_low_u64_be(3000);
    let operations = vec![Operation::Push((2, key)), Operation::Sload, Operation::Stop];

    let mut db = Db::new();
    db.add_accounts(vec![(sender_address, Account::default())]);

    let mut vm = new_vm_with_ops_db(&operations, db).unwrap();
    vm.current_call_frame_mut().unwrap().msg_sender = sender_address;

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    assert_eq!(
        U256::zero(),
        vm.current_call_frame_mut().unwrap().stack.pop().unwrap()
    );
}

#[test]
fn sload_on_not_existing_account() {
    let key = U256::from(80);
    let sender_address = Address::from_low_u64_be(3000);
    let operations = vec![Operation::Push((2, key)), Operation::Sload, Operation::Stop];

    let mut vm = new_vm_with_ops(&operations).unwrap();
    vm.current_call_frame_mut().unwrap().msg_sender = sender_address;

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    assert_eq!(
        U256::zero(),
        vm.current_call_frame_mut().unwrap().stack.pop().unwrap()
    );
}

#[test]
fn log0() {
    let data: [u8; 32] = [0xff; 32];
    let size = 32_u8;
    let memory_offset = 0;
    let mut operations = store_data_in_memory_operations(&data, memory_offset);
    let mut log_operations = vec![
        Operation::Push((1_u8, U256::from(size))),
        Operation::Push((1_u8, U256::from(memory_offset))),
        Operation::Log(0),
        Operation::Stop,
    ];
    operations.append(&mut log_operations);

    let mut vm = new_vm_with_ops(&operations).unwrap();
    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let logs = &vm.current_call_frame_mut().unwrap().logs;
    let data = [0xff_u8; 32].as_slice();
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].data, data.to_vec());
    assert_eq!(logs[0].topics.len(), 0);
    assert_eq!(current_call_frame.gas_used, U256::from(649));
}

#[test]
fn log1() {
    let mut topic1 = [0u8; 32];
    topic1[3] = 1;

    let data: [u8; 32] = [0xff; 32];
    let size = 32_u8;
    let memory_offset = 0;
    let mut operations = store_data_in_memory_operations(&data, memory_offset);
    let mut log_operations = vec![
        Operation::Push((32_u8, U256::from_big_endian(&topic1))),
        Operation::Push((1_u8, U256::from(size))),
        Operation::Push((1_u8, U256::from(memory_offset))),
        Operation::Log(1),
        Operation::Stop,
    ];
    operations.append(&mut log_operations);

    let mut vm = new_vm_with_ops(&operations).unwrap();
    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let logs = &vm.current_call_frame_mut().unwrap().logs;
    let data = [0xff_u8; 32].as_slice();
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].data, data.to_vec());
    assert_eq!(logs[0].topics, vec![H256::from_slice(&topic1)]);
    assert_eq!(current_call_frame.gas_used, U256::from(1027));
}

#[test]
fn log2() {
    let mut topic1 = [0u8; 32];
    topic1[3] = 1;
    let mut topic2 = [0u8; 32];
    topic2[3] = 2;

    let data: [u8; 32] = [0xff; 32];
    let size = 32_u8;
    let memory_offset = 0;
    let mut operations = store_data_in_memory_operations(&data, memory_offset);
    let mut log_operations = vec![
        Operation::Push((32_u8, U256::from_big_endian(&topic2))),
        Operation::Push((32_u8, U256::from_big_endian(&topic1))),
        Operation::Push((1_u8, U256::from(size))),
        Operation::Push((1_u8, U256::from(memory_offset))),
        Operation::Log(2),
        Operation::Stop,
    ];
    operations.append(&mut log_operations);

    let mut vm = new_vm_with_ops(&operations).unwrap();
    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let logs = &vm.current_call_frame_mut().unwrap().logs;
    let data = [0xff_u8; 32].as_slice();
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].data, data.to_vec());
    assert_eq!(
        logs[0].topics,
        vec![H256::from_slice(&topic1), H256::from_slice(&topic2)]
    );
    assert_eq!(current_call_frame.gas_used, U256::from(1405));
}

#[test]
fn log3() {
    let mut topic1 = [0u8; 32];
    topic1[3] = 1;
    let mut topic2 = [0u8; 32];
    topic2[3] = 2;
    let mut topic3 = [0u8; 32];
    topic3[3] = 3;

    let data: [u8; 32] = [0xff; 32];
    let size = 32_u8;
    let memory_offset = 0;
    let mut operations = store_data_in_memory_operations(&data, memory_offset);
    let mut log_operations = vec![
        Operation::Push((32_u8, U256::from_big_endian(&topic3))),
        Operation::Push((32_u8, U256::from_big_endian(&topic2))),
        Operation::Push((32_u8, U256::from_big_endian(&topic1))),
        Operation::Push((1_u8, U256::from(size))),
        Operation::Push((1_u8, U256::from(memory_offset))),
        Operation::Log(3),
        Operation::Stop,
    ];
    operations.append(&mut log_operations);

    let mut vm = new_vm_with_ops(&operations).unwrap();
    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let logs = &vm.current_call_frame_mut().unwrap().logs;
    let data = [0xff_u8; 32].as_slice();
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].data, data.to_vec());
    assert_eq!(
        logs[0].topics,
        vec![
            H256::from_slice(&topic1),
            H256::from_slice(&topic2),
            H256::from_slice(&topic3)
        ]
    );
    assert_eq!(current_call_frame.gas_used, U256::from(1783));
}

#[test]
fn log4() {
    let mut topic1 = [0u8; 32];
    topic1[3] = 1;
    let mut topic2 = [0u8; 32];
    topic2[3] = 2;
    let mut topic3 = [0u8; 32];
    topic3[3] = 3;
    let mut topic4 = [0u8; 32];
    topic4[3] = 4;

    let data: [u8; 32] = [0xff; 32];
    let size = 32_u8;
    let memory_offset = 0;
    let mut operations = store_data_in_memory_operations(&data, memory_offset);
    let mut log_operations = vec![
        Operation::Push((32_u8, U256::from_big_endian(&topic4))),
        Operation::Push((32_u8, U256::from_big_endian(&topic3))),
        Operation::Push((32_u8, U256::from_big_endian(&topic2))),
        Operation::Push((32_u8, U256::from_big_endian(&topic1))),
        Operation::Push((1_u8, U256::from(size))),
        Operation::Push((1_u8, U256::from(memory_offset))),
        Operation::Log(4),
        Operation::Stop,
    ];
    operations.append(&mut log_operations);

    let mut vm = new_vm_with_ops(&operations).unwrap();
    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let logs = &vm.current_call_frame_mut().unwrap().logs;
    let data = [0xff_u8; 32].as_slice();
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].data, data.to_vec());
    assert_eq!(
        logs[0].topics,
        vec![
            H256::from_slice(&topic1),
            H256::from_slice(&topic2),
            H256::from_slice(&topic3),
            H256::from_slice(&topic4)
        ]
    );
    assert_eq!(current_call_frame.gas_used, U256::from(2161));
}

#[test]
fn log_with_0_data_size() {
    let data: [u8; 32] = [0xff; 32];
    let size = 0_u8;
    let memory_offset = 0;
    let mut operations = store_data_in_memory_operations(&data, memory_offset);
    let mut log_operations = vec![
        Operation::Push((1_u8, U256::from(size))),
        Operation::Push((1_u8, U256::from(memory_offset))),
        Operation::Log(0),
        Operation::Stop,
    ];
    operations.append(&mut log_operations);

    let mut vm = new_vm_with_ops(&operations).unwrap();
    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let logs = &vm.current_call_frame_mut().unwrap().logs;
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].data, Vec::new());
    assert_eq!(logs[0].topics.len(), 0);
    assert_eq!(current_call_frame.gas_used, U256::from(393));
}

#[test]
fn cant_create_log_in_static_context() {
    let data: [u8; 32] = [0xff; 32];
    let size = 0_u8;
    let memory_offset = 0;
    let mut operations = store_data_in_memory_operations(&data, memory_offset);
    let mut log_operations = vec![
        Operation::Push((1_u8, U256::from(size))),
        Operation::Push((1_u8, U256::from(memory_offset))),
        Operation::Log(0),
        Operation::Stop,
    ];
    operations.append(&mut log_operations);

    let mut vm: VM = new_vm_with_ops(&operations).unwrap();
    vm.current_call_frame_mut().unwrap().is_static = true;
    let mut current_call_frame = vm.call_frames.pop().unwrap();
    let tx_report = vm.execute(&mut current_call_frame).unwrap();

    assert!(matches!(
        tx_report.result,
        TxResult::Revert(VMError::OpcodeNotAllowedInStaticContext)
    ));
}

#[test]
fn log_with_data_in_memory_smaller_than_size() {
    let data: [u8; 16] = [0xff; 16];
    let size = 32_u8;
    let memory_offset = 0;
    let mut operations = store_data_in_memory_operations(&data, memory_offset);
    let mut log_operations = vec![
        Operation::Push((1_u8, U256::from(size))),
        Operation::Push((1_u8, U256::from(memory_offset))),
        Operation::Log(0),
        Operation::Stop,
    ];
    operations.append(&mut log_operations);

    let mut vm = new_vm_with_ops(&operations).unwrap();
    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let logs = &vm.current_call_frame_mut().unwrap().logs;
    let mut data = vec![0_u8; 16];
    data.extend(vec![0xff_u8; 16]);

    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].data, data);
    assert_eq!(logs[0].topics.len(), 0);
    assert_eq!(current_call_frame.gas_used, U256::from(649));
}

#[test]
fn multiple_logs_of_different_types() {
    let mut topic1 = [0u8; 32];
    topic1[3] = 1;

    let data: [u8; 32] = [0xff; 32];
    let size = 32_u8;
    let memory_offset = 0;
    let mut operations = store_data_in_memory_operations(&data, memory_offset);
    let mut log_operations = vec![
        Operation::Push((32_u8, U256::from_big_endian(&topic1))),
        Operation::Push((1_u8, U256::from(size))),
        Operation::Push((1_u8, U256::from(memory_offset))),
        Operation::Log(1),
        Operation::Push((1_u8, U256::from(size))),
        Operation::Push((1_u8, U256::from(memory_offset))),
        Operation::Log(0),
        Operation::Stop,
    ];
    operations.append(&mut log_operations);

    let mut vm = new_vm_with_ops(&operations).unwrap();
    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let logs = &vm.current_call_frame_mut().unwrap().logs;
    let data = [0xff_u8; 32].as_slice();
    assert_eq!(logs.len(), 2);
    assert_eq!(logs[0].data, data.to_vec());
    assert_eq!(logs[1].data, data.to_vec());
    assert_eq!(logs[0].topics, vec![H256::from_slice(&topic1)]);
    assert_eq!(logs[1].topics.len(), 0);
}

#[test]
fn logs_from_multiple_callers() {
    let callee_address = Address::from_low_u64_be(U256::from(2).low_u64());
    let callee_address_u256 = U256::from(2);

    let data: [u8; 32] = [0xff; 32];
    let size = 32_u8;
    let memory_offset = 0;
    let mut operations = store_data_in_memory_operations(&data, memory_offset);
    let mut log_operations = vec![
        Operation::Push((1_u8, U256::from(size))),
        Operation::Push((1_u8, U256::from(memory_offset))),
        Operation::Log(0),
        Operation::Stop,
    ];
    operations.append(&mut log_operations);
    let callee_bytecode = ops_to_bytecode(&operations).unwrap();
    let callee_account = Account::new(U256::from(500000), callee_bytecode, 0, HashMap::new());

    let mut caller_ops = vec![
        Operation::Push((32, U256::from(32))),      // ret_size
        Operation::Push((32, U256::from(0))),       // ret_offset
        Operation::Push((32, U256::from(0))),       // args_size
        Operation::Push((32, U256::from(0))),       // args_offset
        Operation::Push((32, U256::zero())),        // value
        Operation::Push((32, callee_address_u256)), // address
        Operation::Push((32, U256::from(100_000))), // gas
        Operation::Call,
    ];

    caller_ops.append(&mut operations);

    let mut db = Db::new();
    db.add_accounts(vec![(callee_address, callee_account.clone())]);

    let mut cache = CacheDB::default();
    cache::insert_account(&mut cache, callee_address, callee_account);

    let mut vm = new_vm_with_ops_addr_bal_db(
        ops_to_bytecode(&caller_ops).unwrap(),
        Address::from_low_u64_be(U256::from(1).low_u64()),
        U256::zero(),
        db,
        cache,
    )
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    assert_eq!(current_call_frame.logs.len(), 2)
}

// #[test]
// fn call_return_success_but_caller_halts() {
//     let callee_address = Address::from_low_u64_be(U256::from(2).low_u64());
//     let callee_address_u256 = U256::from(2);

//     let operations = vec![Operation::Pop, Operation::Stop];
//     let callee_bytecode = operations
//         .clone()
//         .iter()
//         .flat_map(Operation::to_bytecode)
//         .collect::<Bytes>();
//     let callee_account = Account::new(
//         callee_address,
//         U256::from(500000),
//         callee_bytecode,
//         0,
//         HashMap::new(),
//     );

//     let caller_ops = vec![
//         Operation::Push((32,U256::from(32))),      // ret_size
//         Operation::Push((32,U256::from(0))),       // ret_offset
//         Operation::Push((32,U256::from(0))),       // args_size
//         Operation::Push((32,U256::from(0))),       // args_offset
//         Operation::Push((32,U256::zero())),        // value
//         Operation::Push((32,callee_address_u256)), // address
//         Operation::Push((32,U256::from(100_000))), // gas
//         Operation::Call,
//         Operation::Stop,
//     ];

//     let mut vm = new_vm_with_ops_addr_bal(
//         &caller_ops,
//         Address::from_low_u64_be(U256::from(1).low_u64()),
//         U256::zero(),
//     );

//     vm.db.add_account(callee_address, callee_account);

//     let mut current_call_frame = vm.call_frames.pop().unwrap();
//     vm.execute(&mut current_call_frame).unwrap();

//     assert_eq!(
//         vm.current_call_frame_mut().unwrap().stack.pop().unwrap(),
//         U256::from(HALT_FOR_CALL)
//     );
// }

#[test]
fn push0_ok() {
    let mut vm = new_vm_with_ops(&[Operation::Push0, Operation::Stop]).unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    assert_eq!(
        vm.current_call_frame_mut().unwrap().stack.stack[0],
        U256::zero()
    );
    assert_eq!(vm.current_call_frame_mut().unwrap().pc(), 2);
}

#[test]
fn push1_ok() {
    let to_push = U256::from_big_endian(&[0xff]);
    let operations = [Operation::Push((1, to_push)), Operation::Stop];
    let mut vm = new_vm_with_ops(&operations).unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    assert_eq!(vm.current_call_frame_mut().unwrap().stack.stack[0], to_push);
    assert_eq!(vm.current_call_frame_mut().unwrap().pc(), 3);
}

#[test]
fn push5_ok() {
    let to_push = U256::from_big_endian(&[0xff, 0xff, 0xff, 0xff, 0xff]);
    let operations = [Operation::Push((5, to_push)), Operation::Stop];
    let mut vm = new_vm_with_ops(&operations).unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    assert_eq!(vm.current_call_frame_mut().unwrap().stack.stack[0], to_push);
    assert_eq!(vm.current_call_frame_mut().unwrap().pc(), 7);
}

#[test]
fn push31_ok() {
    let to_push = U256::from_big_endian(&[0xff; 31]);
    let operations = [Operation::Push((31, to_push)), Operation::Stop];
    let mut vm = new_vm_with_ops(&operations).unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    assert_eq!(vm.current_call_frame_mut().unwrap().stack.stack[0], to_push);
    assert_eq!(vm.current_call_frame_mut().unwrap().pc(), 33);
}

#[test]
fn push32_ok() {
    let to_push = U256::from_big_endian(&[0xff; 32]);
    let operations = [Operation::Push((32, to_push)), Operation::Stop];
    let mut vm = new_vm_with_ops(&operations).unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    assert_eq!(vm.current_call_frame_mut().unwrap().stack.stack[0], to_push);
    assert_eq!(vm.current_call_frame_mut().unwrap().pc(), 34);
}

#[test]
fn dup1_ok() {
    let value = U256::one();
    let operations = [
        Operation::Push((1, value)),
        Operation::Dup(1),
        Operation::Stop,
    ];
    let mut vm = new_vm_with_ops(&operations).unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let stack_len = vm.current_call_frame_mut().unwrap().stack.len();

    assert_eq!(stack_len, 2);
    assert_eq!(vm.current_call_frame_mut().unwrap().pc(), 4);
    assert_eq!(
        vm.current_call_frame_mut().unwrap().stack.stack[stack_len - 1],
        value
    );
    assert_eq!(
        vm.current_call_frame_mut().unwrap().stack.stack[stack_len - 2],
        value
    );
}

#[test]
fn dup16_ok() {
    let value = U256::one();
    let mut operations = vec![Operation::Push((1, value))];
    operations.extend(vec![Operation::Push0; 15]);
    operations.extend(vec![Operation::Dup(16), Operation::Stop]);

    let mut vm = new_vm_with_ops(&operations).unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let stack_len = vm.current_call_frame_mut().unwrap().stack.len();

    assert_eq!(stack_len, 17);
    assert_eq!(vm.current_call_frame_mut().unwrap().pc, 19);
    assert_eq!(
        vm.current_call_frame_mut().unwrap().stack.stack[stack_len - 1],
        value
    );
    assert_eq!(
        vm.current_call_frame_mut().unwrap().stack.stack[stack_len - 17],
        value
    );
}

#[test]
fn dup_halts_if_stack_underflow() {
    let operations = [Operation::Dup(5), Operation::Stop];
    let mut vm = new_vm_with_ops(&operations).unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    let tx_report = vm.execute(&mut current_call_frame).unwrap();

    assert!(matches!(
        tx_report.result,
        TxResult::Revert(VMError::StackUnderflow)
    ));
}

#[test]
fn swap1_ok() {
    let bottom = U256::from_big_endian(&[0xff]);
    let top = U256::from_big_endian(&[0xee]);
    let operations = [
        Operation::Push((1, bottom)),
        Operation::Push((1, top)),
        Operation::Swap(1),
        Operation::Stop,
    ];
    let mut vm = new_vm_with_ops(&operations).unwrap();
    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    assert_eq!(vm.current_call_frame_mut().unwrap().stack.len(), 2);
    assert_eq!(vm.current_call_frame_mut().unwrap().pc(), 6);
    assert_eq!(vm.current_call_frame_mut().unwrap().stack.stack[0], top);
    assert_eq!(vm.current_call_frame_mut().unwrap().stack.stack[1], bottom);
}

#[test]
fn swap16_ok() {
    let bottom = U256::from_big_endian(&[0xff]);
    let top = U256::from_big_endian(&[0xee]);
    let mut operations = vec![Operation::Push((1, bottom))];
    operations.extend(vec![Operation::Push0; 15]);
    operations.extend(vec![Operation::Push((1, top))]);
    operations.extend(vec![Operation::Swap(16), Operation::Stop]);

    let mut vm = new_vm_with_ops(&operations).unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();
    let stack_len = vm.current_call_frame_mut().unwrap().stack.len();

    assert_eq!(stack_len, 17);
    assert_eq!(vm.current_call_frame_mut().unwrap().pc(), 21);
    assert_eq!(
        vm.current_call_frame_mut().unwrap().stack.stack[stack_len - 1],
        bottom
    );
    assert_eq!(
        vm.current_call_frame_mut().unwrap().stack.stack[stack_len - 1 - 16],
        top
    );
}

#[test]
fn swap_halts_if_stack_underflow() {
    let operations = [Operation::Swap(5), Operation::Stop];
    let mut vm = new_vm_with_ops(&operations).unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    let tx_report = vm.execute(&mut current_call_frame).unwrap();

    assert!(matches!(
        tx_report.result,
        TxResult::Revert(VMError::StackUnderflow)
    ));
}

#[test]
fn transient_store() {
    let value = U256::from_big_endian(&[0xaa; 3]);
    let key = U256::from_big_endian(&[0xff; 2]);

    let operations = [
        Operation::Push((32, value)),
        Operation::Push((32, key)),
        Operation::Tstore,
        Operation::Stop,
    ];

    let mut vm = new_vm_with_ops(&operations).unwrap();

    let current_call_frame = vm.current_call_frame_mut().unwrap();

    assert!(current_call_frame.transient_storage.is_empty());

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let current_call_frame = vm.current_call_frame_mut().unwrap();

    assert_eq!(
        *current_call_frame
            .transient_storage
            .get(&(current_call_frame.msg_sender, key))
            .unwrap(),
        value
    )
}

#[test]
fn transient_store_stack_underflow() {
    let operations = [Operation::Tstore, Operation::Stop];

    let mut vm = new_vm_with_ops(&operations).unwrap();
    assert!(vm
        .current_call_frame_mut()
        .unwrap()
        .transient_storage
        .is_empty());

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    let tx_report = vm.execute(&mut current_call_frame).unwrap();

    assert!(matches!(
        tx_report.result,
        TxResult::Revert(VMError::StackUnderflow)
    ));
}

#[test]
fn transient_load() {
    let value = U256::from_big_endian(&[0xaa; 3]);
    let key = U256::from_big_endian(&[0xff; 2]);

    let operations = [
        Operation::Push((32, key)),
        Operation::Tload,
        Operation::Stop,
    ];

    let mut vm = new_vm_with_ops(&operations).unwrap();

    let caller = vm.current_call_frame_mut().unwrap().msg_sender;

    vm.current_call_frame_mut()
        .unwrap()
        .transient_storage
        .insert((caller, key), value);

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    assert_eq!(
        *vm.current_call_frame_mut()
            .unwrap()
            .stack
            .stack
            .last()
            .unwrap(),
        value
    )
}

#[test]
fn create_happy_path() {
    let value_to_transfer = 10;
    let offset = 19;
    let size = 13;
    let sender_nonce = 0;
    let sender_balance = U256::from(25);
    let sender_addr = Address::from_low_u64_be(40);

    // Code that returns the value 0xffffffff putting it in memory
    let initialization_code = hex::decode("63FFFFFFFF6000526004601CF3").unwrap();

    let operations = [
        vec![
            Operation::Push((13, U256::from_big_endian(&initialization_code))),
            Operation::Push0,
            Operation::Mstore,
        ],
        create_opcodes(size, offset, value_to_transfer),
    ]
    .concat();

    let mut vm = new_vm_with_ops_addr_bal_db(
        ops_to_bytecode(&operations).unwrap(),
        sender_addr,
        sender_balance,
        Db::new(),
        CacheDB::default(),
    )
    .unwrap();
    vm.current_call_frame_mut().unwrap().msg_sender = sender_addr;

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let call_frame = vm.current_call_frame_mut().unwrap();
    let returned_address = call_frame.stack.pop().unwrap();

    let expected_address = VM::calculate_create_address(sender_addr, sender_nonce + 1).unwrap();
    assert_eq!(word_to_address(returned_address), expected_address);

    // check the created account is correct
    let new_account = cache::get_account(&vm.cache, &word_to_address(returned_address)).unwrap();
    assert_eq!(new_account.info.balance, U256::from(value_to_transfer));
    assert_eq!(new_account.info.nonce, 0); // This was previously set to 1 but I understand that a new account should have nonce 0

    // Check that the sender account is updated
    let sender_account = cache::get_account(&vm.cache, &sender_addr).unwrap();
    assert_eq!(sender_account.info.nonce, sender_nonce + 1);
    assert_eq!(
        sender_account.info.balance,
        sender_balance - value_to_transfer
    );
}

#[test]
fn cant_create_with_size_longer_than_max_code_size() {
    let value_to_transfer = 10;
    let offset = 19;
    let size = MAX_CODE_SIZE * 2 + 1;
    let sender_nonce = 0;
    let sender_balance = U256::from(25);
    let sender_addr = Address::from_low_u64_be(40);

    let operations = create_opcodes(size, offset, value_to_transfer);

    let mut vm = new_vm_with_ops_addr_bal_db(
        ops_to_bytecode(&operations).unwrap(),
        sender_addr,
        sender_balance,
        Db::new(),
        CacheDB::default(),
    )
    .unwrap();
    vm.current_call_frame_mut().unwrap().msg_sender = sender_addr;

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let call_frame = vm.current_call_frame_mut().unwrap();
    let create_return_value = call_frame.stack.pop().unwrap();
    assert_eq!(create_return_value, U256::from(REVERT_FOR_CREATE));

    // Check that the sender account is updated
    let sender_account = cache::get_account(&vm.cache, &sender_addr).unwrap();
    assert_eq!(sender_account.info.nonce, sender_nonce);
    assert_eq!(sender_account.info.balance, sender_balance);
}

#[test]
fn cant_create_on_static_contexts() {
    let value_to_transfer = 10;
    let offset = 19;
    let size = 10;
    let sender_nonce = 0;
    let sender_balance = U256::from(25);
    let sender_addr = Address::from_low_u64_be(40);

    let operations = create_opcodes(size, offset, value_to_transfer);

    let mut vm = new_vm_with_ops_addr_bal_db(
        ops_to_bytecode(&operations).unwrap(),
        sender_addr,
        sender_balance,
        Db::new(),
        CacheDB::default(),
    )
    .unwrap();
    vm.current_call_frame_mut().unwrap().msg_sender = sender_addr;
    vm.current_call_frame_mut().unwrap().is_static = true;

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let call_frame = vm.current_call_frame_mut().unwrap();
    let create_return_value = call_frame.stack.pop().unwrap();
    assert_eq!(create_return_value, U256::from(REVERT_FOR_CREATE));

    // Check that the sender account is updated
    let sender_account = cache::get_account(&vm.cache, &sender_addr).unwrap();
    assert_eq!(sender_account.info.nonce, sender_nonce);
    assert_eq!(sender_account.info.balance, sender_balance);
}

#[test]
fn cant_create_if_transfer_value_bigger_than_balance() {
    let value_to_transfer = 100;
    let offset = 19;
    let size = 10;
    let sender_nonce = 0;
    let sender_balance = U256::from(25);
    let sender_addr = Address::from_low_u64_be(40);

    let operations = create_opcodes(size, offset, value_to_transfer);

    let mut vm = new_vm_with_ops_addr_bal_db(
        ops_to_bytecode(&operations).unwrap(),
        sender_addr,
        sender_balance,
        Db::new(),
        CacheDB::default(),
    )
    .unwrap();
    vm.current_call_frame_mut().unwrap().msg_sender = sender_addr;

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let call_frame = vm.current_call_frame_mut().unwrap();
    let create_return_value = call_frame.stack.pop().unwrap();
    assert_eq!(create_return_value, U256::from(REVERT_FOR_CREATE));

    // Check that the sender account is updated
    let sender_account = cache::get_account(&vm.cache, &sender_addr).unwrap();
    assert_eq!(sender_account.info.nonce, sender_nonce);
    assert_eq!(sender_account.info.balance, sender_balance);
}

#[test]
fn cant_create_if_sender_nonce_would_overflow() {
    let value_to_transfer = 10;
    let offset = 19;
    let size = 10;
    let sender_nonce = u64::MAX;
    let sender_balance = U256::from(25);
    let sender_addr = Address::from_low_u64_be(40);

    let operations = create_opcodes(size, offset, value_to_transfer);

    let mut db = Db::new();
    db.add_accounts(vec![(
        sender_addr,
        Account::new(sender_balance, Bytes::new(), sender_nonce, HashMap::new()),
    )]);

    let mut vm = new_vm_with_ops_db(&operations, db).unwrap();

    vm.current_call_frame_mut().unwrap().msg_sender = sender_addr;

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let call_frame = vm.current_call_frame_mut().unwrap();
    let create_return_value = call_frame.stack.pop().unwrap();
    assert_eq!(create_return_value, U256::from(REVERT_FOR_CREATE));

    // Check that the sender account is updated
    let sender_account = cache::get_account(&vm.cache, &sender_addr).unwrap();
    assert_eq!(sender_account.info.nonce, sender_nonce);
    assert_eq!(sender_account.info.balance, sender_balance);
}

// #[test]
// fn cant_create_accounts_with_same_address() {
//     let value_to_transfer = 10;
//     let offset = 19;
//     let size = 13;
//     let sender_nonce = 1;
//     let sender_balance = U256::from(25);
//     let sender_addr = Address::from_low_u64_be(40);

//     // Code that returns the value 0xffffffff putting it in memory
//     let initialization_code = hex::decode("63FFFFFFFF6000526004601CF3").unwrap();

//     let operations = [
//         vec![
//             Operation::Push((13, U256::from_big_endian(&initialization_code))),
//             Operation::Push0,
//             Operation::Mstore,
//         ],
//         create_opcodes(size, offset, value_to_transfer),
//     ]
//     .concat();

//     let mut vm = new_vm_with_ops(&operations).unwrap();
//     vm.db.accounts.insert(
//         sender_addr,
//         Account::default()
//             .with_balance(sender_balance)
//             .with_nonce(sender_nonce),
//     );
//     vm.current_call_frame_mut().unwrap().msg_sender = sender_addr;

//     let mut current_call_frame = vm.call_frames.pop().unwrap();
//     vm.execute(&mut current_call_frame).unwrap();

//     let call_frame = vm.current_call_frame_mut().unwrap();

//     let return_of_created_callframe = call_frame.stack.pop().unwrap();

//     assert_eq!(return_of_created_callframe, U256::from(SUCCESS_FOR_RETURN));

//     let returned_addr = call_frame.stack.pop().unwrap();
//     // check the created account is correct
//     let new_account = vm.db.accounts.get(&word_to_address(returned_addr)).unwrap();
//     assert_eq!(new_account.balance, U256::from(value_to_transfer));
//     assert_eq!(new_account.nonce, 1);

//     // Check that the sender account is updated
//     let sender_account = vm.db.accounts.get_mut(&sender_addr).unwrap();
//     assert_eq!(sender_account.nonce, sender_nonce + 1);
//     assert_eq!(sender_account.balance, sender_balance - value_to_transfer);

//     // after a happy create, we do again a create with same inputs, this should revert as we will create
//     // an account with the same address
//     sender_account.nonce = sender_nonce;
//     let mut new_vm = new_vm_with_ops(&operations).unwrap();
//     new_vm.db = vm.db.clone();
//     new_vm.db.accounts = vm.db.accounts.clone();
//     new_vm.current_call_frame_mut().unwrap().msg_sender = sender_addr;

//     let mut current_call_frame = new_vm.call_frames.pop().unwrap();
//     new_vm.execute(&mut current_call_frame).unwrap();
//     let call_frame = new_vm.current_call_frame_mut().unwrap();
//     let return_of_created_callframe = call_frame.stack.pop().unwrap();
//     assert_eq!(return_of_created_callframe, U256::from(REVERT_FOR_CREATE));
// }

#[test]
fn create2_happy_path() {
    let value: u8 = 10;
    let offset: u8 = 19;
    let size: u8 = 13;
    let salt: u8 = 4;
    let sender_nonce = 0;
    let sender_balance = U256::from(25);
    let sender_addr = Address::from_low_u64_be(40);

    // Code that returns the value 0xffffffff putting it in memory
    let initialization_code = hex::decode("63FFFFFFFF6000526004601CF3").unwrap();
    let expected_address = VM::calculate_create2_address(
        Address::from_low_u64_be(42), // this is the addr initializated in new_vm_with_ops_addr_bal_db
        &Bytes::from(initialization_code.clone()),
        U256::from(salt),
    )
    .unwrap();

    let operations = vec![
        // Store initialization code in memory
        Operation::Push((13, U256::from_big_endian(&initialization_code))),
        Operation::Push0,
        Operation::Mstore,
        // Create
        Operation::Push((1, U256::from(salt))),
        Operation::Push((1, U256::from(size))),
        Operation::Push((1, U256::from(offset))),
        Operation::Push((1, U256::from(value))),
        Operation::Create2,
        Operation::Stop,
    ];

    let mut vm = new_vm_with_ops_addr_bal_db(
        ops_to_bytecode(&operations).unwrap(),
        sender_addr,
        sender_balance,
        Db::new(),
        CacheDB::default(),
    )
    .unwrap();
    vm.current_call_frame_mut().unwrap().msg_sender = sender_addr;

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    let call_frame = vm.current_call_frame_mut().unwrap();
    let returned_address = call_frame.stack.pop().unwrap();
    assert_eq!(word_to_address(returned_address), expected_address);
    // check the created account is correct
    let new_account = cache::get_account(&vm.cache, &word_to_address(returned_address)).unwrap();
    assert_eq!(new_account.info.balance, U256::from(value));
    assert_eq!(new_account.info.nonce, 0); // I understand new account should have nonce 0, not 1.

    // Check that the sender account is updated
    let sender_account = cache::get_account(&vm.cache, &sender_addr).unwrap();
    assert_eq!(sender_account.info.nonce, sender_nonce + 1);
    assert_eq!(sender_account.info.balance, sender_balance - value);
}

// #[test]
// fn create_on_create() {
//     let value_to_transfer = 10;
//     let offset = 19;
//     let size = 13;
//     let sender_balance = U256::from(25);
//     let sender_addr = Address::from_low_u64_be(40);

//     // push0, push0, mstore, push1 0, push1 0, push1 0, create, push0, push0, return
//     let initialization_code = hex::decode("5f5f52600060006000f05f5ff3").unwrap();

//     let operations = [
//         vec![
//             Operation::Push((13, U256::from_big_endian(&initialization_code))),
//             Operation::Push0,
//             Operation::Mstore,
//         ],
//         create_opcodes(size, offset, value_to_transfer),
//     ]
//     .concat();

//     let mut vm = new_vm_with_ops_addr_bal(ops_to_bytecode(&operations).unwrap(), sender_addr, sender_balance);

//     vm.current_call_frame_mut().unwrap().msg_sender = sender_addr;

//     let mut current_call_frame = vm.call_frames.pop().unwrap();
//     vm.execute(&mut current_call_frame).unwrap();
//     assert_eq!(vm.db.accounts.len(), 4);
// }

#[test]
fn caller_op() {
    let caller = Address::from_low_u64_be(0x100);
    let address_that_has_the_code = Address::from_low_u64_be(0x42);

    let operations = [Operation::Caller, Operation::Stop];

    let mut db = Db::default();
    db.add_accounts(vec![(
        address_that_has_the_code,
        Account::default().with_bytecode(ops_to_bytecode(&operations).unwrap()),
    )]);

    let mut cache = CacheDB::default();
    cache::insert_account(
        &mut cache,
        address_that_has_the_code,
        Account::default().with_bytecode(ops_to_bytecode(&operations).unwrap()),
    );

    let env = Environment::default_from_address(caller);

    let mut vm = VM::new(
        TxKind::Call(address_that_has_the_code),
        env,
        Default::default(),
        Default::default(),
        Arc::new(db),
        cache,
        Vec::new(),
    )
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    assert_eq!(
        vm.current_call_frame_mut().unwrap().stack.pop().unwrap(),
        U256::from(caller.as_bytes())
    );
    assert_eq!(current_call_frame.gas_used, gas_cost::CALLER);
}

#[test]
fn origin_op() {
    let address_that_has_the_code = Address::from_low_u64_be(0x42);
    let msg_sender = Address::from_low_u64_be(0x999);

    let operations = [Operation::Origin, Operation::Stop];

    let mut db = Db::default();
    db.add_accounts(vec![(
        address_that_has_the_code,
        Account::default().with_bytecode(ops_to_bytecode(&operations).unwrap()),
    )]);

    let mut cache = CacheDB::default();
    cache::insert_account(
        &mut cache,
        msg_sender,
        Account::default().with_bytecode(ops_to_bytecode(&operations).unwrap()),
    );

    let env = Environment::default_from_address(msg_sender);

    let mut vm = VM::new(
        TxKind::Call(address_that_has_the_code),
        env,
        Default::default(),
        Default::default(),
        Arc::new(db),
        cache,
        Vec::new(),
    )
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    assert_eq!(
        vm.current_call_frame_mut().unwrap().stack.pop().unwrap(),
        U256::from(msg_sender.as_bytes())
    );
    assert_eq!(current_call_frame.gas_used, gas_cost::ORIGIN);
}

#[test]
fn balance_op() {
    let address = 0x999;

    let operations = [
        Operation::Push((32, U256::from(address))),
        Operation::Balance,
        Operation::Stop,
    ];

    let mut vm = new_vm_with_ops_addr_bal_db(
        ops_to_bytecode(&operations).unwrap(),
        Address::from_low_u64_be(address),
        U256::from(1234),
        Db::new(),
        CacheDB::default(),
    )
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    assert_eq!(
        vm.current_call_frame_mut().unwrap().stack.pop().unwrap(),
        U256::from(1234)
    )
}

#[test]
fn address_op() {
    let address_that_has_the_code = Address::from_low_u64_be(0x42);

    let operations = [Operation::Address, Operation::Stop];

    let mut db = Db::default();
    db.add_accounts(vec![(
        address_that_has_the_code,
        Account::default().with_bytecode(ops_to_bytecode(&operations).unwrap()),
    )]);

    let mut cache = CacheDB::default();
    cache::insert_account(
        &mut cache,
        address_that_has_the_code,
        Account::default().with_bytecode(ops_to_bytecode(&operations).unwrap()),
    );

    let env = Environment::default_from_address(Address::from_low_u64_be(42));

    let mut vm = VM::new(
        TxKind::Call(address_that_has_the_code),
        env,
        Default::default(),
        Default::default(),
        Arc::new(db),
        cache,
        Vec::new(),
    )
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    assert_eq!(
        vm.current_call_frame_mut().unwrap().stack.pop().unwrap(),
        U256::from(address_that_has_the_code.as_bytes())
    );
    assert_eq!(current_call_frame.gas_used, gas_cost::ADDRESS);
}

#[test]
fn selfbalance_op() {
    let address_that_has_the_code = Address::from_low_u64_be(0x42);
    let balance = U256::from(999);

    let operations = [Operation::SelfBalance, Operation::Stop];

    let mut db = Db::default();
    db.add_accounts(vec![(
        address_that_has_the_code,
        Account::default()
            .with_bytecode(ops_to_bytecode(&operations).unwrap())
            .with_balance(balance),
    )]);

    let mut cache = CacheDB::default();
    cache::insert_account(
        &mut cache,
        address_that_has_the_code,
        Account::default()
            .with_bytecode(ops_to_bytecode(&operations).unwrap())
            .with_balance(balance),
    );

    let env = Environment::default_from_address(Address::from_low_u64_be(42));

    let mut vm = VM::new(
        TxKind::Call(address_that_has_the_code),
        env,
        Default::default(),
        Default::default(),
        Arc::new(db),
        cache,
        Vec::new(),
    )
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    assert_eq!(
        vm.current_call_frame_mut().unwrap().stack.pop().unwrap(),
        balance
    );
    assert_eq!(current_call_frame.gas_used, gas_cost::SELFBALANCE);
}

#[test]
fn callvalue_op() {
    let address_that_has_the_code = Address::from_low_u64_be(0x42);
    let value = U256::from(0x1234);

    let operations = [Operation::Callvalue, Operation::Stop];

    let mut db = Db::default();

    db.add_accounts(vec![(
        address_that_has_the_code,
        Account::default().with_bytecode(ops_to_bytecode(&operations).unwrap()),
    )]);

    let mut cache = CacheDB::default();
    cache::insert_account(
        &mut cache,
        address_that_has_the_code,
        Account::default().with_bytecode(ops_to_bytecode(&operations).unwrap()),
    );

    let env = Environment::default_from_address(Address::from_low_u64_be(42));

    let mut vm = VM::new(
        TxKind::Call(address_that_has_the_code),
        env,
        value,
        Default::default(),
        Arc::new(db),
        cache,
        Vec::new(),
    )
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    assert_eq!(
        vm.current_call_frame_mut().unwrap().stack.pop().unwrap(),
        value
    );
    assert_eq!(current_call_frame.gas_used, gas_cost::CALLVALUE);
}

#[test]
fn codesize_op() {
    let address_that_has_the_code = Address::from_low_u64_be(0x42);

    let operations = [Operation::Codesize, Operation::Stop];

    let mut db = Db::default();

    db.add_accounts(vec![(
        address_that_has_the_code,
        Account::default().with_bytecode(ops_to_bytecode(&operations).unwrap()),
    )]);

    let mut cache = CacheDB::default();
    cache::insert_account(
        &mut cache,
        address_that_has_the_code,
        Account::default().with_bytecode(ops_to_bytecode(&operations).unwrap()),
    );

    let env = Environment::default_from_address(Address::from_low_u64_be(42));

    let mut vm = VM::new(
        TxKind::Call(address_that_has_the_code),
        env,
        Default::default(),
        Default::default(),
        Arc::new(db),
        cache,
        Vec::new(),
    )
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();
    assert_eq!(
        vm.current_call_frame_mut().unwrap().stack.pop().unwrap(),
        U256::from(2)
    );
    assert_eq!(current_call_frame.gas_used, gas_cost::CODESIZE);
}

#[test]
fn gasprice_op() {
    let address_that_has_the_code = Address::from_low_u64_be(0x42);
    let operations = [Operation::Gasprice, Operation::Stop];

    let mut db = Db::default();

    db.add_accounts(vec![(
        address_that_has_the_code,
        Account::default().with_bytecode(ops_to_bytecode(&operations).unwrap()),
    )]);

    let mut cache = CacheDB::default();
    cache::insert_account(
        &mut cache,
        address_that_has_the_code,
        Account::default().with_bytecode(ops_to_bytecode(&operations).unwrap()),
    );

    let mut env = Environment::default_from_address(Address::from_low_u64_be(42));
    env.gas_price = U256::from_str_radix("9876", 16).unwrap();

    let mut vm = VM::new(
        TxKind::Call(address_that_has_the_code),
        env,
        Default::default(),
        Default::default(),
        Arc::new(db),
        cache,
        Vec::new(),
    )
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();
    assert_eq!(
        vm.current_call_frame_mut().unwrap().stack.pop().unwrap(),
        U256::from(0x9876)
    );
    assert_eq!(current_call_frame.gas_used, gas_cost::GASPRICE);
}

#[test]
fn codecopy_op() {
    // Copies two bytes of the code, with offset 2, and loads them beginning at offset 3 in memory.
    let address_that_has_the_code = Address::from_low_u64_be(0x42);
    // https://www.evm.codes/playground?fork=cancun&unit=Wei&codeType=Mnemonic&code=%27~2z~2z~3zCODECOPY%27~PUSH1%200x0z%5Cn%01z~_
    let operations = [
        Operation::Push((1, 0x02.into())), // size
        Operation::Push((1, 0x02.into())), // offset
        Operation::Push((1, 0x03.into())), // destination offset
        Operation::Codecopy,
        Operation::Stop,
    ];

    let expected_memory_bytes = [
        [0x00; 3].to_vec(),
        [[0x60], [0x02]].concat(),
        [0x00; 27].to_vec(),
    ]
    .concat();

    let expected_memory = U256::from_big_endian(&expected_memory_bytes);

    let mut db = Db::default();

    db.add_accounts(vec![(
        address_that_has_the_code,
        Account::default().with_bytecode(ops_to_bytecode(&operations).unwrap()),
    )]);

    let mut cache = CacheDB::default();
    cache::insert_account(
        &mut cache,
        address_that_has_the_code,
        Account::default().with_bytecode(ops_to_bytecode(&operations).unwrap()),
    );

    let env = Environment::default_from_address(Address::from_low_u64_be(42));

    let mut vm = VM::new(
        TxKind::Call(address_that_has_the_code),
        env,
        Default::default(),
        Default::default(),
        Arc::new(db),
        cache,
        Vec::new(),
    )
    .unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    assert_eq!(
        memory::load_word(
            &mut vm.current_call_frame_mut().unwrap().memory,
            U256::zero()
        )
        .unwrap(),
        expected_memory
    );
    assert_eq!(
        current_call_frame.gas_used,
        U256::from(9) + U256::from(3) * gas_cost::PUSHN
    );
}

#[test]
fn extcodesize_existing_account() {
    let address_with_code = Address::from_low_u64_be(0x42);
    let operations = [
        Operation::Push((20, address_with_code.as_bytes().into())),
        Operation::ExtcodeSize,
        Operation::Stop,
    ];

    let mut db = Db::default();
    db.add_accounts(vec![(
        address_with_code,
        Account::default().with_bytecode(ops_to_bytecode(&operations).unwrap()),
    )]);

    let mut vm = new_vm_with_ops_db(&operations, db).unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();
    assert_eq!(
        vm.current_call_frame_mut().unwrap().stack.pop().unwrap(),
        23.into()
    );
    assert_eq!(current_call_frame.gas_used, 2603.into());
}

#[test]
fn extcodesize_non_existing_account() {
    // EVM Playground: https://www.evm.codes/playground?fork=cancun&unit=Wei&codeType=Mnemonic&code='PUSH20%200x42%5CnEXTCODESIZE%5CnSTOP'_
    let operations = [
        Operation::Push((20, "0x42".into())),
        Operation::ExtcodeSize,
        Operation::Stop,
    ];

    let mut vm = new_vm_with_ops(&operations).unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();
    assert_eq!(
        vm.current_call_frame_mut().unwrap().stack.pop().unwrap(),
        0.into()
    );
    assert_eq!(current_call_frame.gas_used, 2603.into());
}

#[test]
fn extcodecopy_existing_account() {
    let address_with_code = Address::from_low_u64_be(0x42);
    let size: usize = 1;

    let operations = [
        Operation::Push((1, size.into())),
        Operation::Push0, // offset
        Operation::Push0, // destOffset
        Operation::Push((20, address_with_code.as_bytes().into())),
        Operation::ExtcodeCopy,
        Operation::Stop,
    ];

    let mut db = Db::new();
    db.add_accounts(vec![(
        address_with_code,
        Account::default().with_bytecode(ops_to_bytecode(&operations).unwrap()),
    )]);

    let mut vm = new_vm_with_ops_db(&operations, db).unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();
    assert_eq!(
        memory::load_range(
            &mut vm.current_call_frame_mut().unwrap().memory,
            U256::zero(),
            size
        )
        .unwrap(),
        vec![0x60]
    );
    assert_eq!(current_call_frame.gas_used, 2616.into());
}

#[test]
fn extcodecopy_non_existing_account() {
    // EVM Playground: https://www.evm.codes/playground?fork=cancun&unit=Wei&codeType=Mnemonic&code='y1%201~~~20%200x42zEXTCODECOPYzSTOP'~0zyz%5CnyPUSH%01yz~_
    let size: usize = 10;

    let operations = [
        Operation::Push((1, size.into())),
        Operation::Push0, // offset
        Operation::Push0, // destOffset
        Operation::Push((20, "0x42".into())),
        Operation::ExtcodeCopy,
        Operation::Stop,
    ];

    let mut vm = new_vm_with_ops(&operations).unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();
    assert_eq!(
        memory::load_range(
            &mut vm.current_call_frame_mut().unwrap().memory,
            U256::zero(),
            size
        )
        .unwrap(),
        vec![0; size]
    );
    assert_eq!(current_call_frame.gas_used, 2616.into());
}

#[test]
fn extcodehash_account_with_empty_code() {
    let address_with_code = Address::from_low_u64_be(0x42);
    let operations = [
        Operation::Push((20, address_with_code.as_bytes().into())),
        Operation::ExtcodeHash,
        Operation::Stop,
    ];

    let mut db = Db::default();
    db.add_accounts(vec![(address_with_code, Account::default())]);

    let mut vm = new_vm_with_ops_db(&operations, db).unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();
    assert_eq!(
        vm.current_call_frame_mut().unwrap().stack.pop().unwrap(),
        "c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470".into()
    );
    assert_eq!(current_call_frame.gas_used, 2603.into());
}

#[test]
fn extcodehash_non_existing_account() {
    // EVM Playground: https://www.evm.codes/playground?fork=cancun&unit=Wei&codeType=Mnemonic&code='PUSH20%200x42%5CnEXTCODEHASH%5CnSTOP'_
    let operations = [
        Operation::Push((20, "0x42".into())),
        Operation::ExtcodeHash,
        Operation::Stop,
    ];

    let mut vm = new_vm_with_ops(&operations).unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();
    assert_eq!(
        vm.current_call_frame_mut().unwrap().stack.pop().unwrap(),
        "c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470".into()
    );
    assert_eq!(current_call_frame.gas_used, 2603.into());
}

#[test]
fn invalid_opcode() {
    let operations = [Operation::Invalid, Operation::Stop];

    let mut vm = new_vm_with_ops(&operations).unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    let tx_report = vm.execute(&mut current_call_frame).unwrap();

    assert!(matches!(
        tx_report.result,
        TxResult::Revert(VMError::InvalidOpcode)
    ));
}

// Revert Opcode has correct output and result
#[test]
fn revert_opcode() {
    let ops = vec![
        Operation::Push((32, U256::from(0xA))),  // value
        Operation::Push((32, U256::from(0xFF))), // offset
        Operation::Mstore,
        Operation::Push((32, U256::from(32))),   // size
        Operation::Push((32, U256::from(0xFF))), // offset
        Operation::Revert,
    ];

    let mut vm = new_vm_with_ops(&ops).unwrap();

    let mut current_call_frame = vm.call_frames.pop().unwrap();
    let tx_report = vm.execute(&mut current_call_frame).unwrap();

    assert_eq!(U256::from_big_endian(&tx_report.output), U256::from(0xA));
    assert!(matches!(
        tx_report.result,
        TxResult::Revert(VMError::RevertOpcode)
    ));
}

// Store something in the database, then revert. Database should be like it was before the store.
#[test]
fn revert_sstore() {
    let key = U256::from(80);
    let value = U256::from(100);
    let sender_address = Address::from_low_u64_be(3000);
    let operations = vec![
        Operation::Push((1, value)),
        Operation::Push((1, key)),
        Operation::Sstore,
        Operation::Revert,
    ];

    let mut vm = new_vm_with_ops(&operations).unwrap();
    vm.current_call_frame_mut().unwrap().code_address = sender_address;
    cache::insert_account(&mut vm.cache, sender_address, Account::default());

    let mut current_call_frame = vm.call_frames.pop().unwrap();

    // Cache state before the SSTORE
    let cache_backup = vm.cache.clone();

    vm.execute(&mut current_call_frame).unwrap();

    assert_eq!(vm.cache, cache_backup);
}
