use bytes::Bytes;
use ethereum_rust_core::U256;
use ethereum_rust_levm::utils::new_vm_with_bytecode;

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
