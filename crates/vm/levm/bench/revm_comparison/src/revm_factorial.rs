use revm_comparison::{generate_calldata, run_with_revm, FACTORIAL_BYTECODE};
use std::env;

fn main() {
    let runs = env::args().nth(1).unwrap();
    let number_of_iterations: u64 = env::args()
        .nth(2)
        .expect("Arg not present")
        .parse()
        .expect("Could not parse");
    let calldata = generate_calldata("factorial", number_of_iterations);

    run_with_revm(FACTORIAL_BYTECODE, runs.parse().unwrap(), &calldata);
    // NOTE: for really big numbers the result is zero due to
    // one every two iterations involving an even number.
}
