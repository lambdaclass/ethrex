use guest_program::{execution::execution_program, input::JSONProgramInput};
use risc0_zkvm::guest::env;

fn main() {
    let input: JSONProgramInput = env::read();
    let output = execution_program(input.0).unwrap();

    env::commit_slice(&output.encode());
}
