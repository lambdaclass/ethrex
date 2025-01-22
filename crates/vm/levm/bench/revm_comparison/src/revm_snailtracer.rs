use revm_comparison::{run_with_revm, SNAILTRACER_BYTECODE};
use std::env;

fn main() {
    let runs = env::args().nth(1).unwrap();

    run_with_revm(SNAILTRACER_BYTECODE, runs.parse().unwrap(), "30627b7c");
}
