use bytes::Bytes;
use clap::Parser;
use custom_runner::benchmark::{BenchAccount, BenchTransaction, RunnerInput};
use ethrex_blockchain::vm::StoreVmDatabase;
use ethrex_common::{
    Address, H160, H256, U256,
    types::{Account, LegacyTransaction, Transaction},
};
use ethrex_levm::{
    EVMConfig, Environment,
    db::gen_db::GeneralizedDatabase,
    opcodes::Opcode,
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

    #[arg(long, short, action = clap::ArgAction::SetTrue)]
    verbose: bool,
}

fn main() {
    let cli = Cli::parse();

    if cli.input.is_none() && cli.code.is_none() {
        println!("Error: Either --input or --code must be provided.");
        return;
    }

    // Mutable just to assign the code to the transaction if necessary
    let mut runner_input: RunnerInput = if let Some(input_file_path) = cli.input {
        if cli.verbose {
            println!("Reading input file: {}", input_file_path);
        }
        let input_file = File::open(&input_file_path)
            .unwrap_or_else(|_| panic!("Input file '{}' not found", input_file_path));
        let reader = BufReader::new(input_file);
        serde_json::from_reader(reader).expect("Failed to parse input file")
    } else {
        if cli.verbose {
            println!("No input file provided, using default RunnerInput.");
        }
        RunnerInput::default()
    };

    let mnemonic: Vec<String> = if let Some(code_file_path) = cli.code {
        if cli.verbose {
            println!("Reading code file: {}", code_file_path);
        }
        fs::read_to_string(&code_file_path)
            .expect("Failed to read bytecode file")
            .split_ascii_whitespace()
            .map(String::from)
            .collect()
    } else {
        vec![]
    };

    let bytecode = mnemonic_to_bytecode(mnemonic, cli.verbose);

    if cli.verbose {
        println!("Final bytecode: 0x{}", hex::encode(bytecode.clone()));
    }

    // Now we want to initialize the VM, so we set up the environment and database.
    // Env
    let env = Environment {
        origin: runner_input.transaction.sender,
        gas_limit: runner_input.transaction.gas_limit,
        gas_price: runner_input.transaction.gas_price,
        block_gas_limit: u64::MAX,
        config: EVMConfig::new(
            runner_input.fork,
            EVMConfig::canonical_values(runner_input.fork),
        ),
        coinbase: COINBASE,
        ..Default::default()
    };

    // DB
    let initial_state = setup_initial_state(&mut runner_input, bytecode);
    let in_memory_db = Store::new("", ethrex_storage::EngineType::InMemory).unwrap();
    let store: DynVmDatabase = Box::new(StoreVmDatabase::new(in_memory_db, H256::zero()));
    let mut db = GeneralizedDatabase::new(Arc::new(store), initial_state);

    // Initialize VM
    let mut vm = VM::new(
        env,
        &mut db,
        &Transaction::LegacyTransaction(LegacyTransaction::from(runner_input.transaction.clone())),
        LevmCallTracer::disabled(),
        VMType::L1,
    )
    .expect("Failed to initialize VM");

    // Set initial stack and memory
    println!("Setting initial stack: {:?}", runner_input.initial_stack);
    let stack = &mut vm.current_call_frame_mut().unwrap().stack;
    for elem in runner_input.initial_stack {
        stack.push(&[elem]).expect("Stack Overflow");
    }
    println!(
        "Setting initial memory: 0x{:x}",
        runner_input.initial_memory
    );
    vm.current_call_frame_mut().unwrap().memory = runner_input.initial_memory.into();

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
        &callframe.stack.values[callframe.stack.offset..]
            .iter()
            .rev()
            .map(|value| format!("0x{:x}", value))
            .collect::<Vec<_>>()
    );
    println!("Final Memory: 0x{}", hex::encode(callframe.memory));

    // Print Accounts diff
    compare_initial_and_current_accounts(
        db.initial_accounts_state,
        db.current_accounts_state,
        &runner_input.transaction,
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
        // Instead of the if-else chain
        let account_label = match &addr {
            a if *a == transaction.sender => "Sender ",
            a if Some(*a) == transaction.to => "Recipient ",
            a if *a == COINBASE => "Coinbase ",
            _ => "",
        };
        println!("\n Checking {}Account: {:#x}", account_label, addr);

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
    runner_input: &mut RunnerInput,
    bytecode: Bytes,
) -> HashMap<Address, Account> {
    // Default state has sender with some balance to send Tx, it can be overwritten though.
    let mut initial_state = HashMap::from([(
        runner_input.transaction.sender,
        Account::from(BenchAccount::default()),
    )]);
    let benchmark_pre_state: HashMap<Address, Account> = runner_input
        .pre
        .iter()
        .map(|(addr, acc)| (addr.clone(), Account::from(acc.clone())))
        .collect();
    initial_state.extend(benchmark_pre_state);
    // Contract bytecode or initcode
    if bytecode != Bytes::new() {
        if let Some(to) = runner_input.transaction.to {
            // Contract Bytecode, set code of recipient.
            let acc = initial_state.entry(to).or_default();
            acc.code = bytecode;
        } else {
            // Initcode should be data of transaction
            runner_input.transaction.data = bytecode;
        }
    }

    initial_state
}

/// Parse mnemonics, converting them into bytecode.
fn mnemonic_to_bytecode(mnemonic: Vec<String>, verbose: bool) -> Bytes {
    let mut mnemonic_iter = mnemonic.into_iter();
    let mut bytecode: Vec<u8> = Vec::new();

    while let Some(symbol) = mnemonic_iter.next() {
        let opcode = serde_json::from_str::<Opcode>(&format!("\"{}\"", symbol))
            .expect(&format!("Failed to parse Opcode from symbol {symbol}"));

        bytecode.push(opcode.into());

        if (Opcode::PUSH1..=Opcode::PUSH32).contains(&opcode) {
            let push_size = (opcode as u8 - Opcode::PUSH1 as u8 + 1) as usize;
            let value = mnemonic_iter
                .next()
                .expect("Expected a value after PUSH opcode");
            let mut decoded_value = if value.starts_with("0x") {
                hex::decode(value.trim_start_matches("0x"))
                    .expect("Failed to decode PUSH value as hex")
            } else {
                let decimal_value: u64 = value
                    .parse()
                    .expect("Failed to parse PUSH value as decimal");
                let mut bytes = vec![];
                let mut temp = decimal_value;
                while temp > 0 {
                    bytes.push((temp & 0xFF) as u8);
                    temp >>= 8;
                }
                bytes.reverse();
                bytes
            };
            if decoded_value.len() < push_size {
                let padding = vec![0u8; push_size - decoded_value.len()];
                decoded_value = [padding, decoded_value].concat();
            }
            if verbose {
                println!("Parsed PUSH{} 0x{}", push_size, hex::encode(&decoded_value));
            }

            bytecode.append(&mut decoded_value);
        } else {
            if verbose {
                println!("Parsed {}", symbol);
            }
        }
    }

    bytecode.into()
}
