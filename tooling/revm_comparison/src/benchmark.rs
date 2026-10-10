use ethrex_crypto::keccak::keccak_hash;
use revm_comparison::{levm_bench::run_with_levm, revm_bench::run_with_revm};
use std::{fs::File, io::Read};

enum VM {
    Revm,
    Levm,
}

const DEFAULT_REPETITIONS: u64 = 10;
const DEFAULT_ITERATIONS: u64 = 100;

fn main() {
    let usage = "usage: benchmark [revm/levm] [bench_name] (#repetitions) (#iterations)";

    let vm = std::env::args().nth(1).expect(usage);
    let vm = match vm.as_str() {
        "levm" => VM::Levm,
        "revm" => VM::Revm,
        _ => {
            eprintln!("{usage}");
            std::process::exit(1);
        }
    };

    let benchmark = std::env::args().nth(2).expect(usage);

    let runs: u64 = std::env::args()
        .nth(3)
        .unwrap_or_else(|| DEFAULT_REPETITIONS.to_string())
        .parse()
        .expect("Invalid number of repetitions: must be an integer");

    let number_of_iterations: u64 = std::env::args()
        .nth(4)
        .unwrap_or_else(|| DEFAULT_ITERATIONS.to_string())
        .parse()
        .expect("Invalid number of iterations: must be an integer");

    let bytecode = load_contract_bytecode(&benchmark);
    let calldata = generate_calldata("Benchmark", number_of_iterations);

    match vm {
        VM::Levm => run_with_levm(&bytecode, runs, &calldata),
        VM::Revm => run_with_revm(&bytecode, runs, &calldata),
    }
}

// Auxiliary functions for getting calldata and bytecode.

fn generate_calldata(function: &str, n: u64) -> String {
    let function_signature = format!("{function}(uint256)");
    let hash = keccak_hash(function_signature.as_bytes());
    let function_selector = &hash[..4];

    // Encode argument n (uint256, padded to 32 bytes)
    let mut encoded_n = [0u8; 32];
    encoded_n[24..].copy_from_slice(&n.to_be_bytes());

    // Combine the function selector and the encoded argument
    let calldata: Vec<u8> = function_selector
        .iter()
        .chain(encoded_n.iter())
        .copied()
        .collect();

    hex::encode(calldata)
}

fn load_contract_bytecode(bench_name: &str) -> String {
    let path = format!(
        "{}/contracts/bin/{bench_name}.bin-runtime",
        env!("CARGO_MANIFEST_DIR"),
    );

    println!("Loading bytecode from file {path}");

    let mut file = File::open(path).unwrap();
    let mut contents = String::new();
    file.read_to_string(&mut contents).unwrap();
    contents
}
