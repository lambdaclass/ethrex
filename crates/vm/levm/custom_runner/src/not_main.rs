use bytes::Bytes;
use ethrex_common::{
    Address, H160, H256, U256,
    types::{LegacyTransaction, Transaction, TxKind},
};
use ethrex_levm::{Environment, db::gen_db::GeneralizedDatabase, tracing::LevmCallTracer, vm::VM};
use ethrex_storage::Store;
use ethrex_vm::DynVmDatabase;
use hex::FromHex;
use std::{collections::HashMap, env};

const DEFAULT_SENDER: H160 = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x01,
]);

const DEFAULT_CONTRACT: H160 = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x42,
]);

fn not_main() {
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
        origin: DEFAULT_SENDER,
        gas_limit: u64::MAX,
        base_fee_per_gas: U256::zero(),
        gas_price: U256::zero(),
        block_gas_limit: u64::MAX,
        ..Default::default()
    };

    // let mut db = init_db(bytecode);

    let tx = Transaction::LegacyTransaction(LegacyTransaction {
        to: TxKind::Call(DEFAULT_CONTRACT),
        data: calldata.clone(),
        ..Default::default()
    });

    let mut vm = VM::new(env, &mut db, &tx, LevmCallTracer::disabled(), L1);

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

fn init_db(bytecode: Bytes) -> GeneralizedDatabase {
    // The store type for this bench shouldn't matter as all operations use the LEVM cache
    let in_memory_db = Store::new("", ethrex_storage::EngineType::InMemory).unwrap();
    let store: DynVmDatabase = Box::new(DynVmDatabase::new(in_memory_db, H256::zero()));

    let cache = HashMap::from([
        (
            Address::from_low_u64_be(CONTRACT_ADDRESS),
            Account::new(U256::MAX, bytecode.clone(), 0, HashMap::new()),
        ),
        (
            Address::from_low_u64_be(SENDER_ADDRESS),
            Account::new(U256::MAX, Bytes::new(), 0, HashMap::new()),
        ),
    ]);

    GeneralizedDatabase::new(Arc::new(store), cache)
}
