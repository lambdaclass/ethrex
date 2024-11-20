use bytes::Bytes;
use ethrex_core::U256;
use ethrex_levm::{
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
    vm.execute(&mut current_call_frame);
}

#[test]
fn test_overflow_mcopy() {
    let mut vm = new_vm_with_bytecode(Bytes::copy_from_slice(&[90, 90, 90, 94])).unwrap();
    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame);
}

#[test]
fn test_overflow_call() {
    let mut vm =
        new_vm_with_bytecode(Bytes::copy_from_slice(&[61, 48, 56, 54, 51, 51, 51, 241])).unwrap();
    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame);
}

#[test]
fn test_usize_overflow_revert() {
    let mut vm = new_vm_with_bytecode(Bytes::copy_from_slice(&[61, 63, 61, 253])).unwrap();
    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame);
}

#[test]
fn test_overflow_returndatacopy() {
    let mut vm = new_vm_with_bytecode(Bytes::copy_from_slice(&[50, 49, 48, 51, 62])).unwrap();
    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame);
}

#[test]
fn test_overflow_keccak256() {
    let mut vm = new_vm_with_bytecode(Bytes::copy_from_slice(&[51, 63, 61, 32])).unwrap();
    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame);
}

#[test]
fn test_arithmetic_operation_overflow_selfdestruct() {
    let mut vm = new_vm_with_bytecode(Bytes::copy_from_slice(&[50, 255])).unwrap();
    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame);
}

#[test]
fn test_overflow_swap() {
    let mut vm = new_vm_with_bytecode(Bytes::copy_from_slice(&[48, 144])).unwrap();
    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame);
}

#[test]
fn test_end_of_range_swap() {
    let mut vm = new_vm_with_bytecode(Bytes::copy_from_slice(&[58, 50, 50, 51, 57])).unwrap();
    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame);
}

#[test]
fn test_usize_overflow_blobhash() {
    let mut vm = new_vm_with_bytecode(Bytes::copy_from_slice(&[71, 73])).unwrap();
    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame);
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
    vm.execute(&mut current_call_frame);

    assert_eq!(vm.current_call_frame_mut().unwrap().pc(), 34);
}

#[test]
fn test_is_negative() {
    let mut vm = new_vm_with_bytecode(Bytes::copy_from_slice(&[58, 63, 58, 5])).unwrap();
    let mut current_call_frame = vm.call_frames.pop().unwrap();
    vm.execute(&mut current_call_frame);
}
