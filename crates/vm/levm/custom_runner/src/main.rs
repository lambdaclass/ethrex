use bytes::Bytes;
use clap::Parser;
use custom_runner::benchmark::{BenchAccount, BenchTransaction, ExecutionInput};
use ethrex_blockchain::vm::StoreVmDatabase;
use ethrex_common::{
    Address, H160, H256, U256,
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

const COINBASE: H160 = H160([0x77; 20]);

#[derive(Parser)]
struct Cli {
    #[arg(long)]
    input: Option<String>,

    #[arg(long)]
    code: Option<String>,
}

fn main() {
    let cli = Cli::parse();

    if cli.input.is_none() && cli.code.is_none() {
        println!("Error: Either --input or --code must be provided.");
        return;
    }

    // Mutable just to assign the code to the transaction if necessary
    let mut benchmark: ExecutionInput = if let Some(input_file_path) = cli.input {
        let input_file = File::open(&input_file_path)
            .unwrap_or_else(|_| panic!("Input file '{}' not found", input_file_path));
        let reader = BufReader::new(input_file);
        serde_json::from_reader(reader).expect("Failed to parse input file")
    } else {
        ExecutionInput::default()
    };

    // Code can be implicitely provided in the input so we don't have this as requirement.
    let code: Option<Bytes> = if let Some(code_file_path) = cli.code {
        let code = fs::read_to_string(&code_file_path)
            .expect("Failed to read bytecode file")
            .trim_start_matches("0x")
            .to_string();
        Some(
            hex::decode(code.trim_end())
                .expect("Failed to decode hex string into bytes")
                .into(),
        )
    } else {
        None
    };

    // Now we want to initialize the VM, so we set up the environment and database.
    // Env
    let env = Environment {
        origin: benchmark.transaction.sender,
        gas_limit: benchmark.transaction.gas_limit,
        gas_price: benchmark.transaction.gas_price,
        block_gas_limit: u64::MAX,
        config: EVMConfig::new(benchmark.fork, EVMConfig::canonical_values(benchmark.fork)),
        coinbase: COINBASE,
        ..Default::default()
    };

    // DB
    let initial_state = setup_initial_state(&mut benchmark, code);
    let in_memory_db = Store::new("", ethrex_storage::EngineType::InMemory).unwrap();
    let store: DynVmDatabase = Box::new(StoreVmDatabase::new(in_memory_db, H256::zero()));
    let mut db = GeneralizedDatabase::new(Arc::new(store), initial_state);

    // Initialize VM
    let mut vm = VM::new(
        env,
        &mut db,
        &Transaction::LegacyTransaction(LegacyTransaction::from(benchmark.transaction.clone())),
        LevmCallTracer::disabled(),
        VMType::L1,
    )
    .expect("Failed to initialize VM");

    // Set initial stack and memory
    println!("Setting initial stack: {:?}", benchmark.initial_stack);
    let stack = &mut vm.current_call_frame_mut().unwrap().stack;
    for elem in benchmark.initial_stack {
        stack.push(&[elem]).expect("Stack Overflow");
    }
    println!("Setting initial memory: 0x{:x}", benchmark.initial_memory);
    vm.current_call_frame_mut().unwrap().memory = benchmark.initial_memory.into();

    // Execute Transaction
    let result = vm.execute();

    // Print execution result
    print!("\n\nResult:");
    match result {
        Ok(report) => println!(" {:?}\n", report),
        Err(e) => println!(" Error: {}\n", e.to_string()),
    }

    // Print final stack and memory
    let callframe = vm.pop_call_frame().unwrap();
    println!(
        "Final Stack (bottom to top): {:?}",
        &callframe.stack.values[callframe.stack.offset - 1..]
            .iter()
            .rev()
            .collect::<Vec<_>>()
    );
    println!("Final Memory: 0x{}", hex::encode(callframe.memory));

    // Print Accounts diff
    compare_initial_and_current_accounts(
        db.initial_accounts_state,
        db.current_accounts_state,
        &benchmark.transaction,
    );
}

/// Prints on screen difference between initial state and current one.
fn compare_initial_and_current_accounts(
    initial_accounts: HashMap<Address, Account>,
    current_accounts: HashMap<Address, Account>,
    transaction: &BenchTransaction,
) {
    println!("\nState Diff:");
    for (addr, acc) in current_accounts {
        if transaction.sender == addr {
            println!("\n Checking Sender Account: {:#x}", addr);
        } else if transaction.to.map_or(false, |to| to == addr) {
            println!("\n Checking Recipient Account: {:#x}", addr);
        } else if addr == COINBASE {
            println!("\n Checking Coinbase Account: {:#x}", addr);
        } else {
            println!("\n Checking Account: {:#x}", addr);
        };

        if let Some(prev) = initial_accounts.get(&addr) {
            if prev.info.balance != acc.info.balance {
                let balance_diff = acc.info.balance.abs_diff(prev.info.balance);
                let balance_diff_sign = if acc.info.balance >= prev.info.balance {
                    ""
                } else {
                    "-"
                };
                println!(
                    "    Balance changed: {} -> {} (Diff: {}{})",
                    prev.info.balance, acc.info.balance, balance_diff_sign, balance_diff
                );
            }

            if prev.info.nonce != acc.info.nonce {
                println!(
                    "    Nonce changed: {} -> {}",
                    prev.info.nonce, acc.info.nonce,
                );
            }

            if prev.code != acc.code {
                println!("    Code changed: {:?} -> {:?}", prev.code, acc.code);
            }

            for (slot, value) in &acc.storage {
                let default_value = U256::default();
                let prev_value = prev.storage.get(slot).unwrap_or(&default_value);
                if prev_value != value {
                    println!(
                        "    Storage slot {:?} changed: {:?} -> {:?}",
                        slot, prev_value, value
                    );
                }
            }
        }
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
    bytecode: Option<Bytes>,
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
    if let Some(code) = bytecode {
        if let Some(to) = benchmark.transaction.to {
            // Contract Bytecode, set code of recipient.
            let acc = initial_state.entry(to).or_default();
            acc.code = code;
        } else {
            // Initcode should be data of transaction
            benchmark.transaction.data = code;
        }
    }

    initial_state
}
