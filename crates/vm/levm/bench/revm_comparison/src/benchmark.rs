use revm_comparison::{generate_calldata, load_contract_bytecode, run_with_levm, run_with_revm};

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
            eprintln!("{}", usage);
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
