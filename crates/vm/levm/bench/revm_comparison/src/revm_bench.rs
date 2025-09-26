use revm::{
    ExecuteEvm, MainBuilder, MainContext,
    bytecode::Bytecode,
    context::TxEnv,
    database::BenchmarkDB,
    primitives::{Address, address},
};
use std::hint::black_box;

pub fn run_with_revm(contract_code: &str, runs: u64, calldata: &str) {
    let rich_acc_address = address!("1000000000000000000000000000000000000000");
    let bytes = hex::decode(contract_code).unwrap();
    let raw_bytecode = Bytecode::new_raw(bytes.clone().into());
    let mut evm = revm::context::Context::mainnet()
        .with_db(BenchmarkDB::new_bytecode(raw_bytecode))
        .build_mainnet();
    let tx_env = TxEnv::builder()
        .caller(rich_acc_address)
        .to(Address::ZERO)
        .data(hex::decode(calldata).unwrap().into())
        .build()
        .unwrap();

    for _ in 0..runs - 1 {
        let result = black_box(evm.transact(tx_env.clone())).unwrap();
        assert!(result.result.is_success(), "{:?}", result.result);
    }
    let result = black_box(evm.transact(tx_env)).unwrap();
    assert!(result.result.is_success(), "{:?}", result.result);

    println!("output: \t\t{}", result.result.into_output().unwrap());
}
