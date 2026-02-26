//! CLI entry point for the tokamak-debugger binary.

pub mod commands;
pub mod formatter;
pub mod repl;

use std::sync::Arc;

use bytes::Bytes;
use clap::{Parser, Subcommand};
use ethrex_common::{
    Address, U256,
    constants::EMPTY_TRIE_HASH,
    types::{Account, BlockHeader, Code, EIP1559Transaction, Transaction, TxKind},
};
use ethrex_levm::{Environment, db::gen_db::GeneralizedDatabase};
use rustc_hash::FxHashMap;

use crate::engine::ReplayEngine;
use crate::error::DebuggerError;
use crate::types::ReplayConfig;

/// Tokamak EVM time-travel debugger.
#[derive(Parser)]
#[command(name = "tokamak-debugger", about = "Tokamak EVM time-travel debugger")]
pub struct Args {
    #[command(subcommand)]
    pub command: InputMode,
}

/// Input mode for the debugger.
#[derive(Subcommand)]
pub enum InputMode {
    /// Debug raw EVM bytecode
    #[command(name = "bytecode")]
    Bytecode {
        /// Hex-encoded bytecode (with or without 0x prefix)
        #[arg(long)]
        code: String,

        /// Gas limit for execution
        #[arg(long, default_value = "9223372036854775806")]
        gas_limit: u64,
    },
}

/// Run the debugger CLI.
pub fn run(args: Args) -> Result<(), DebuggerError> {
    match args.command {
        InputMode::Bytecode { code, gas_limit } => run_bytecode(&code, gas_limit),
    }
}

const CONTRACT_ADDR: u64 = 0x42;
const SENDER_ADDR: u64 = 0x100;

fn run_bytecode(code_hex: &str, gas_limit: u64) -> Result<(), DebuggerError> {
    let hex_str = code_hex.strip_prefix("0x").unwrap_or(code_hex);
    let bytecode =
        hex::decode(hex_str).map_err(|e| DebuggerError::InvalidBytecode(e.to_string()))?;

    let contract_addr = Address::from_low_u64_be(CONTRACT_ADDR);
    let sender_addr = Address::from_low_u64_be(SENDER_ADDR);

    let mut db = make_cli_db(contract_addr, sender_addr, bytecode)?;
    let env = Environment {
        origin: sender_addr,
        gas_limit,
        block_gas_limit: gas_limit,
        ..Default::default()
    };
    let tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        to: TxKind::Call(contract_addr),
        data: Bytes::new(),
        ..Default::default()
    });

    let engine = ReplayEngine::record(&mut db, env, &tx, ReplayConfig::default())?;

    println!("Recorded {} steps. Starting debugger...\n", engine.len());

    repl::start(engine)
}

fn make_cli_db(
    contract_addr: Address,
    sender_addr: Address,
    bytecode: Vec<u8>,
) -> Result<GeneralizedDatabase, DebuggerError> {
    let store = ethrex_storage::Store::new("", ethrex_storage::EngineType::InMemory)
        .map_err(|e| DebuggerError::Cli(format!("Failed to create store: {e}")))?;
    let header = BlockHeader {
        state_root: *EMPTY_TRIE_HASH,
        ..Default::default()
    };
    let vm_db: ethrex_vm::DynVmDatabase = Box::new(
        ethrex_blockchain::vm::StoreVmDatabase::new(store, header)
            .map_err(|e| DebuggerError::Cli(format!("Failed to create VM database: {e}")))?,
    );

    let mut cache = FxHashMap::default();
    cache.insert(
        contract_addr,
        Account::new(
            U256::zero(),
            Code::from_bytecode(Bytes::from(bytecode)),
            0,
            FxHashMap::default(),
        ),
    );
    cache.insert(
        sender_addr,
        Account::new(
            U256::MAX,
            Code::from_bytecode(Bytes::new()),
            0,
            FxHashMap::default(),
        ),
    );

    Ok(GeneralizedDatabase::new_with_account_state(
        Arc::new(vm_db),
        cache,
    ))
}
