use revm::{
    Evm,
    db::BenchmarkDB,
    primitives::{Address, Bytecode, TransactTo, address},
};
use std::hint::black_box;

pub fn run_with_revm(contract_code: &str, runs: u64, calldata: &str) {
    let rich_acc_address = address!("1000000000000000000000000000000000000000");
    let bytes = hex::decode(contract_code).unwrap();
    let raw_bytecode = Bytecode::new_raw(bytes.clone().into());

    let mut evm = Evm::builder()
        .modify_tx_env(|tx| {
            tx.caller = rich_acc_address;
            tx.transact_to = TransactTo::Call(Address::ZERO);
            tx.data = hex::decode(calldata).unwrap().into();
        })
        .with_db(BenchmarkDB::new_bytecode(raw_bytecode))
        .build();

    for _ in 0..runs - 1 {
        let result = black_box(evm.transact()).unwrap();
        assert!(result.result.is_success(), "{:?}", result.result);
    }
    let result = black_box(evm.transact()).unwrap();
    assert!(result.result.is_success(), "{:?}", result.result);

    println!("output: \t\t{}", result.result.into_output().unwrap());
}
