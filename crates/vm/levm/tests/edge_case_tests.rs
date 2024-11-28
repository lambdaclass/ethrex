use std::str::FromStr;

use bytes::Bytes;
use ethrex_core::U256;
use ethrex_levm::{
    errors::{TxResult, VMError},
    operations::Operation,
    utils::{new_vm_with_bytecode, new_vm_with_ops},
};

#[test]
fn test_extcodecopy_memory_allocation() {
    let mut vm = new_vm_with_bytecode(Bytes::copy_from_slice(&[
        95, 100, 68, 68, 102, 68, 68, 95, 95, 60,
    ]))
    .unwrap();
    let mut current_call_frame = vm.call_frames.pop().unwrap();
    current_call_frame.gas_limit = U256::from(100_000_000);
    vm.env.gas_price = U256::from(10_000);
    vm.execute(&mut current_call_frame).unwrap();
}

#[test]
fn test_overflow_mcopy() {
    let mut vm = new_vm_with_bytecode(Bytes::copy_from_slice(&[90, 90, 90, 94])).unwrap();
    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();
}

#[test]
fn test_overflow_call() {
    let mut vm =
        new_vm_with_bytecode(Bytes::copy_from_slice(&[61, 48, 56, 54, 51, 51, 51, 241])).unwrap();
    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();
}

#[test]
fn test_usize_overflow_revert() {
    let mut vm = new_vm_with_bytecode(Bytes::copy_from_slice(&[61, 63, 61, 253])).unwrap();
    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();
}

#[test]
fn test_overflow_returndatacopy() {
    let mut vm = new_vm_with_bytecode(Bytes::copy_from_slice(&[50, 49, 48, 51, 62])).unwrap();
    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();
}

#[test]
fn test_overflow_keccak256() {
    let mut vm = new_vm_with_bytecode(Bytes::copy_from_slice(&[51, 63, 61, 32])).unwrap();
    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();
}

#[test]
fn test_arithmetic_operation_overflow_selfdestruct() {
    let mut vm = new_vm_with_bytecode(Bytes::copy_from_slice(&[50, 255])).unwrap();
    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();
}

#[test]
fn test_overflow_swap() {
    let mut vm = new_vm_with_bytecode(Bytes::copy_from_slice(&[48, 144])).unwrap();
    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();
}

#[test]
fn test_end_of_range_swap() {
    let mut vm = new_vm_with_bytecode(Bytes::copy_from_slice(&[58, 50, 50, 51, 57])).unwrap();
    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();
}

#[test]
fn test_usize_overflow_blobhash() {
    let mut vm = new_vm_with_bytecode(Bytes::copy_from_slice(&[71, 73])).unwrap();
    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();
}

#[test]
fn add_op() {
    let mut vm = new_vm_with_ops(&[
        Operation::Push((32, U256::MAX)),
        Operation::Jump,
        Operation::Stop,
    ])
    .unwrap();
    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();

    assert_eq!(vm.current_call_frame_mut().unwrap().pc(), 34);
}

#[test]
fn test_is_negative() {
    let mut vm = new_vm_with_bytecode(Bytes::copy_from_slice(&[58, 63, 58, 5])).unwrap();
    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();
}

#[test]
fn test_non_compliance_keccak256() {
    let mut vm = new_vm_with_bytecode(Bytes::copy_from_slice(&[88, 88, 32, 89])).unwrap();
    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();
    assert_eq!(
        *current_call_frame.stack.stack.first().unwrap(),
        U256::from_str("0xc5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470")
            .unwrap()
    );
    assert_eq!(
        *current_call_frame.stack.stack.get(1).unwrap(),
        U256::zero()
    );
}

#[test]
fn test_sdiv_zero_dividend_and_negative_divisor() {
    let mut vm = new_vm_with_bytecode(Bytes::copy_from_slice(&[
        0x7F, 0xC5, 0xD2, 0x46, 0x01, 0x86, 0xF7, 0x23, 0x3C, 0x92, 0x7E, 0x7D, 0xB2, 0xDC, 0xC7,
        0x03, 0xC0, 0xE5, 0x00, 0xB6, 0x53, 0xCA, 0x82, 0x27, 0x3B, 0x7B, 0xFA, 0xD8, 0x04, 0x5D,
        0x85, 0xA4, 0x70, 0x5F, 0x05,
    ]))
    .unwrap();
    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();
    assert_eq!(current_call_frame.stack.pop().unwrap(), U256::zero());
}

#[test]
fn test_non_compliance_returndatacopy() {
    let mut vm =
        new_vm_with_bytecode(Bytes::copy_from_slice(&[56, 56, 56, 56, 56, 56, 62, 56])).unwrap();
    let mut current_call_frame = vm.call_frames.pop().unwrap();
    let txreport = vm.execute(&mut current_call_frame).unwrap();
    assert_eq!(txreport.result, TxResult::Revert(VMError::VeryLargeNumber));
}

#[test]
fn test_non_compliance_extcodecopy() {
    let mut vm = new_vm_with_bytecode(Bytes::copy_from_slice(&[88, 88, 88, 89, 60, 89])).unwrap();
    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();
    assert_eq!(current_call_frame.stack.stack.pop().unwrap(), U256::zero());
}

#[test]
fn test_non_compliance_extcodecopy_memory_resize() {
    let mut vm = new_vm_with_bytecode(Bytes::copy_from_slice(&[
        0x60, 12, 0x5f, 0x5f, 0x5f, 0x3c, 89,
    ]))
    .unwrap();
    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();
    assert_eq!(current_call_frame.stack.pop().unwrap(), U256::from(32));
}

#[test]
fn test_non_compliance_calldatacopy_memory_resize() {
    let mut vm =
        new_vm_with_bytecode(Bytes::copy_from_slice(&[0x60, 34, 0x5f, 0x5f, 55, 89])).unwrap();
    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();
    assert_eq!(
        *current_call_frame.stack.stack.first().unwrap(),
        U256::from(64)
    );
}

#[test]
fn test_non_compliance_addmod() {
    let mut vm = new_vm_with_bytecode(Bytes::copy_from_slice(&[
        0x60, 0x01, 0x60, 5, 0x7f, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 8,
    ]))
    .unwrap();
    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();
    assert_eq!(
        current_call_frame.stack.stack.first().unwrap(),
        &U256::zero()
    );
}

#[test]
fn test_non_compliance_addmod2() {
    let mut vm = new_vm_with_bytecode(Bytes::copy_from_slice(&[
        // PUSH20 divisor
        0x73, 0x12, 0x34, 0x56, 0x78, 0x90, 0x12, 0x34, 0x56, 0x78, 0x90, 0x12, 0x34, 0x56, 0x78,
        0x90, 0x12, 0x34, 0x56, 0x78, 0x90, // PUSH1 addend
        0x60, 0x08, // PUSH32 augend
        0x7F, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        0xff, 0xff, 0xfd, // ADDMOD opcode
        0x08, // STOP opcode
        0x00,
    ]))
    .unwrap();
    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();
    assert_eq!(
        current_call_frame.stack.stack.first().unwrap(),
        &U256::from("0xfc7490ee00fc74a0ee00fc7490ee00fc7490ee5")
    );
}

#[test]
fn test_non_compliance_codecopy() {
    let mut vm = new_vm_with_bytecode(Bytes::copy_from_slice(&[
        0x5f, 0x60, 5, 0x60, 5, 0x39, 0x59,
    ]))
    .unwrap();
    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();
    assert_eq!(
        current_call_frame.stack.stack.first().unwrap(),
        &U256::zero()
    );
}

#[test]
fn test_non_compliance_smod() {
    let mut vm =
        new_vm_with_bytecode(Bytes::copy_from_slice(&[0x60, 1, 0x60, 1, 0x19, 0x07])).unwrap();
    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();
    assert_eq!(
        current_call_frame.stack.stack.first().unwrap(),
        &U256::zero()
    );
}

#[test]
fn test_non_compliance_extcodecopy_size_and_destoffset() {
    let mut vm = new_vm_with_bytecode(Bytes::copy_from_slice(&[
        0x60, 17, 0x60, 17, 0x60, 17, 0x60, 17, 0x3c, 0x59,
    ]))
    .unwrap();
    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();
    assert_eq!(
        current_call_frame.stack.stack.first().unwrap(),
        &U256::from(64)
    );
}

#[test]
fn test_non_compliance_log() {
    let mut vm = new_vm_with_bytecode(Bytes::copy_from_slice(&[95, 97, 89, 0, 160, 89])).unwrap();
    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame).unwrap();
    assert_eq!(
        current_call_frame.stack.stack.first().unwrap(),
        &U256::zero()
    );
}
