use bytes::Bytes;
use ethrex_levm::{call_frame::CallFrame, errors::TxResult, utils::new_vm_with_bytecode};
use revm::{
    db::BenchmarkDB,
    primitives::{address, Bytecode, TransactTo},
    Evm,
};
use sha3::{Digest, Keccak256};
use std::fs::File;
use std::hint::black_box;
use std::io::Read;

pub fn run_with_levm(program: &str, runs: usize, calldata: &str) {
    println!("calldata:\t\t0x{}", calldata);
    let bytecode = Bytes::from(hex::decode(program).unwrap());
    let mut call_frame = CallFrame::new_from_bytecode(bytecode);
    call_frame.calldata = Bytes::from(hex::decode(calldata).unwrap());

    for _ in 0..runs - 1 {
        let mut vm = new_vm_with_bytecode(Bytes::new()).unwrap();
        *vm.current_call_frame_mut().unwrap() = call_frame.clone();
        let mut current_call_frame = vm.call_frames.pop().unwrap();
        let tx_report = black_box(vm.execute(&mut current_call_frame).unwrap());
        assert!(tx_report.result == TxResult::Success);
    }
    let mut vm = new_vm_with_bytecode(Bytes::new()).unwrap();
    *vm.current_call_frame_mut().unwrap() = call_frame.clone();
    let mut current_call_frame = vm.call_frames.pop().unwrap();
    let tx_report = black_box(vm.execute(&mut current_call_frame).unwrap());
    assert!(tx_report.result == TxResult::Success);

    match tx_report.result {
        TxResult::Success => {
            println!("output: \t\t0x{}", hex::encode(tx_report.output));
        }
        TxResult::Revert(error) => panic!("Execution failed: {:?}", error),
    }
}

pub fn run_with_revm(program: &str, runs: usize, calldata: &str) {
    println!("calldata:\t\t0x{}", calldata);
    println!("program:\t\t0x{}", program);
    let bytes = hex::decode(program).unwrap();
    let raw = Bytecode::new_raw(bytes.into());
    let mut evm = Evm::builder()
        .with_db(BenchmarkDB::new_bytecode(raw))
        .modify_tx_env(|tx| {
            tx.caller = address!("1000000000000000000000000000000000000000");
            tx.transact_to = TransactTo::Call(address!("0000000000000000000000000000000000000000"));
            tx.data = hex::decode(calldata).unwrap().into();
        })
        .build();

    for _ in 0..runs - 1 {
        let result = black_box(evm.transact()).unwrap();
        assert!(result.result.is_success());
    }
    let result = black_box(evm.transact()).unwrap();
    assert!(result.result.is_success());

    println!("output: \t\t{}", result.result.into_output().unwrap());
}

pub fn generate_calldata(function: &str, n: u64) -> String {
    let function_signature = format!("{}(uint256)", function);
    let hash = Keccak256::digest(function_signature.as_bytes());
    let function_selector = &hash[..4];

    // Encode argument n (uint256, padded to 32 bytes)
    let mut encoded_n = vec![0u8; 32];
    encoded_n[24..].copy_from_slice(&n.to_be_bytes());

    // Combine the function selector and the encoded argument
    let calldata: Vec<u8> = function_selector
        .iter()
        .chain(encoded_n.iter())
        .copied()
        .collect();

    hex::encode(calldata)
}

pub fn generate_calldata_no_params(function: &str) -> String {
    let function_signature = format!("{}()", function);
    let hash = Keccak256::digest(function_signature.as_bytes());
    let function_selector = &hash[..4];

    hex::encode(function_selector)
}

pub fn load_contract_bytecode(bench_name: &str) -> String {
    let path = format!(
        "bench/revm_comparison/contracts/{}/{}.bin-runtime",
        bench_name, bench_name
    );
    println!("Current directory: {:?}", std::env::current_dir().unwrap());
    println!("Loading bytecode from file {}", path);
    let mut file = File::open(path).unwrap();
    let mut contents = String::new();
    file.read_to_string(&mut contents).unwrap();
    contents
}
