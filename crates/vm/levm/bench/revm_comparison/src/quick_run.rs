use bytes::Bytes;
use ethrex_common::{
    Address, U256,
    types::{LegacyTransaction, Transaction, TxKind},
};
use ethrex_levm::{Environment, tracing::LevmCallTracer, vm::VM};
use hex::FromHex;
use revm_comparison::levm_bench::{CONTRACT_ADDRESS, SENDER_ADDRESS, init_db};
use std::env;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() != 3 {
        eprintln!("Usage: {} <bytecode_hex> <calldata_hex>", args[0]);
        std::process::exit(1);
    }

    let bytecode: Bytes = Vec::from_hex(&args[1])
        .expect("Invalid bytecode hex")
        .into();
    let calldata: Bytes = Vec::from_hex(&args[2])
        .expect("Invalid calldata hex")
        .into();

    let env = Environment {
        origin: Address::from_low_u64_be(SENDER_ADDRESS),
        gas_limit: u64::MAX,
        base_fee_per_gas: U256::zero(),
        gas_price: U256::zero(),
        block_gas_limit: u64::MAX,
        ..Default::default()
    };

    let mut db = init_db(bytecode);

    let tx = Transaction::LegacyTransaction(LegacyTransaction {
        to: TxKind::Call(Address::from_low_u64_be(CONTRACT_ADDRESS)),
        data: calldata.clone(),
        ..Default::default()
    });

    let mut vm = VM::new(env, &mut db, &tx, LevmCallTracer::disabled());

    match vm.stateless_execute() {
        Ok(report) => {
            println!("{:?}", report);
            if report.is_success() {
                println!("Success");
            } else {
                println!("Revert");
            }
        }
        Err(e) => panic!("Error: {}", e),
    };
}
