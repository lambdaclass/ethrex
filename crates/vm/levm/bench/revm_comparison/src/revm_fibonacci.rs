use revm_comparison::{generate_calldata, run_with_revm, FIBONACCI_BYTECODE};
use std::env;

fn main() {
    let runs = env::args().nth(1).expect("Runs not present");
    let number_of_iterations: u64 = env::args()
        .nth(2)
        .expect("Arg not present")
        .parse()
        .expect("Could not parse");
    let calldata = generate_calldata("fibonacci", number_of_iterations);

    run_with_revm(FIBONACCI_BYTECODE, runs.parse().unwrap(), &calldata);
}
