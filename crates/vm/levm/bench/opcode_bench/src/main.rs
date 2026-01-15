use bytes::Bytes;
use ethrex_blockchain::vm::StoreVmDatabase;
use ethrex_common::{
    Address, H256, U256,
    constants::EMPTY_TRIE_HASH,
    types::{Account, BlockHeader, Code, EIP1559Transaction, Transaction, TxKind},
};
use ethrex_levm::{
    Environment,
    db::gen_db::GeneralizedDatabase,
    errors::{TxResult, VMError},
    tracing::LevmCallTracer,
    vm::{VM, VMType},
};
use ethrex_storage::Store;
use ethrex_vm::DynVmDatabase;
use rustc_hash::FxHashMap;
use serde::Deserialize;
use std::{
    fs::{File, OpenOptions},
    hint::black_box,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};

const SENDER_ADDRESS: u64 = 0x100;
const CONTRACT_ADDRESS: u64 = 0x42;

const DEFAULT_REPETITIONS: u64 = 10;
const DEFAULT_ITERATIONS: u32 = 100_000;
const BASELINE_NAME: &str = "baseline_loop";

#[derive(Debug, Deserialize)]
struct Fixture {
    name: String,
    category: String,
    description: Option<String>,
    body_hex: String,
    calldata_hex: Option<String>,
    storage: Option<Vec<StorageSlot>>,
    repeat: Option<u32>,
    counter_offset: Option<u8>,
    env: Option<FixtureEnv>,
}

#[derive(Debug, Deserialize)]
struct StorageSlot {
    key: String,
    value: String,
}

#[derive(Debug, Deserialize)]
struct FixtureEnv {
    block_number: Option<String>,
    timestamp: Option<String>,
    coinbase: Option<String>,
    prev_randao: Option<String>,
    difficulty: Option<String>,
    chain_id: Option<String>,
    base_fee_per_gas: Option<String>,
    gas_price: Option<String>,
    gas_limit: Option<u64>,
    block_gas_limit: Option<u64>,
}

#[derive(Debug, Clone, Copy)]
struct BenchConfig {
    warmup_runs: u64,
    reset_db: bool,
}

fn main() {
    let _usage = "usage: opcode_bench [fixture|list|category:<name>] (#repetitions) (#iterations)";

    let fixture_arg = std::env::args().nth(1).unwrap_or_else(|| "list".to_string());
    if fixture_arg == "list" {
        list_fixtures().unwrap();
        return;
    }
    if let Some(category) = parse_category_arg(&fixture_arg) {
        let runs: u64 = std::env::args()
            .nth(2)
            .unwrap_or_else(|| DEFAULT_REPETITIONS.to_string())
            .parse()
            .expect("Invalid number of repetitions: must be an integer");
        assert!(runs > 0, "repetitions must be greater than zero");

        let iterations: u32 = std::env::args()
            .nth(3)
            .unwrap_or_else(|| DEFAULT_ITERATIONS.to_string())
            .parse()
            .expect("Invalid number of iterations: must be an integer");

        let mut fixtures = load_all_fixtures().unwrap();
        fixtures.retain(|fixture| fixture.category == category);
        fixtures.sort_by(|a, b| a.name.cmp(&b.name));
        if fixtures.is_empty() {
            eprintln!("No fixtures found for category: {category}");
            std::process::exit(1);
        }
        for fixture in fixtures {
            run_fixture_with_baseline(&fixture, runs, iterations);
        }
        return;
    }

    let runs: u64 = std::env::args()
        .nth(2)
        .unwrap_or_else(|| DEFAULT_REPETITIONS.to_string())
        .parse()
        .expect("Invalid number of repetitions: must be an integer");
    assert!(runs > 0, "repetitions must be greater than zero");

    let iterations: u32 = std::env::args()
        .nth(3)
        .unwrap_or_else(|| DEFAULT_ITERATIONS.to_string())
        .parse()
        .expect("Invalid number of iterations: must be an integer");

    let fixture = load_fixture(&fixture_arg).unwrap();
    run_fixture_with_baseline(&fixture, runs, iterations);
}

fn list_fixtures() -> Result<(), std::io::Error> {
    let fixtures_dir = fixtures_dir();
    let mut entries: Vec<_> = std::fs::read_dir(&fixtures_dir)?
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().extension().and_then(|ext| ext.to_str()) == Some("json"))
        .collect();
    entries.sort_by_key(|entry| entry.path());

    println!("Available fixtures:");
    for entry in entries {
        let path = entry.path();
        let fixture = load_fixture_from_path(&path)?;
        println!("- {} ({})", fixture.name, fixture.category);
    }
    Ok(())
}

fn load_fixture(name: &str) -> Result<Fixture, std::io::Error> {
    let path = fixtures_dir().join(format!("{name}.json"));
    load_fixture_from_path(&path)
}

fn load_all_fixtures() -> Result<Vec<Fixture>, std::io::Error> {
    let mut fixtures = Vec::new();
    for entry in std::fs::read_dir(fixtures_dir())? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        fixtures.push(load_fixture_from_path(&path)?);
    }
    Ok(fixtures)
}

fn load_fixture_from_path(path: &Path) -> Result<Fixture, std::io::Error> {
    let mut file = File::open(path)?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)?;
    let fixture: Fixture = serde_json::from_str(&contents).expect("Invalid fixture JSON");
    Ok(fixture)
}

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

fn parse_category_arg(arg: &str) -> Option<String> {
    arg.strip_prefix("category:")
        .or_else(|| arg.strip_prefix("cat:"))
        .map(str::to_string)
}

fn run_fixture_with_baseline(fixture: &Fixture, runs: u64, iterations: u32) {
    let baseline_ns = if fixture.name == BASELINE_NAME {
        None
    } else {
        let baseline = baseline_fixture_for(fixture);
        Some(run_fixture_internal(&baseline, runs, iterations, true))
    };

    let ns_per_iter = run_fixture_internal(fixture, runs, iterations, false);
    let repeat = fixture.repeat.unwrap_or(1).max(1) as f64;
    let ns_per_op = ns_per_iter / repeat;

    if fixture.name == BASELINE_NAME {
        write_csv_row(
            fixture,
            runs,
            iterations,
            ns_per_iter,
            ns_per_op,
            None,
            None,
            None,
        );
        return;
    }
    if let Some(baseline) = baseline_ns {
        let adjusted = if ns_per_iter > baseline {
            ns_per_iter - baseline
        } else {
            0.0
        };
        let adjusted_per_op = adjusted / repeat;
        println!("time: adjusted={:.2} ns/iter (baseline={:.2})", adjusted, baseline);
        write_csv_row(
            fixture,
            runs,
            iterations,
            ns_per_iter,
            ns_per_op,
            Some(baseline),
            Some(adjusted),
            Some(adjusted_per_op),
        );
    } else {
        write_csv_row(
            fixture,
            runs,
            iterations,
            ns_per_iter,
            ns_per_op,
            None,
            None,
            None,
        );
    }
}

fn run_fixture_internal(fixture: &Fixture, runs: u64, iterations: u32, silent: bool) -> f64 {
    if !silent {
        if let Some(desc) = &fixture.description {
            println!("Fixture: {} ({}) - {}", fixture.name, fixture.category, desc);
        } else {
            println!("Fixture: {} ({})", fixture.name, fixture.category);
        }
    }

    let body = hex::decode(&fixture.body_hex).expect("Invalid body_hex in fixture");
    let body = repeat_body(&body, fixture.repeat.unwrap_or(1));
    let counter_offset = fixture.counter_offset.unwrap_or(0x80);
    let bytecode = build_loop_bytecode(&body, black_box(iterations), counter_offset);
    let bytecode = black_box(bytecode);
    let calldata = fixture
        .calldata_hex
        .as_deref()
        .map(|hex| Bytes::from(hex::decode(hex).expect("Invalid calldata_hex")))
        .unwrap_or_else(Bytes::new);
    let calldata = black_box(calldata);

    let config = bench_config();

    let mut total_elapsed = std::time::Duration::from_secs(0);
    if config.reset_db {
        for _ in 0..config.warmup_runs {
            let mut db = init_db(bytecode.clone(), fixture);
            let mut vm = init_vm(&mut db, 0, calldata.clone(), fixture).unwrap();
            let tx_report = black_box(vm.stateless_execute().unwrap());
            assert!(tx_report.is_success());
        }

        let mut last_report = None;
        for _ in 0..runs {
            let mut db = init_db(bytecode.clone(), fixture);
            let mut vm = init_vm(&mut db, 0, calldata.clone(), fixture).unwrap();
            let start = Instant::now();
            let tx_report = black_box(vm.stateless_execute().unwrap());
            total_elapsed += start.elapsed();
            assert!(tx_report.is_success());
            last_report = Some(tx_report);
        }

        if let Some(tx_report) = last_report {
            if !silent {
                match tx_report.result {
                    TxResult::Success => {
                        println!("output: \t\t0x{}", hex::encode(tx_report.output));
                    }
                    TxResult::Revert(error) => panic!("Execution failed: {error:?}"),
                }
            }
        }
    } else {
        let mut db = init_db(bytecode, fixture);
        for _ in 0..config.warmup_runs {
            let mut vm = init_vm(&mut db, 0, calldata.clone(), fixture).unwrap();
            let tx_report = black_box(vm.stateless_execute().unwrap());
            assert!(tx_report.is_success());
        }

        let mut last_report = None;
        for _ in 0..runs {
            let mut vm = init_vm(&mut db, 0, calldata.clone(), fixture).unwrap();
            let start = Instant::now();
            let tx_report = black_box(vm.stateless_execute().unwrap());
            total_elapsed += start.elapsed();
            assert!(tx_report.is_success());
            last_report = Some(tx_report);
        }

        if let Some(tx_report) = last_report {
            if !silent {
                match tx_report.result {
                    TxResult::Success => {
                        println!("output: \t\t0x{}", hex::encode(tx_report.output));
                    }
                    TxResult::Revert(error) => panic!("Execution failed: {error:?}"),
                }
            }
        }
    }

    let total_iters = (runs as u128) * (iterations as u128);
    if total_iters > 0 {
        let ns_per_iter = total_elapsed.as_nanos() as f64 / total_iters as f64;
        if !silent {
            let repeat = fixture.repeat.unwrap_or(1).max(1) as f64;
            let ns_per_op = ns_per_iter / repeat;
            println!(
                "time: total={:?}, runs={}, iterations/run={}, avg={:.2} ns/iter, {:.2} ns/op",
                total_elapsed, runs, iterations, ns_per_iter, ns_per_op
            );
        }
        return ns_per_iter;
    }
    0.0
}

fn write_csv_row(
    fixture: &Fixture,
    runs: u64,
    iterations: u32,
    avg_ns_per_iter: f64,
    avg_ns_per_op: f64,
    baseline_ns: Option<f64>,
    adjusted_ns: Option<f64>,
    adjusted_ns_per_op: Option<f64>,
) {
    let path = match std::env::var("OPCODE_BENCH_CSV") {
        Ok(path) if !path.trim().is_empty() => PathBuf::from(path),
        _ => return,
    };

    let needs_header = std::fs::metadata(&path).map(|meta| meta.len() == 0).unwrap_or(true);
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .expect("Failed to open CSV file");

    if needs_header {
        writeln!(
            file,
            "fixture,category,runs,iterations,repeat,counter_offset,avg_ns_per_iter,avg_ns_per_op,baseline_ns,adjusted_ns,adjusted_ns_per_op"
        )
        .expect("Failed to write CSV header");
    }

    let repeat = fixture.repeat.unwrap_or(1);
    let counter_offset = fixture.counter_offset.unwrap_or(0x80);
    let baseline_str = baseline_ns
        .map(|v| format!("{v:.2}"))
        .unwrap_or_default();
    let adjusted_str = adjusted_ns
        .map(|v| format!("{v:.2}"))
        .unwrap_or_default();
    let adjusted_op_str = adjusted_ns_per_op
        .map(|v| format!("{v:.2}"))
        .unwrap_or_default();

    writeln!(
        file,
        "{},{},{},{},{},{},{:.2},{:.2},{},{},{}",
        fixture.name,
        fixture.category,
        runs,
        iterations,
        repeat,
        counter_offset,
        avg_ns_per_iter,
        avg_ns_per_op,
        baseline_str,
        adjusted_str,
        adjusted_op_str
    )
    .expect("Failed to write CSV row");
}

fn repeat_body(body: &[u8], repeat: u32) -> Bytes {
    let mut out = Vec::with_capacity(body.len() * repeat as usize);
    for _ in 0..repeat {
        out.extend_from_slice(body);
    }
    Bytes::from(out)
}

fn init_db(bytecode: Bytes, fixture: &Fixture) -> GeneralizedDatabase {
    let in_memory_db = Store::new("", ethrex_storage::EngineType::InMemory).unwrap();
    let header = BlockHeader {
        state_root: *EMPTY_TRIE_HASH,
        ..Default::default()
    };
    let store: DynVmDatabase = Box::new(StoreVmDatabase::new(in_memory_db, header).unwrap());

    let mut storage = FxHashMap::default();
    if let Some(slots) = &fixture.storage {
        for slot in slots {
            let key = parse_h256(&slot.key);
            let value = parse_u256(&slot.value);
            storage.insert(key, value);
        }
    }

    let mut cache = FxHashMap::default();
    cache.insert(
        Address::from_low_u64_be(CONTRACT_ADDRESS),
        Account::new(
            U256::MAX,
            Code::from_bytecode(bytecode.clone()),
            0,
            storage,
        ),
    );
    cache.insert(
        Address::from_low_u64_be(SENDER_ADDRESS),
        Account::new(
            U256::MAX,
            Code::from_bytecode(Bytes::new()),
            0,
            FxHashMap::default(),
        ),
    );

    GeneralizedDatabase::new_with_account_state(Arc::new(store), cache)
}

fn init_vm<'a>(
    db: &'a mut GeneralizedDatabase,
    nonce: u64,
    calldata: Bytes,
    fixture: &Fixture,
) -> Result<VM<'a>, VMError> {
    let mut env = Environment {
        origin: Address::from_low_u64_be(SENDER_ADDRESS),
        tx_nonce: nonce,
        gas_limit: (i64::MAX - 1) as u64,
        block_gas_limit: (i64::MAX - 1) as u64,
        ..Default::default()
    };
    if let Some(overrides) = &fixture.env {
        apply_env_overrides(&mut env, overrides);
    }

    let tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        to: TxKind::Call(Address::from_low_u64_be(CONTRACT_ADDRESS)),
        data: calldata,
        ..Default::default()
    });
    VM::new(env, db, &tx, LevmCallTracer::disabled(), VMType::L1)
}

fn parse_h256(value: &str) -> H256 {
    let bytes = decode_hex(value);
    assert!(bytes.len() == 32, "Expected 32-byte hex string");
    H256::from_slice(&bytes)
}

fn parse_u256(value: &str) -> U256 {
    let bytes = decode_hex(value);
    assert!(bytes.len() == 32, "Expected 32-byte hex string");
    U256::from_big_endian(&bytes)
}

fn parse_u256_scalar(value: &str) -> U256 {
    let trimmed = value.trim();
    if let Some(hex) = trimmed.strip_prefix("0x") {
        let bytes = hex::decode(hex).expect("Invalid hex value");
        assert!(bytes.len() <= 32, "Expected <= 32-byte hex string");
        let mut padded = [0u8; 32];
        padded[32 - bytes.len()..].copy_from_slice(&bytes);
        U256::from_big_endian(&padded)
    } else {
        U256::from_dec_str(trimmed).expect("Invalid decimal U256 value")
    }
}

fn parse_address(value: &str) -> Address {
    let bytes = decode_hex(value);
    assert!(bytes.len() == 20, "Expected 20-byte hex string");
    Address::from_slice(&bytes)
}

fn decode_hex(value: &str) -> Vec<u8> {
    let trimmed = value.trim();
    let trimmed = trimmed.strip_prefix("0x").unwrap_or(trimmed);
    hex::decode(trimmed).expect("Invalid hex value")
}

fn baseline_fixture_for(fixture: &Fixture) -> Fixture {
    Fixture {
        name: BASELINE_NAME.to_string(),
        category: "baseline".to_string(),
        description: Some("Loop overhead only (empty body)".to_string()),
        body_hex: String::new(),
        calldata_hex: None,
        storage: None,
        repeat: None,
        counter_offset: fixture.counter_offset,
        env: None,
    }
}

fn bench_config() -> BenchConfig {
    let warmup_runs = std::env::var("OPCODE_BENCH_WARMUP")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0);
    let reset_db = std::env::var("OPCODE_BENCH_RESET_DB")
        .ok()
        .map(|value| parse_env_bool(&value))
        .unwrap_or(false);
    BenchConfig {
        warmup_runs,
        reset_db,
    }
}

fn parse_env_bool(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "y"
    )
}

fn apply_env_overrides(env: &mut Environment, overrides: &FixtureEnv) {
    if let Some(value) = overrides.block_number.as_deref() {
        env.block_number = parse_u256_scalar(value);
    }
    if let Some(value) = overrides.timestamp.as_deref() {
        env.timestamp = parse_u256_scalar(value);
    }
    if let Some(value) = overrides.coinbase.as_deref() {
        env.coinbase = parse_address(value);
    }
    if let Some(value) = overrides.prev_randao.as_deref() {
        env.prev_randao = Some(parse_h256(value));
    }
    if let Some(value) = overrides.difficulty.as_deref() {
        env.difficulty = parse_u256_scalar(value);
    }
    if let Some(value) = overrides.chain_id.as_deref() {
        env.chain_id = parse_u256_scalar(value);
    }
    if let Some(value) = overrides.base_fee_per_gas.as_deref() {
        env.base_fee_per_gas = parse_u256_scalar(value);
    }
    if let Some(value) = overrides.gas_price.as_deref() {
        env.gas_price = parse_u256_scalar(value);
    }
    if let Some(value) = overrides.gas_limit {
        env.gas_limit = value;
    }
    if let Some(value) = overrides.block_gas_limit {
        env.block_gas_limit = value;
    }
}

fn build_loop_bytecode(body: &[u8], iterations: u32, counter_offset: u8) -> Bytes {
    let mut code = Vec::new();
    let mut patches: Vec<(usize, &'static str)> = Vec::new();
    let mut labels: FxHashMap<&'static str, usize> = FxHashMap::default();

    // Store loop counter in memory slot 0.
    emit_push4(&mut code, iterations);
    emit_push1(&mut code, counter_offset);
    emit_opcode(&mut code, 0x52); // MSTORE

    emit_label(&mut code, &mut labels, "loop_start");

    emit_push1(&mut code, counter_offset);
    emit_opcode(&mut code, 0x51); // MLOAD
    emit_opcode(&mut code, 0x80); // DUP1
    emit_opcode(&mut code, 0x15); // ISZERO
    emit_push2_label(&mut code, &mut patches, "loop_end");
    emit_opcode(&mut code, 0x57); // JUMPI

    emit_push1(&mut code, 0x01);
    emit_opcode(&mut code, 0x90); // SWAP1
    emit_opcode(&mut code, 0x03); // SUB
    emit_opcode(&mut code, 0x80); // DUP1
    emit_push1(&mut code, counter_offset);
    emit_opcode(&mut code, 0x52); // MSTORE
    emit_opcode(&mut code, 0x50); // POP

    code.extend_from_slice(body);

    emit_push2_label(&mut code, &mut patches, "loop_start");
    emit_opcode(&mut code, 0x56); // JUMP

    emit_label(&mut code, &mut labels, "loop_end");
    emit_opcode(&mut code, 0x00); // STOP

    apply_patches(&mut code, &patches, &labels);
    Bytes::from(code)
}

fn emit_label(code: &mut Vec<u8>, labels: &mut FxHashMap<&'static str, usize>, name: &'static str) {
    labels.insert(name, code.len());
    emit_opcode(code, 0x5b); // JUMPDEST
}

fn emit_push1(code: &mut Vec<u8>, value: u8) {
    emit_opcode(code, 0x60);
    code.push(value);
}

fn emit_push2_label(
    code: &mut Vec<u8>,
    patches: &mut Vec<(usize, &'static str)>,
    label: &'static str,
) {
    emit_opcode(code, 0x61);
    patches.push((code.len(), label));
    code.extend_from_slice(&[0u8; 2]);
}

fn emit_push4(code: &mut Vec<u8>, value: u32) {
    emit_opcode(code, 0x63);
    code.extend_from_slice(&value.to_be_bytes());
}

fn emit_opcode(code: &mut Vec<u8>, opcode: u8) {
    code.push(opcode);
}

fn apply_patches(
    code: &mut Vec<u8>,
    patches: &[(usize, &'static str)],
    labels: &FxHashMap<&'static str, usize>,
) {
    for (pos, label) in patches {
        let offset = *labels.get(label).expect("Unknown label");
        assert!(offset <= u16::MAX as usize, "Jump offset too large");
        let offset_bytes = (offset as u16).to_be_bytes();
        code[*pos] = offset_bytes[0];
        code[*pos + 1] = offset_bytes[1];
    }
}
