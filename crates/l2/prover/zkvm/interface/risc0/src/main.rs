use risc0_zkvm::guest::env;
use zkvm_interface::{io::JSONProgramInput, execution::execution_program};

fn main() {
    let input: JSONProgramInput = env::read();
    let output = execution_program(input.0).unwrap();

    env::commit_slice(&output.encode());
}
