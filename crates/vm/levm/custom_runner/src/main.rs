use bytes::Bytes;
use custom_runner::benchmark::{BenchAccount, ExecutionInput};
use ethrex_blockchain::vm::StoreVmDatabase;
use ethrex_common::{
    Address, H256,
    types::{Account, LegacyTransaction, Transaction},
};
use ethrex_levm::{
    EVMConfig, Environment,
    db::gen_db::GeneralizedDatabase,
    tracing::LevmCallTracer,
    vm::{VM, VMType},
};
use ethrex_storage::Store;
use ethrex_vm::DynVmDatabase;
use std::{
    collections::HashMap,
    fs::{self, File},
    io::BufReader,
    sync::Arc,
    u64,
};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (input_file_path, bytecode_file_path) = if args.len() == 3 {
        (args[1].clone(), args[2].clone())
    } else {
        ("input.json".to_string(), "bytecode.txt".to_string())
    };

    let input_file = File::open(&input_file_path).expect("Failed to open input file");
    let reader = BufReader::new(input_file);
    let mut benchmark: ExecutionInput = serde_json::from_reader(reader).unwrap();

    let bytecode = fs::read_to_string(&bytecode_file_path)
        .expect("Failed to read bytecode file")
        .trim_start_matches("0x")
        .to_string();
    let bytecode: Bytes = hex::decode(bytecode.trim_end())
        .expect("Failed to decode hex string into bytes")
        .into();

    // Now we want to initialize the VM, so we set up the environment, database and transaction.

    let env = Environment {
        origin: benchmark.transaction.sender,
        gas_limit: benchmark.transaction.gas_limit,
        gas_price: benchmark.transaction.gas_price,
        block_gas_limit: u64::MAX,
        config: EVMConfig::new(benchmark.fork, EVMConfig::canonical_values(benchmark.fork)),
        coinbase: Address::from_low_u64_be(50), // Using origin as coinbase for now
        ..Default::default()
    };

    let initial_state = setup_initial_state(&mut benchmark, bytecode);

    let in_memory_db = Store::new("", ethrex_storage::EngineType::InMemory).unwrap();
    let store: DynVmDatabase = Box::new(StoreVmDatabase::new(in_memory_db, H256::zero()));
    let mut db = GeneralizedDatabase::new(Arc::new(store), initial_state);

    let mut vm = VM::new(
        env,
        &mut db,
        &Transaction::LegacyTransaction(LegacyTransaction::from(benchmark.transaction)),
        LevmCallTracer::disabled(),
        VMType::L1,
    )
    .expect("Failed to initialize VM");

    //TODO: See what to do with stack pool... Is it necessary to do sth with that?
    let stack = &mut vm.current_call_frame_mut().unwrap().stack;
    for elem in benchmark.initial_stack {
        stack.push(&[elem]).expect("Stack Overflow");
    }
    vm.current_call_frame_mut().unwrap().memory = benchmark.initial_memory.into();

    let result = vm.execute();

    match result {
        Ok(report) => println!("Successful: {:?}", report),
        Err(e) => println!("Error: {}", e.to_string()),
    }
}

/// ## Sets up the initial state
/// - Inserts sender account into state with some balance for sending the transaction
/// - Takes all accounts defined in the `pre` field of the json and inserts them in the state
/// - Assigns the code to the corresponding place:
///   - Call to a contract: Sets contract's code
///   - Create contract: Code becomes transaction calldata
fn setup_initial_state(
    benchmark: &mut ExecutionInput,
    bytecode: Bytes,
) -> HashMap<Address, Account> {
    // Default state has sender with some balance to send Tx, it can be overwritten though.
    let mut initial_state = HashMap::from([(
        benchmark.transaction.sender,
        Account::from(BenchAccount::default()),
    )]);
    let benchmark_pre_state: HashMap<Address, Account> = benchmark
        .pre
        .iter()
        .map(|(addr, acc)| (addr.clone(), Account::from(acc.clone())))
        .collect();
    initial_state.extend(benchmark_pre_state);
    // Contract bytecode or initcode
    if let Some(to) = benchmark.transaction.to {
        // Contract Bytecode, set code of recipient.
        let acc = initial_state.entry(to).or_default();
        acc.code = bytecode;
    } else {
        // Initcode should be data of transaction
        benchmark.transaction.data = bytecode;
    }

    initial_state
}
