use revm_comparison::{run_with_revm, FACTORIAL_BYTECODE};
use std::env;

fn main() {
    let runs = env::args().nth(1).unwrap();
    let number_of_iterations = env::args().nth(2).unwrap();

    run_with_revm(
        FACTORIAL_BYTECODE,
        runs.parse().unwrap(),
        number_of_iterations.parse().unwrap(),
    );
    // NOTE: for really big numbers the result is zero due to
    // one every two iterations involving an even number.
}
