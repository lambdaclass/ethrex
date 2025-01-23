use revm_comparison::{generate_calldata, load_contract_bytecode, parse_args, run_with_revm};

fn main() {
    let (runs, number_of_iterations) = parse_args();
    let bytecode = load_contract_bytecode("ManyHashes");
    let calldata = generate_calldata("Benchmark", number_of_iterations);

    run_with_revm(&bytecode, runs, &calldata);
}
