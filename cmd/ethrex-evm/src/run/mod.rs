//! Execution body for the `run` subcommand.
//!
//! Executes raw EVM bytecode in a minimal pre-state, matching the CLI surface
//! and output of geth's `evm run` command for differential testing purposes.
//!
//! ## Gas accounting note
//!
//! geth's `evm run` uses `runtime.Call` which bypasses standard transaction
//! validation and calls the EVM interpreter directly with the provided gas
//! limit. LEVM always goes through full transaction processing, which deducts
//! intrinsic gas (21000 + calldata cost) before entering the interpreter.
//!
//! To match geth's visible gas values (the `gas` field in each trace step), we
//! add the intrinsic gas to the gas limit before execution and subtract it from
//! the reported `gasUsed` in the summary line.
//!
//! ### Known limitations of the compensation
//!
//! - **Fork sensitivity.** The compensation assumes the EIP-8037 state-gas
//!   reservoir is inactive (i.e. fork < Amsterdam). With `--ethrex-fork=Osaka`
//!   or later, the reservoir would pre-consume part of the gas remaining and
//!   the visible per-step `gas` would diverge from geth's. Default `Prague` is
//!   safe; cross-fork differential testing should stay under Amsterdam.
//! - **Refund interaction.** For bytecode that earns large SSTORE refunds
//!   AND uses less than `TX_BASE_COST` gas in opcodes, the EIP-7623 floor in
//!   `compute_actual_gas_used` clamps `gas_spent` to 21_000. The compensation
//!   then reports `evm_gas = 0` regardless of actual opcode consumption.
//!   Trivial bytecode (PUSH/ADD/STOP/REVERT) is unaffected.

use std::{collections::BTreeMap, io::Read, path::Path, time::Instant};

use ethrex_common::{
    Address, U256,
    types::{
        Account, AccountInfo, Code, EIP1559Transaction, Fork, Genesis, GenesisAccount, Transaction,
        TxKind,
    },
};
use ethrex_crypto::NativeCrypto;
use ethrex_evm::statetest::{error_map::vm_error_to_geth_string, state_root::build_generalized_db};
use ethrex_levm::{
    EVMConfig, Environment, gas_cost,
    tracing::{LevmCallTracer, LevmOpcodeTracer, OpcodeTracerConfig},
    utils::get_base_fee_per_blob_gas,
    vm::{VM, VMType},
};
use ethrex_storage::{EngineType, Store};
use rustc_hash::FxHashMap;

use crate::RunArgs;

/// Default gas limit matching geth's `GasFlag` default of `10_000_000_000`.
const DEFAULT_GAS: u64 = 10_000_000_000;

/// Base intrinsic gas cost for a non-create, non-create2 transaction (EIP-2).
const TX_BASE_COST: u64 = 21_000;

/// Default sender: `common.BytesToAddress([]byte("sender"))` in geth.
///
/// geth's `BytesToAddress` left-pads the input slice to 20 bytes.
/// "sender" = [0x73, 0x65, 0x6e, 0x64, 0x65, 0x72] (6 bytes), so:
/// 0x000000000000000000000000000073656e646572
const DEFAULT_SENDER_HEX: &str = "0x000000000000000000000000000073656e646572";

/// Default receiver: `common.BytesToAddress([]byte("receiver"))` in geth.
///
/// "receiver" = [0x72, 0x65, 0x63, 0x65, 0x69, 0x76, 0x65, 0x72] (8 bytes), so:
/// 0x0000000000000000000000007265636569766572
const DEFAULT_RECEIVER_HEX: &str = "0x0000000000000000000000007265636569766572";

/// Entry point for the `run` subcommand.
pub fn execute(args: RunArgs) -> eyre::Result<()> {
    // 1. Decode bytecode from the appropriate source.
    let bytecode = load_bytecode(&args)?;

    // 2. Resolve sender and receiver addresses.
    let sender: Address = match &args.sender {
        Some(s) => s
            .parse()
            .map_err(|e| eyre::eyre!("invalid --sender: {e}"))?,
        None => DEFAULT_SENDER_HEX
            .parse()
            .map_err(|e| eyre::eyre!("default sender parse: {e}"))?,
    };
    let receiver: Address = match &args.receiver {
        Some(r) => r
            .parse()
            .map_err(|e| eyre::eyre!("invalid --receiver: {e}"))?,
        None => DEFAULT_RECEIVER_HEX
            .parse()
            .map_err(|e| eyre::eyre!("default receiver parse: {e}"))?,
    };

    // 3. Decode calldata.
    let input_bytes = decode_calldata(&args)?;

    // 4. Compute intrinsic gas and effective gas limit.
    // LEVM deducts intrinsic gas before entering the interpreter; geth's
    // runtime.Call does not. To make the visible gas values match geth, we add
    // intrinsic gas to the user-supplied limit and subtract it from gas_used.
    let calldata_cost = gas_cost::tx_calldata(&bytes::Bytes::copy_from_slice(&input_bytes))
        .map_err(|e| eyre::eyre!("calldata gas: {e}"))?;
    let intrinsic_gas = TX_BASE_COST
        .checked_add(calldata_cost)
        .ok_or_else(|| eyre::eyre!("intrinsic gas overflow"))?;
    let user_gas = args.gas.unwrap_or(DEFAULT_GAS);
    let vm_gas_limit = user_gas.checked_add(intrinsic_gas).ok_or_else(|| {
        eyre::eyre!("--gas too large: {user_gas} + {intrinsic_gas} overflows u64")
    })?;

    // 5. Build minimal pre-state: sender + receiver.
    let pre_state = build_minimal_pre_state(sender, receiver, &bytecode);

    // 6. Build fork / EVMConfig.
    // Default: Prague (latest stable). `--ethrex-fork=NAME` overrides.
    let fork = match args.ethrex_fork.as_deref() {
        None => Fork::Prague,
        Some(name) => parse_fork(name)?,
    };
    let blob_schedule = EVMConfig::canonical_values(fork);
    let config = EVMConfig::new(fork, blob_schedule);

    let base_blob_fee_per_gas =
        get_base_fee_per_blob_gas(None, &config).map_err(|e| eyre::eyre!("base blob fee: {e}"))?;

    // 7. Build genesis and in-memory store.
    let genesis = build_genesis(&pre_state);
    let rt = tokio::runtime::Runtime::new()?;
    let (store, _block_hash) = rt.block_on(async {
        let mut store =
            Store::new("./temp", EngineType::InMemory).map_err(|e| eyre::eyre!("{e}"))?;
        store
            .add_initial_state(genesis.clone())
            .await
            .map_err(|e| eyre::eyre!("{e}"))?;
        let block_hash = genesis.get_block().hash();
        Ok::<_, eyre::Error>((store, block_hash))
    })?;

    let mut db = build_generalized_db(store, &genesis)?;

    // 8. Build the transaction. `--value` accepts hex (with `0x`) or decimal,
    // matching geth's `math.HexOrDecimal256`.
    let value = match &args.value {
        Some(v) => parse_hex_or_dec_u256(v)?,
        None => U256::zero(),
    };

    let tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        to: TxKind::Call(receiver),
        value,
        data: input_bytes.into(),
        gas_limit: vm_gas_limit,
        ..Default::default()
    });

    // 9. Build LEVM Environment.
    let levm_env = Environment {
        origin: sender,
        gas_limit: vm_gas_limit,
        config,
        block_number: 0,
        coinbase: Address::default(),
        timestamp: 0,
        prev_randao: None,
        difficulty: U256::zero(),
        slot_number: U256::zero(),
        chain_id: U256::from(1),
        base_fee_per_gas: U256::zero(),
        base_blob_fee_per_gas,
        gas_price: U256::zero(),
        block_excess_blob_gas: None,
        block_blob_gas_used: None,
        tx_blob_hashes: vec![],
        tx_max_priority_fee_per_gas: None,
        tx_max_fee_per_gas: None,
        tx_max_fee_per_blob_gas: None,
        tx_nonce: 0,
        block_gas_limit: vm_gas_limit,
        is_privileged: false,
        fee_token: None,
        disable_balance_check: false,
    };

    // 10. Build tracer.
    // When --json is set, enable streaming tracer with geth-compatible defaults:
    //   --nomemory      defaults to true  (memory disabled)
    //   --nostack       defaults to false (stack enabled)
    //   --noreturndata  defaults to true  (return data disabled)
    let mut tracer = if args.json {
        let cfg = OpcodeTracerConfig {
            // nostack=true means stack is suppressed; disable_stack is the direct mapping
            disable_stack: args.nostack,
            // nomemory=true means memory is disabled; enable_memory is the inverse
            enable_memory: !args.nomemory,
            disable_storage: false,
            // noreturndata=true means return data is disabled; enable_return_data is the inverse
            enable_return_data: !args.noreturndata,
            limit: 0,
        };
        LevmOpcodeTracer::streaming(cfg, Box::new(std::io::stderr()))
    } else {
        LevmOpcodeTracer::disabled()
    };

    // 11. Execute.
    let start = Instant::now();
    let call_tracer = LevmCallTracer::disabled();
    let exec_result = VM::new(
        levm_env,
        &mut db,
        &tx,
        call_tracer,
        VMType::L1,
        &NativeCrypto,
    )
    .map_err(|e| eyre::eyre!("VM init: {e}"))
    .map(|mut vm| {
        vm.opcode_tracer = std::mem::replace(&mut tracer, LevmOpcodeTracer::disabled());
        let result = vm.execute();
        tracer = std::mem::replace(&mut vm.opcode_tracer, LevmOpcodeTracer::disabled());
        result
    });
    let elapsed = start.elapsed();

    // 12. Extract output/gas/error from execution result.
    // Subtract the intrinsic gas offset to report EVM-only gas used (matching geth).
    let (output, gas_used, error_str) = match &exec_result {
        Ok(Ok(report)) => {
            let err = match &report.result {
                ethrex_levm::errors::TxResult::Revert(e) => Some(vm_error_to_geth_string(e)),
                ethrex_levm::errors::TxResult::Success => None,
            };
            let evm_gas = report.gas_spent.saturating_sub(intrinsic_gas);
            (report.output.to_vec(), evm_gas, err)
        }
        Ok(Err(vm_err)) => {
            let err_str = vm_error_to_geth_string(vm_err);
            (vec![], 0, Some(err_str))
        }
        Err(e) => {
            return Err(eyre::eyre!("VM setup: {e}"));
        }
    };

    // 13. Emit streaming summary when --json.
    if args.json {
        tracer.flush_summary(&output, gas_used, error_str.as_deref())?;
        if let Some(stream_err) = tracer.take_stream_error() {
            return Err(eyre::eyre!("stream write error: {stream_err}"));
        }
    }

    // 14. --statdump output (stderr, matches geth runner.go:362-368).
    // We don't have Go's memstats so allocations and allocated bytes are 0.
    if args.statdump {
        eprintln!(
            "EVM gas used:    {gas_used}\nexecution time:  {elapsed:?}\nallocations:     0\nallocated bytes: 0"
        );
    }

    // 15. Non-JSON output to stdout (matches geth runner.go:369-374).
    if !args.json {
        // Print output as hex to stdout.
        println!("0x{}", hex::encode(&output));
        // Print error to stderr if present (with leading space to match geth).
        if let Some(ref err) = error_str {
            eprintln!(" error: {err}");
        }
    }

    Ok(())
}

/// Loads bytecode from the appropriate source (priority order matches geth runner.go:245-267):
///   1. `--codefile -`       → read entire stdin
///   2. `--codefile <path>`  → read from file
///   3. positional bytecode argument
///   4. error (process exit 1) if none of the three is provided
fn load_bytecode(args: &RunArgs) -> eyre::Result<Vec<u8>> {
    let hexcode = match &args.codefile {
        Some(path) if path == Path::new("-") => {
            // Read from stdin.
            let mut buf = String::new();
            std::io::stdin()
                .read_to_string(&mut buf)
                .map_err(|e| eyre::eyre!("reading stdin: {e}"))?;
            buf
        }
        Some(path) => {
            // Read from file.
            std::fs::read_to_string(path)
                .map_err(|e| eyre::eyre!("reading codefile {}: {e}", path.display()))?
        }
        None => match &args.bytecode {
            Some(b) => b.clone(),
            None => {
                eprintln!("error: no bytecode provided (use positional arg or --codefile)");
                std::process::exit(1);
            }
        },
    };

    let trimmed = hexcode.trim();
    let reported_len = trimmed.len();
    let hexcode = trimmed.trim_start_matches("0x");
    if hexcode.len() % 2 != 0 {
        eprintln!("Invalid input length for hex data ({})", reported_len);
        std::process::exit(1);
    }

    hex::decode(hexcode).map_err(|e| eyre::eyre!("hex decode error: {e}"))
}

/// Decodes calldata from `--input` (hex string with optional `0x` prefix).
fn decode_calldata(args: &RunArgs) -> eyre::Result<Vec<u8>> {
    match &args.input {
        None => Ok(vec![]),
        Some(s) => {
            let s = s.trim().trim_start_matches("0x");
            hex::decode(s).map_err(|e| eyre::eyre!("invalid --input hex: {e}"))
        }
    }
}

/// Builds the minimal pre-state: sender (u128::MAX balance, EOA) + receiver (bytecode deployed).
fn build_minimal_pre_state(
    sender: Address,
    receiver: Address,
    bytecode: &[u8],
) -> FxHashMap<Address, Account> {
    let crypto = NativeCrypto;
    let mut map = FxHashMap::default();

    // Sender: huge balance so it can pay for gas and value; no code.
    map.insert(
        sender,
        Account {
            info: AccountInfo {
                balance: U256::from(u128::MAX),
                ..AccountInfo::default()
            },
            code: Code::default(),
            storage: FxHashMap::default(),
        },
    );

    // Receiver: the bytecode deployed as contract code.
    if !bytecode.is_empty() {
        let code = Code::from_bytecode(bytes::Bytes::copy_from_slice(bytecode), &crypto);
        let code_hash = code.hash;
        map.insert(
            receiver,
            Account {
                info: AccountInfo {
                    balance: U256::zero(),
                    nonce: 0,
                    code_hash,
                },
                code,
                storage: FxHashMap::default(),
            },
        );
    }

    map
}

/// Builds a minimal [`Genesis`] from the pre-state account map.
fn build_genesis(pre_state: &FxHashMap<Address, Account>) -> Genesis {
    let alloc: BTreeMap<Address, GenesisAccount> = pre_state
        .iter()
        .map(|(addr, account)| {
            let storage: BTreeMap<U256, U256> = account
                .storage
                .iter()
                .map(|(k, v)| (U256::from_big_endian(k.as_bytes()), *v))
                .collect();
            let ga = GenesisAccount {
                code: account.code.bytecode.clone(),
                storage,
                balance: account.info.balance,
                nonce: account.info.nonce,
            };
            (*addr, ga)
        })
        .collect();

    Genesis {
        alloc,
        gas_limit: 30_000_000,
        ..Default::default()
    }
}

/// Parses a Fork name (e.g. `"Prague"`, `"Cancun"`) into a [`Fork`] variant.
/// Matches geth's CamelCase fork naming.
fn parse_fork(name: &str) -> eyre::Result<Fork> {
    match name {
        "Frontier" => Ok(Fork::Frontier),
        "Homestead" => Ok(Fork::Homestead),
        "DaoFork" => Ok(Fork::DaoFork),
        "Tangerine" | "EIP150" => Ok(Fork::Tangerine),
        "SpuriousDragon" | "EIP158" => Ok(Fork::SpuriousDragon),
        "Byzantium" => Ok(Fork::Byzantium),
        "Constantinople" => Ok(Fork::Constantinople),
        "Petersburg" | "ConstantinopleFix" => Ok(Fork::Petersburg),
        "Istanbul" => Ok(Fork::Istanbul),
        "MuirGlacier" => Ok(Fork::MuirGlacier),
        "Berlin" => Ok(Fork::Berlin),
        "London" => Ok(Fork::London),
        "ArrowGlacier" => Ok(Fork::ArrowGlacier),
        "GrayGlacier" => Ok(Fork::GrayGlacier),
        "Merge" | "Paris" => Ok(Fork::Paris),
        "Shanghai" => Ok(Fork::Shanghai),
        "Cancun" => Ok(Fork::Cancun),
        "Prague" => Ok(Fork::Prague),
        "Osaka" => Ok(Fork::Osaka),
        other => Err(eyre::eyre!("unknown fork: {other}")),
    }
}

/// Parses a `U256` from a string in hex (`"0x..."`) or decimal form. Mirrors
/// geth's `math.HexOrDecimal256` so `--value 1000` and `--value 0x3e8` both
/// yield 1000 wei.
fn parse_hex_or_dec_u256(s: &str) -> eyre::Result<U256> {
    let s = s.trim();
    if let Some(hex_part) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        U256::from_str_radix(hex_part, 16).map_err(|e| eyre::eyre!("invalid hex U256 {s:?}: {e}"))
    } else {
        U256::from_dec_str(s).map_err(|e| eyre::eyre!("invalid decimal U256 {s:?}: {e}"))
    }
}
