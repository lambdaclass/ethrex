use revm_comparison::{run_with_levm_calldata, SNAILTRACER_BYTECODE};
use std::env;

fn main() {
    let runs = env::args().nth(1).unwrap();
    let number_of_iterations = env::args().nth(2).unwrap();

    run_with_levm_calldata(SNAILTRACER_BYTECODE, runs.parse().unwrap(), "30627b7c");
}
