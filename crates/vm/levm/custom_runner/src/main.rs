use custom_runner::benchmark::{BenchAccount, ExecutionInput};
use ethrex_blockchain::vm::StoreVmDatabase;
use ethrex_common::{
    Address, H256, U256,
    types::{Account, LegacyTransaction},
};
use ethrex_levm::{
    EVMConfig, Environment,
    call_frame::Stack,
    db::gen_db::GeneralizedDatabase,
    tracing::LevmCallTracer,
    vm::{VM, VMType},
};
use ethrex_storage::Store;
use ethrex_vm::DynVmDatabase;
use std::{collections::HashMap, sync::Arc, u64};

fn main() {
    let json = r#"
    {
        env: {
            "gas_limit": "100",
            "origin": "0x0000000000000000000000000000000000000001",
            "gas_price": "0x1"
        },
        initial_memory: "0x239019031905",
        transaction: {
            "nonce": "0",
            "gas_limit": "21000",
            "value": "0x10000"
        },
        initial_stack: [
            "0x1",
            "0x2",
            "0x3"
        ],
        pre: {
            "0x0000000000000000000000000000000000000001": {
                "balance": "0x10000000000000000",
                "code": "0x",
                "storage": {
                    "0x1": "0x2",
                    "0x3": "0x4"
                }
            }
        },
    }
    "#;

    //json5 because it is more flexible than normal json: trailing commas allowed, comments, unquoted keys, etc.
    let benchmark: ExecutionInput = json5::from_str(json).unwrap();
    println!("{:#?}", benchmark);

    let env = Environment {
        origin: benchmark.transaction.sender,
        gas_limit: benchmark.transaction.gas_limit,
        gas_price: benchmark.transaction.gas_price,
        block_gas_limit: u64::MAX,
        config: EVMConfig::new(benchmark.fork, EVMConfig::canonical_values(benchmark.fork)),
        block_number: U256::zero(),
        coinbase: Address::from_low_u64_be(50), // Using origin as coinbase for now
        timestamp: U256::zero(),
        prev_randao: None,
        difficulty: U256::zero(),
        chain_id: U256::from(1),
        base_fee_per_gas: U256::zero(),
        block_excess_blob_gas: None,
        block_blob_gas_used: None,
        tx_blob_hashes: Vec::new(),
        tx_max_priority_fee_per_gas: None,
        tx_max_fee_per_gas: None,
        tx_max_fee_per_blob_gas: None,
        tx_nonce: 0,
        is_privileged: false,
    };
    let in_memory_db = Store::new("", ethrex_storage::EngineType::InMemory).unwrap();
    let store: DynVmDatabase = Box::new(StoreVmDatabase::new(in_memory_db, H256::zero()));

    // Default state has sender with some balance to send Tx, it can be overwritten though.
    let mut initial_state = HashMap::from([
        (
            benchmark.transaction.sender,
            Account::from(BenchAccount::default()),
        ),
        // (DEFAULT_CONTRACT, Account::default()),
    ]);
    let benchmark_pre_state: HashMap<Address, Account> = benchmark
        .pre
        .iter()
        .map(|(addr, acc)| (addr.clone(), Account::from(acc.clone())))
        .collect();
    initial_state.extend(benchmark_pre_state);

    let mut db = GeneralizedDatabase::new(Arc::new(store), initial_state);

    let mut vm = VM::new(
        env,
        &mut db,
        &ethrex_common::types::Transaction::LegacyTransaction(LegacyTransaction::from(
            benchmark.transaction,
        )),
        LevmCallTracer::disabled(),
        VMType::L1,
    )
    .expect("Failed to initialize VM");

    vm.current_call_frame_mut().unwrap().stack = Stack::default();
    vm.current_call_frame_mut().unwrap().memory = Vec::new();

    let result = vm.execute();

    match result {
        Ok(report) => println!("Successful: {:?}", report),
        Err(e) => println!("Error: {}", e.to_string()),
    }
}
