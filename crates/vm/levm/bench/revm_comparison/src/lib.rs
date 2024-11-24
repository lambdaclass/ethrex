use bytes::Bytes;
use ethrex_levm::{call_frame::CallFrame, errors::TxResult, utils::new_vm_with_bytecode};
use revm::{
    db::BenchmarkDB,
    primitives::{address, Bytecode, TransactTo},
    Evm,
};
use std::hint::black_box;

pub const FIBONACCI_BYTECODE: &str =
    "5f355f60015b8215601a578181019150909160019003916005565b9150505f5260205ff3";
pub const FACTORIAL_BYTECODE: &str =
    "5f355f60015b8215601b57906001018091029160019003916005565b9150505f5260205ff3";

pub fn run_with_levm(program: &str, runs: usize, number_of_iterations: u32) {
    let bytecode = Bytes::from(hex::decode(program).unwrap());
    let mut call_frame = CallFrame::new_from_bytecode(bytecode);
    let mut calldata = vec![0x00; 32];
    calldata[28..32].copy_from_slice(&number_of_iterations.to_be_bytes());
    call_frame.calldata = Bytes::from(calldata);

    for _ in 0..runs - 1 {
        let mut vm = new_vm_with_bytecode(Bytes::new()).unwrap();
        *vm.current_call_frame_mut().unwrap() = call_frame.clone();
        let mut current_call_frame = vm.call_frames.pop().unwrap();
        let tx_report = black_box(vm.execute(&mut current_call_frame));
        assert!(tx_report.unwrap().result == TxResult::Success);
    }
    let mut vm = new_vm_with_bytecode(Bytes::new()).unwrap();
    *vm.current_call_frame_mut().unwrap() = call_frame.clone();
    let mut current_call_frame = vm.call_frames.pop().unwrap();
    let tx_report = black_box(vm.execute(&mut current_call_frame));
    assert!(tx_report.clone().unwrap().result == TxResult::Success);

    match tx_report.unwrap().result {
        TxResult::Success => {
            println!("\t\t0x{}", hex::encode(current_call_frame.returndata));
        }
        TxResult::Revert(error) => panic!("Execution failed: {:?}", error),
    }
}

pub fn run_with_revm(program: &str, runs: usize, number_of_iterations: u32) {
    let bytes = hex::decode(program).unwrap();
    let raw = Bytecode::new_raw(bytes.into());
    let mut calldata = [0; 32];
    calldata[28..32].copy_from_slice(&number_of_iterations.to_be_bytes());
    let mut evm = Evm::builder()
        .with_db(BenchmarkDB::new_bytecode(raw))
        .modify_tx_env(|tx| {
            tx.caller = address!("1000000000000000000000000000000000000000");
            tx.transact_to = TransactTo::Call(address!("0000000000000000000000000000000000000000"));
            tx.data = calldata.into();
        })
        .build();

    for _ in 0..runs - 1 {
        let result = black_box(evm.transact()).unwrap();
        assert!(result.result.is_success());
    }
    let result = black_box(evm.transact()).unwrap();
    assert!(result.result.is_success());

    println!("\t\t{}", result.result.into_output().unwrap());
}
