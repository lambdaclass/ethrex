use revm_comparison::{generate_calldata, load_contract_bytecode, run_with_revm};
use std::env;

fn main() {
    let runs = env::args().nth(1).unwrap();
    let bytecode = load_contract_bytecode("ManyHashes");
    let calldata = generate_calldata("manyHashes", 20);

    run_with_revm(&bytecode, runs.parse().unwrap(), &calldata);
}
