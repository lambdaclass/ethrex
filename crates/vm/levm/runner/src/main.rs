use bytes::Bytes;
use clap::Parser;
use env_logger::Env;
use ethrex_blockchain::vm::StoreVmDatabase;
use ethrex_common::{
    Address, H160, H256, U256,
    types::{Account, LegacyTransaction, Transaction},
};
use ethrex_levm::{
    EVMConfig, Environment,
    account::LevmAccount,
    db::gen_db::GeneralizedDatabase,
    opcodes::Opcode,
    tracing::LevmCallTracer,
    vm::{VM, VMType},
};
use ethrex_storage::Store;
use ethrex_vm::DynVmDatabase;
use log::{debug, error, info};
use num_bigint::BigUint;
use num_traits::Num;
use runner::input::{InputAccount, InputTransaction, RunnerInput};
use std::{collections::BTreeMap, io::Write};
use std::{
    fs::{self, File},
    io::BufReader,
    sync::Arc,
};

const COINBASE: H160 = H160([0x77; 20]);

#[derive(Parser)]
struct Cli {
    #[arg(long, short, help = "Path to the input JSON file")]
    input: Option<String>,

    #[arg(long, short, help = "Path to the bytecode/mnemonics file to execute")]
    code: Option<String>,

    #[arg(long, short, action = clap::ArgAction::SetTrue, help = "Enable verbose logging")]
    verbose: bool,

    #[arg(
        long,
        short = 'b',
        help = "Converts mnemonics file into a bytecode file"
    )]
    emit_bytes: Option<String>,
}

fn main() {
    let cli = Cli::parse();

    let log_level = if cli.verbose { "debug" } else { "info" };
    env_logger::Builder::from_env(Env::default().default_filter_or(log_level))
        .format(|buf, record| writeln!(buf, "{}", record.args()))
        .init();

    // Subcommand for just converting mnemonics to bytecode without executing
    if let Some(mnemonics_path) = cli.emit_bytes {
        let file_content =
            fs::read_to_string(&mnemonics_path).expect("Failed to read bytecode file");

        let mnemonics = file_content
            .split_ascii_whitespace()
            .map(String::from)
            .collect();
        // We convert to Bytes and then to String just because of convenience
        // We could change the logic so that it is Mnemonic -> String -> Bytes (and here skip the last step) but it's unimportant IMO
        let bytecode = mnemonics_to_bytecode(mnemonics);
        let hex_string = format!("0x{}", hex::encode(&bytecode));

        let output_path = format!("{}_bytes.txt", mnemonics_path.trim_end_matches(".txt"));
        let mut output_file = File::create(&output_path).expect("Failed to create output file");
        output_file
            .write_all(hex_string.as_bytes())
            .expect("Failed to write bytecode to file");

        info!("Bytecode emitted to file: {output_path}");
        return;
    }

    // Parse input
    // Input is mutable just to assign bytecode to the transaction recipient if provided
    let mut runner_input: RunnerInput = if let Some(input_file_path) = cli.input {
        debug!("Reading input file: {input_file_path}");
        let input_file = File::open(&input_file_path)
            .unwrap_or_else(|_| panic!("Input file '{input_file_path}' not found"));
        let reader = BufReader::new(input_file);
        serde_json::from_reader(reader).expect("Failed to parse input file")
    } else {
        debug!("No input file provided, using default RunnerInput.");
        RunnerInput::default()
    };

    // Parse bytecode, either from raw bytecode or mnemonics.
    let bytecode: Bytes = if let Some(code_file_path) = cli.code {
        debug!("Reading file: {code_file_path}");
        let file_content =
            fs::read_to_string(&code_file_path).expect("Failed to read bytecode file");

        let strings: Vec<String> = file_content
            .split_ascii_whitespace()
            .map(String::from)
            .collect();
        // We interpret as bytecode if there's only one string and that string is not an opcode
        // Otherwise, if there are multiple strings or if the string is for example ADD we'll know it's mnemonics
        let bytecode = if strings.len() == 1 && strings[0].parse::<Opcode>().is_err() {
            debug!("Decoding raw bytecode");
            let code = strings[0].trim_start_matches("0x");
            Bytes::from(hex::decode(code).expect("Failed to decode hex string"))
        } else {
            debug!("Parsing mnemonics");
            mnemonics_to_bytecode(strings)
        };

        debug!("Final bytecode: 0x{}", hex::encode(bytecode.clone()));

        bytecode
    } else {
        debug!("No code file provided, using the bytecode set in the input pre-state.");
        // If bytecode is empty bytes then it won't be assigned to the contract during the setup
        Bytes::new()
    };

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
    let mut db = GeneralizedDatabase::new_with_account_state(Arc::new(store), initial_state);

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
    info!("Setting initial stack: {:?}", runner_input.initial_stack);
    let stack = &mut vm.current_call_frame.stack;
    for elem in runner_input.initial_stack {
        stack.push(&[elem]).expect("Stack Overflow");
    }
    info!(
        "Setting initial memory: 0x{:x}",
        runner_input.initial_memory
    );
    let _ = vm
        .current_call_frame
        .memory
        .store_data(0, &runner_input.initial_memory);

    // Execute Transaction
    let result = vm.execute();

    // Print execution result
    info!("\n\nResult:");
    match result {
        Ok(report) => info!(" {report:?}\n"),
        Err(e) => error!(" Error: {e}\n"),
    }

    // Print final stack and memory
    let callframe = vm.current_call_frame;
    info!(
        "Final Stack (bottom to top): {:?}",
        &callframe.stack.values[callframe.stack.offset..]
            .iter()
            .rev()
            .map(|value| format!("0x{value:x}"))
            .collect::<Vec<_>>()
    );
    let final_memory: Vec<u8> = callframe.memory.buffer.borrow()[0..callframe.memory.len].to_vec();
    info!("Final Memory: 0x{}", hex::encode(final_memory));

    // Print Accounts diff
    compare_initial_and_current_accounts(
        db.initial_accounts_state,
        db.current_accounts_state,
        &runner_input.transaction,
    );
}

/// Prints on screen difference between initial state and current one.
fn compare_initial_and_current_accounts(
    initial_accounts: BTreeMap<Address, LevmAccount>,
    current_accounts: BTreeMap<Address, LevmAccount>,
    transaction: &InputTransaction,
) {
    info!("\nState Diff:");
    for (addr, acc) in current_accounts {
        // Instead of the if-else chain
        let acc_type = match &addr {
            a if *a == transaction.sender => "Sender ",
            a if Some(*a) == transaction.to => "Recipient ",
            a if *a == COINBASE => "Coinbase ",
            _ => "",
        };
        info!("\n Checking {acc_type}Account: {addr:#x}");

        if let Some(prev) = initial_accounts.get(&addr) {
            if prev.info.balance != acc.info.balance {
                let balance_diff = acc.info.balance.abs_diff(prev.info.balance);
                let balance_diff_sign = if acc.info.balance >= prev.info.balance {
                    ""
                } else {
                    "-"
                };
                info!(
                    "    Balance changed: {} -> {} (Diff: {}{})",
                    prev.info.balance, acc.info.balance, balance_diff_sign, balance_diff
                );
            }

            if prev.info.nonce != acc.info.nonce {
                info!(
                    "    Nonce changed: {} -> {}",
                    prev.info.nonce, acc.info.nonce,
                );
            }

            if prev.info.code_hash != acc.info.code_hash {
                info!(
                    "    Code hash changed: {:?} -> {:?}",
                    prev.info.code_hash, acc.info.code_hash
                );
            }

            for (slot, value) in &acc.storage {
                let default_value = U256::default();
                let prev_value = prev.storage.get(slot).unwrap_or(&default_value);
                if prev_value != value {
                    info!("    Storage slot {slot:?} changed: {prev_value:?} -> {value:?}");
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
) -> BTreeMap<Address, Account> {
    // Default state has sender with some balance to send Tx, it can be overwritten though.
    let mut initial_state = BTreeMap::from([(
        runner_input.transaction.sender,
        Account::from(InputAccount::default()),
    )]);
    let input_pre_state: BTreeMap<Address, Account> = runner_input
        .pre
        .iter()
        .map(|(addr, acc)| (*addr, Account::from(acc.clone())))
        .collect();
    initial_state.extend(input_pre_state);
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
fn mnemonics_to_bytecode(mnemonics: Vec<String>) -> Bytes {
    let mut mnemonic_iter = mnemonics.into_iter();
    let mut bytecode: Vec<u8> = Vec::new();

    while let Some(symbol) = mnemonic_iter.next() {
        let opcode = symbol.parse::<Opcode>().expect("Invalid opcode");

        bytecode.push(opcode.into());

        if (Opcode::PUSH1..=Opcode::PUSH32).contains(&opcode) {
            let push_size = (opcode as u8 - Opcode::PUSH1 as u8 + 1) as usize;
            let value = mnemonic_iter
                .next()
                .expect("Expected a value after PUSH opcode");
            let mut decoded_value = {
                let s = value.trim_start_matches("0x");
                let radix = if s.len() != value.len() { 16 } else { 10 };
                BigUint::from_str_radix(s, radix)
                    .expect("Failed to parse PUSH value")
                    .to_bytes_be()
            };
            if decoded_value.len() > push_size {
                panic!(
                    "Value {value} exceeds the maximum size of {push_size} bytes for PUSH{push_size}"
                );
            }
            if decoded_value.len() < push_size {
                let padding = vec![0u8; push_size - decoded_value.len()];
                decoded_value = [padding, decoded_value].concat();
            }

            debug!("Parsed PUSH{} 0x{}", push_size, hex::encode(&decoded_value));

            bytecode.append(&mut decoded_value);
        } else {
            debug!("Parsed {symbol}");
        }
    }

    bytecode.into()
}
