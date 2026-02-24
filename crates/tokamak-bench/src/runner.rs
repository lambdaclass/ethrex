use std::fs;
use std::hint::black_box;
use std::sync::Arc;
use std::time::Instant;

use bytes::Bytes;
use ethrex_blockchain::vm::StoreVmDatabase;
use ethrex_common::{
    Address, U256,
    constants::EMPTY_TRIE_HASH,
    types::{Account, BlockHeader, Code, EIP1559Transaction, Transaction, TxKind},
};
use ethrex_crypto::keccak::keccak_hash;
use ethrex_levm::{
    Environment,
    db::gen_db::GeneralizedDatabase,
    timings::OPCODE_TIMINGS,
    tracing::LevmCallTracer,
    vm::{VM, VMType},
};
use ethrex_storage::Store;
use ethrex_vm::DynVmDatabase;
use rustc_hash::FxHashMap;

use crate::types::{BenchResult, BenchSuite, OpcodeEntry};

pub(crate) const SENDER_ADDRESS: u64 = 0x100;
pub(crate) const CONTRACT_ADDRESS: u64 = 0x42;

/// Default scenarios matching the revm_comparison benchmark suite.
pub struct Scenario {
    pub name: &'static str,
    pub iterations: u64,
}

pub fn default_scenarios() -> Vec<Scenario> {
    vec![
        Scenario {
            name: "Fibonacci",
            iterations: 57,
        },
        Scenario {
            name: "FibonacciRecursive",
            iterations: 15,
        },
        Scenario {
            name: "Factorial",
            iterations: 57,
        },
        Scenario {
            name: "FactorialRecursive",
            iterations: 57,
        },
        Scenario {
            name: "Push",
            iterations: 0,
        },
        Scenario {
            name: "MstoreBench",
            iterations: 0,
        },
        Scenario {
            name: "SstoreBench_no_opt",
            iterations: 0,
        },
        Scenario {
            name: "ManyHashes",
            iterations: 57,
        },
        Scenario {
            name: "BubbleSort",
            iterations: 100,
        },
        Scenario {
            name: "ERC20Approval",
            iterations: 500,
        },
        Scenario {
            name: "ERC20Transfer",
            iterations: 500,
        },
        Scenario {
            name: "ERC20Mint",
            iterations: 500,
        },
    ]
}

/// Path to the compiled contract binaries directory.
fn contracts_bin_dir() -> String {
    format!(
        "{}/../vm/levm/bench/revm_comparison/contracts/bin",
        env!("CARGO_MANIFEST_DIR")
    )
}

pub(crate) fn load_contract_bytecode(name: &str) -> Result<String, String> {
    let path = format!("{}/{name}.bin-runtime", contracts_bin_dir());
    fs::read_to_string(&path).map_err(|e| format!("Failed to load {path}: {e}"))
}

pub(crate) fn generate_calldata(iterations: u64) -> Bytes {
    let hash = keccak_hash(b"Benchmark(uint256)");
    let selector = &hash[..4];

    let mut encoded_n = [0u8; 32];
    encoded_n[24..].copy_from_slice(&iterations.to_be_bytes());

    let calldata: Vec<u8> = selector.iter().chain(encoded_n.iter()).copied().collect();
    Bytes::from(calldata)
}

pub(crate) fn init_db(bytecode: Bytes) -> GeneralizedDatabase {
    let store = Store::new("", ethrex_storage::EngineType::InMemory)
        .expect("Failed to create in-memory store");
    let header = BlockHeader {
        state_root: *EMPTY_TRIE_HASH,
        ..Default::default()
    };
    let vm_db: DynVmDatabase =
        Box::new(StoreVmDatabase::new(store, header).expect("Failed to create StoreVmDatabase"));

    let mut cache = FxHashMap::default();
    cache.insert(
        Address::from_low_u64_be(CONTRACT_ADDRESS),
        Account::new(
            U256::MAX,
            Code::from_bytecode(bytecode),
            0,
            FxHashMap::default(),
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

    GeneralizedDatabase::new_with_account_state(Arc::new(vm_db), cache)
}

pub(crate) fn init_vm(db: &mut GeneralizedDatabase, calldata: Bytes) -> VM<'_> {
    let env = Environment {
        origin: Address::from_low_u64_be(SENDER_ADDRESS),
        tx_nonce: 0,
        gas_limit: (i64::MAX - 1) as u64,
        block_gas_limit: (i64::MAX - 1) as u64,
        ..Default::default()
    };

    let tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        to: TxKind::Call(Address::from_low_u64_be(CONTRACT_ADDRESS)),
        data: calldata,
        ..Default::default()
    });

    VM::new(env, db, &tx, LevmCallTracer::disabled(), VMType::L1).expect("Failed to create VM")
}

/// Run a single benchmark scenario and collect opcode timing data.
///
/// **Not thread-safe**: This function resets and reads the global `OPCODE_TIMINGS`
/// singleton. Concurrent calls will produce incorrect results.
pub fn run_scenario(name: &str, bytecode_hex: &str, runs: u64, iterations: u64) -> BenchResult {
    let bytecode = Bytes::from(hex::decode(bytecode_hex).expect("Invalid hex bytecode"));
    let calldata = generate_calldata(iterations);

    // Reset global timings
    OPCODE_TIMINGS
        .lock()
        .expect("OPCODE_TIMINGS poisoned")
        .reset();

    let start = Instant::now();
    for _ in 0..runs {
        let mut db = init_db(bytecode.clone());
        let mut vm = init_vm(&mut db, calldata.clone());
        let report = black_box(vm.stateless_execute().expect("VM execution failed"));
        assert!(
            report.is_success(),
            "VM execution reverted: {:?}",
            report.result
        );
    }
    let total_duration = start.elapsed();

    // Extract opcode timings
    let timings = OPCODE_TIMINGS.lock().expect("OPCODE_TIMINGS poisoned");
    let raw_totals = timings.raw_totals();
    let raw_counts = timings.raw_counts();

    let mut opcode_timings: Vec<OpcodeEntry> = raw_totals
        .iter()
        .filter_map(|(opcode, total)| {
            let count = raw_counts.get(opcode).copied().unwrap_or(0);
            if count == 0 {
                return None;
            }
            let total_ns = total.as_nanos();
            let avg_ns = total_ns / u128::from(count);
            Some(OpcodeEntry {
                opcode: format!("{opcode:?}"),
                avg_ns,
                total_ns,
                count,
            })
        })
        .collect();

    // Sort by total time descending
    opcode_timings.sort_by(|a, b| b.total_ns.cmp(&a.total_ns));

    BenchResult {
        scenario: name.to_string(),
        total_duration_ns: total_duration.as_nanos(),
        runs,
        opcode_timings,
    }
}

/// Run the full benchmark suite.
///
/// Scenarios are executed sequentially. Not thread-safe due to global `OPCODE_TIMINGS`.
pub fn run_suite(scenarios: &[Scenario], runs: u64, commit: &str) -> BenchSuite {
    let mut results = Vec::new();

    for scenario in scenarios {
        let bytecode = match load_contract_bytecode(scenario.name) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("Skipping {}: {e}", scenario.name);
                continue;
            }
        };

        eprintln!("Running {} ({} runs)...", scenario.name, runs);
        let result = run_scenario(scenario.name, &bytecode, runs, scenario.iterations);
        eprintln!(
            "  {} total: {:.3}ms",
            scenario.name,
            result.total_duration_ns as f64 / 1_000_000.0
        );
        results.push(result);
    }

    BenchSuite {
        timestamp: unix_timestamp_secs(),
        commit: commit.to_string(),
        results,
    }
}

fn unix_timestamp_secs() -> String {
    // Simple UTC timestamp without chrono dependency
    let duration = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}", duration.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_calldata() {
        let calldata = generate_calldata(100);
        // 4-byte selector + 32-byte uint256
        assert_eq!(calldata.len(), 36);
    }

    #[test]
    fn test_contracts_bin_dir() {
        let dir = contracts_bin_dir();
        assert!(dir.contains("revm_comparison/contracts/bin"));
    }
}
