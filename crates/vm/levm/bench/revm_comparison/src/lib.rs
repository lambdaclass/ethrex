use bytes::Bytes;
use ethrex_levm::{errors::TxResult, testing::new_vm_with_bytecode};
use revm::{
    db::BenchmarkDB,
    primitives::{address, Address, Bytecode, TransactTo},
    Evm,
};
use sha3::{Digest, Keccak256};
use std::fs::File;
use std::hint::black_box;
use std::io::Read;

pub fn run_with_levm(program: &str, runs: usize, calldata: &str) {
    let bytecode = Bytes::from(hex::decode(program).unwrap());
    let calldata = Bytes::from(hex::decode(calldata).unwrap());

    for _ in 0..runs - 1 {
        let mut vm = new_vm_with_bytecode(bytecode.clone()).unwrap();
        vm.call_frames.last_mut().unwrap().calldata = calldata.clone();
        vm.env.gas_limit = 100_000_000;
        vm.env.block_gas_limit = 100_000_001;
        let tx_report = black_box(vm.execute().unwrap());
        assert!(tx_report.result == TxResult::Success);
    }
    let mut vm = new_vm_with_bytecode(bytecode.clone()).unwrap();
    vm.call_frames.last_mut().unwrap().calldata = calldata.clone();
    vm.env.gas_limit = 100_000_000;
    vm.env.block_gas_limit = 100_000_001;
    let tx_report = black_box(vm.execute().unwrap());
    assert!(tx_report.result == TxResult::Success);

    match tx_report.result {
        TxResult::Success => {
            println!("output: \t\t0x{}", hex::encode(tx_report.output));
        }
        TxResult::Revert(error) => panic!("Execution failed: {:?}", error),
    }
}

pub fn run_with_revm(program: &str, runs: usize, calldata: &str) {
    let rich_acc_address = address!("1000000000000000000000000000000000000000");
    let bytes = hex::decode(program).unwrap();
    let raw = Bytecode::new_raw(bytes.clone().into());

    let mut evm = Evm::builder()
        .modify_tx_env(|tx| {
            tx.caller = rich_acc_address;
            tx.transact_to = TransactTo::Call(Address::ZERO);
            tx.data = hex::decode(calldata).unwrap().into();
        })
        .with_db(BenchmarkDB::new_bytecode(raw))
        .build();

    let result = evm.transact().unwrap();
    assert!(result.result.is_success());

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
    let mut encoded_n = [0u8; 32];
    encoded_n[24..].copy_from_slice(&n.to_be_bytes());

    // Combine the function selector and the encoded argument
    let calldata: Vec<u8> = function_selector
        .iter()
        .chain(encoded_n.iter())
        .copied()
        .collect();

    hex::encode(calldata)
}

pub fn load_contract_bytecode(bench_name: &str) -> String {
    let path = format!(
        "bench/revm_comparison/contracts/bin/{}.bin-runtime",
        bench_name
    );
    load_file_bytecode(&path)
}

fn load_file_bytecode(path: &str) -> String {
    println!("Current directory: {:?}", std::env::current_dir().unwrap());
    println!("Loading bytecode from file {}", path);
    let mut file = File::open(path).unwrap();
    let mut contents = String::new();
    file.read_to_string(&mut contents).unwrap();
    contents
}
